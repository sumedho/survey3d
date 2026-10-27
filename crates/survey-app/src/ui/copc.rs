//! COPC point clouds: set a data directory, convert LAS/LAZ to COPC (natively,
//! see `survey_io::copc_write`), and browse and load the `.copc.laz` files
//! found there. Native only — gated at the `mod` declaration.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Color32, RichText};
use egui_cad::icons;

/// What the window asks the app to do; the app owns the settings and session.
#[derive(Clone, Debug)]
pub enum CopcAction {
    /// Remember this folder as the data directory.
    SetDataDir(PathBuf),
    /// Load this `.copc.laz` into the project.
    Load(PathBuf),
}

/// A finished conversion, delivered from the worker thread.
struct Done {
    name: String,
    result: Result<PathBuf, String>,
}

/// A conversion running on a worker thread.
struct Job {
    name: String,
    /// Fraction done, as `f32` bits.
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
}

pub struct CopcWindow {
    pub open: bool,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    jobs: Vec<Job>,
    /// Recent status lines: `(is_error, text)`.
    messages: Vec<(bool, String)>,
    /// `.copc.laz` files found in the data directory.
    entries: Vec<PathBuf>,
    /// The directory `entries` was scanned from (`None` forces a rescan).
    scanned: Option<PathBuf>,
    /// A conversion waiting for the user to agree to replace its output:
    /// `(source, output)`.
    replace: Option<(PathBuf, PathBuf)>,
    /// The project's coordinate system (WKT), recorded in conversions of
    /// files that have none.
    pub project_wkt: Option<String>,
}

impl Default for CopcWindow {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            open: false,
            tx,
            rx,
            jobs: Vec::new(),
            messages: Vec::new(),
            entries: Vec::new(),
            scanned: None,
            replace: None,
            project_wkt: None,
        }
    }
}

impl CopcWindow {
    pub fn open(&mut self) {
        self.open = true;
        self.scanned = None; // rescan on next show
    }

    pub fn show(&mut self, ctx: &egui::Context, data_dir: Option<&Path>) -> Option<CopcAction> {
        // Collect finished conversions.
        while let Ok(done) = self.rx.try_recv() {
            self.jobs.retain(|j| j.name != done.name);
            match done.result {
                Ok(_) => self.messages.push((false, format!("Converted {}", done.name))),
                Err(e) => self.messages.push((true, format!("{}: {e}", done.name))),
            }
            self.scanned = None; // refresh the list
        }
        if !self.open {
            return None;
        }
        if !self.jobs.is_empty() {
            ctx.request_repaint();
        }
        if self.scanned.as_deref() != data_dir {
            self.rescan(data_dir);
        }

        let mut action = None;
        let mut open = self.open;
        egui::Window::new("Point clouds (COPC)")
            .open(&mut open)
            .resizable(true)
            .default_width(380.0)
            .show(ctx, |ui| action = self.contents(ui, data_dir));
        self.open = open;
        action
    }

    fn contents(&mut self, ui: &mut egui::Ui, data_dir: Option<&Path>) -> Option<CopcAction> {
        let mut action = None;

        ui.label(RichText::new("Data directory").strong());
        ui.horizontal(|ui| {
            let shown = data_dir.map_or_else(|| "(none set)".to_string(), |p| p.display().to_string());
            ui.label(RichText::new(shown).monospace());
        });
        if ui.button("Change…").clicked()
            && let Some(dir) = rfd::FileDialog::new().pick_folder()
        {
            action = Some(CopcAction::SetDataDir(dir));
        }

        ui.separator();

        ui.label(RichText::new("Convert").strong());
        ui.horizontal(|ui| {
            let enabled = data_dir.is_some();
            let convert = ui
                .add_enabled(enabled, egui::Button::new("Convert LAS/LAZ to COPC…"))
                .on_disabled_hover_text("Set a data directory first.");
            if convert.clicked()
                && let Some(dir) = data_dir
            {
                self.start_conversion(dir);
            }
        });
        if let Some((src, dst)) = self.replace.clone() {
            let name = file_name(&dst);
            ui.label(format!("{name} already exists. Replace it?"));
            ui.label(
                RichText::new("The existing file is kept until the new one is finished.")
                    .small()
                    .weak(),
            );
            ui.horizontal(|ui| {
                if ui.button("Replace").clicked() {
                    self.replace = None;
                    self.convert(src, dst);
                }
                if ui.button("Cancel").clicked() {
                    self.replace = None;
                }
            });
        }
        for job in &self.jobs {
            ui.horizontal(|ui| {
                let done = f32::from_bits(job.progress.load(Ordering::Relaxed));
                ui.add(egui::ProgressBar::new(done).desired_width(220.0).text(format!(
                    "{} {:.0}%",
                    job.name,
                    done * 100.0
                )));
                if ui.button("Cancel").clicked() {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            });
        }

        ui.separator();

        ui.horizontal(|ui| {
            ui.label(RichText::new("COPC files").strong());
            if ui.button("Refresh").clicked() {
                self.scanned = None;
            }
        });
        if data_dir.is_none() {
            ui.label("Set a data directory to list files.");
        } else if self.entries.is_empty() {
            ui.label("No .copc.laz files yet.");
        } else {
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                for path in &self.entries {
                    ui.horizontal(|ui| {
                        if ui
                            .add(egui::Button::new(icons::FILE_ARROW_UP).frame(false))
                            .on_hover_text("Load")
                            .clicked()
                            || ui.button("Load").clicked()
                        {
                            action = Some(CopcAction::Load(path.clone()));
                        }
                        ui.label(file_name(path));
                    });
                }
            });
        }

        if !self.messages.is_empty() {
            ui.separator();
            for (is_error, text) in self.messages.iter().rev().take(4) {
                let color = if *is_error {
                    Color32::from_rgb(210, 90, 90)
                } else {
                    Color32::from_rgb(110, 180, 110)
                };
                ui.label(RichText::new(text).color(color));
            }
        }

        action
    }

    /// Picks a source file and converts it into `dir`, asking first when that
    /// would replace a file there.
    fn start_conversion(&mut self, dir: &Path) {
        let Some(src) = rfd::FileDialog::new()
            .add_filter("LAS/LAZ point cloud", &["laz", "las"])
            .pick_file()
        else {
            return;
        };
        let stem = src
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "cloud".into());
        let dst = dir.join(format!("{stem}.copc.laz"));
        if dst.exists() {
            self.replace = Some((src, dst));
        } else {
            self.convert(src, dst);
        }
    }

    /// Converts `src` to `dst` on a worker thread.
    fn convert(&mut self, src: PathBuf, dst: PathBuf) {
        let name = file_name(&dst);
        if self.jobs.iter().any(|j| j.name == name) {
            return; // already converting it
        }
        let job = Job {
            name: name.clone(),
            progress: Arc::new(AtomicU32::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let (progress, cancel) = (job.progress.clone(), job.cancel.clone());
        self.jobs.push(job);
        let tx = self.tx.clone();
        let options = survey_io::copc_write::ConvertOptions {
            fallback_wkt: self.project_wkt.clone(),
            ..Default::default()
        };
        std::thread::spawn(move || {
            let result = survey_io::copc_write::convert_to_copc(&src, &dst, options, &mut |f| {
                progress.store(f.to_bits(), Ordering::Relaxed);
                !cancel.load(Ordering::Relaxed)
            })
            .map(|_| dst)
            .map_err(|e| e.to_string());
            let _ = tx.send(Done { name, result });
        });
    }

    fn rescan(&mut self, data_dir: Option<&Path>) {
        self.entries.clear();
        if let Some(dir) = data_dir
            && let Ok(read) = std::fs::read_dir(dir)
        {
            for entry in read.flatten() {
                let path = entry.path();
                let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase());
                if name.is_some_and(|n| n.ends_with(".copc.laz")) {
                    self.entries.push(path);
                }
            }
            self.entries.sort();
        }
        self.scanned = data_dir.map(Path::to_path_buf);
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}
