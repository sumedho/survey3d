//! Volumes between two surfaces, or a surface and a level, within an optional
//! boundary; with a report and an optional cut/fill (difference) surface.

use egui::RichText;
use egui_cad::chrome::segmented;
use egui_cad::{CadTheme, fonts};
use survey_model::{Document, EntityId, Surface, SurfaceId};

use crate::compute::volumes::{Side as Datum, Volumes, cut_fill_surface, format_volume, volumes};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Surface,
    Level,
}

struct Side {
    kind: Kind,
    surface: Option<SurfaceId>,
    level: String,
}

impl Default for Side {
    fn default() -> Self {
        Self {
            kind: Kind::Surface,
            surface: None,
            level: String::new(),
        }
    }
}

#[derive(Default)]
pub struct VolumesWindow {
    pub open: bool,
    base: Side,
    top: Side,
    boundary: Option<EntityId>,
    /// Optional density (t/m³) for tonnes.
    density: String,
    result: Option<Result<Volumes, String>>,
}

pub enum VolumesAction {
    Report(String, String),
    /// A cut/fill surface (top − base).
    CreateSurface(Box<Surface>),
}

impl VolumesWindow {
    pub fn open(&mut self, doc: &Document, active: Option<SurfaceId>) {
        self.open = true;
        let ids: Vec<SurfaceId> = doc.surfaces().map(|(id, _)| id).collect();
        if self.base.surface.is_none_or(|id| doc.surface(id).is_none()) {
            self.base.surface = ids.first().copied();
        }
        if self.top.surface.is_none_or(|id| doc.surface(id).is_none()) {
            self.top.surface = active.or_else(|| ids.get(1).copied()).or(ids.first().copied());
        }
        // With a single surface, the natural question is its volume above a level.
        if ids.len() == 1 && self.base.kind == Kind::Surface && self.top.kind == Kind::Surface {
            self.base.kind = Kind::Level;
        }
    }

    fn datum(doc: &Document, side: &Side, label: &str) -> Result<Datum, String> {
        match side.kind {
            Kind::Surface => side
                .surface
                .filter(|id| doc.surface(*id).is_some())
                .map(Datum::Surface)
                .ok_or_else(|| format!("Choose the {} surface", label.to_lowercase())),
            Kind::Level => doc
                .conventions()
                .parse_length(&side.level)
                .map(Datum::Level)
                .map_err(|e| format!("{label} level: {e}")),
        }
    }

    fn density(&self) -> Option<f64> {
        self.density.trim().parse::<f64>().ok().filter(|d| *d > 0.0)
    }

    fn compute(&self, doc: &Document) -> Result<Volumes, String> {
        let base = Self::datum(doc, &self.base, "Base")?;
        let top = Self::datum(doc, &self.top, "Top")?;
        volumes(doc, base, top, self.boundary, self.density())
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        selected_closed: &[EntityId],
    ) -> Option<VolumesAction> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let conv = doc.conventions().clone();
        let mut action = None;
        let mut open = self.open;
        egui::Window::new("Volumes")
            .open(&mut open)
            .default_width(440.0)
            .show(ctx, |ui| {
                if doc.surfaces().next().is_none() {
                    ui.label(
                        RichText::new("Create a surface first (Surface → Create surface…).")
                            .color(theme.text_secondary),
                    );
                    return;
                }
                let mut changed = false;
                egui::Grid::new("volume_sides")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        for (label, side) in [("Base", &mut self.base), ("Top", &mut self.top)] {
                            ui.label(label);
                            ui.horizontal(|ui| {
                                changed |= segmented(
                                    ui,
                                    &mut side.kind,
                                    &[(Kind::Surface, "Surface"), (Kind::Level, "Level")],
                                );
                                match side.kind {
                                    Kind::Surface => {
                                        let text = side
                                            .surface
                                            .and_then(|id| doc.surface(id))
                                            .map_or("Choose…".to_string(), |s| s.name.clone());
                                        egui::ComboBox::from_id_salt(("volume_surface", label))
                                            .selected_text(text)
                                            .show_ui(ui, |ui| {
                                                for (id, s) in doc.surfaces() {
                                                    changed |= ui
                                                        .selectable_value(
                                                            &mut side.surface,
                                                            Some(id),
                                                            &s.name,
                                                        )
                                                        .changed();
                                                }
                                            });
                                    }
                                    Kind::Level => {
                                        changed |= ui
                                            .add_sized(
                                                [100.0, 22.0],
                                                egui::TextEdit::singleline(&mut side.level)
                                                    .hint_text("height")
                                                    .font(fonts::mono(12.0)),
                                            )
                                            .changed();
                                    }
                                }
                            });
                            ui.end_row();
                        }
                        ui.label("Boundary");
                        ui.horizontal(|ui| {
                            ui.label(if self.boundary.is_some() {
                                "selected polyline"
                            } else {
                                "whole overlap"
                            });
                            if ui
                                .add_enabled(selected_closed.len() == 1, egui::Button::new("Use selected"))
                                .on_disabled_hover_text("Select one closed polyline")
                                .clicked()
                            {
                                self.boundary = selected_closed.first().copied();
                                changed = true;
                            }
                            if self.boundary.is_some() && ui.button("Clear").clicked() {
                                self.boundary = None;
                                changed = true;
                            }
                        });
                        ui.end_row();
                    });
                ui.horizontal(|ui| {
                    ui.label("Density");
                    changed |= ui
                        .add_sized(
                            [70.0, 22.0],
                            egui::TextEdit::singleline(&mut self.density)
                                .hint_text("t/m³")
                                .font(fonts::mono(12.0)),
                        )
                        .changed();
                    ui.label(RichText::new("optional, for tonnes").small());
                });
                ui.label(
                    RichText::new("Fill is where the top is above the base; cut where it is below.")
                        .small()
                        .color(theme.text_secondary),
                );
                if changed {
                    self.result = None;
                }
                ui.add_space(4.0);
                if ui.button("Compute").clicked() {
                    self.result = Some(self.compute(doc));
                }
                ui.separator();
                match &self.result {
                    None => {}
                    Some(Err(e)) => {
                        ui.label(RichText::new(e).color(theme.error));
                    }
                    Some(Ok(v)) => {
                        let r = &v.result;
                        let vol = |x: f64| format_volume(&conv, x);
                        egui::Grid::new("volume_result")
                            .num_columns(3)
                            .spacing([16.0, 4.0])
                            .show(ui, |ui| {
                                for (label, v, a, color) in [
                                    ("Fill", r.fill, Some(r.fill_area), theme.error),
                                    (
                                        "Cut",
                                        r.cut,
                                        Some(r.cut_area),
                                        egui::Color32::from_rgb(90, 150, 235),
                                    ),
                                    ("Net (fill − cut)", r.net(), None, theme.text),
                                ] {
                                    ui.label(RichText::new(label).color(theme.text_secondary));
                                    ui.label(RichText::new(vol(v)).font(fonts::mono(13.0)).color(color));
                                    ui.label(
                                        RichText::new(a.map(|a| conv.format_area(a)).unwrap_or_default())
                                            .small()
                                            .color(theme.text_secondary),
                                    );
                                    ui.end_row();
                                }
                                if let Some(d) = self.density() {
                                    ui.label(
                                        RichText::new("Tonnes (fill / cut / net)")
                                            .color(theme.text_secondary),
                                    );
                                    ui.label(
                                        RichText::new(format!(
                                            "{:.1} / {:.1} / {:.1} t",
                                            r.fill * d,
                                            r.cut * d,
                                            r.net() * d
                                        ))
                                        .font(fonts::mono(12.0)),
                                    );
                                    ui.end_row();
                                }
                                ui.label(RichText::new("Area compared").color(theme.text_secondary));
                                ui.label(RichText::new(conv.format_area(r.area)).font(fonts::mono(12.0)));
                                ui.end_row();
                            });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui.button("Add to reports").clicked() {
                                action = Some(VolumesAction::Report("Volume".into(), v.report.clone()));
                            }
                            if ui
                                .button("Create cut/fill surface")
                                .on_hover_text("A surface of top − base, coloured blue (cut) to red (fill)")
                                .clicked()
                                && let (Ok(base), Ok(top)) = (
                                    Self::datum(doc, &self.base, "Base"),
                                    Self::datum(doc, &self.top, "Top"),
                                )
                            {
                                action = Some(VolumesAction::CreateSurface(Box::new(cut_fill_surface(
                                    doc,
                                    base,
                                    top,
                                    v.difference.clone(),
                                ))));
                            }
                        });
                    }
                }
            });
        self.open = open;
        action
    }
}
