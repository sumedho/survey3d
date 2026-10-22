//! Stockpile volumes: a surface above its base inside a toe polygon, with
//! tonnage from a density.

use std::fmt::Write as _;

use egui::RichText;
use egui_cad::chrome::segmented;
use egui_cad::{CadTheme, fonts};
use survey_core::volume::{Stockpile, StockpileBase, stockpile};
use survey_model::surface::outline;
use survey_model::{Document, EntityId, SurfaceId};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Base {
    Toe,
    LowestToe,
    Level,
}

pub struct StockpileWindow {
    pub open: bool,
    surface: Option<SurfaceId>,
    toe: Option<EntityId>,
    base: Base,
    level: String,
    density: String,
    name: String,
    result: Option<Result<Stockpile, String>>,
}

impl Default for StockpileWindow {
    fn default() -> Self {
        Self {
            open: false,
            surface: None,
            toe: None,
            base: Base::Toe,
            level: String::new(),
            density: String::new(),
            name: String::new(),
            result: None,
        }
    }
}

impl StockpileWindow {
    pub fn open_with(&mut self, doc: &Document, active: Option<SurfaceId>, toe: Option<EntityId>) {
        self.open = true;
        if self.surface.is_none_or(|id| doc.surface(id).is_none()) {
            self.surface = active.or_else(|| doc.surfaces().next().map(|(id, _)| id));
        }
        if toe.is_some() {
            self.toe = toe;
            self.result = None;
        }
    }

    fn density(&self) -> Option<f64> {
        self.density.trim().parse::<f64>().ok().filter(|d| *d > 0.0)
    }

    /// Returns a (title, text) report when one is requested.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        selected_closed: &[EntityId],
    ) -> Option<(String, String)> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let conv = doc.conventions().clone();
        let mut report = None;
        let mut open = self.open;
        egui::Window::new("Stockpile volume")
            .open(&mut open)
            .default_width(420.0)
            .show(ctx, |ui| {
                if doc.surfaces().next().is_none() {
                    ui.label(
                        RichText::new("Create a surface of the stockpile pickup first.")
                            .color(theme.text_secondary),
                    );
                    return;
                }
                let mut changed = false;
                egui::Grid::new("stockpile")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Name");
                        ui.add_sized(
                            [180.0, 22.0],
                            egui::TextEdit::singleline(&mut self.name).hint_text("e.g. ROM stockpile 3"),
                        );
                        ui.end_row();
                        ui.label("Surface");
                        let text = self
                            .surface
                            .and_then(|id| doc.surface(id))
                            .map_or("Choose…".into(), |s| s.name.clone());
                        egui::ComboBox::from_id_salt("stockpile_surface")
                            .selected_text(text)
                            .show_ui(ui, |ui| {
                                for (id, s) in doc.surfaces() {
                                    changed |= ui
                                        .selectable_value(&mut self.surface, Some(id), &s.name)
                                        .changed();
                                }
                            });
                        ui.end_row();
                        ui.label("Toe");
                        ui.horizontal(|ui| {
                            ui.label(if self.toe.is_some() {
                                "selected polyline"
                            } else {
                                "none"
                            });
                            if ui
                                .add_enabled(selected_closed.len() == 1, egui::Button::new("Use selected"))
                                .on_disabled_hover_text("Select the closed toe polyline")
                                .clicked()
                            {
                                self.toe = selected_closed.first().copied();
                                changed = true;
                            }
                        });
                        ui.end_row();
                        ui.label("Base");
                        ui.horizontal(|ui| {
                            changed |= segmented(
                                ui,
                                &mut self.base,
                                &[
                                    (Base::Toe, "Toe surface"),
                                    (Base::LowestToe, "Lowest toe"),
                                    (Base::Level, "Level"),
                                ],
                            );
                            if self.base == Base::Level {
                                changed |= ui
                                    .add_sized(
                                        [80.0, 22.0],
                                        egui::TextEdit::singleline(&mut self.level).font(fonts::mono(12.0)),
                                    )
                                    .changed();
                            }
                        });
                        ui.end_row();
                        ui.label("Density");
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [70.0, 22.0],
                                egui::TextEdit::singleline(&mut self.density)
                                    .hint_text("t/m³")
                                    .font(fonts::mono(12.0)),
                            );
                            ui.label(RichText::new("t/m³ (optional, for tonnes)").small());
                        });
                        ui.end_row();
                    });
                ui.label(
                    RichText::new(
                        "Toe surface: the base is triangulated from the surface heights along the toe.",
                    )
                    .small()
                    .color(theme.text_secondary),
                );
                if changed {
                    self.result = None;
                }
                if ui.button("Compute").clicked() {
                    self.result = Some(self.compute(doc));
                }
                ui.separator();
                match &self.result {
                    Some(Ok(s)) => {
                        let mpu = conv.linear_unit.metres_per_unit();
                        let vol = |v: f64| format!("{:.3} {}³", v / mpu.powi(3), conv.linear_unit.suffix());
                        let r = &s.result;
                        egui::Grid::new("stockpile_result")
                            .num_columns(2)
                            .spacing([16.0, 4.0])
                            .show(ui, |ui| {
                                ui.label(RichText::new("Volume").color(theme.text_secondary));
                                ui.label(RichText::new(vol(r.net())).font(fonts::mono(14.0)).strong());
                                ui.end_row();
                                if let Some(d) = self.density() {
                                    ui.label(RichText::new("Tonnes").color(theme.text_secondary));
                                    ui.label(
                                        RichText::new(format!("{:.1} t", r.net() * d))
                                            .font(fonts::mono(14.0))
                                            .strong(),
                                    );
                                    ui.end_row();
                                }
                                ui.label(RichText::new("Above / below base").color(theme.text_secondary));
                                ui.label(
                                    RichText::new(format!("{} / {}", vol(r.fill), vol(r.cut)))
                                        .font(fonts::mono(12.0)),
                                );
                                ui.end_row();
                                ui.label(RichText::new("Plan area").color(theme.text_secondary));
                                ui.label(RichText::new(conv.format_area(r.area)).font(fonts::mono(12.0)));
                                ui.end_row();
                                ui.label(RichText::new("Toe heights").color(theme.text_secondary));
                                ui.label(
                                    RichText::new(format!(
                                        "{} to {}",
                                        conv.format_length(s.base_min),
                                        conv.format_length(s.base_max)
                                    ))
                                    .font(fonts::mono(12.0)),
                                );
                                ui.end_row();
                            });
                        if ui.button("Add to reports").clicked() {
                            report = Some(self.report(doc, s));
                        }
                    }
                    Some(Err(e)) => {
                        ui.label(RichText::new(e).color(theme.error));
                    }
                    None => {}
                }
            });
        self.open = open;
        report
    }

    fn compute(&self, doc: &Document) -> Result<Stockpile, String> {
        let s = self
            .surface
            .and_then(|id| doc.surface(id))
            .ok_or("Choose the surface")?;
        let toe = self
            .toe
            .and_then(|id| outline(doc, id))
            .ok_or("Select the closed toe polyline")?;
        let base = match self.base {
            Base::Toe => StockpileBase::Toe,
            Base::LowestToe => StockpileBase::LowestToe,
            Base::Level => StockpileBase::Level(
                doc.conventions()
                    .parse_length(&self.level)
                    .map_err(|e| format!("Base level: {e}"))?,
            ),
        };
        stockpile(&s.tin, &toe, base).map_err(|e| e.to_string())
    }

    fn report(&self, doc: &Document, s: &Stockpile) -> (String, String) {
        let conv = doc.conventions();
        let mpu = conv.linear_unit.metres_per_unit();
        let vol = |v: f64| format!("{:.3} {}³", v / mpu.powi(3), conv.linear_unit.suffix());
        let name = if self.name.trim().is_empty() {
            "Stockpile".to_string()
        } else {
            self.name.trim().to_string()
        };
        let surface = self
            .surface
            .and_then(|id| doc.surface(id))
            .map_or(String::new(), |s| s.name.clone());
        let base = match self.base {
            Base::Toe => "triangulated toe".to_string(),
            Base::LowestToe => format!("lowest toe {}", conv.format_length(s.base_min)),
            Base::Level => format!("level {}", self.level.trim()),
        };
        let r = &s.result;
        let mut t = String::new();
        let _ = writeln!(t, "STOCKPILE  {name}");
        let _ = writeln!(t, "Surface      {surface}");
        let _ = writeln!(t, "Base         {base}");
        let _ = writeln!(t);
        let _ = writeln!(t, "Volume       {}", vol(r.net()));
        if let Some(d) = self.density() {
            let _ = writeln!(t, "Density      {d} t/m³");
            let _ = writeln!(t, "Tonnes       {:.1} t", r.net() * d);
        }
        let _ = writeln!(t, "Above base   {}", vol(r.fill));
        let _ = writeln!(t, "Below base   {}", vol(r.cut));
        let _ = writeln!(t, "Plan area    {}", conv.format_area(r.area));
        let _ = writeln!(
            t,
            "Toe heights  {} to {}",
            conv.format_length(s.base_min),
            conv.format_length(s.base_max)
        );
        (format!("Stockpile {name}"), t)
    }
}
