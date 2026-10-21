//! Feature code library editor and field-to-finish processing.
//!
//! Edits are made to a working copy; "Apply" stores it in the project as one
//! undoable change, so typing does not flood the undo history.

use egui::RichText;
use egui_cad::chrome::{IconButton, segmented, switch_row};
use egui_cad::symbols::{line_style_picker, paint_line_sample, symbol_picker};
use egui_cad::{CadTheme, fonts, icons};
use survey_core::field_to_finish::{FeatureCode, FeatureLibrary};
use survey_core::style::LabelSet;
use survey_model::Document;

#[derive(Default)]
pub struct FeatureCodesWindow {
    pub open: bool,
    /// Working copy; `None` until opened (then initialised from the project).
    working: Option<FeatureLibrary>,
    only_selected: bool,
    assign_layers: bool,
    /// Code whose style is being edited.
    editing: Option<usize>,
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
                        lib.codes
                            .push(FeatureCode::new("", "", "", [200, 200, 200], true));
                        self.editing = Some(lib.codes.len() - 1);
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
                codes_table(ui, &theme, lib, &mut self.editing);
                if let Some(i) = self.editing.filter(|i| *i < lib.codes.len()) {
                    ui.add_space(6.0);
                    style_editor(ui, &theme, &mut lib.codes[i]);
                }
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
                        "Point codes: CODE[string no.] [controls…], e.g. KB1, KB1 ST, BLD CL, EP PC … EP PT. \
                         Codes may use * and ? wildcards (MH*), matched when no plain code fits.",
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

fn codes_table(ui: &mut egui::Ui, theme: &CadTheme, lib: &mut FeatureLibrary, editing: &mut Option<usize>) {
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
                .num_columns(8)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    for h in [
                        "Code",
                        "Description",
                        "Layer",
                        "Colour",
                        "Symbol",
                        "Lines",
                        "",
                        "",
                    ] {
                        ui.label(RichText::new(h).small().color(theme.text_secondary));
                    }
                    ui.end_row();
                    for (i, c) in lib.codes.iter_mut().enumerate() {
                        ui.add_sized(
                            [70.0, 22.0],
                            egui::TextEdit::singleline(&mut c.code).font(fonts::mono(12.0)),
                        );
                        ui.add_sized([200.0, 22.0], egui::TextEdit::singleline(&mut c.description));
                        ui.add_sized([120.0, 22.0], egui::TextEdit::singleline(&mut c.layer));
                        ui.color_edit_button_srgb(&mut c.color);
                        let [r, g, b] = c.color;
                        symbol_picker(
                            ui,
                            ("code_symbol", i),
                            &mut c.style.symbol,
                            egui::Color32::from_rgb(r, g, b),
                        );
                        ui.checkbox(&mut c.linework, "")
                            .on_hover_text("Join points with this code into lines");
                        let open = *editing == Some(i);
                        if IconButton::new(icons::SLIDERS_HORIZONTAL, "Style")
                            .description("Symbol size, colour, line style and labels")
                            .selected(open)
                            .size(20.0)
                            .show(ui)
                            .clicked()
                        {
                            *editing = if open { None } else { Some(i) };
                        }
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
        *editing = None;
    }
}

/// Display style of one code.
fn style_editor(ui: &mut egui::Ui, theme: &CadTheme, c: &mut FeatureCode) {
    let [r, g, b] = c.color;
    let color = egui::Color32::from_rgb(r, g, b);
    egui::Frame::new()
        .fill(theme.bg)
        .stroke(egui::Stroke::new(1.0, theme.border))
        .corner_radius(theme.radius)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!(
                    "STYLE — {}",
                    if c.code.is_empty() { "new code" } else { &c.code }
                ))
                .font(fonts::medium(11.0))
                .color(theme.text_secondary),
            );
            egui::Grid::new("code_style")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Symbol");
                    ui.horizontal(|ui| {
                        symbol_picker(ui, "code_style_symbol", &mut c.style.symbol, color);
                        ui.label(RichText::new(c.style.symbol.label()).small());
                        ui.add(
                            egui::DragValue::new(&mut c.style.symbol_size)
                                .range(3.0..=40.0)
                                .speed(0.2)
                                .suffix(" pt"),
                        );
                    });
                    ui.end_row();
                    ui.label("Colour");
                    segmented(
                        ui,
                        &mut c.style.by_layer,
                        &[(true, "Feature's layer"), (false, "Feature colour")],
                    );
                    ui.end_row();
                    ui.label("");
                    ui.label(
                        RichText::new(if c.style.by_layer {
                            format!(
                                "Uses the colour of layer {} (or this colour until it exists)",
                                c.layer
                            )
                        } else {
                            "Always uses this colour, whatever layer the object is on".to_string()
                        })
                        .small()
                        .color(theme.text_secondary),
                    );
                    ui.end_row();
                    ui.label("Lines");
                    ui.horizontal(|ui| {
                        line_style_picker(ui, "code_style_line", &mut c.style.line_style, color);
                        ui.add(
                            egui::DragValue::new(&mut c.style.line_weight)
                                .range(0.5..=8.0)
                                .speed(0.1)
                                .suffix(" pt"),
                        );
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(80.0, 18.0), egui::Sense::hover());
                        paint_line_sample(ui.painter(), rect, color, c.style.line_weight, c.style.line_style);
                    });
                    ui.end_row();
                    ui.label("Labels");
                    ui.horizontal(|ui| {
                        let mut custom = c.style.labels.is_some();
                        segmented(ui, &mut custom, &[(false, "As settings"), (true, "Custom")]);
                        if custom {
                            let set = c.style.labels.get_or_insert(LabelSet {
                                name: true,
                                ..Default::default()
                            });
                            ui.checkbox(&mut set.name, "Name");
                            ui.checkbox(&mut set.code, "Code");
                            ui.checkbox(&mut set.height, "Height");
                            ui.checkbox(&mut set.description, "Description");
                        } else {
                            c.style.labels = None;
                        }
                    });
                    ui.end_row();
                });
        });
}
