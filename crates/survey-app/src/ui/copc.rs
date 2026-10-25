//! COPC point clouds: set a data directory, convert LAS/LAZ to COPC (by
//! shelling out to PDAL), and browse and load the `.copc.laz` files found
//! there. Native only — gated at the `mod` declaration.

use std::path::{Path, PathBuf};
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

pub struct CopcWindow {
    pub open: bool,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    in_flight: usize,
    /// Recent status lines: `(is_error, text)`.
    messages: Vec<(bool, String)>,
    /// `.copc.laz` files found in the data directory.
    entries: Vec<PathBuf>,
    /// The directory `entries` was scanned from (`None` forces a rescan).
    scanned: Option<PathBuf>,
    /// Whether `pdal` is on the PATH (checked lazily).
    pdal: Option<bool>,
}

impl Default for CopcWindow {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            open: false,
            tx,
            rx,
            in_flight: 0,
            messages: Vec::new(),
            entries: Vec::new(),
            scanned: None,
            pdal: None,
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
            self.in_flight = self.in_flight.saturating_sub(1);
            match done.result {
                Ok(_) => self.messages.push((false, format!("Converted {}", done.name))),
                Err(e) => self.messages.push((true, format!("{}: {e}", done.name))),
            }
            self.scanned = None; // refresh the list
        }
        if !self.open {
            return None;
        }
        if self.in_flight > 0 {
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
        if self.pdal == Some(false) {
            ui.label(
                RichText::new("PDAL was not found on your PATH. Install PDAL to convert, or drop pre-made .copc.laz files in the data directory.")
                    .color(Color32::from_rgb(200, 140, 60)),
            );
        }
        if self.in_flight > 0 {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!(
                    "Converting {} file{}…",
                    self.in_flight,
                    if self.in_flight == 1 { "" } else { "s" }
                ));
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
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        ui.label(name);
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

    /// Picks a source file and spawns a PDAL conversion into `dir`.
    fn start_conversion(&mut self, dir: &Path) {
        if self.pdal.is_none() {
            self.pdal = Some(pdal_available());
        }
        if self.pdal != Some(true) {
            return; // the hint in the UI explains why
        }
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
        let name = format!("{stem}.copc.laz");
        let tx = self.tx.clone();
        self.in_flight += 1;
        std::thread::spawn(move || {
            let result = run_pdal(&src, &dst);
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

/// Whether `pdal` responds to `--version`.
fn pdal_available() -> bool {
    std::process::Command::new("pdal")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Runs `pdal translate <src> <dst> --writer writers.copc`. The writer must be
/// forced: PDAL picks it from the `.laz` extension and would otherwise write a
/// plain LAZ (no `copc` VLR, no octree) that this reader rejects. Returns the
/// output path or PDAL's error text.
fn run_pdal(src: &Path, dst: &Path) -> Result<PathBuf, String> {
    let output = std::process::Command::new("pdal")
        .arg("translate")
        .arg(src)
        .arg(dst)
        .arg("--writer")
        .arg("writers.copc")
        .output()
        .map_err(|e| format!("could not run pdal: {e}"))?;
    if output.status.success() {
        Ok(dst.to_path_buf())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        let err = err.trim();
        Err(if err.is_empty() {
            "PDAL conversion failed".into()
        } else {
            err.to_string()
        })
    }
}
