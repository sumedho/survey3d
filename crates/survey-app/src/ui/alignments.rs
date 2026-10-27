//! Construction alignments: list, element tables and set-out.

use egui::RichText;
use egui_cad::chrome::{IconButton, switch_row};
use egui_cad::{CadTheme, fonts, icons};
use survey_core::alignment::{SetoutPoint, Transition};
use survey_model::{AlignmentRecord, Document, EntityId, RecordId};

pub use crate::compute::alignment::{
    arcs, centreline, element_report, geometry, offset_label, setout_report,
};

pub enum AlignmentsAction {
    Create,
    Update(RecordId, AlignmentRecord),
    Delete(RecordId),
    Select(EntityId),
    Report(String, String),
    /// Create set-out points: alignment, points, code.
    SetOut(RecordId, Vec<SetoutPoint>, String),
    /// Draw the centreline with its transitions as a new polyline.
    DrawCentreline(RecordId),
}

pub struct AlignmentsWindow {
    pub open: bool,
    active: Option<RecordId>,
    editing: Option<(RecordId, String)>,
    interval: f64,
    offsets: String,
    key_points: bool,
    code: String,
}

impl Default for AlignmentsWindow {
    fn default() -> Self {
        Self {
            open: false,
            active: None,
            editing: None,
            interval: 10.0,
            offsets: "0".into(),
            key_points: true,
            code: "SO".into(),
        }
    }
}

impl AlignmentsWindow {
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        can_create: bool,
    ) -> Option<AlignmentsAction> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let conv = doc.conventions().clone();
        let list = doc.alignments();
        let mut action = None;
        let mut open = self.open;
        egui::Window::new("Alignments")
            .open(&mut open)
            .default_width(620.0)
            .show(ctx, |ui| {
                if ui
                    .add_enabled(can_create, egui::Button::new(format!("{} From selected polyline", icons::PATH)))
                    .on_disabled_hover_text("Select the centreline polyline")
                    .clicked()
                {
                    action = Some(AlignmentsAction::Create);
                }
                ui.add_space(6.0);
                if list.is_empty() {
                    ui.label(
                        RichText::new("No alignments yet. Select a centreline polyline (straights and arcs) and click “From selected polyline”.")
                            .color(theme.text_secondary),
                    );
                    return;
                }
                egui::Grid::new("alignments").num_columns(7).spacing([10.0, 4.0]).striped(true).show(ui, |ui| {
                    for h in ["Name", "Start ch", "Length", "Ticks every", "Show", "", ""] {
                        ui.label(RichText::new(h).small().color(theme.text_secondary));
                    }
                    ui.end_row();
                    for a in list {
                        let i = a.id;
                        let mut name = match &self.editing {
                            Some((k, n)) if *k == i => n.clone(),
                            _ => a.name.clone(),
                        };
                        let r = ui.add_sized([110.0, 22.0], egui::TextEdit::singleline(&mut name));
                        if r.changed() {
                            self.editing = Some((i, name.clone()));
                        }
                        if r.lost_focus()
                            && let Some((k, n)) = self.editing.take()
                            && k == i
                            && n != a.name
                        {
                            action = Some(AlignmentsAction::Update(i, AlignmentRecord { name: n, ..a.record.clone() }));
                        }
                        let mut start = a.start_chainage;
                        if ui.add(egui::DragValue::new(&mut start).speed(1.0).max_decimals(3)).changed() {
                            action = Some(AlignmentsAction::Update(i, AlignmentRecord { start_chainage: start, ..a.record.clone() }));
                        }
                        let length = match geometry(doc, a) {
                            None => "missing".to_string(),
                            Some(Ok(g)) => conv.format_length_with_unit(g.length()),
                            Some(Err(_)) => "transition error".to_string(),
                        };
                        ui.label(RichText::new(length).font(fonts::mono(12.0)));
                        let mut tick = a.tick_interval;
                        if ui
                            .add(egui::DragValue::new(&mut tick).range(0.1..=10_000.0).speed(0.5).suffix(" m"))
                            .changed()
                        {
                            action = Some(AlignmentsAction::Update(i, AlignmentRecord { tick_interval: tick, ..a.record.clone() }));
                        }
                        let mut show = a.show_chainages;
                        if ui.checkbox(&mut show, "").changed() {
                            action = Some(AlignmentsAction::Update(i, AlignmentRecord { show_chainages: show, ..a.record.clone() }));
                        }
                        ui.horizontal(|ui| {
                            if ui.selectable_label(self.active == Some(i), "Set out").clicked() {
                                self.active = if self.active == Some(i) { None } else { Some(i) };
                            }
                            if IconButton::new(icons::CROSSHAIR, "Select centreline").size(20.0).show(ui).clicked() {
                                action = Some(AlignmentsAction::Select(a.polyline));
                            }
                            if IconButton::new(icons::FILE_TEXT, "Element report").size(20.0).show(ui).clicked()
                                && let Some(text) = element_report(doc, a)
                            {
                                action = Some(AlignmentsAction::Report(format!("Alignment {}", a.name), text));
                            }
                        });
                        if IconButton::new(icons::X, "Remove alignment (keeps the polyline)").size(20.0).show(ui).clicked() {
                            action = Some(AlignmentsAction::Delete(i));
                        }
                        ui.end_row();
                    }
                });
                // The alignment may have gone (undo, delete).
                let Some(a) = self.active.and_then(|i| list.iter().find(|a| a.id == i)) else {
                    self.active = None;
                    return;
                };
                let i = a.id;
                let Some(l) = centreline(doc, a) else {
                    return;
                };
                ui.add_space(6.0);
                ui.separator();
                let curves = arcs(l);
                if !curves.is_empty() {
                    ui.label(RichText::new(format!("TRANSITIONS — {}", a.name)).font(fonts::medium(11.0)).color(theme.text_secondary));
                    ui.label(
                        RichText::new("Clothoid spirals into and out of each arc (0 for none). The arc keeps its radius and moves inwards; its tangent points move back along the straights.")
                            .small()
                            .color(theme.text_secondary),
                    );
                    egui::Grid::new("transitions").num_columns(4).spacing([10.0, 4.0]).show(ui, |ui| {
                        for h in ["Arc", "Radius", "Spiral in", "Spiral out"] {
                            ui.label(RichText::new(h).small().color(theme.text_secondary));
                        }
                        ui.end_row();
                        for (seg, radius, right) in &curves {
                            let current = a.transitions.iter().find(|t| t.segment == *seg).copied().unwrap_or(Transition {
                                segment: *seg,
                                ..Default::default()
                            });
                            ui.label(format!("Segment {} ({})", seg + 1, if *right { "right" } else { "left" }));
                            ui.label(RichText::new(conv.format_length(*radius)).font(fonts::mono(12.0)));
                            let mut t = current;
                            let changed = ui.add(egui::DragValue::new(&mut t.entry).range(0.0..=10_000.0).speed(0.5).suffix(" m")).changed()
                                | ui.add(egui::DragValue::new(&mut t.exit).range(0.0..=10_000.0).speed(0.5).suffix(" m")).changed();
                            if changed {
                                let mut record = a.record.clone();
                                record.transitions.retain(|x| x.segment != *seg);
                                if t.entry > 0.0 || t.exit > 0.0 {
                                    record.transitions.push(t);
                                    record.transitions.sort_by_key(|x| x.segment);
                                }
                                action = Some(AlignmentsAction::Update(i, record));
                            }
                            ui.end_row();
                        }
                    });
                    match geometry(doc, a) {
                        Some(Err(e)) => {
                            ui.label(RichText::new(e).color(theme.error));
                        }
                        Some(Ok(_))
                            if !a.transitions.is_empty()
                                && ui
                                    .button("Draw centreline with transitions")
                                    .on_hover_text("A new polyline following the spirals, for plans and DXF (the alignment keeps its own)")
                                    .clicked() =>
                        {
                            action = Some(AlignmentsAction::DrawCentreline(i));
                        }
                        _ => {}
                    }
                    ui.separator();
                }
                ui.label(RichText::new(format!("SET OUT — {}", a.name)).font(fonts::medium(11.0)).color(theme.text_secondary));
                ui.horizontal(|ui| {
                    ui.label("Every");
                    ui.add(egui::DragValue::new(&mut self.interval).range(0.0..=10_000.0).speed(0.5).suffix(" m"));
                    ui.label("Offsets");
                    ui.add_sized(
                        [120.0, 22.0],
                        egui::TextEdit::singleline(&mut self.offsets)
                            .hint_text("-5, 0, 5")
                            .font(fonts::mono(12.0)),
                    )
                    .on_hover_text("Negative left, positive right of the direction of travel");
                    ui.label("Code");
                    ui.add_sized([50.0, 22.0], egui::TextEdit::singleline(&mut self.code).font(fonts::mono(12.0)));
                });
                switch_row(ui, &mut self.key_points, "Include tangent points (TP, TS, SC, CS, ST)");
                let offsets: Result<Vec<f64>, String> = self
                    .offsets
                    .split([',', ' '])
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(|t| conv.parse_length(t).map_err(|e| format!("Offset '{t}': {e}")))
                    .collect();
                match offsets {
                    Ok(offsets) if !offsets.is_empty() => {
                        let Some(Ok(g)) = geometry(doc, a) else {
                            return;
                        };
                        let pts = g.setout(self.interval, &offsets, self.key_points);
                        ui.label(RichText::new(format!("{} points", pts.len())).color(theme.text_secondary));
                        ui.horizontal(|ui| {
                            if ui.add_enabled(!pts.is_empty(), egui::Button::new("Create points")).clicked() {
                                action = Some(AlignmentsAction::SetOut(i, pts.clone(), self.code.trim().to_string()));
                            }
                        });
                    }
                    Ok(_) => {
                        ui.label(RichText::new("Enter at least one offset (0 for the centreline).").color(theme.text_secondary));
                    }
                    Err(e) => {
                        ui.label(RichText::new(e).color(theme.error));
                    }
                }
            });
        self.open = open;
        action
    }
}
