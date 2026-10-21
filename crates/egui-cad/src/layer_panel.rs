//! Layer list with current/visible/locked toggles, colour swatches and rename.

use egui::{Color32, RichText, Sense};

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

/// Shows the layer list; returns the actions the user requested.
pub fn layer_panel(ui: &mut egui::Ui, rows: &[LayerRow]) -> Vec<LayerAction> {
    let theme = CadTheme::current(ui.ctx());
    let mut actions = Vec::new();
    let rename_id = ui.scope_id().with("egui_cad_layer_rename");
    let mut renaming: Option<(i64, String)> = ui.data(|d| d.get_temp(rename_id));

    ui.horizontal(|ui| {
        if icon_button(ui, icons::PLUS, "New layer", &theme).clicked() {
            actions.push(LayerAction::Add);
        }
    });

    for row in rows {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            // Current-layer marker.
            let dot = if row.current { icons::CHECK } else { " " };
            let current = ui
                .add_sized(
                    [16.0, 18.0],
                    egui::Label::new(RichText::new(dot).color(theme.accent)).sense(Sense::click()),
                )
                .on_hover_text("Make current");
            if current.clicked() && !row.current {
                actions.push(LayerAction::SetCurrent(row.id));
            }

            let mut color = row.color;
            if egui::color_picker::color_edit_button_srgba(ui, &mut color, egui::color_picker::Alpha::Opaque)
                .changed()
            {
                actions.push(LayerAction::SetColor(row.id, color));
            }

            let eye = if row.visible { icons::EYE } else { icons::EYE_SLASH };
            if icon_toggle(ui, eye, row.visible, "Visible", &theme).clicked() {
                actions.push(LayerAction::SetVisible(row.id, !row.visible));
            }
            let lock = if row.locked { icons::LOCK } else { icons::LOCK_OPEN };
            if icon_toggle(ui, lock, !row.locked, "Locked", &theme).clicked() {
                actions.push(LayerAction::SetLocked(row.id, !row.locked));
            }

            match &mut renaming {
                Some((id, text)) if *id == row.id => {
                    let r =
                        ui.add(egui::TextEdit::singleline(text).desired_width(ui.available_width() - 40.0));
                    r.request_focus();
                    if r.lost_focus() {
                        if !ui.input(|i| i.key_pressed(egui::Key::Escape)) && !text.trim().is_empty() {
                            actions.push(LayerAction::Rename(row.id, text.trim().to_string()));
                        }
                        renaming = None;
                    }
                }
                _ => {
                    let name_color = if row.visible { theme.text } else { theme.text_dim };
                    let mut text = RichText::new(&row.name).color(name_color);
                    if row.current {
                        text = text.strong();
                    }
                    let r = ui
                        .add(egui::Label::new(text).sense(Sense::click()).truncate())
                        .on_hover_text("Double-click to rename");
                    if r.double_clicked() {
                        renaming = Some((row.id, row.name.clone()));
                    }
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let deletable = row.count == 0 && !row.current && rows.len() > 1;
                if deletable && icon_button(ui, icons::TRASH, "Delete empty layer", &theme).clicked() {
                    actions.push(LayerAction::Delete(row.id));
                }
                ui.label(RichText::new(row.count.to_string()).small().color(theme.text_dim));
            });
        });
    }
    ui.data_mut(|d| match renaming {
        Some(r) => {
            d.insert_temp(rename_id, r);
        }
        None => {
            d.remove::<(i64, String)>(rename_id);
        }
    });
    actions
}

fn icon_button(ui: &mut egui::Ui, icon: &str, tooltip: &str, theme: &CadTheme) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(icon).color(theme.text)).frame(false))
        .on_hover_text(tooltip)
}

fn icon_toggle(ui: &mut egui::Ui, icon: &str, on: bool, tooltip: &str, theme: &CadTheme) -> egui::Response {
    let color = if on {
        theme.text
    } else {
        theme.text_dim.gamma_multiply(0.7)
    };
    ui.add(egui::Button::new(RichText::new(icon).color(color)).frame(false))
        .on_hover_text(tooltip)
}
