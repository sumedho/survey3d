//! Stake-out sheet: from an instrument station and backsight, the horizontal
//! angle and distance to each point to be set out.

use std::fmt::Write as _;

use egui::RichText;
use egui_cad::{CadTheme, fonts};
use survey_core::cogo::inverse;
use survey_core::{Angle, Bearing, Conventions};
use survey_model::{Document, Entity, EntityId, SurveyPoint};

#[derive(Default)]
pub struct StakeoutWindow {
    pub open: bool,
    station: String,
    backsight: String,
}

struct Row {
    name: String,
    angle: Option<Angle>,
    bearing: Option<Bearing>,
    distance: f64,
    dz: Option<f64>,
    description: String,
}

impl StakeoutWindow {
    fn rows(&self, doc: &Document, targets: &[EntityId]) -> Result<(String, Vec<Row>), String> {
        let conv = doc.conventions();
        let station = doc
            .find_point_by_name(self.station.trim())
            .map(|(_, p)| p)
            .ok_or_else(|| "Enter the instrument station (a point name).".to_string())?;
        let bs = self.backsight.trim();
        let backsight: Option<Bearing> = if bs.is_empty() {
            None
        } else if let Some((_, p)) = doc.find_point_by_name(bs) {
            let d = p.position - station.position;
            Some(Bearing::from_delta(d.x, d.y).ok_or("The backsight is at the station")?)
        } else {
            Some(
                conv.parse_bearing(bs)
                    .map_err(|e| format!("Backsight: not a point or bearing ({e})"))?,
            )
        };
        let points: Vec<&SurveyPoint> = targets
            .iter()
            .filter_map(|id| match doc.entity(*id) {
                Some(Entity::Point(p)) if p.name != station.name => Some(p),
                _ => None,
            })
            .collect();
        if points.is_empty() {
            return Err("Select the points to set out.".into());
        }
        let rows = points
            .into_iter()
            .map(|p| {
                let inv = inverse(station.position, p.position);
                Row {
                    name: p.name.clone(),
                    angle: match (inv.bearing, backsight) {
                        (Some(b), Some(bs)) => Some((b.angle() - bs.angle()).normalized()),
                        _ => None,
                    },
                    bearing: inv.bearing,
                    distance: inv.horizontal,
                    dz: (station.height_known && p.height_known).then_some(inv.delta.z),
                    description: p.description.clone(),
                }
            })
            .collect();
        let header = match backsight {
            Some(b) => format!(
                "Station {}  backsight {} ({})",
                station.name,
                bs,
                conv.format_bearing(b)
            ),
            None => format!("Station {}  (no backsight: bearings only)", station.name),
        };
        Ok((header, rows))
    }

    /// Returns a (title, text) report when one is requested.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        targets: &[EntityId],
    ) -> Option<(String, String)> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let conv = doc.conventions().clone();
        let mut report = None;
        let mut open = self.open;
        egui::Window::new("Stake-out sheet")
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                egui::Grid::new("stakeout_setup")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Instrument station");
                        ui.add_sized(
                            [140.0, 22.0],
                            egui::TextEdit::singleline(&mut self.station)
                                .hint_text("point name")
                                .font(fonts::mono(12.0)),
                        );
                        ui.end_row();
                        ui.label("Backsight");
                        ui.add_sized(
                            [200.0, 22.0],
                            egui::TextEdit::singleline(&mut self.backsight)
                                .hint_text("point name or bearing")
                                .font(fonts::mono(12.0)),
                        );
                        ui.end_row();
                    });
                ui.label(
                    RichText::new(
                        "Angles are clockwise from the backsight. The points are the current selection.",
                    )
                    .small()
                    .color(theme.text_secondary),
                );
                ui.separator();
                match self.rows(doc, targets) {
                    Ok((header, rows)) => {
                        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                            egui::Grid::new("stakeout")
                                .num_columns(5)
                                .spacing([14.0, 3.0])
                                .striped(true)
                                .show(ui, |ui| {
                                    for h in ["Point", "Angle", "Bearing", "Distance", "dZ"] {
                                        ui.label(RichText::new(h).small().color(theme.text_secondary));
                                    }
                                    ui.end_row();
                                    for r in &rows {
                                        ui.label(&r.name);
                                        let mono = |t: String| RichText::new(t).font(fonts::mono(12.0));
                                        ui.label(mono(
                                            r.angle.map(|a| conv.format_angle(a)).unwrap_or_default(),
                                        ));
                                        ui.label(mono(
                                            r.bearing.map(|b| conv.format_bearing(b)).unwrap_or_default(),
                                        ));
                                        ui.label(mono(conv.format_length(r.distance)));
                                        ui.label(mono(
                                            r.dz.map(|d| conv.format_length(d)).unwrap_or_default(),
                                        ));
                                        ui.end_row();
                                    }
                                });
                        });
                        if ui.button("Add to reports").clicked() {
                            report = Some(("Stake-out".to_string(), text(&conv, &header, &rows)));
                        }
                    }
                    Err(e) => {
                        ui.label(RichText::new(e).color(theme.text_secondary));
                    }
                }
            });
        self.open = open;
        report
    }
}

fn text(conv: &Conventions, header: &str, rows: &[Row]) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "STAKE-OUT");
    let _ = writeln!(s, "{header}");
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "{:<10} {:>14} {:>16} {:>12} {:>9}  Description",
        "Point", "Angle", "Bearing", "Distance", "dZ"
    );
    for r in rows {
        let _ = writeln!(
            s,
            "{:<10} {:>14} {:>16} {:>12} {:>9}  {}",
            r.name,
            r.angle.map(|a| conv.format_angle(a)).unwrap_or_default(),
            r.bearing.map(|b| conv.format_bearing(b)).unwrap_or_default(),
            conv.format_length(r.distance),
            r.dz.map(|d| conv.format_length(d)).unwrap_or_default(),
            r.description
        );
    }
    s
}
