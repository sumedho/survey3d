//! Traverse entry, live reduction, saved traverses and point creation.
//!
//! The form is text; it converts to a [`TraverseRecord`] (typed observations,
//! stations by name) which is what gets saved with the project, and the record
//! resolves against the project's points to a [`Traverse`] for reduction, so a
//! saved traverse picks up any change to its control.

use egui::RichText;
use egui_cad::chrome::{IconButton, segmented, switch_row};
use egui_cad::{CadTheme, fonts, icons};
use survey_core::accuracy::{Check, Tolerances};
use survey_core::traverse::{Adjustment, Direction, Leg, Reference, TraverseRecord, TraverseResult, report};
use survey_core::{Angle, Conventions};
use survey_model::{Command, Document};

use crate::compute::traverse::{create_points, parse_deflection, parse_reference as reference, reduce};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Observed {
    AnglesRight,
    AnglesLeft,
    Deflections,
    Bearings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Distances {
    Horizontal,
    Slope,
}

#[derive(Clone, Default)]
struct Row {
    to: String,
    direction: String,
    distance: String,
    zenith: String,
    dz: String,
    hi: String,
    ht: String,
}

pub struct TraverseWindow {
    pub open: bool,
    name: String,
    start: String,
    backsight: String,
    observed: Observed,
    distances: Distances,
    rows: Vec<Row>,
    close_on: String,
    closing_reference: String,
    closing_angle: String,
    adjustment: Adjustment,
    adjust_angles: bool,
    scale: String,
    add_polyline: bool,
}

impl Default for TraverseWindow {
    fn default() -> Self {
        Self {
            open: false,
            name: String::new(),
            start: String::new(),
            backsight: String::new(),
            observed: Observed::AnglesRight,
            distances: Distances::Horizontal,
            rows: vec![Row::default()],
            close_on: String::new(),
            closing_reference: String::new(),
            closing_angle: String::new(),
            adjustment: Adjustment::Bowditch,
            adjust_angles: true,
            scale: String::new(),
            add_polyline: true,
        }
    }
}

pub enum TraverseAction {
    /// Create the computed stations: the command and a one-line summary.
    Create(Box<dyn Command>, String),
    /// Add the report (title, text) to the Reports window.
    Report(String, String),
    /// Save (or replace by name) a traverse in the project.
    Save(TraverseRecord),
    /// Remove a saved traverse by name.
    Delete(String),
}

fn format_reference(conv: &Conventions, r: &Option<Reference>) -> String {
    match r {
        None => String::new(),
        Some(Reference::Point(name)) => name.clone(),
        Some(Reference::Bearing(b)) => conv.format_bearing(*b),
    }
}

fn format_deflection(conv: &Conventions, a: Angle) -> String {
    let side = if a.radians() < 0.0 { "L" } else { "R" };
    format!(
        "{} {side}",
        conv.format_angle(Angle::from_radians(a.radians().abs()))
    )
}

impl TraverseWindow {
    /// The record described by the form, or the first problem with it.
    fn record(&self, doc: &Document) -> Result<TraverseRecord, String> {
        let conv = doc.conventions();
        let blank = |s: &str| s.trim().is_empty();
        let length = |label: String, text: &str| -> Result<Option<f64>, String> {
            if blank(text) {
                return Ok(None);
            }
            conv.parse_length(text)
                .map(Some)
                .map_err(|e| format!("{label}: {e}"))
        };
        let mut legs = Vec::new();
        for (i, r) in self.rows.iter().enumerate() {
            if blank(&r.to) && blank(&r.direction) && blank(&r.distance) {
                continue;
            }
            let n = i + 1;
            let direction = match self.observed {
                Observed::AnglesRight => conv.parse_angle(&r.direction).map(Direction::Angle),
                Observed::AnglesLeft => conv.parse_angle(&r.direction).map(Direction::AngleLeft),
                Observed::Bearings => conv.parse_bearing(&r.direction).map(Direction::Bearing),
                Observed::Deflections => parse_deflection(conv, &r.direction).map(Direction::Deflection),
            }
            .map_err(|e| format!("Leg {n} direction: {e}"))?;
            let distance = length(format!("Leg {n} distance"), &r.distance)?
                .ok_or(format!("Leg {n}: enter the distance"))?;
            let to = if blank(&r.to) {
                format!("T{n}")
            } else {
                r.to.trim().to_string()
            };
            let mut leg = Leg::new(to, direction, distance);
            match self.distances {
                Distances::Horizontal => leg.dz = length(format!("Leg {n} dZ"), &r.dz)?,
                Distances::Slope => {
                    if blank(&r.zenith) {
                        return Err(format!("Leg {n}: enter the zenith angle (90° is level)"));
                    }
                    leg.zenith = Some(
                        conv.parse_angle(&r.zenith)
                            .map_err(|e| format!("Leg {n} zenith: {e}"))?,
                    );
                    leg.instrument_height = length(format!("Leg {n} HI"), &r.hi)?.unwrap_or(0.0);
                    leg.target_height = length(format!("Leg {n} HT"), &r.ht)?.unwrap_or(0.0);
                }
            }
            legs.push(leg);
        }
        let scale_factor = if blank(&self.scale) {
            1.0
        } else {
            self.scale
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("Scale factor: '{}' is not a number", self.scale.trim()))?
        };
        let closing_angle = if blank(&self.closing_angle) {
            None
        } else {
            Some(
                conv.parse_angle(&self.closing_angle)
                    .map_err(|e| format!("Closing angle: {e}"))?,
            )
        };
        Ok(TraverseRecord {
            name: self.name.trim().to_string(),
            start: self.start.trim().to_string(),
            backsight: reference(doc, conv, &self.backsight).map_err(|e| format!("Backsight: {e}"))?,
            legs,
            close_on: (!blank(&self.close_on)).then(|| self.close_on.trim().to_string()),
            closing_reference: reference(doc, conv, &self.closing_reference)
                .map_err(|e| format!("Closing reference: {e}"))?,
            closing_angle,
            adjustment: self.adjustment,
            adjust_angles: self.adjust_angles,
            scale_factor,
        })
    }

    /// Fills the form from a saved record.
    fn load(&mut self, conv: &Conventions, rec: &TraverseRecord) {
        let len = |m: f64| conv.format_length(m);
        self.name = rec.name.clone();
        self.start = rec.start.clone();
        self.backsight = format_reference(conv, &rec.backsight);
        self.observed = match rec.legs.first().map(|l| l.direction) {
            Some(Direction::AngleLeft(_)) => Observed::AnglesLeft,
            Some(Direction::Deflection(_)) => Observed::Deflections,
            Some(Direction::Bearing(_)) => Observed::Bearings,
            _ => Observed::AnglesRight,
        };
        self.distances = if rec.legs.iter().any(|l| l.zenith.is_some()) {
            Distances::Slope
        } else {
            Distances::Horizontal
        };
        self.rows = rec
            .legs
            .iter()
            .map(|l| Row {
                to: l.to.clone(),
                direction: match l.direction {
                    Direction::Angle(a) | Direction::AngleLeft(a) => conv.format_angle(a),
                    Direction::Deflection(a) => format_deflection(conv, a),
                    Direction::Bearing(b) => conv.format_bearing(b),
                },
                distance: len(l.distance),
                zenith: l.zenith.map(|z| conv.format_angle(z)).unwrap_or_default(),
                dz: l.dz.map(len).unwrap_or_default(),
                hi: if l.zenith.is_some() {
                    len(l.instrument_height)
                } else {
                    String::new()
                },
                ht: if l.zenith.is_some() {
                    len(l.target_height)
                } else {
                    String::new()
                },
            })
            .collect();
        if self.rows.is_empty() {
            self.rows.push(Row::default());
        }
        self.close_on = rec.close_on.clone().unwrap_or_default();
        self.closing_reference = format_reference(conv, &rec.closing_reference);
        self.closing_angle = rec
            .closing_angle
            .map(|a| conv.format_angle(a))
            .unwrap_or_default();
        self.adjustment = rec.adjustment;
        self.adjust_angles = rec.adjust_angles;
        self.scale = if rec.scale_factor == 1.0 {
            String::new()
        } else {
            format!("{}", rec.scale_factor)
        };
    }

    pub fn show(&mut self, ctx: &egui::Context, doc: &Document) -> Option<TraverseAction> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let conv = doc.conventions().clone();
        let tolerances = doc.meta().tolerances;
        let mut action = None;
        let mut open = self.open;
        egui::Window::new("Traverse")
            .open(&mut open)
            .default_size([780.0, 600.0])
            .show(ctx, |ui| {
                self.saved_bar(ui, doc, &conv, &mut action);
                ui.add_space(6.0);
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    self.form(ui, &theme, doc);
                    ui.add_space(8.0);
                    ui.separator();
                    let result = self.record(doc).and_then(|rec| reduce(doc, &rec));
                    match result {
                        Ok((t, r)) => {
                            results(ui, &theme, &conv, &tolerances, &r);
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                if ui.button("Create points").clicked() {
                                    let (cs, summary) = create_points(doc, &t, &r, self.add_polyline);
                                    action = Some(TraverseAction::Create(Box::new(cs), summary));
                                }
                                if ui.button("Add to reports").clicked() {
                                    let title = if self.name.trim().is_empty() {
                                        format!("Traverse {}", t.start_name)
                                    } else {
                                        format!("Traverse {}", self.name.trim())
                                    };
                                    action = Some(TraverseAction::Report(
                                        title,
                                        report(&t, &r, &tolerances, &conv),
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

    /// Name, saved traverses, save and delete.
    fn saved_bar(
        &mut self,
        ui: &mut egui::Ui,
        doc: &Document,
        conv: &Conventions,
        action: &mut Option<TraverseAction>,
    ) {
        let saved = doc.traverses();
        let name = self.name.trim().to_string();
        let exists = saved.iter().any(|r| r.name == name);
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.add_sized(
                [160.0, 22.0],
                egui::TextEdit::singleline(&mut self.name).hint_text("to save it"),
            );
            let record = self.record(doc);
            let can_save = !name.is_empty() && record.is_ok();
            let save = ui
                .add_enabled(
                    can_save,
                    egui::Button::new(if exists { "Update" } else { "Save" }),
                )
                .on_hover_text("Saved with the project; stations are kept by name");
            if save.clicked()
                && let Ok(rec) = record
            {
                *action = Some(TraverseAction::Save(rec));
            }
            if exists && ui.button("Delete").clicked() {
                *action = Some(TraverseAction::Delete(name.clone()));
            }
            let mut chosen = None;
            egui::ComboBox::from_id_salt("saved_traverses")
                .selected_text(if saved.is_empty() {
                    "No saved traverses"
                } else {
                    "Open saved…"
                })
                .show_ui(ui, |ui| {
                    for rec in saved {
                        if ui.selectable_label(rec.name == name, &rec.name).clicked() {
                            chosen = Some(rec.record.clone());
                        }
                    }
                });
            if ui.button("New").clicked() {
                *self = Self {
                    open: true,
                    ..Self::default()
                };
            }
            if let Some(rec) = chosen {
                self.load(conv, &rec);
            }
        });
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
        egui::Grid::new("traverse_setup")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Start point");
                field(ui, &mut self.start, "point name", 140.0);
                ui.end_row();
                ui.label("Backsight");
                field(ui, &mut self.backsight, "point name or bearing", 200.0);
                ui.end_row();
                ui.label("Directions");
                segmented(
                    ui,
                    &mut self.observed,
                    &[
                        (Observed::AnglesRight, "Angles right"),
                        (Observed::AnglesLeft, "Angles left"),
                        (Observed::Deflections, "Deflections"),
                        (Observed::Bearings, "Bearings"),
                    ],
                );
                ui.end_row();
                ui.label("Distances");
                segmented(
                    ui,
                    &mut self.distances,
                    &[
                        (Distances::Horizontal, "Horizontal"),
                        (Distances::Slope, "Slope + zenith"),
                    ],
                );
                ui.end_row();
                ui.label("Scale factor");
                ui.horizontal(|ui| {
                    field(ui, &mut self.scale, "1", 120.0);
                    let start = doc.find_point_by_name(self.start.trim()).map(|(_, p)| p);
                    let factor = start.and_then(|p| {
                        let h = if p.height_known { p.position.z } else { 0.0 };
                        doc.combined_factor_at(p.position.truncate().extend(h))
                    });
                    if ui
                        .add_enabled(factor.is_some(), egui::Button::new("From project"))
                        .on_hover_text("Combined (point × height) factor at the start point from the project's projection")
                        .on_disabled_hover_text("Needs a projection (Settings → Coordinates) and a start point")
                        .clicked()
                        && let Some(k) = factor
                    {
                        self.scale = format!("{k:.9}");
                    }
                });
                ui.end_row();
            });
        ui.add_space(6.0);

        let direction = match self.observed {
            Observed::AnglesRight => "Angle (clockwise)",
            Observed::AnglesLeft => "Angle (anticlockwise)",
            Observed::Deflections => "Deflection (R / L)",
            Observed::Bearings => "Bearing",
        };
        let slope = self.distances == Distances::Slope;
        let headers: Vec<&str> = if slope {
            vec![
                "#",
                "To station",
                direction,
                "Slope dist",
                "Zenith",
                "HI",
                "HT",
                "",
            ]
        } else {
            vec!["#", "To station", direction, "Distance", "dZ", ""]
        };
        let mut remove = None;
        egui::Grid::new("traverse_legs")
            .num_columns(headers.len())
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                for h in &headers {
                    ui.label(RichText::new(*h).small().color(theme.text_secondary));
                }
                ui.end_row();
                for (i, r) in self.rows.iter_mut().enumerate() {
                    ui.label(RichText::new((i + 1).to_string()).color(theme.text_secondary));
                    field(ui, &mut r.to, "name", 80.0);
                    field(ui, &mut r.direction, "", 170.0);
                    field(ui, &mut r.distance, "", 90.0);
                    if slope {
                        field(ui, &mut r.zenith, "", 120.0);
                        field(ui, &mut r.hi, "0", 60.0);
                        field(ui, &mut r.ht, "0", 60.0);
                    } else {
                        field(ui, &mut r.dz, "optional", 80.0);
                    }
                    if IconButton::new(icons::X, "Remove leg")
                        .size(20.0)
                        .show(ui)
                        .clicked()
                    {
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
        if ui.button(format!("{} Add leg", icons::PLUS)).clicked() {
            let last = self.rows.last().cloned().unwrap_or_default();
            let to = last
                .to
                .trim()
                .parse::<u64>()
                .map(|n| (n + 1).to_string())
                .unwrap_or_default();
            // Instrument and target heights usually repeat from leg to leg.
            self.rows.push(Row {
                to,
                hi: last.hi,
                ht: last.ht,
                ..Row::default()
            });
        }
        ui.add_space(8.0);

        egui::Grid::new("traverse_closing")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Close on point");
                ui.horizontal(|ui| {
                    field(ui, &mut self.close_on, "blank for an open traverse", 200.0);
                    if ui
                        .button("Loop")
                        .on_hover_text("Close back on the start, checking against the backsight")
                        .clicked()
                    {
                        self.close_on = self.start.clone();
                        self.closing_reference = self.backsight.clone();
                    }
                });
                ui.end_row();
                ui.label("Closing reference");
                field(
                    ui,
                    &mut self.closing_reference,
                    "point or bearing (optional)",
                    200.0,
                );
                ui.end_row();
                ui.label("Closing angle");
                field(
                    ui,
                    &mut self.closing_angle,
                    "clockwise, at the close point",
                    200.0,
                );
                ui.end_row();
                ui.label("Adjustment");
                let options: Vec<(Adjustment, &str)> =
                    Adjustment::ALL.iter().map(|a| (*a, a.label())).collect();
                segmented(ui, &mut self.adjustment, &options);
                ui.end_row();
            });
        ui.add_space(4.0);
        switch_row(
            ui,
            &mut self.adjust_angles,
            "Distribute angular misclosure equally",
        );
        switch_row(ui, &mut self.add_polyline, "Also draw the traverse as a polyline");
    }
}

fn results(
    ui: &mut egui::Ui,
    theme: &CadTheme,
    conv: &Conventions,
    tolerances: &Tolerances,
    r: &TraverseResult,
) {
    egui::Grid::new("traverse_results")
        .num_columns(4)
        .spacing([16.0, 3.0])
        .striped(false)
        .show(ui, |ui| {
            let (ea, nb) = conv.horizontal_labels();
            for h in ["Station", "Bearing", "Hz distance", &format!("{ea} / {nb}")] {
                ui.label(RichText::new(h).small().color(theme.text_secondary));
            }
            ui.end_row();
            for st in &r.stations {
                ui.label(&st.name);
                ui.label(RichText::new(conv.format_bearing(st.bearing)).font(fonts::mono(12.0)));
                ui.label(RichText::new(conv.format_length(st.distance)).font(fonts::mono(12.0)));
                ui.label(
                    RichText::new(conv.format_coord(st.position, st.height_known)).font(fonts::mono(12.0)),
                );
                ui.end_row();
            }
        });
    ui.add_space(6.0);
    let line = |ui: &mut egui::Ui, label: &str, value: String, color: egui::Color32| {
        ui.horizontal(|ui| {
            ui.add_sized(
                [150.0, 16.0],
                egui::Label::new(RichText::new(label).color(theme.text_secondary)),
            );
            ui.label(RichText::new(value).font(fonts::mono(12.0)).color(color));
        });
    };
    let check = |ui: &mut egui::Ui, c: Option<Check>, allowed: &dyn Fn(f64) -> String| {
        if let Some(c) = c {
            let (text, color) = if c.pass {
                ("PASS", theme.success)
            } else {
                ("FAIL", theme.error)
            };
            line(ui, "  allowed", format!("{}  {text}", allowed(c.allowed)), color);
        }
    };
    line(ui, "Length", conv.format_length_with_unit(r.length), theme.text);
    if let Some(a) = r.angular_misclosure {
        line(
            ui,
            "Angular misclosure",
            format!("{}  ({} angles)", conv.format_angle(a), r.angles),
            theme.text,
        );
        check(ui, tolerances.check_angular(a, r.angles), &|v| {
            conv.format_angle(Angle::from_radians(v))
        });
    }
    match (r.linear_misclosure, r.precision()) {
        (Some(m), Some(n)) => {
            line(
                ui,
                "Linear misclosure",
                conv.format_length_with_unit(m.length()),
                theme.text,
            );
            line(ui, "Precision", format!("1 : {n:.0}"), theme.text);
            check(ui, tolerances.check_linear(m.length(), r.length), &|v| {
                conv.format_length_with_unit(v)
            });
        }
        (Some(_), None) => line(ui, "Linear misclosure", "closes exactly".into(), theme.success),
        (None, _) => line(
            ui,
            "Misclosure",
            "open traverse — no check".into(),
            theme.text_secondary,
        ),
    }
    if let Some(v) = r.vertical_misclosure {
        line(
            ui,
            "Vertical misclosure",
            conv.format_length_with_unit(v),
            theme.text,
        );
        check(ui, tolerances.check_vertical(v, r.length), &|v| {
            conv.format_length_with_unit(v)
        });
    }
}
