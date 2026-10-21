//! Small building blocks for CAD application chrome: tool buttons, status-bar
//! chips, panel section headers and frames.

use egui::{Color32, CornerRadius, Margin, RichText, Sense, Stroke, Vec2};

use crate::theme::CadTheme;

/// Square icon button for tool palettes. Highlighted with the accent colour when `selected`.
pub fn tool_button(ui: &mut egui::Ui, icon: &str, tooltip: &str, selected: bool) -> egui::Response {
    let theme = CadTheme::current(ui.ctx());
    let size = Vec2::splat(30.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let (bg, fg) = if selected {
        (theme.accent, theme.accent_text)
    } else if response.hovered() {
        (theme.surface, theme.text)
    } else {
        (Color32::TRANSPARENT, theme.text)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(4), bg);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(17.0),
        fg,
    );
    response.on_hover_text(tooltip)
}

/// Small toggle chip for the status bar (e.g. snap modes). Returns the click response.
pub fn status_chip(ui: &mut egui::Ui, label: &str, on: bool, tooltip: &str) -> egui::Response {
    let theme = CadTheme::current(ui.ctx());
    let text =
        RichText::new(label)
            .small()
            .strong()
            .color(if on { theme.accent_text } else { theme.text_dim });
    let fill = if on { theme.accent } else { theme.surface };
    ui.add(
        egui::Button::new(text)
            .fill(fill)
            .stroke(Stroke::new(1.0, if on { theme.accent } else { theme.border }))
            .corner_radius(CornerRadius::same(3))
            .min_size(Vec2::new(0.0, 18.0)),
    )
    .on_hover_text(tooltip)
}

/// Uppercase section header used at the top of side-panel sections.
pub fn section_header(ui: &mut egui::Ui, title: &str) {
    let theme = CadTheme::current(ui.ctx());
    ui.add_space(4.0);
    ui.label(
        RichText::new(title.to_uppercase())
            .small()
            .strong()
            .color(theme.text_dim),
    );
    ui.add_space(2.0);
}

/// Frame for side panels.
pub fn panel_frame(theme: &CadTheme) -> egui::Frame {
    egui::Frame::new()
        .fill(theme.panel)
        .inner_margin(Margin::symmetric(8, 6))
        .stroke(Stroke::new(1.0, theme.border))
}

/// Frame for thin bars (toolbar, status bar).
pub fn bar_frame(theme: &CadTheme) -> egui::Frame {
    egui::Frame::new()
        .fill(theme.panel)
        .inner_margin(Margin::symmetric(8, 3))
        .stroke(Stroke::new(1.0, theme.border))
}
