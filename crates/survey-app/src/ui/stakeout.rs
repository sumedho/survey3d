//! Stake-out sheet: from an instrument station and backsight, the horizontal
//! angle and distance to each point to be set out.

use egui::RichText;
use egui_cad::{CadTheme, fonts};
use survey_model::{Document, EntityId};

use crate::compute::setout::{Setout, setout};

#[derive(Default)]
pub struct StakeoutWindow {
    pub open: bool,
    station: String,
    backsight: String,
    hi: String,
    ht: String,
}

impl StakeoutWindow {
    fn compute(&self, doc: &Document, targets: &[EntityId]) -> Result<Setout, String> {
        let conv = doc.conventions();
        let height = |text: &str, what: &str| {
            if text.trim().is_empty() {
                Ok(0.0)
            } else {
                conv.parse_length(text).map_err(|e| format!("{what}: {e}"))
            }
        };
        setout(
            doc,
            &self.station,
            &self.backsight,
            targets,
            height(&self.hi, "Instrument height")?,
            height(&self.ht, "Target height")?,
        )
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
                        ui.label("Instrument / target height");
                        ui.horizontal(|ui| {
                            for (text, hint) in [(&mut self.hi, "HI"), (&mut self.ht, "HT")] {
                                ui.add_sized(
                                    [64.0, 22.0],
                                    egui::TextEdit::singleline(text)
                                        .hint_text(hint)
                                        .font(fonts::mono(12.0)),
                                );
                            }
                        });
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
                match self.compute(doc, targets) {
                    Ok(set) => {
                        ui.label(RichText::new(set.header(&conv)).small());
                        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                            egui::Grid::new("stakeout")
                                .num_columns(7)
                                .spacing([14.0, 3.0])
                                .striped(true)
                                .show(ui, |ui| {
                                    for h in
                                        ["Point", "Angle", "Bearing", "Distance", "dZ", "Slope", "Zenith"]
                                    {
                                        ui.label(RichText::new(h).small().color(theme.text_secondary));
                                    }
                                    ui.end_row();
                                    for r in &set.rows {
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
                                        ui.label(mono(
                                            r.slope.map(|d| conv.format_length(d)).unwrap_or_default(),
                                        ));
                                        ui.label(mono(
                                            r.zenith.map(|z| conv.format_angle(z)).unwrap_or_default(),
                                        ));
                                        ui.end_row();
                                    }
                                });
                        });
                        if ui.button("Add to reports").clicked() {
                            report = Some(("Stake-out".to_string(), set.report(&conv)));
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
