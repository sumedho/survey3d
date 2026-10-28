//! Images: georeferenced orthophotos under the drawing — add, show/hide,
//! opacity and height, and placing an image by control points with their
//! residuals.

use egui::RichText;
use egui_cad::chrome::{IconButton, segmented};
use egui_cad::{CadTheme, fonts, icons};
use survey_core::georef::FitKind;
use survey_model::{ImageRecord, Placement, RecordId};

use crate::imagery::ImageStatus;
use crate::session::Session;

pub enum ImagesAction {
    Add,
    Update(RecordId, ImageRecord),
    /// Refit after changing control points or the fit; the label for undo.
    Refit(RecordId, ImageRecord, String),
    Remove(RecordId),
    ZoomTo(RecordId),
    /// Start picking control points on this image.
    PickControl(String),
    SaveWorldFile(RecordId),
    Relink(RecordId),
    Report(RecordId),
}

#[derive(Default)]
pub struct ImagesWindow {
    pub open: bool,
    /// The image whose control points are shown.
    active: Option<RecordId>,
}

impl ImagesWindow {
    pub fn show(&mut self, ctx: &egui::Context, session: &Session) -> Option<ImagesAction> {
        if !self.open {
            return None;
        }
        let doc = &session.project.doc;
        let conv = doc.conventions().clone();
        let theme = CadTheme::current(ctx);
        let mut action = None;
        let mut open = self.open;
        egui::Window::new("Images")
            .open(&mut open)
            .default_width(640.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button(format!("{} Add image…", icons::PLUS)).clicked() {
                        action = Some(ImagesAction::Add);
                    }
                    ui.label(
                        RichText::new("GeoTIFF (any size), or JPEG/PNG with a world file; others are placed by control points.")
                            .small()
                            .color(theme.text_secondary),
                    );
                });
                ui.add_space(4.0);
                let list = doc.images();
                if list.is_empty() {
                    ui.label(RichText::new("No images yet.").color(theme.text_secondary));
                    return;
                }
                egui::Grid::new("images").num_columns(7).spacing([10.0, 4.0]).striped(true).show(ui, |ui| {
                    for h in ["", "Image", "Placed by", "Opacity", "Height", "", ""] {
                        ui.label(RichText::new(h).small().color(theme.text_secondary));
                    }
                    ui.end_row();
                    for r in list {
                        let id = r.id;
                        let mut visible = r.visible;
                        if ui.checkbox(&mut visible, "").on_hover_text("Show").changed() {
                            action = Some(ImagesAction::Update(id, ImageRecord { visible, ..r.record.clone() }));
                        }
                        ui.vertical(|ui| {
                            ui.label(&r.name);
                            let status = session.imagery.status(&r.path);
                            let (text, color) = match &status {
                                ImageStatus::Ready => (
                                    format!("{} × {} px, {} per px", r.width, r.height, conv.format_length(r.transform.pixel_size())),
                                    theme.text_secondary,
                                ),
                                ImageStatus::Building(f) => (format!("building tiles {:.0}%", f * 100.0), theme.accent),
                                ImageStatus::Failed(e) => (format!("could not read: {e}"), theme.error),
                                ImageStatus::Missing => (format!("file not found: {}", r.path.display()), theme.error),
                            };
                            ui.label(RichText::new(text).small().color(color));
                            if let ImageStatus::Building(f) = status {
                                ui.add(egui::ProgressBar::new(f).desired_width(160.0));
                            }
                            if status == ImageStatus::Missing && ui.small_button("Relink…").clicked() {
                                action = Some(ImagesAction::Relink(id));
                            }
                        });
                        ui.label(match r.placement {
                            Placement::File if r.epsg.is_some() => format!("GeoTIFF (EPSG:{})", r.epsg.unwrap_or_default()),
                            Placement::File => "file".to_string(),
                            Placement::ControlPoints => format!("{} control points", r.control_points.len()),
                            Placement::Rough => "not placed".to_string(),
                        });
                        let mut opacity = r.opacity;
                        if ui.add(egui::Slider::new(&mut opacity, 0.0..=1.0).show_value(false)).drag_stopped() {
                            action = Some(ImagesAction::Update(id, ImageRecord { opacity, ..r.record.clone() }));
                        }
                        let mut elevation = r.elevation;
                        if ui
                            .add(egui::DragValue::new(&mut elevation).speed(0.1).suffix(" m"))
                            .on_hover_text("Height the image lies at in 3D views")
                            .changed()
                        {
                            action = Some(ImagesAction::Update(id, ImageRecord { elevation, ..r.record.clone() }));
                        }
                        ui.horizontal(|ui| {
                            if IconButton::new(icons::CORNERS_OUT, "Zoom to image").size(20.0).show(ui).clicked() {
                                action = Some(ImagesAction::ZoomTo(id));
                            }
                            if ui.selectable_label(self.active == Some(id), "Control points").clicked() {
                                self.active = if self.active == Some(id) { None } else { Some(id) };
                            }
                        });
                        if IconButton::new(icons::X, "Remove image (the file stays)").size(20.0).show(ui).clicked() {
                            action = Some(ImagesAction::Remove(id));
                        }
                        ui.end_row();
                    }
                });

                let Some(r) = self.active.and_then(|id| list.iter().find(|r| r.id == id)) else {
                    self.active = None;
                    return;
                };
                let id = r.id;
                ui.separator();
                ui.label(RichText::new(format!("CONTROL POINTS — {}", r.name)).font(fonts::medium(11.0)).color(theme.text_secondary));
                ui.horizontal(|ui| {
                    let mut fit = r.fit;
                    if segmented(
                        ui,
                        &mut fit,
                        &[(FitKind::Helmert, "Helmert (2+)"), (FitKind::Affine, "Affine (3+)")],
                    ) {
                        let rec = ImageRecord { fit, ..r.record.clone() };
                        action = Some(ImagesAction::Refit(id, rec, format!("{} fit", fit.label())));
                    }
                    if ui
                        .button(format!("{} Pick points", icons::CROSSHAIR))
                        .on_hover_text("Click a feature on the image, then its true position (a point, a snap or typed E,N)")
                        .clicked()
                    {
                        action = Some(ImagesAction::PickControl(r.name.clone()));
                    }
                    if ui.button("Save world file").clicked() {
                        action = Some(ImagesAction::SaveWorldFile(id));
                    }
                    if ui.button("Report").clicked() {
                        action = Some(ImagesAction::Report(id));
                    }
                });
                let fitted = survey_core::georef::fit(&r.control_points, r.fit).ok();
                match &fitted {
                    Some(f) => {
                        ui.label(format!("RMS {} from {} points", conv.format_length_with_unit(f.rms), f.used));
                    }
                    None => {
                        ui.label(
                            RichText::new(format!(
                                "{} of {} points needed: pick pairs to place the image",
                                r.control_points.iter().filter(|p| p.enabled).count(),
                                r.fit.min_points()
                            ))
                            .color(theme.text_secondary),
                        );
                    }
                }
                if r.control_points.is_empty() {
                    return;
                }
                egui::Grid::new("image_control").num_columns(6).spacing([12.0, 3.0]).striped(true).show(ui, |ui| {
                    let (ea, nb) = conv.horizontal_labels();
                    for h in ["#", "Image (col, row)", ea, nb, "Residual", "Use"] {
                        ui.label(RichText::new(h).small().color(theme.text_secondary));
                    }
                    ui.end_row();
                    let mut changed: Option<ImageRecord> = None;
                    for (i, p) in r.control_points.iter().enumerate() {
                        ui.label(format!("{}", i + 1));
                        ui.label(RichText::new(format!("{:.1}, {:.1}", p.pixel[0], p.pixel[1])).font(fonts::mono(12.0)));
                        let (a, b) = conv.horizontal_values(glam::DVec2::from(p.world).extend(0.0));
                        ui.label(RichText::new(conv.format_length(a)).font(fonts::mono(12.0)));
                        ui.label(RichText::new(conv.format_length(b)).font(fonts::mono(12.0)));
                        let residual = fitted.as_ref().and_then(|f| f.residuals.get(i)).map(|v| v.length());
                        let worst = fitted
                            .as_ref()
                            .map(|f| f.rms > 0.0 && residual.is_some_and(|v| v > 3.0 * f.rms))
                            .unwrap_or(false);
                        ui.label(
                            RichText::new(residual.map(|v| conv.format_length(v)).unwrap_or_default())
                                .font(fonts::mono(12.0))
                                .color(if worst { theme.error } else { theme.text }),
                        );
                        ui.horizontal(|ui| {
                            let mut enabled = p.enabled;
                            if ui.checkbox(&mut enabled, "").on_hover_text("Use in the fit (else only checked)").changed() {
                                let mut rec = r.record.clone();
                                rec.control_points[i].enabled = enabled;
                                changed = Some(rec);
                            }
                            if IconButton::new(icons::X, "Delete point").size(18.0).show(ui).clicked() {
                                let mut rec = r.record.clone();
                                rec.control_points.remove(i);
                                changed = Some(rec);
                            }
                        });
                        ui.end_row();
                    }
                    if let Some(rec) = changed {
                        action = Some(ImagesAction::Refit(id, rec, "Control points".into()));
                    }
                });
            });
        self.open = open;
        action
    }
}
