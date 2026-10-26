//! As-built comparison: the selected shots against design points, the
//! selected design lines or a design surface, with tolerances, a table of
//! differences, a report and labels on the drawing.

use egui::RichText;
use egui_cad::chrome::segmented;
use egui_cad::{CadTheme, fonts};
use survey_model::{Command, Document, Entity, EntityId, SurfaceId};

use crate::asbuilt::{Against, Comparison, compare, labels, report};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Points,
    Lines,
    Surface,
}

pub struct AsBuiltWindow {
    pub open: bool,
    mode: Mode,
    strip: String,
    nearest: bool,
    radius: f64,
    surface: Option<SurfaceId>,
    tolerance_h: f64,
    tolerance_v: f64,
    layer: String,
    result: Option<Result<Comparison, String>>,
}

impl Default for AsBuiltWindow {
    fn default() -> Self {
        Self {
            open: false,
            mode: Mode::Points,
            strip: String::new(),
            nearest: true,
            radius: 0.5,
            surface: None,
            tolerance_h: 0.02,
            tolerance_v: 0.02,
            layer: "AS-BUILT".into(),
            result: None,
        }
    }
}

pub enum AsBuiltAction {
    Report(String, String),
    Label(Box<dyn Command>, String),
}

impl AsBuiltWindow {
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        selection: &[EntityId],
    ) -> Option<AsBuiltAction> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let conv = doc.conventions().clone();
        let shots: Vec<EntityId> = selection
            .iter()
            .copied()
            .filter(|id| matches!(doc.entity(*id), Some(Entity::Point(_))))
            .collect();
        let lines: Vec<EntityId> = selection
            .iter()
            .copied()
            .filter(|id| matches!(doc.entity(*id), Some(Entity::Polyline(_))))
            .collect();
        if self.surface.is_none_or(|id| doc.surface(id).is_none()) {
            self.surface = doc.surfaces().next().map(|(id, _)| id);
        }
        let mut action = None;
        let mut open = self.open;
        egui::Window::new("As-built comparison")
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Shots: the {} selected points.{}",
                    shots.len(),
                    if self.mode == Mode::Lines {
                        format!(" Design lines: the {} selected polylines.", lines.len())
                    } else {
                        String::new()
                    }
                ));
                ui.horizontal(|ui| {
                    ui.label("Against");
                    segmented(
                        ui,
                        &mut self.mode,
                        &[(Mode::Points, "Design points"), (Mode::Lines, "Lines"), (Mode::Surface, "Surface")],
                    );
                });
                egui::Grid::new("asbuilt_opts").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                    match self.mode {
                        Mode::Points => {
                            ui.label("Shot names");
                            ui.horizontal(|ui| {
                                ui.label("match design names after removing");
                                ui.add_sized(
                                    [50.0, 20.0],
                                    egui::TextEdit::singleline(&mut self.strip)
                                        .hint_text("S")
                                        .font(fonts::mono(12.0)),
                                );
                            });
                            ui.end_row();
                            ui.label("Otherwise");
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut self.nearest, "the nearest design point within");
                                ui.add_enabled(
                                    self.nearest,
                                    egui::DragValue::new(&mut self.radius)
                                        .speed(0.01)
                                        .range(0.001..=100.0)
                                        .suffix(" m"),
                                );
                            });
                            ui.end_row();
                        }
                        Mode::Lines => {}
                        Mode::Surface => {
                            ui.label("Surface");
                            let name = self
                                .surface
                                .and_then(|id| doc.surface(id))
                                .map_or("(none)".to_string(), |s| s.name.clone());
                            egui::ComboBox::from_id_salt("asbuilt_surface")
                                .selected_text(name)
                                .show_ui(ui, |ui| {
                                    for (id, s) in doc.surfaces() {
                                        ui.selectable_value(&mut self.surface, Some(id), &s.name);
                                    }
                                });
                            ui.end_row();
                        }
                    }
                    ui.label("Tolerance");
                    ui.horizontal(|ui| {
                        if self.mode != Mode::Surface {
                            ui.label("plan");
                            ui.add(
                                egui::DragValue::new(&mut self.tolerance_h)
                                    .speed(0.001)
                                    .range(0.0..=10.0)
                                    .max_decimals(3)
                                    .suffix(" m"),
                            );
                        }
                        ui.label("height");
                        ui.add(
                            egui::DragValue::new(&mut self.tolerance_v)
                                .speed(0.001)
                                .range(0.0..=10.0)
                                .max_decimals(3)
                                .suffix(" m"),
                        );
                    });
                    ui.end_row();
                });
                if ui.button(RichText::new("Compare").strong()).clicked() {
                    let against = match self.mode {
                        Mode::Points => Some(Against::Points {
                            strip: self.strip.clone(),
                            radius: self.nearest.then_some(self.radius),
                        }),
                        Mode::Lines => Some(Against::Lines(lines.clone())),
                        Mode::Surface => self.surface.map(Against::Surface),
                    };
                    self.result = Some(match against {
                        Some(a) => compare(doc, &shots, &a, self.tolerance_h, self.tolerance_v),
                        None => Err("There is no surface to compare with".into()),
                    });
                }
                ui.separator();
                match &self.result {
                    None => {
                        ui.label(
                            RichText::new("Select the surveyed points (and, for lines, the design polylines), then Compare.")
                                .small()
                                .color(theme.text_secondary),
                        );
                    }
                    Some(Err(e)) => {
                        ui.label(RichText::new(e).color(theme.error));
                    }
                    Some(Ok(c)) => {
                        let colour = if c.over() > 0 { theme.warning } else { theme.success };
                        ui.label(RichText::new(c.summary()).color(colour));
                        let mono = |t: String| RichText::new(t).font(fonts::mono(12.0));
                        let signed = |v: f64| format!("{}{}", if v < 0.0 { "-" } else { "+" }, conv.format_length(v.abs()));
                        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                            egui::Grid::new("asbuilt_rows").num_columns(5).striped(true).spacing([12.0, 2.0]).show(
                                ui,
                                |ui| {
                                    let plan = if c.rows.first().is_some_and(|r| r.offset.is_some()) {
                                        "Offset"
                                    } else {
                                        "Plan"
                                    };
                                    for h in ["Shot", "Against", plan, "dZ", ""] {
                                        ui.label(RichText::new(h).small().color(theme.text_secondary));
                                    }
                                    ui.end_row();
                                    for r in &c.rows {
                                        ui.label(&r.name);
                                        ui.label(&r.reference);
                                        ui.label(mono(
                                            r.offset
                                                .map(signed)
                                                .or(r.delta.map(|d| conv.format_length(d.length())))
                                                .unwrap_or_else(|| "-".into()),
                                        ));
                                        ui.label(mono(r.dz.map(signed).unwrap_or_else(|| "-".into())));
                                        if r.over {
                                            ui.label(RichText::new("over").color(theme.warning));
                                        } else {
                                            ui.label("");
                                        }
                                        ui.end_row();
                                    }
                                },
                            );
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Add to reports").clicked() {
                                action = Some(AsBuiltAction::Report("As-built".into(), report(doc, c)));
                            }
                            if ui
                                .button("Label on drawing")
                                .on_hover_text("Text beside each shot with its differences")
                                .clicked()
                            {
                                let cmd = labels(doc, c, &self.layer);
                                let summary = format!("Labelled {} as-built shots on {}", cmd.len(), self.layer);
                                action = Some(AsBuiltAction::Label(Box::new(cmd), summary));
                            }
                            ui.label("on layer");
                            ui.add_sized([90.0, 20.0], egui::TextEdit::singleline(&mut self.layer));
                        });
                    }
                }
            });
        self.open = open;
        action
    }
}
