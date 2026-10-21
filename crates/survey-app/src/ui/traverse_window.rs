//! Traverse entry, live reduction and point creation.
//!
//! Stations and references can be point names from the project or typed
//! bearings; the reduction updates as the observations are typed.

use egui::RichText;
use egui_cad::chrome::{IconButton, segmented, switch_row};
use egui_cad::{CadTheme, fonts, icons};
use glam::DVec3;
use survey_core::traverse::{Adjustment, Closing, Direction, Leg, Traverse, TraverseResult, compute, report};
use survey_core::{Bearing, Conventions};
use survey_model::{ChangeSet, Command, Document, Entity, Polyline, SurveyPoint};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Observed {
    Angles,
    Bearings,
}

#[derive(Clone, Default)]
struct Row {
    to: String,
    direction: String,
    distance: String,
    dz: String,
}

pub struct TraverseWindow {
    pub open: bool,
    start: String,
    backsight: String,
    observed: Observed,
    rows: Vec<Row>,
    close_on: String,
    closing_reference: String,
    closing_angle: String,
    adjustment: Adjustment,
    adjust_angles: bool,
    add_polyline: bool,
}

impl Default for TraverseWindow {
    fn default() -> Self {
        Self {
            open: false,
            start: String::new(),
            backsight: String::new(),
            observed: Observed::Angles,
            rows: vec![Row::default()],
            close_on: String::new(),
            closing_reference: String::new(),
            closing_angle: String::new(),
            adjustment: Adjustment::Bowditch,
            adjust_angles: true,
            add_polyline: true,
        }
    }
}

pub enum TraverseAction {
    /// Create the computed stations: the command and a one-line summary.
    Create(Box<dyn Command>, String),
    /// Add the report (title, text) to the Reports window.
    Report(String, String),
}

/// A point by name, or `None` with an explanation.
fn point<'a>(doc: &'a Document, name: &str) -> Result<&'a SurveyPoint, String> {
    doc.find_point_by_name(name.trim())
        .map(|(_, p)| p)
        .ok_or_else(|| format!("no point named '{}'", name.trim()))
}

/// A bearing from `from` to the named point, or a typed bearing.
fn bearing_or_point(doc: &Document, conv: &Conventions, from: DVec3, text: &str) -> Result<Bearing, String> {
    if let Ok(p) = point(doc, text) {
        let d = p.position - from;
        return Bearing::from_delta(d.x, d.y).ok_or_else(|| format!("'{}' is at the station", text.trim()));
    }
    conv.parse_bearing(text)
        .map_err(|e| format!("'{}': not a point name or bearing ({e})", text.trim()))
}

impl TraverseWindow {
    /// The traverse described by the form, or the first problem with it.
    fn build(&self, doc: &Document) -> Result<Traverse, String> {
        let conv = doc.conventions();
        if self.start.trim().is_empty() {
            return Err("Enter the start point and the observations to reduce the traverse.".into());
        }
        let start = point(doc, &self.start).map_err(|e| format!("Start: {e}"))?;
        let backsight = if self.backsight.trim().is_empty() {
            None
        } else {
            Some(
                bearing_or_point(doc, conv, start.position, &self.backsight)
                    .map_err(|e| format!("Backsight: {e}"))?,
            )
        };
        let mut legs = Vec::new();
        for (i, r) in self.rows.iter().enumerate() {
            if r.to.trim().is_empty() && r.direction.trim().is_empty() && r.distance.trim().is_empty() {
                continue;
            }
            let n = i + 1;
            let direction = match self.observed {
                Observed::Angles => conv.parse_angle(&r.direction).map(Direction::Angle),
                Observed::Bearings => conv.parse_bearing(&r.direction).map(Direction::Bearing),
            }
            .map_err(|e| format!("Leg {n} direction: {e}"))?;
            let distance = conv
                .parse_length(&r.distance)
                .map_err(|e| format!("Leg {n} distance: {e}"))?;
            let dz = if r.dz.trim().is_empty() {
                None
            } else {
                Some(conv.parse_length(&r.dz).map_err(|e| format!("Leg {n} dZ: {e}"))?)
            };
            let to = if r.to.trim().is_empty() {
                format!("T{n}")
            } else {
                r.to.trim().to_string()
            };
            legs.push(Leg {
                to,
                direction,
                distance,
                dz,
            });
        }
        let closing = if self.close_on.trim().is_empty() {
            None
        } else {
            let p = point(doc, &self.close_on).map_err(|e| format!("Close on: {e}"))?;
            let angle_check =
                if self.closing_reference.trim().is_empty() || self.closing_angle.trim().is_empty() {
                    None
                } else {
                    let b = bearing_or_point(doc, conv, p.position, &self.closing_reference)
                        .map_err(|e| format!("Closing reference: {e}"))?;
                    let a = conv
                        .parse_angle(&self.closing_angle)
                        .map_err(|e| format!("Closing angle: {e}"))?;
                    Some((b, a))
                };
            Some(Closing {
                point: p.position.truncate(),
                height: p.height_known.then_some(p.position.z),
                angle_check,
            })
        };
        Ok(Traverse {
            start_name: start.name.clone(),
            start: start.position,
            start_height_known: start.height_known,
            backsight,
            legs,
            closing,
            adjustment: self.adjustment,
            adjust_angles: self.adjust_angles,
        })
    }

    /// Points (and optionally a polyline) for the computed stations. The last
    /// station of a closed traverse is the known point and is not recreated;
    /// stations whose names already exist are skipped.
    fn create(&self, doc: &Document, t: &Traverse, r: &TraverseResult) -> (ChangeSet, String) {
        let precision = r.precision().map(|n| format!(", 1:{n:.0}")).unwrap_or_default();
        let mut cs = ChangeSet::new(format!(
            "Traverse from {} ({}{precision})",
            t.start_name,
            r.adjustment.label()
        ));
        let mut created = 0;
        let mut skipped = Vec::new();
        let last = r.stations.len() - 1;
        for (i, st) in r.stations.iter().enumerate() {
            if i == last && t.closing.is_some() {
                continue;
            }
            if doc.find_point_by_name(&st.name).is_some() {
                skipped.push(st.name.clone());
                continue;
            }
            cs.push_insert_entity(Entity::Point(SurveyPoint {
                name: st.name.clone(),
                position: st.position,
                height_known: st.height_known,
                code: "TRAV".into(),
                description: format!("Traverse from {}", t.start_name),
                layer: doc.current_layer(),
            }));
            created += 1;
        }
        if self.add_polyline {
            let mut vertices = vec![t.start];
            vertices.extend(r.stations.iter().map(|s| s.position));
            let closed = vertices.len() > 3
                && vertices[0]
                    .truncate()
                    .distance(vertices[vertices.len() - 1].truncate())
                    < 1e-6;
            if closed {
                vertices.pop();
            }
            cs.push_insert_entity(Entity::Polyline(Polyline {
                name: format!("Traverse {}", t.start_name),
                vertices,
                bulges: Vec::new(),
                closed,
                code: "TRAV".into(),
                layer: doc.current_layer(),
            }));
        }
        let mut summary = format!("Traverse: created {created} points{precision}");
        if !skipped.is_empty() {
            summary.push_str(&format!(" (existing, not changed: {})", skipped.join(", ")));
        }
        (cs, summary)
    }

    pub fn show(&mut self, ctx: &egui::Context, doc: &Document) -> Option<TraverseAction> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let conv = doc.conventions().clone();
        let mut action = None;
        let mut open = self.open;
        egui::Window::new("Traverse")
            .open(&mut open)
            .default_size([720.0, 560.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    self.form(ui, &theme);
                    ui.add_space(8.0);
                    ui.separator();
                    let built = self.build(doc);
                    let result = built
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|t| compute(t).map(|r| (t, r)).map_err(|e| e.to_string()));
                    match result {
                        Ok((t, r)) => {
                            results(ui, &theme, &conv, &r);
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                if ui.button("Create points").clicked() {
                                    let (cs, summary) = self.create(doc, t, &r);
                                    action = Some(TraverseAction::Create(Box::new(cs), summary));
                                }
                                if ui.button("Add to reports").clicked() {
                                    action = Some(TraverseAction::Report(
                                        format!("Traverse {}", t.start_name),
                                        report(t, &r, &conv),
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

    fn form(&mut self, ui: &mut egui::Ui, theme: &CadTheme) {
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
                ui.label("Observations");
                segmented(
                    ui,
                    &mut self.observed,
                    &[(Observed::Angles, "Angles"), (Observed::Bearings, "Bearings")],
                );
                ui.end_row();
            });
        ui.add_space(6.0);

        let direction = match self.observed {
            Observed::Angles => "Angle (clockwise from backsight)",
            Observed::Bearings => "Bearing",
        };
        let mut remove = None;
        egui::Grid::new("traverse_legs")
            .num_columns(6)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                for h in ["#", "To station", direction, "Distance", "dZ", ""] {
                    ui.label(RichText::new(h).small().color(theme.text_secondary));
                }
                ui.end_row();
                for (i, r) in self.rows.iter_mut().enumerate() {
                    ui.label(RichText::new((i + 1).to_string()).color(theme.text_secondary));
                    field(ui, &mut r.to, "name", 80.0);
                    field(ui, &mut r.direction, "", 190.0);
                    field(ui, &mut r.distance, "", 90.0);
                    field(ui, &mut r.dz, "optional", 80.0);
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
            let to = self
                .rows
                .last()
                .and_then(|r| r.to.trim().parse::<u64>().ok())
                .map(|n| (n + 1).to_string())
                .unwrap_or_default();
            self.rows.push(Row { to, ..Row::default() });
        }
        ui.add_space(8.0);

        egui::Grid::new("traverse_closing")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Close on point");
                field(ui, &mut self.close_on, "blank for an open traverse", 200.0);
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
                field(ui, &mut self.closing_angle, "observed at the close point", 200.0);
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

fn results(ui: &mut egui::Ui, theme: &CadTheme, conv: &Conventions, r: &TraverseResult) {
    egui::Grid::new("traverse_results")
        .num_columns(4)
        .spacing([16.0, 3.0])
        .striped(false)
        .show(ui, |ui| {
            let (ea, nb) = conv.horizontal_labels();
            for h in ["Station", "Bearing", "Distance", &format!("{ea} / {nb}")] {
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
    line(ui, "Length", conv.format_length_with_unit(r.length), theme.text);
    if let Some(a) = r.angular_misclosure {
        line(ui, "Angular misclosure", conv.format_angle(a), theme.text);
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
    }
}
