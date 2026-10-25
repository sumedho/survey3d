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
    /// Replace the cloud with one point per spacing cube.
    Thin(PointCloudId, f64),
}

pub struct CloudPanel {
    pub active: Option<PointCloudId>,
    cell: f64,
    thin: Thin,
    max_edge: f64,
    spacing: f64,
}

impl Default for CloudPanel {
    fn default() -> Self {
        Self {
            active: None,
            cell: 1.0,
            thin: Thin::Lowest,
            max_edge: 0.0,
            spacing: 0.05,
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
                // Delete and the count keep their room; a long file name is
                // cut to what is left (full name on hover) rather than
                // widening the panel past its edge.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if IconButton::new(icons::TRASH, "Delete point cloud")
                        .size(20.0)
                        .show(ui)
                        .clicked()
                    {
                        action = Some(CloudAction::Delete(id));
                    }
                    let count = if c.is_out_of_core() {
                        RichText::new(format!("{} pts ⛁", c.full_count()))
                            .small()
                            .color(theme.text_secondary)
                    } else {
                        RichText::new(format!("{} pts", c.len()))
                            .small()
                            .color(theme.text_secondary)
                    };
                    let label = ui.label(count);
                    if c.is_out_of_core() {
                        let resident = c.len();
                        label.on_hover_text(if resident == 0 {
                            "On disk (COPC). Not loaded — re-link from File → Point clouds (COPC)."
                                .to_string()
                        } else {
                            format!("On disk (COPC). Showing a preview of {resident} points.")
                        });
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        let selected = self.active == Some(id);
                        if ui
                            .add(
                                egui::Button::selectable(selected, RichText::new(&c.name).strong())
                                    .truncate(),
                            )
                            .on_hover_text(&c.name)
                            .clicked()
                        {
                            self.active = if selected { None } else { Some(id) };
                        }
                    });
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
                ui.horizontal_wrapped(|ui| {
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
                ui.horizontal(|ui| {
                    ui.label("Thin to");
                    ui.add(
                        egui::DragValue::new(&mut self.spacing)
                            .range(0.001..=100.0)
                            .speed(0.01)
                            .suffix(" m"),
                    );
                    if ui
                        .button("Thin")
                        .on_hover_text("Keep one point per cube of this size (undoable)")
                        .clicked()
                    {
                        action = Some(CloudAction::Thin(id, self.spacing));
                    }
                });
                ui.separator();
                ui.label(
                    RichText::new("SURFACE FROM CLOUD")
                        .small()
                        .color(theme.text_secondary),
                );
                ui.horizontal_wrapped(|ui| {
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
                ui.horizontal_wrapped(|ui| {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A long file name must not widen the side panel past its edge (it was
    /// clipped on the left).
    #[test]
    fn long_name_fits_a_narrow_panel() {
        let mut doc = Document::default();
        let id = doc.alloc_cloud_id();
        let cloud = survey_model::PointCloud::from_points(
            "MAPPERPLUS_FORE_SCRX_M300_80mAGL_8ms_SACLCO_SCARBOROUGH.copc",
            &[glam::DVec3::ZERO, glam::DVec3::ONE],
            None,
        );
        doc.set_cloud(id, Some(cloud));
        let mut panel = CloudPanel {
            active: Some(id),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut overflow = 0.0;
        // A few frames, so layout settles.
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(208.0, 800.0),
                )),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let available = ui.available_width();
                    let used = ui.scope(|ui| panel.show(ui, &doc)).response.rect.width();
                    overflow = used - available;
                });
            });
            out.textures_delta.clear();
        }
        assert!(overflow <= 0.5, "content overflows the panel by {overflow}px");
    }
}
