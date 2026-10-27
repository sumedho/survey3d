//! Resection (free station): fix an instrument station from directions and/or
//! distances observed to known points.

use egui::RichText;
use egui_cad::chrome::{IconButton, segmented};
use egui_cad::{CadTheme, fonts, icons};
use survey_core::resection::{Resection, Sighting, Weights};
use survey_core::{Angle, Conventions};
use survey_model::{Command, Document};

use crate::compute::resection::{Observed, report, sightings, solve, station_command};

#[derive(Clone, Default)]
struct Row {
    target: String,
    direction: String,
    distance: String,
}

pub struct ResectionWindow {
    pub open: bool,
    station: String,
    rows: Vec<Row>,
    /// Standard deviations: directions in seconds, distances in millimetres.
    sigma_direction: f64,
    sigma_distance_mm: f64,
    solution: usize,
}

impl Default for ResectionWindow {
    fn default() -> Self {
        Self {
            open: false,
            station: String::new(),
            rows: vec![Row::default(); 3],
            sigma_direction: 5.0,
            sigma_distance_mm: 5.0,
            solution: 0,
        }
    }
}

pub enum ResectionAction {
    Create(Box<dyn Command>, String),
    Report(String, String),
}

impl ResectionWindow {
    /// The sightings with the names of their targets.
    fn sightings(&self, doc: &Document) -> Result<Vec<(String, Sighting)>, String> {
        let conv = doc.conventions();
        let blank = |s: &str| s.trim().is_empty();
        let mut observed = Vec::new();
        for (i, r) in self.rows.iter().enumerate() {
            if blank(&r.target) && blank(&r.direction) && blank(&r.distance) {
                continue;
            }
            let n = i + 1;
            let direction = (!blank(&r.direction))
                .then(|| conv.parse_angle(&r.direction))
                .transpose()
                .map_err(|e| format!("Row {n} direction: {e}"))?;
            let distance = (!blank(&r.distance))
                .then(|| conv.parse_length(&r.distance))
                .transpose()
                .map_err(|e| format!("Row {n} distance: {e}"))?;
            observed.push(Observed {
                point: r.target.trim().to_string(),
                direction,
                distance,
            });
        }
        sightings(doc, &observed)
    }

    fn weights(&self) -> Weights {
        Weights {
            direction: Angle::from_degrees(self.sigma_direction.max(0.01) / 3600.0),
            distance: self.sigma_distance_mm.max(0.01) / 1000.0,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, doc: &Document) -> Option<ResectionAction> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let conv = doc.conventions().clone();
        let mut action = None;
        let mut open = self.open;
        egui::Window::new("Resection")
            .open(&mut open)
            .default_size([620.0, 520.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    self.form(ui, &theme, doc);
                    ui.add_space(6.0);
                    ui.separator();
                    let result = self
                        .sightings(doc)
                        .and_then(|s| solve(&s, self.weights()).map(|r| (s, r)));
                    match result {
                        Ok((sightings, solutions)) => {
                            if solutions.len() > 1 {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("Two solutions").color(theme.text_secondary));
                                    self.solution = self.solution.min(solutions.len() - 1);
                                    segmented(
                                        ui,
                                        &mut self.solution,
                                        &[(0, "Solution 1"), (1, "Solution 2")],
                                    );
                                });
                            }
                            let r = &solutions[self.solution.min(solutions.len() - 1)];
                            results(ui, &theme, &conv, &sightings, r);
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                let name = self.station_name(doc);
                                let exists = doc.find_point_by_name(&name).is_some();
                                let create = ui
                                    .add_enabled(!exists, egui::Button::new(format!("Create station {name}")))
                                    .on_disabled_hover_text("A point with this name already exists");
                                if create.clicked()
                                    && let Ok((cs, summary)) = station_command(doc, &name, r, None)
                                {
                                    action = Some(ResectionAction::Create(Box::new(cs), summary));
                                }
                                if ui.button("Add to reports").clicked() {
                                    action = Some(ResectionAction::Report(
                                        format!("Resection {name}"),
                                        report(&conv, &name, &sightings, r),
                                    ));
                                }
                            });
                        }
                        Err(e) => {
                            ui.label(RichText::new(e).color(theme.text_secondary));
                        }
                    }
                });
            });
        self.open = open;
        action
    }

    fn station_name(&self, doc: &Document) -> String {
        if self.station.trim().is_empty() {
            doc.next_point_name()
        } else {
            self.station.trim().to_string()
        }
    }

    fn form(&mut self, ui: &mut egui::Ui, theme: &CadTheme, doc: &Document) {
        let field = |ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32| {
            ui.add_sized(
                [width, 22.0],
                egui::TextEdit::singleline(text)
                    .hint_text(hint)
                    .font(fonts::mono(12.0)),
            );
        };
        egui::Grid::new("resection_setup")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Station name");
                let next = doc.next_point_name();
                field(ui, &mut self.station, &next, 140.0);
                ui.end_row();
                ui.label("Std. deviations");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.sigma_direction)
                            .range(0.1..=600.0)
                            .speed(0.1),
                    );
                    ui.label(RichText::new("″ directions").small());
                    ui.add_space(8.0);
                    ui.add(
                        egui::DragValue::new(&mut self.sigma_distance_mm)
                            .range(0.1..=1000.0)
                            .speed(0.1),
                    );
                    ui.label(RichText::new("mm distances").small());
                });
                ui.end_row();
            });
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "Directions are horizontal circle readings; distances are horizontal. \
                 Use three directions, two distances, or any mix — more gives residuals.",
            )
            .small()
            .color(theme.text_secondary),
        );
        ui.add_space(4.0);
        let mut remove = None;
        egui::Grid::new("resection_rows")
            .num_columns(5)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                for h in ["#", "Known point", "Direction", "Distance", ""] {
                    ui.label(RichText::new(h).small().color(theme.text_secondary));
                }
                ui.end_row();
                for (i, r) in self.rows.iter_mut().enumerate() {
                    ui.label(RichText::new((i + 1).to_string()).color(theme.text_secondary));
                    field(ui, &mut r.target, "name", 100.0);
                    field(ui, &mut r.direction, "optional", 150.0);
                    field(ui, &mut r.distance, "optional", 100.0);
                    if IconButton::new(icons::X, "Remove").size(20.0).show(ui).clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = remove {
            self.rows.remove(i);
            if self.rows.is_empty() {
                self.rows.push(Row::default());
            }
        }
        if ui.button(format!("{} Add known point", icons::PLUS)).clicked() {
            self.rows.push(Row::default());
        }
    }
}

fn results(
    ui: &mut egui::Ui,
    theme: &CadTheme,
    conv: &Conventions,
    sightings: &[(String, Sighting)],
    r: &Resection,
) {
    let line = |ui: &mut egui::Ui, label: &str, value: String| {
        ui.horizontal(|ui| {
            ui.add_sized(
                [140.0, 16.0],
                egui::Label::new(RichText::new(label).color(theme.text_secondary)),
            );
            ui.label(RichText::new(value).font(fonts::mono(12.0)));
        });
    };
    line(ui, "Station", conv.format_coord(r.position.extend(0.0), false));
    if let Some(o) = r.orientation {
        line(ui, "Orientation", conv.format_angle(o));
    }
    line(ui, "Redundancy", r.redundancy.to_string());
    if let (Some(s), Some(s0)) = (r.sigma, r.sigma0) {
        let (ea, nb) = conv.horizontal_labels();
        line(
            ui,
            "Std. deviation",
            format!(
                "{ea} {}  {nb} {}",
                conv.format_length_with_unit(s.x),
                conv.format_length_with_unit(s.y)
            ),
        );
        line(ui, "σ₀ (unit weight)", format!("{s0:.2}"));
    }
    if r.redundancy > 0 {
        ui.add_space(4.0);
        egui::Grid::new("resection_residuals")
            .num_columns(3)
            .spacing([16.0, 3.0])
            .show(ui, |ui| {
                for h in ["Known point", "Direction residual", "Distance residual"] {
                    ui.label(RichText::new(h).small().color(theme.text_secondary));
                }
                ui.end_row();
                for ((name, _), (dir, dist)) in sightings.iter().zip(&r.residuals) {
                    ui.label(name);
                    ui.label(
                        RichText::new(
                            dir.map(|a| format!("{:+.1}″", a.degrees() * 3600.0))
                                .unwrap_or_default(),
                        )
                        .font(fonts::mono(12.0)),
                    );
                    ui.label(
                        RichText::new(dist.map(|d| format!("{:+.1} mm", d * 1000.0)).unwrap_or_default())
                            .font(fonts::mono(12.0)),
                    );
                    ui.end_row();
                }
            });
    }
}
