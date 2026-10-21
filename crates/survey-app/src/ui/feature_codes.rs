//! Feature code library editor and field-to-finish processing.
//!
//! Edits are made to a working copy; "Apply" stores it in the project as one
//! undoable change, so typing does not flood the undo history.

use egui::RichText;
use egui_cad::chrome::{IconButton, switch_row};
use egui_cad::{CadTheme, fonts, icons};
use survey_core::field_to_finish::{FeatureCode, FeatureLibrary};
use survey_model::Document;

#[derive(Default)]
pub struct FeatureCodesWindow {
    pub open: bool,
    /// Working copy; `None` until opened (then initialised from the project).
    working: Option<FeatureLibrary>,
    only_selected: bool,
    assign_layers: bool,
}

pub enum CodesAction {
    /// Store the library in the project.
    Apply(FeatureLibrary),
    /// Store the library (if changed) and build linework.
    Process {
        library: FeatureLibrary,
        only_selected: bool,
        assign_layers: bool,
    },
    Import,
    /// Save the code sheet (CSV text).
    Export(String),
}

impl FeatureCodesWindow {
    pub fn open(&mut self) {
        self.open = true;
    }

    /// Merges imported codes into the working copy (same code → replaced).
    pub fn merge(&mut self, doc: &Document, codes: Vec<FeatureCode>) -> usize {
        let lib = self
            .working
            .get_or_insert_with(|| doc.meta().feature_codes.clone());
        let n = codes.len();
        for c in codes {
            match lib
                .codes
                .iter_mut()
                .find(|e| e.code.eq_ignore_ascii_case(&c.code))
            {
                Some(existing) => *existing = c,
                None => lib.codes.push(c),
            }
        }
        self.open = true;
        n
    }

    pub fn show(&mut self, ctx: &egui::Context, doc: &Document, has_selection: bool) -> Option<CodesAction> {
        if !self.open {
            // Pick up undo/redo or other changes next time it opens.
            self.working = None;
            return None;
        }
        let theme = CadTheme::current(ctx);
        let stored = &doc.meta().feature_codes;
        let mut action = None;
        let mut open = self.open;
        let lib = self.working.get_or_insert_with(|| stored.clone());
        let changed = lib != stored;
        egui::Window::new("Feature codes")
            .open(&mut open)
            .default_size([700.0, 520.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button(format!("{} Add code", icons::PLUS)).clicked() {
                        lib.codes.push(FeatureCode {
                            code: String::new(),
                            description: String::new(),
                            layer: String::new(),
                            color: [200, 200, 200],
                            linework: true,
                        });
                    }
                    if ui
                        .button("Add examples")
                        .on_hover_text("Adds a small generic set of codes that are not already defined")
                        .clicked()
                    {
                        for c in FeatureLibrary::example().codes {
                            if lib.find(&c.code).is_none() {
                                lib.codes.push(c);
                            }
                        }
                    }
                    if ui.button("Import CSV…").clicked() {
                        action = Some(CodesAction::Import);
                    }
                    if ui.button("Export CSV…").clicked() {
                        action = Some(CodesAction::Export(survey_io::code_sheet::write_code_sheet(
                            &lib.codes,
                        )));
                    }
                });
                ui.add_space(6.0);
                codes_table(ui, &theme, lib);
                ui.add_space(8.0);
                ui.label(
                    RichText::new("CONTROL CODES")
                        .font(fonts::medium(11.0))
                        .color(theme.text_secondary),
                );
                ui.horizontal_wrapped(|ui| {
                    let c = &mut lib.controls;
                    for (label, value) in [
                        ("Start", &mut c.start),
                        ("End", &mut c.end),
                        ("Close", &mut c.close),
                        ("Arc start", &mut c.arc_start),
                        ("Arc end", &mut c.arc_end),
                    ] {
                        ui.label(label);
                        ui.add_sized(
                            [54.0, 22.0],
                            egui::TextEdit::singleline(value).font(fonts::mono(12.0)),
                        );
                        ui.add_space(6.0);
                    }
                });
                ui.label(
                    RichText::new(
                        "Point codes: CODE[string no.] [controls…], e.g. KB1, KB1 ST, BLD CL, EP PC … EP PT.",
                    )
                    .small()
                    .color(theme.text_secondary),
                );
                ui.add_space(8.0);
                ui.separator();
                ui.add_enabled_ui(has_selection, |ui| {
                    switch_row(ui, &mut self.only_selected, "Only the selected points");
                });
                switch_row(
                    ui,
                    &mut self.assign_layers,
                    "Move points onto their feature's layer",
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(changed, egui::Button::new("Apply changes"))
                        .clicked()
                    {
                        action = Some(CodesAction::Apply(lib.clone()));
                    }
                    if ui
                        .button(format!("{} Process linework", icons::MAGIC_WAND))
                        .on_hover_text("Joins coded points into strings; replaces linework processed earlier")
                        .clicked()
                    {
                        action = Some(CodesAction::Process {
                            library: lib.clone(),
                            only_selected: self.only_selected && has_selection,
                            assign_layers: self.assign_layers,
                        });
                    }
                    if changed {
                        ui.label(RichText::new("● unapplied changes").small().color(theme.warning));
                    }
                });
            });
        self.open = open;
        action
    }
}

fn codes_table(ui: &mut egui::Ui, theme: &CadTheme, lib: &mut FeatureLibrary) {
    if lib.codes.is_empty() {
        ui.label(
            RichText::new(
                "No feature codes yet — add some, import a code sheet, or start from the examples.",
            )
            .color(theme.text_secondary),
        );
        return;
    }
    let mut remove = None;
    egui::ScrollArea::vertical()
        .max_height(280.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Grid::new("feature_codes")
                .num_columns(6)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    for h in ["Code", "Description", "Layer", "Colour", "Lines", ""] {
                        ui.label(RichText::new(h).small().color(theme.text_secondary));
                    }
                    ui.end_row();
                    for (i, c) in lib.codes.iter_mut().enumerate() {
                        ui.add_sized(
                            [70.0, 22.0],
                            egui::TextEdit::singleline(&mut c.code).font(fonts::mono(12.0)),
                        );
                        ui.add_sized([220.0, 22.0], egui::TextEdit::singleline(&mut c.description));
                        ui.add_sized([130.0, 22.0], egui::TextEdit::singleline(&mut c.layer));
                        ui.color_edit_button_srgb(&mut c.color);
                        ui.checkbox(&mut c.linework, "")
                            .on_hover_text("Join points with this code into lines");
                        if IconButton::new(icons::X, "Remove code")
                            .size(20.0)
                            .show(ui)
                            .clicked()
                        {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
        });
    if let Some(i) = remove {
        lib.codes.remove(i);
    }
}
