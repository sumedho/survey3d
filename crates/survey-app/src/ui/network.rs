//! Least-squares network adjustment of saved traverses and imported field
//! data: sources, a priori precision, control, results and error ellipses.

use std::collections::BTreeSet;

use egui::RichText;
use egui_cad::{CadTheme, fonts};
use glam::DVec3;
use survey_core::Angle;
use survey_core::adjustment::AdjustResult;
use survey_core::network::{Network, NetworkOptions, Setup, report};
use survey_model::{Command, Document};

use crate::compute::network::{all_sources, apply, free_by_default, run, setups, updates};

pub struct NetworkWindow {
    pub open: bool,
    /// Selected sources: "t:<traverse>" or "f:<field data>".
    sources: BTreeSet<String>,
    options: Option<NetworkOptions>,
    /// Known stations the user has freed (adjusted rather than held fixed).
    free: BTreeSet<String>,
    /// Stations seen in the selected sources: (name, known in the project).
    stations: Vec<(String, bool)>,
    pub result: Option<Result<(Network, AdjustResult), String>>,
    pub show_ellipses: bool,
    pub exaggeration: f64,
}

impl Default for NetworkWindow {
    fn default() -> Self {
        Self {
            open: false,
            sources: BTreeSet::new(),
            options: None,
            free: BTreeSet::new(),
            stations: Vec::new(),
            result: None,
            show_ellipses: true,
            exaggeration: 1000.0,
        }
    }
}

pub enum NetworkAction {
    Apply(Box<dyn Command>, String),
    Report(String, String),
}

impl NetworkWindow {
    pub fn open_with(&mut self, doc: &Document) {
        self.open = true;
        if self.sources.is_empty() {
            self.sources = all_sources(doc);
        }
    }

    /// Adjusts just this source next (after an import).
    pub fn select_only(&mut self, key: String) {
        self.sources.clear();
        self.sources.insert(key);
        self.stations.clear();
        self.result = None;
    }

    fn setups(&self, doc: &Document) -> Vec<Setup> {
        setups(doc, &self.sources)
    }

    fn run(&self, doc: &Document, options: &NetworkOptions) -> Result<(Network, AdjustResult), String> {
        run(doc, &self.setups(doc), &self.free, options)
    }

    fn refresh_stations(&mut self, doc: &Document) {
        let mut seen = BTreeSet::new();
        self.stations.clear();
        // Points the chosen raw data placed, or an earlier adjustment made, start free.
        let free = free_by_default(doc, &self.sources);
        for s in self.setups(doc) {
            for name in std::iter::once(s.station).chain(s.observations.into_iter().map(|o| o.target)) {
                if seen.insert(name.clone()) {
                    if free.contains(&name) {
                        self.free.insert(name.clone());
                    }
                    let known = doc.find_point_by_name(&name).is_some();
                    self.stations.push((name, known));
                }
            }
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, doc: &Document) -> Option<NetworkAction> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let mut options = *self.options.get_or_insert(doc.meta().network_options);
        let mut action = None;
        let mut open = self.open;
        egui::Window::new("Network adjustment")
            .open(&mut open)
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("SOURCES").small().color(theme.text_secondary));
                if doc.traverses().is_empty() && doc.field_data().is_empty() {
                    ui.label(
                        RichText::new(
                            "Save a traverse or import raw data (File → Import raw data) to adjust it.",
                        )
                        .color(theme.text_secondary),
                    );
                }
                let mut changed = false;
                let mut toggle = |ui: &mut egui::Ui, key: String, label: String| {
                    let mut on = self.sources.contains(&key);
                    if ui.checkbox(&mut on, label).changed() {
                        changed = true;
                        if on {
                            self.sources.insert(key);
                        } else {
                            self.sources.remove(&key);
                        }
                    }
                };
                for t in doc.traverses() {
                    toggle(
                        ui,
                        format!("t:{}", t.name),
                        format!("Traverse {} ({} legs)", t.name, t.legs.len()),
                    );
                }
                for f in doc.field_data() {
                    toggle(
                        ui,
                        format!("f:{}", f.name),
                        format!("{} — {} ({} setups)", f.name, f.format, f.setups.len()),
                    );
                }
                if changed || self.stations.is_empty() {
                    self.refresh_stations(doc);
                }

                ui.add_space(4.0);
                ui.label(
                    RichText::new("A PRIORI PRECISION")
                        .small()
                        .color(theme.text_secondary),
                );
                egui::Grid::new("net_opts")
                    .num_columns(2)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| {
                        let mut dir = options.direction_sigma.degrees() * 3600.0;
                        ui.label("Directions");
                        if ui
                            .add(
                                egui::DragValue::new(&mut dir)
                                    .speed(0.1)
                                    .range(0.1..=600.0)
                                    .suffix("\""),
                            )
                            .changed()
                        {
                            options.direction_sigma = Angle::from_degrees(dir / 3600.0);
                        }
                        ui.end_row();
                        let mut mm = options.distance_sigma * 1000.0;
                        ui.label("Distances");
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::DragValue::new(&mut mm)
                                        .speed(0.1)
                                        .range(0.01..=1000.0)
                                        .suffix(" mm"),
                                )
                                .changed()
                            {
                                options.distance_sigma = mm / 1000.0;
                            }
                            ui.label("+");
                            ui.add(
                                egui::DragValue::new(&mut options.distance_ppm)
                                    .speed(0.1)
                                    .range(0.0..=1000.0)
                                    .suffix(" ppm"),
                            );
                        });
                        ui.end_row();
                        let mut c = options.centring * 1000.0;
                        ui.label("Centring");
                        if ui
                            .add(
                                egui::DragValue::new(&mut c)
                                    .speed(0.1)
                                    .range(0.0..=100.0)
                                    .suffix(" mm"),
                            )
                            .changed()
                        {
                            options.centring = c / 1000.0;
                        }
                        ui.end_row();
                        ui.label("Heights");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut options.heights, "");
                            let mut h = options.height_sigma * 1000.0;
                            if ui
                                .add_enabled(
                                    options.heights,
                                    egui::DragValue::new(&mut h)
                                        .speed(0.1)
                                        .range(0.01..=1000.0)
                                        .suffix(" mm"),
                                )
                                .changed()
                            {
                                options.height_sigma = h / 1000.0;
                            }
                        });
                        ui.end_row();
                        ui.label("Scale factor");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut options.scale_factor)
                                    .speed(0.00001)
                                    .max_decimals(9)
                                    .range(0.9..=1.1),
                            );
                            // Combined factor at the mean of the known stations.
                            let pts: Vec<DVec3> = self
                                .stations
                                .iter()
                                .filter_map(|(n, _)| doc.find_point_by_name(n).map(|(_, p)| p.position))
                                .collect();
                            let factor = (!pts.is_empty())
                                .then(|| pts.iter().sum::<DVec3>() / pts.len() as f64)
                                .and_then(|c| doc.combined_factor_at(c));
                            if ui
                                .add_enabled(factor.is_some(), egui::Button::new("From project"))
                                .on_hover_text("Combined factor at the centre of the known stations")
                                .on_disabled_hover_text("Needs a projection (Settings → Coordinates)")
                                .clicked()
                                && let Some(k) = factor
                            {
                                options.scale_factor = k;
                            }
                        });
                        ui.end_row();
                    });

                ui.add_space(4.0);
                ui.label(
                    RichText::new("CONTROL (ticked stations are held fixed)")
                        .small()
                        .color(theme.text_secondary),
                );
                egui::ScrollArea::vertical()
                    .id_salt("net_stations")
                    .max_height(120.0)
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for (name, known) in &self.stations {
                                if *known {
                                    let mut fixed = !self.free.contains(name);
                                    if ui.checkbox(&mut fixed, name).changed() {
                                        if fixed {
                                            self.free.remove(name);
                                        } else {
                                            self.free.insert(name.clone());
                                        }
                                    }
                                } else {
                                    ui.label(
                                        RichText::new(format!("{name} (new)")).color(theme.text_secondary),
                                    );
                                }
                            }
                        });
                    });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.sources.is_empty(), egui::Button::new("Adjust"))
                        .clicked()
                    {
                        self.result = Some(self.run(doc, &options));
                    }
                    ui.checkbox(&mut self.show_ellipses, "Error ellipses ×");
                    ui.add(
                        egui::DragValue::new(&mut self.exaggeration)
                            .speed(10.0)
                            .range(1.0..=100_000.0),
                    );
                });
                let mut refresh = false;
                match &self.result {
                    Some(Ok((net, r))) => {
                        let (verdict, colour) = match r.chi_square {
                            Some(c) if c.pass => ("χ² test passed", theme.success),
                            Some(c) if c.value > c.upper => (
                                "χ² test failed: residuals larger than the a priori precision allows (σ₀ > 1) — check for blunders",
                                theme.error,
                            ),
                            Some(_) => (
                                "χ² test failed: a priori precision is pessimistic (σ₀ < 1) — tighten the standard deviations",
                                theme.warning,
                            ),
                            None => ("no redundancy", theme.text_secondary),
                        };
                        ui.label(
                            RichText::new(format!(
                                "σ₀ = {:.3}   dof {}   {} observations   {} iterations",
                                r.sigma0,
                                r.dof,
                                net.observations.len(),
                                r.iterations
                            ))
                            .font(fonts::mono(12.0)),
                        );
                        ui.label(RichText::new(verdict).color(colour));
                        let worst = r
                            .residuals
                            .iter()
                            .zip(&net.labels)
                            .filter_map(|(v, l)| v.standardised.map(|w| (w.abs(), l)))
                            .max_by(|a, b| a.0.total_cmp(&b.0));
                        if let Some((w, label)) = worst {
                            let text = format!("Largest standardised residual {w:.2}: {label}");
                            ui.label(RichText::new(text).color(if w > 3.0 {
                                theme.error
                            } else {
                                theme.text_secondary
                            }));
                        }
                        let worst_ellipse = r
                            .ellipses
                            .iter()
                            .zip(&net.stations)
                            .filter_map(|(e, s)| e.map(|e| (e.semi_major, &s.name)))
                            .max_by(|a, b| a.0.total_cmp(&b.0));
                        if let Some((a, name)) = worst_ellipse {
                            ui.label(format!("Largest 95% ellipse {:.1} mm at {name}", a * 1000.0));
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Report").clicked() {
                                action = Some(NetworkAction::Report(
                                    "Network adjustment".into(),
                                    report(net, r, &options, doc.conventions()),
                                ));
                            }
                            let updates = updates(net, r);
                            if ui
                                .add_enabled(
                                    updates > 0,
                                    egui::Button::new(format!("Update {updates} points")),
                                )
                                .on_hover_text("Create new stations and move adjusted ones (undoable)")
                                .clicked()
                            {
                                let (cs, summary) = apply(doc, net, r, &options);
                                action = Some(NetworkAction::Apply(Box::new(cs), summary));
                                // Points written by this adjustment stay free on the next run.
                                for &i in &net.new_stations {
                                    self.free.insert(net.stations[i].name.clone());
                                }
                                refresh = true;
                            }
                        });
                    }
                    Some(Err(e)) => {
                        ui.label(RichText::new(e).color(theme.error));
                    }
                    None => {}
                }
                // The control list is rebuilt next frame, once the points exist.
                if refresh {
                    self.stations.clear();
                }
            });
        self.options = Some(options);
        self.open = open;
        action
    }
}
