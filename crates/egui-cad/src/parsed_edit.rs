//! A single-line editor for values with custom parsing/formatting
//! (angles, distances, coordinates…). The application supplies the closures, so
//! the widget stays independent of any particular unit or convention.

use egui::{AsIdSalt, IdSalt, Key, Stroke, TextEdit};

use crate::theme::CadTheme;

type Formatter<'a, T> = Box<dyn Fn(&T) -> String + 'a>;
type Parser<'a, T> = Box<dyn Fn(&str) -> Result<T, String> + 'a>;

pub struct ParsedEdit<'a, T> {
    id_salt: IdSalt,
    /// `None` displays as "varies" (multi-selection with differing values).
    value: Option<&'a T>,
    format: Formatter<'a, T>,
    parse: Parser<'a, T>,
    width: f32,
    hint: &'a str,
    empty_hint: &'a str,
}

impl<'a, T> ParsedEdit<'a, T> {
    pub fn new(
        id_salt: impl AsIdSalt,
        value: Option<&'a T>,
        format: impl Fn(&T) -> String + 'a,
        parse: impl Fn(&str) -> Result<T, String> + 'a,
    ) -> Self {
        Self {
            id_salt: IdSalt::new(id_salt),
            value,
            format: Box::new(format),
            parse: Box::new(parse),
            width: f32::INFINITY,
            hint: "varies",
            empty_hint: "",
        }
    }

    pub fn desired_width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Hint shown when `value` is `None` (multi-selection with differing values).
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = hint;
        self
    }

    /// Hint shown when a single value formats as empty text (e.g. "unknown").
    pub fn empty_hint(mut self, hint: &'a str) -> Self {
        self.empty_hint = hint;
        self
    }

    /// Returns the new value when the user commits (Enter or focus loss) a valid,
    /// changed entry. Escape abandons the edit.
    pub fn show(self, ui: &mut egui::Ui) -> (egui::Response, Option<T>) {
        let theme = CadTheme::current(ui.ctx());
        let id = ui.scope_id().with_salt(self.id_salt);
        let formatted = self.value.map(|v| (self.format)(v)).unwrap_or_default();
        let mut text: String = ui
            .data(|d| d.get_temp::<String>(id))
            .unwrap_or_else(|| formatted.clone());

        let output = TextEdit::singleline(&mut text)
            .id(id)
            .hint_text(if self.value.is_some() {
                self.empty_hint
            } else {
                self.hint
            })
            .desired_width(self.width)
            .show(ui);
        let response = output.response.response;

        let error = (response.has_focus() && text != formatted)
            .then(|| (self.parse)(&text).err())
            .flatten();
        if let Some(err) = &error {
            ui.painter().rect_stroke(
                response.rect.expand(1.0),
                3.0,
                Stroke::new(1.0, theme.error),
                egui::StrokeKind::Outside,
            );
            response.clone().on_hover_text(err);
        }

        let mut result = None;
        if response.has_focus() {
            ui.data_mut(|d| d.insert_temp(id, text.clone()));
        } else if response.lost_focus() {
            ui.data_mut(|d| d.remove::<String>(id));
            let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
            if !cancelled && text != formatted {
                result = (self.parse)(&text).ok();
            }
        }
        (response, result)
    }
}
