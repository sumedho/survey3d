//! Delimited-text point import dialog with live preview.

use egui::RichText;
use egui_cad::CadTheme;
use survey_core::conventions::LinearUnit;
use survey_io::{
    CsvField, CsvLayout, CsvPointImporter, HeaderMode, ImportedPoint, PointImport, PointImporter,
};
use survey_model::{Color, Document, Entity, SurveyPoint};

use crate::import_command::{ImportEntities, LayerRef};

pub struct CsvImportDialog {
    file_name: String,
    bytes: Vec<u8>,
    layout: CsvLayout,
    preview: Result<PointImport, String>,
    new_layer: bool,
    new_layer_name: String,
}

pub enum CsvDialogResult {
    Open,
    Cancelled,
    Import(Box<ImportEntities>, usize),
}

const DELIMITERS: [(Option<char>, &str); 5] = [
    (None, "Auto"),
    (Some(','), "Comma"),
    (Some('\t'), "Tab"),
    (Some(';'), "Semicolon"),
    (Some(' '), "Space"),
];

impl CsvImportDialog {
    pub fn new(file_name: String, bytes: Vec<u8>, layout: CsvLayout) -> Self {
        let stem = file_name
            .rsplit_once('.')
            .map_or(file_name.as_str(), |(s, _)| s)
            .to_string();
        let mut d = Self {
            file_name,
            bytes,
            layout,
            preview: Err(String::new()),
            new_layer: false,
            new_layer_name: stem,
        };
        d.refresh();
        d
    }

    pub fn layout(&self) -> &CsvLayout {
        &self.layout
    }

    fn refresh(&mut self) {
        // A single space delimiter means "whitespace" for the importer.
        let mut layout = self.layout.clone();
        if layout.delimiter == Some(' ') {
            layout.delimiter = None;
        }
        self.preview = CsvPointImporter::new(layout)
            .import(&self.bytes)
            .map_err(|e| e.to_string());
    }

    pub fn show(&mut self, ctx: &egui::Context, doc: &Document) -> CsvDialogResult {
        let theme = CadTheme::current(ctx);
        let mut open = true;
        let mut result = CsvDialogResult::Open;
        egui::Window::new(format!("Import points — {}", self.file_name))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .show(ctx, |ui| {
                let mut changed = false;
                egui::Grid::new("csv_opts")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Preset");
                        egui::ComboBox::from_id_salt("csv_preset")
                            .selected_text(self.layout.letters())
                            .show_ui(ui, |ui| {
                                for (name, preset) in CsvLayout::presets() {
                                    if ui.selectable_label(self.layout.letters() == name, name).clicked() {
                                        self.layout.fields = preset.fields;
                                        changed = true;
                                    }
                                }
                            });
                        ui.end_row();

                        ui.label("Columns");
                        ui.horizontal_wrapped(|ui| {
                            let mut remove = None;
                            for (i, field) in self.layout.fields.iter_mut().enumerate() {
                                egui::ComboBox::from_id_salt(("csv_col", i))
                                    .width(92.0)
                                    .selected_text(field.label())
                                    .show_ui(ui, |ui| {
                                        for f in CsvField::ALL {
                                            changed |= ui.selectable_value(field, f, f.label()).changed();
                                        }
                                    });
                                if ui.small_button("×").on_hover_text("Remove column").clicked() {
                                    remove = Some(i);
                                }
                            }
                            if let Some(i) = remove {
                                self.layout.fields.remove(i);
                                changed = true;
                            }
                            if ui.small_button("+").on_hover_text("Add column").clicked() {
                                self.layout.fields.push(CsvField::Skip);
                                changed = true;
                            }
                        });
                        ui.end_row();

                        ui.label("Delimiter");
                        ui.horizontal(|ui| {
                            for (d, label) in DELIMITERS {
                                changed |= ui
                                    .selectable_value(&mut self.layout.delimiter, d, label)
                                    .changed();
                            }
                        });
                        ui.end_row();

                        ui.label("Header row");
                        ui.horizontal(|ui| {
                            for (m, label) in [
                                (HeaderMode::Auto, "Auto"),
                                (HeaderMode::Yes, "Yes"),
                                (HeaderMode::No, "No"),
                            ] {
                                changed |= ui.selectable_value(&mut self.layout.header, m, label).changed();
                            }
                        });
                        ui.end_row();

                        ui.label("Units");
                        egui::ComboBox::from_id_salt("csv_units")
                            .selected_text(self.layout.unit.label())
                            .show_ui(ui, |ui| {
                                for u in LinearUnit::ALL {
                                    changed |=
                                        ui.selectable_value(&mut self.layout.unit, u, u.label()).changed();
                                }
                            });
                        ui.end_row();

                        ui.label("Layer");
                        ui.horizontal(|ui| {
                            let current = doc
                                .layer(doc.current_layer())
                                .map_or("current", |l| l.name.as_str());
                            ui.radio_value(&mut self.new_layer, false, format!("Current ({current})"));
                            ui.radio_value(&mut self.new_layer, true, "New:");
                            ui.add_enabled(
                                self.new_layer,
                                egui::TextEdit::singleline(&mut self.new_layer_name).desired_width(140.0),
                            );
                        });
                        ui.end_row();
                    });
                if changed {
                    self.refresh();
                }

                ui.separator();
                match &self.preview {
                    Ok(import) => {
                        ui.label(format!("{} points", import.points.len()));
                        preview_table(ui, &import.points);
                        if !import.warnings.is_empty() {
                            egui::CollapsingHeader::new(
                                RichText::new(format!("{} lines skipped", import.warnings.len()))
                                    .color(theme.warning),
                            )
                            .show(ui, |ui| {
                                for w in import.warnings.iter().take(50) {
                                    ui.label(RichText::new(w).small().monospace());
                                }
                            });
                        }
                    }
                    Err(e) => {
                        ui.label(RichText::new(e).color(theme.error));
                    }
                }

                ui.separator();
                ui.horizontal(|ui| {
                    let can_import =
                        self.preview.is_ok() && (!self.new_layer || !self.new_layer_name.trim().is_empty());
                    if ui.add_enabled(can_import, egui::Button::new("Import")).clicked()
                        && let Ok(import) = &self.preview
                    {
                        let n = import.points.len();
                        result =
                            CsvDialogResult::Import(Box::new(self.build_command(doc, &import.points)), n);
                    }
                    if ui.button("Cancel").clicked() {
                        result = CsvDialogResult::Cancelled;
                    }
                });
            });
        if !open {
            result = CsvDialogResult::Cancelled;
        }
        result
    }

    fn build_command(&self, doc: &Document, points: &[ImportedPoint]) -> ImportEntities {
        let label = format!("Import {} points from {}", points.len(), self.file_name);
        let mut cmd = ImportEntities::new(label);
        let layer = if self.new_layer {
            cmd.add_layer(
                self.new_layer_name.trim().to_string(),
                Color::rgb(0x7d, 0xd3, 0xfc),
                true,
            )
        } else {
            LayerRef::Existing(doc.current_layer())
        };
        let mut next: u64 = doc.next_point_name().parse().unwrap_or(1);
        for p in points {
            let name = if p.name.is_empty() {
                next += 1;
                (next - 1).to_string()
            } else {
                p.name.clone()
            };
            let point = SurveyPoint {
                name,
                position: glam::DVec3::new(p.easting, p.northing, p.height.unwrap_or(0.0)),
                height_known: p.height.is_some(),
                code: p.code.clone(),
                description: p.description.clone(),
                layer: doc.current_layer(),
            };
            cmd.add(layer, Entity::Point(point));
        }
        cmd
    }
}

fn preview_table(ui: &mut egui::Ui, points: &[ImportedPoint]) {
    egui::Grid::new("csv_preview")
        .striped(true)
        .spacing([14.0, 2.0])
        .show(ui, |ui| {
            for h in ["Name", "Easting", "Northing", "Height", "Code"] {
                ui.label(RichText::new(h).small().strong());
            }
            ui.end_row();
            for p in points.iter().take(8) {
                ui.label(RichText::new(&p.name).monospace());
                ui.label(RichText::new(format!("{:.3}", p.easting)).monospace());
                ui.label(RichText::new(format!("{:.3}", p.northing)).monospace());
                ui.label(RichText::new(p.height.map(|h| format!("{h:.3}")).unwrap_or_default()).monospace());
                ui.label(RichText::new(&p.code).monospace());
                ui.end_row();
            }
        });
    if points.len() > 8 {
        ui.label(RichText::new(format!("… and {} more", points.len() - 8)).small());
    }
}
