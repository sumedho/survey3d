//! Two-column property editor (label | editor) with section headers.

use egui::{Color32, RichText, WidgetText};

use crate::fonts;

use crate::parsed_edit::ParsedEdit;
use crate::theme::CadTheme;

/// Shows a property grid. Rows are added through [`PropertyGridUi`].
pub fn property_grid<R>(
    ui: &mut egui::Ui,
    id_salt: impl egui::AsIdSalt,
    add: impl FnOnce(&mut PropertyGridUi<'_>) -> R,
) -> R {
    let theme = CadTheme::current(ui.ctx());
    let label_width = (ui.available_width() * 0.38).clamp(70.0, 140.0);
    egui::Grid::new(id_salt)
        .num_columns(2)
        .striped(false)
        .spacing([10.0, 3.0])
        .min_col_width(label_width)
        .show(ui, |ui| {
            let mut grid = PropertyGridUi { ui, theme };
            add(&mut grid)
        })
        .inner
}

pub struct PropertyGridUi<'a> {
    ui: &'a mut egui::Ui,
    theme: CadTheme,
}

impl PropertyGridUi<'_> {
    pub fn ui(&mut self) -> &mut egui::Ui {
        self.ui
    }

    pub fn section(&mut self, title: &str) {
        // (No add_space here: egui grids do not allow it.)
        self.ui.label(
            RichText::new(title.to_uppercase())
                .font(fonts::medium(10.5))
                .color(self.theme.text_secondary),
        );
        self.ui.label("");
        self.ui.end_row();
    }

    fn label(&mut self, label: &str) {
        self.ui
            .label(RichText::new(label).size(12.0).color(self.theme.text_secondary));
    }

    /// A generic row with a custom editor.
    pub fn row<R>(&mut self, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
        self.label(label);
        let r = add(self.ui);
        self.ui.end_row();
        r
    }

    pub fn readonly(&mut self, label: &str, value: impl Into<WidgetText>) {
        self.row(label, |ui| ui.label(value));
    }

    /// A monospace read-only value (numbers line up).
    pub fn readonly_mono(&mut self, label: &str, value: &str) {
        let color = self.theme.text;
        self.row(label, |ui| {
            ui.label(RichText::new(value).monospace().color(color))
        });
    }

    /// Text field; returns the new text when committed.
    pub fn text(&mut self, label: &str, value: Option<&String>) -> Option<String> {
        self.parsed(label, value, |s: &String| s.clone(), |s| Ok(s.to_string()))
    }

    /// Value with custom format/parse; returns the new value when committed.
    pub fn parsed<T>(
        &mut self,
        label: &str,
        value: Option<&T>,
        format: impl Fn(&T) -> String,
        parse: impl Fn(&str) -> Result<T, String>,
    ) -> Option<T> {
        self.parsed_with_hint(label, value, "", format, parse)
    }

    /// Like [`Self::parsed`], showing `empty_hint` when the value formats as empty text.
    pub fn parsed_with_hint<T>(
        &mut self,
        label: &str,
        value: Option<&T>,
        empty_hint: &str,
        format: impl Fn(&T) -> String,
        parse: impl Fn(&str) -> Result<T, String>,
    ) -> Option<T> {
        self.row(label, |ui| {
            ParsedEdit::new(label, value, format, parse)
                .empty_hint(empty_hint)
                .show(ui)
                .1
        })
    }

    /// Checkbox; `None` shows an indeterminate state. Returns the new value when clicked.
    pub fn checkbox(&mut self, label: &str, value: Option<bool>) -> Option<bool> {
        self.row(label, |ui| {
            let mut v = value.unwrap_or(false);
            let response = ui.add(egui::Checkbox::new(&mut v, "").indeterminate(value.is_none()));
            response.clicked().then_some(v)
        })
    }

    /// Combo box over `options`; returns the chosen index when changed.
    pub fn combo(&mut self, label: &str, selected: Option<usize>, options: &[String]) -> Option<usize> {
        self.row(label, |ui| {
            let text = selected
                .and_then(|i| options.get(i))
                .map_or("varies", String::as_str);
            let mut chosen = None;
            egui::ComboBox::from_id_salt(label)
                .selected_text(text)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for (i, opt) in options.iter().enumerate() {
                        if ui.selectable_label(selected == Some(i), opt).clicked() && selected != Some(i) {
                            chosen = Some(i);
                        }
                    }
                });
            chosen
        })
    }

    /// Colour swatch with picker; returns the new colour when changed.
    pub fn color(&mut self, label: &str, value: Color32) -> Option<Color32> {
        self.row(label, |ui| {
            let mut c = value;
            egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque)
                .changed()
                .then_some(c)
        })
    }
}
