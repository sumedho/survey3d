//! Layer list: one compact row per layer with colour swatch, visibility and
//! lock toggles, name (double-click to rename) and object count. The current
//! layer is marked with an accent bar. Add a "new layer" button to the
//! surrounding section header (see [`crate::chrome::section`]).

use egui::{Align2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::fonts;
use crate::icons;
use crate::theme::CadTheme;

#[derive(Clone, Debug)]
pub struct LayerRow {
    pub id: i64,
    pub name: String,
    pub color: Color32,
    pub visible: bool,
    pub locked: bool,
    pub current: bool,
    /// Number of objects on the layer (shown, and used to guard deletion).
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayerAction {
    SetCurrent(i64),
    SetVisible(i64, bool),
    SetLocked(i64, bool),
    SetColor(i64, Color32),
    Rename(i64, String),
    Add,
    Delete(i64),
}

const ROW_HEIGHT: f32 = 24.0;

/// Shows the layer list; returns the actions the user requested.
pub fn layer_panel(ui: &mut egui::Ui, rows: &[LayerRow]) -> Vec<LayerAction> {
    let theme = CadTheme::current(ui.ctx());
    let mut actions = Vec::new();
    let rename_id = ui.scope_id().with("egui_cad_layer_rename");
    // The layer being renamed, its new name, and whether the field has had focus.
    let mut renaming: Option<(i64, String, bool)> = ui.data(|d| d.get_temp(rename_id));

    for row in rows {
        let (rect, row_resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
        let hovered = ui.rect_contains_pointer(rect);
        if row.current {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(theme.radius), theme.accent_subtle);
            ui.painter().rect_filled(
                Rect::from_min_size(rect.min + vec2(0.0, 4.0), vec2(3.0, rect.height() - 8.0)),
                CornerRadius::same(2),
                theme.accent,
            );
        } else if hovered {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(theme.radius), theme.raised);
        }
        if row_resp.clicked() && !row.current {
            actions.push(LayerAction::SetCurrent(row.id));
        }

        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(vec2(8.0, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let cui = &mut child;
        cui.spacing_mut().item_spacing.x = 4.0;

        // Colour swatch (opens a picker).
        let mut color = row.color;
        let swatch = cui
            .scope(|ui| {
                ui.spacing_mut().interact_size = vec2(16.0, 14.0);
                egui::color_picker::color_edit_button_srgba(ui, &mut color, egui::color_picker::Alpha::Opaque)
            })
            .inner
            .on_hover_text("Layer colour");
        if swatch.changed() {
            actions.push(LayerAction::SetColor(row.id, color));
        }

        let eye = if row.visible { icons::EYE } else { icons::EYE_SLASH };
        if small_toggle(cui, eye, row.visible, "Visibility", &theme).clicked() {
            actions.push(LayerAction::SetVisible(row.id, !row.visible));
        }
        let lock = if row.locked { icons::LOCK } else { icons::LOCK_OPEN };
        if small_toggle(cui, lock, !row.locked, "Lock", &theme).clicked() {
            actions.push(LayerAction::SetLocked(row.id, !row.locked));
        }
        cui.add_space(2.0);

        match &mut renaming {
            Some((id, text, focused)) if *id == row.id => {
                let width = cui.available_width() - 30.0;
                let r = cui.add(egui::TextEdit::singleline(text).desired_width(width));
                // Focus once: asking every frame would take focus straight back
                // when Enter (or a click elsewhere) gives it up, so it never commits.
                if !*focused {
                    r.request_focus();
                    *focused = true;
                } else if r.lost_focus() {
                    if !cui.input(|i| i.key_pressed(egui::Key::Escape)) && !text.trim().is_empty() {
                        actions.push(LayerAction::Rename(row.id, text.trim().to_string()));
                    }
                    renaming = None;
                }
            }
            _ => {
                let color = if row.visible {
                    theme.text
                } else {
                    theme.text_disabled
                };
                let font = if row.current {
                    fonts::medium(12.5)
                } else {
                    fonts::ui(12.5)
                };
                let label = egui::Label::new(RichText::new(&row.name).font(font).color(color))
                    .truncate()
                    .sense(Sense::click());
                let name = cui
                    .add(label)
                    .on_hover_text("Click to make current · double-click to rename");
                if name.double_clicked() {
                    renaming = Some((row.id, row.name.clone(), false));
                } else if name.clicked() && !row.current {
                    actions.push(LayerAction::SetCurrent(row.id));
                }
            }
        }

        // Object count, or a delete button for empty layers on hover.
        let deletable = row.count == 0 && !row.current && rows.len() > 1;
        let slot = Rect::from_center_size(pos2(rect.right() - 16.0, rect.center().y), vec2(20.0, 20.0));
        if deletable && hovered {
            let button = egui::Button::new(
                RichText::new(icons::TRASH)
                    .font(fonts::icon(13.0))
                    .color(theme.text_secondary),
            )
            .frame(false);
            if cui
                .put(slot, button)
                .on_hover_text("Delete empty layer")
                .clicked()
            {
                actions.push(LayerAction::Delete(row.id));
            }
        } else {
            cui.painter().text(
                pos2(rect.right() - 8.0, rect.center().y),
                Align2::RIGHT_CENTER,
                row.count.to_string(),
                fonts::mono(11.0),
                theme.text_secondary,
            );
        }
    }
    ui.data_mut(|d| match renaming {
        Some(r) => {
            d.insert_temp(rename_id, r);
        }
        None => {
            d.remove::<(i64, String, bool)>(rename_id);
        }
    });
    actions
}

/// A 20 px icon toggle: full colour when on, faded when off.
fn small_toggle(ui: &mut egui::Ui, icon: &str, on: bool, tooltip: &str, theme: &CadTheme) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme.radius_small), theme.overlay);
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(theme.radius_small),
            Stroke::new(1.0, theme.border),
            StrokeKind::Inside,
        );
    }
    let color = if on {
        theme.text_secondary
    } else {
        theme.text_disabled
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        fonts::icon(13.0),
        color,
    );
    resp.on_hover_text(tooltip)
}
