//! Point clouds in the side panel: visibility, colouring, size, delete, and
//! building a surface by thinning to a grid.

use egui::RichText;
use egui_cad::chrome::{IconButton, segmented};
use egui_cad::{CadTheme, icons};
use survey_core::tin::Thin;
use survey_model::{CloudColor, CloudStyle, Document, PointCloudId};

pub enum CloudAction {
    Style(PointCloudId, CloudStyle),
    Delete(PointCloudId),
    /// Build a surface: cloud, cell size, method, max edge.
    Surface(PointCloudId, f64, Thin, Option<f64>),
}

pub struct CloudPanel {
    pub active: Option<PointCloudId>,
    cell: f64,
    thin: Thin,
    max_edge: f64,
}

impl Default for CloudPanel {
    fn default() -> Self {
        Self {
            active: None,
            cell: 1.0,
            thin: Thin::Lowest,
            max_edge: 0.0,
        }
    }
}

impl CloudPanel {
    pub fn show(&mut self, ui: &mut egui::Ui, doc: &Document) -> Option<CloudAction> {
        let theme = CadTheme::current(ui.ctx());
        let mut action = None;
        if self.active.is_some_and(|id| doc.cloud(id).is_none()) {
            self.active = None;
        }
        for (id, c) in doc.clouds() {
            ui.horizontal(|ui| {
                let eye = if c.style.visible {
                    icons::EYE
                } else {
                    icons::EYE_SLASH
                };
                if IconButton::new(eye, if c.style.visible { "Hide" } else { "Show" })
                    .size(20.0)
                    .show(ui)
                    .clicked()
                {
                    action = Some(CloudAction::Style(
                        id,
                        CloudStyle {
                            visible: !c.style.visible,
                            ..c.style
                        },
                    ));
                }
                let selected = self.active == Some(id);
                if ui
                    .selectable_label(selected, RichText::new(&c.name).strong())
                    .clicked()
                {
                    self.active = if selected { None } else { Some(id) };
                }
                ui.label(
                    RichText::new(format!("{} pts", c.len()))
                        .small()
                        .color(theme.text_secondary),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if IconButton::new(icons::TRASH, "Delete point cloud")
                        .size(20.0)
                        .show(ui)
                        .clicked()
                    {
                        action = Some(CloudAction::Delete(id));
                    }
                });
            });
        }
        let Some(id) = self.active else {
            return action;
        };
        let Some(c) = doc.cloud(id) else {
            return action;
        };
        let mut style = c.style;
        egui::Frame::new()
            .fill(theme.bg)
            .stroke(egui::Stroke::new(1.0, theme.border))
            .corner_radius(theme.radius)
            .inner_margin(6.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Colour");
                    segmented(
                        ui,
                        &mut style.color,
                        &[
                            (CloudColor::Rgb, "RGB"),
                            (CloudColor::Height, "Height"),
                            (CloudColor::Single, "Single"),
                        ],
                    );
                    if style.color == CloudColor::Single {
                        let mut rgb = [style.single.0[0], style.single.0[1], style.single.0[2]];
                        if ui.color_edit_button_srgb(&mut rgb).changed() {
                            style.single.0 = [rgb[0], rgb[1], rgb[2], 255];
                        }
                    }
                });
                if c.colors.is_none() && style.color == CloudColor::Rgb {
                    ui.label(
                        RichText::new("No colours in the file: shown by height.")
                            .small()
                            .color(theme.text_secondary),
                    );
                }
                ui.horizontal(|ui| {
                    ui.label("Point size");
                    ui.add(
                        egui::DragValue::new(&mut style.point_size)
                            .range(1.0..=12.0)
                            .speed(0.1)
                            .suffix(" pt"),
                    );
                });
                ui.separator();
                ui.label(
                    RichText::new("SURFACE FROM CLOUD")
                        .small()
                        .color(theme.text_secondary),
                );
                ui.horizontal(|ui| {
                    ui.label("Grid");
                    ui.add(
                        egui::DragValue::new(&mut self.cell)
                            .range(0.01..=1000.0)
                            .speed(0.05)
                            .suffix(" m"),
                    );
                    segmented(
                        ui,
                        &mut self.thin,
                        &[
                            (Thin::Lowest, "Lowest"),
                            (Thin::Mean, "Mean"),
                            (Thin::Highest, "Highest"),
                        ],
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Max edge");
                    ui.add(
                        egui::DragValue::new(&mut self.max_edge)
                            .range(0.0..=10_000.0)
                            .speed(0.5)
                            .custom_formatter(|v, _| {
                                if v <= 0.0 {
                                    "none".into()
                                } else {
                                    format!("{v:.1} m")
                                }
                            }),
                    );
                    if ui.button("Create surface").clicked() {
                        let max_edge = (self.max_edge > 0.0).then_some(self.max_edge);
                        action = Some(CloudAction::Surface(id, self.cell, self.thin, max_edge));
                    }
                });
            });
        if style != c.style {
            action = Some(CloudAction::Style(id, style));
        }
        action
    }
}
