//! Raw data editor: a file's setups and shots as recorded, with office
//! corrections (station, instrument height, backsight, target, code, target
//! height, excluded shots) typed over them. Corrected values show in the accent
//! colour with the recorded value on hover; each correction is one undoable
//! step, and a reduced file is reduced again with it.

use egui::RichText;
use egui_cad::{CadTheme, fonts};
use survey_core::network::{FieldData, RawEdit};
use survey_model::Document;

/// A cell being typed in: (setup, shot, field) and its text.
type Editing = Option<((usize, Option<usize>, &'static str), String)>;

#[derive(Default)]
pub struct RawDataWindow {
    pub open: bool,
    file: String,
    setup: usize,
    editing: Editing,
}

pub enum RawAction {
    /// Correct (or with `true`, revert) a setup or shot of the named file.
    Edit(String, RawEdit, bool),
    Reduce(String),
    Report(String),
    Network(String),
}

impl RawDataWindow {
    pub fn open_on(&mut self, name: &str) {
        self.open = true;
        if self.file != name {
            self.file = name.to_string();
            self.setup = 0;
            self.editing = None;
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, doc: &Document) -> Option<RawAction> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let conv = doc.conventions().clone();
        let files = doc.field_data();
        if !files.iter().any(|f| f.name == self.file) {
            self.file = files.first().map(|f| f.name.clone()).unwrap_or_default();
            self.setup = 0;
        }
        let mut action = None;
        let mut open = self.open;
        egui::Window::new("Raw data")
            .open(&mut open)
            .default_width(760.0)
            .default_height(480.0)
            .show(ctx, |ui| {
                let Some(f) = files.iter().find(|f| f.name == self.file) else {
                    ui.label(
                        RichText::new(
                            "No raw data yet: import a GSI, JobXML or RW5 file (Survey → Import raw data).",
                        )
                        .color(theme.text_secondary),
                    );
                    return;
                };
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("raw_file")
                        .selected_text(&self.file)
                        .show_ui(ui, |ui| {
                            for g in files {
                                if ui.selectable_label(g.name == self.file, &g.name).clicked() {
                                    self.file = g.name.clone();
                                    self.setup = 0;
                                    self.editing = None;
                                }
                            }
                        });
                    ui.label(RichText::new(&f.format).color(theme.text_secondary));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button("Network adjustment…")
                            .on_hover_text("Adjust the control in this file by least squares")
                            .clicked()
                        {
                            action = Some(RawAction::Network(f.name.clone()));
                        }
                        if ui.button("Report").clicked() {
                            action = Some(RawAction::Report(f.name.clone()));
                        }
                        let reduce = if f.reduced.is_empty() {
                            "Reduce"
                        } else {
                            "Reduce again"
                        };
                        if ui
                            .button(RichText::new(reduce).strong())
                            .on_hover_text(
                                "Orient each setup on its backsight and place its shots as coded points \
                                 (points from an earlier reduction of this file are updated)",
                            )
                            .clicked()
                        {
                            action = Some(RawAction::Reduce(f.name.clone()));
                        }
                    });
                });
                let shots: usize = f.setups.iter().map(|s| s.observations.len()).sum();
                ui.label(
                    RichText::new(format!(
                        "{} setups, {shots} shots, {} corrections; {}",
                        f.setups.len(),
                        f.edits.len(),
                        if f.reduced.is_empty() {
                            "not reduced yet".to_string()
                        } else {
                            format!(
                                "{} points from the last reduction (corrections re-reduce them)",
                                f.reduced.len()
                            )
                        }
                    ))
                    .small()
                    .color(theme.text_secondary),
                );
                for w in &f.warnings {
                    ui.label(RichText::new(w).small().color(theme.warning));
                }
                ui.separator();
                self.setup = self.setup.min(f.setups.len().saturating_sub(1));
                ui.horizontal_top(|ui| {
                    // Setups.
                    ui.vertical(|ui| {
                        ui.set_width(150.0);
                        ui.label(RichText::new("SETUPS").small().color(theme.text_secondary));
                        egui::ScrollArea::vertical().id_salt("raw_setups").show(ui, |ui| {
                            for (i, s) in f.setups.iter().enumerate() {
                                let edited = f.edits.iter().any(|e| e.setup == i);
                                let station = f
                                    .edit_for(i, None)
                                    .and_then(|e| e.station.clone())
                                    .unwrap_or_else(|| s.station.clone());
                                let text = format!("{:>3}  {station}  ({})", i + 1, s.observations.len());
                                let mut t = RichText::new(text).font(fonts::mono(12.0));
                                if edited {
                                    t = t.color(theme.accent);
                                }
                                if ui.selectable_label(self.setup == i, t).clicked() {
                                    self.setup = i;
                                    self.editing = None;
                                }
                            }
                        });
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        if let Some(a) = self.setup_panel(ui, &theme, &conv, f) {
                            action = Some(a);
                        }
                    });
                });
            });
        self.open = open;
        action
    }

    fn setup_panel(
        &mut self,
        ui: &mut egui::Ui,
        theme: &CadTheme,
        conv: &survey_core::Conventions,
        f: &FieldData,
    ) -> Option<RawAction> {
        let i = self.setup;
        let s = f.setups.get(i)?;
        let mut action = None;
        let file = f.name.clone();
        let se = f.edit_for(i, None);
        let edit = |e: RawEdit| RawAction::Edit(file.clone(), e, false);
        egui::Grid::new("raw_setup")
            .num_columns(6)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("Station");
                let v = se
                    .and_then(|e| e.station.clone())
                    .unwrap_or_else(|| s.station.clone());
                if let Some(t) = cell(
                    ui,
                    theme,
                    &mut self.editing,
                    (i, None, "station"),
                    &v,
                    se.and_then(|e| e.station.as_ref()).map(|_| s.station.clone()),
                    110.0,
                ) {
                    action = Some(edit(RawEdit {
                        setup: i,
                        station: Some(t.trim().to_string()),
                        ..Default::default()
                    }));
                }
                ui.label("Instrument height");
                let v = se
                    .and_then(|e| e.instrument_height)
                    .unwrap_or(s.instrument_height);
                if let Some(t) = cell(
                    ui,
                    theme,
                    &mut self.editing,
                    (i, None, "hi"),
                    &conv.format_length(v),
                    se.and_then(|e| e.instrument_height)
                        .map(|_| conv.format_length(s.instrument_height)),
                    70.0,
                ) {
                    match conv.parse_length(&t) {
                        Ok(h) => {
                            action = Some(edit(RawEdit {
                                setup: i,
                                instrument_height: Some(h),
                                ..Default::default()
                            }));
                        }
                        Err(_) => self.editing = None,
                    }
                }
                ui.label("Backsight");
                let v = se
                    .and_then(|e| e.backsight.clone())
                    .or_else(|| s.backsight.clone())
                    .unwrap_or_default();
                if let Some(t) = cell(
                    ui,
                    theme,
                    &mut self.editing,
                    (i, None, "backsight"),
                    &v,
                    se.and_then(|e| e.backsight.as_ref())
                        .map(|_| s.backsight.clone().unwrap_or_else(|| "(none)".into())),
                    90.0,
                ) {
                    action = Some(edit(RawEdit {
                        setup: i,
                        backsight: Some(t.trim().to_string()),
                        ..Default::default()
                    }));
                }
                ui.end_row();
            });
        ui.horizontal(|ui| {
            if se.is_some() && ui.small_button("Revert setup").clicked() {
                action = Some(RawAction::Edit(
                    file.clone(),
                    RawEdit {
                        setup: i,
                        ..Default::default()
                    },
                    true,
                ));
            }
            ui.label(
                RichText::new(
                    "Blank backsight: orient on the first shot to a known point. Type over a value to correct it.",
                )
                .small()
                .color(theme.text_secondary),
            );
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical().id_salt("raw_shots").show(ui, |ui| {
            egui::Grid::new("raw_shots_grid")
                .num_columns(9)
                .spacing([8.0, 2.0])
                .striped(true)
                .show(ui, |ui| {
                    for h in [
                        "#",
                        "Target",
                        "Code",
                        "Target ht",
                        "Horizontal",
                        "Zenith",
                        "Distance",
                        "Use",
                        "",
                    ] {
                        ui.label(RichText::new(h).small().color(theme.text_secondary));
                    }
                    ui.end_row();
                    for (k, o) in s.observations.iter().enumerate() {
                        let e = f.edit_for(i, Some(k));
                        let shot = |e: RawEdit| RawAction::Edit(file.clone(), e, false);
                        ui.label(RichText::new(format!("{}", k + 1)).font(fonts::mono(12.0)));
                        let v = e
                            .and_then(|e| e.target.clone())
                            .unwrap_or_else(|| o.target.clone());
                        if let Some(t) = cell(
                            ui,
                            theme,
                            &mut self.editing,
                            (i, Some(k), "target"),
                            &v,
                            e.and_then(|e| e.target.as_ref()).map(|_| o.target.clone()),
                            80.0,
                        ) && !t.trim().is_empty()
                        {
                            action = Some(shot(RawEdit {
                                setup: i,
                                observation: Some(k),
                                target: Some(t.trim().to_string()),
                                ..Default::default()
                            }));
                        }
                        let v = e.and_then(|e| e.code.clone()).unwrap_or_else(|| o.code.clone());
                        if let Some(t) = cell(
                            ui,
                            theme,
                            &mut self.editing,
                            (i, Some(k), "code"),
                            &v,
                            e.and_then(|e| e.code.as_ref()).map(|_| format!("'{}'", o.code)),
                            110.0,
                        ) {
                            action = Some(shot(RawEdit {
                                setup: i,
                                observation: Some(k),
                                code: Some(t.trim().to_string()),
                                ..Default::default()
                            }));
                        }
                        let v = e.and_then(|e| e.target_height).unwrap_or(o.target_height);
                        if let Some(t) = cell(
                            ui,
                            theme,
                            &mut self.editing,
                            (i, Some(k), "ht"),
                            &conv.format_length(v),
                            e.and_then(|e| e.target_height)
                                .map(|_| conv.format_length(o.target_height)),
                            60.0,
                        ) {
                            match conv.parse_length(&t) {
                                Ok(h) => {
                                    action = Some(shot(RawEdit {
                                        setup: i,
                                        observation: Some(k),
                                        target_height: Some(h),
                                        ..Default::default()
                                    }));
                                }
                                Err(_) => self.editing = None,
                            }
                        }
                        let mono = |t: String| RichText::new(t).font(fonts::mono(12.0));
                        ui.label(mono(
                            o.hz.map(|a| conv.format_angle(a)).unwrap_or_else(|| "-".into()),
                        ));
                        ui.label(mono(
                            o.zenith
                                .map(|a| conv.format_angle(a))
                                .unwrap_or_else(|| "-".into()),
                        ));
                        ui.label(mono(
                            o.distance
                                .map(|d| conv.format_length(d))
                                .unwrap_or_else(|| "-".into()),
                        ));
                        let excluded = e.and_then(|e| e.excluded).unwrap_or(false);
                        let mut used = !excluded;
                        if ui
                            .checkbox(&mut used, "")
                            .on_hover_text("Untick to leave the shot out of reductions and adjustments")
                            .changed()
                        {
                            action = Some(shot(RawEdit {
                                setup: i,
                                observation: Some(k),
                                excluded: Some(!used),
                                ..Default::default()
                            }));
                        }
                        if e.is_some() {
                            if ui
                                .small_button("↺")
                                .on_hover_text("Back to the recorded values")
                                .clicked()
                            {
                                action = Some(RawAction::Edit(
                                    file.clone(),
                                    RawEdit {
                                        setup: i,
                                        observation: Some(k),
                                        ..Default::default()
                                    },
                                    true,
                                ));
                            }
                        } else {
                            ui.label("");
                        }
                        ui.end_row();
                    }
                });
        });
        action
    }
}

/// An editable cell showing `value`; `recorded` is the value as recorded
/// when a correction is in force (drawn in the accent colour, shown on hover).
/// Returns the new text when an edit is committed (Enter or leaving the cell).
fn cell(
    ui: &mut egui::Ui,
    theme: &CadTheme,
    editing: &mut Editing,
    key: (usize, Option<usize>, &'static str),
    value: &str,
    recorded: Option<String>,
    width: f32,
) -> Option<String> {
    let mut text = match editing {
        Some((k, t)) if *k == key => t.clone(),
        _ => value.to_string(),
    };
    let mut edit = egui::TextEdit::singleline(&mut text).font(fonts::mono(12.0));
    if recorded.is_some() {
        edit = edit.text_color(theme.accent);
    }
    let mut r = ui.add_sized([width, 18.0], edit);
    if let Some(rec) = &recorded {
        r = r.on_hover_text(format!("Recorded: {rec}"));
    }
    if r.changed() {
        *editing = Some((key, text.clone()));
    }
    if r.lost_focus()
        && let Some((k, t)) = editing.take()
    {
        if k == key && t != value {
            return Some(t);
        }
        if k != key {
            *editing = Some((k, t));
        }
    }
    None
}
