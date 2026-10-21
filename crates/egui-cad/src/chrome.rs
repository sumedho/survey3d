//! Building blocks for application chrome: icon buttons with rich tooltips,
//! collapsible sections, segmented controls, switches, keycaps, frames and
//! empty states.
//!
//! Every control has four visual states — rest, hover, pressed, selected — plus
//! disabled, all derived from [`CadTheme`] tokens.

use egui::{
    Align2, Color32, CornerRadius, Margin, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui,
    Vec2, pos2, vec2,
};

use crate::fonts;
use crate::theme::CadTheme;

// ====================================================================== tooltips

/// Attaches a tooltip with a title, optional description and optional shortcut keycap.
pub fn rich_tooltip(response: Response, title: &str, description: &str, shortcut: Option<&str>) -> Response {
    response.on_hover_ui(|ui| {
        let theme = CadTheme::current(ui.ctx());
        ui.set_max_width(260.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).font(fonts::medium(12.5)).color(theme.text));
            if let Some(s) = shortcut {
                ui.add_space(8.0);
                keycap(ui, s);
            }
        });
        if !description.is_empty() {
            ui.label(RichText::new(description).size(11.5).color(theme.text_secondary));
        }
    })
}

/// Draws a keyboard shortcut as a small key cap, e.g. `Ctrl+S`.
/// A compact on/off chip for status bars (e.g. snap modes). Returns the
/// response; the caller flips its state on click.
pub fn status_toggle(ui: &mut Ui, on: bool, text: &str, tooltip: &str) -> Response {
    let theme = CadTheme::current(ui.ctx());
    let font = fonts::medium(10.5);
    let color = if on { theme.accent } else { theme.text_secondary };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let size = vec2(galley.size().x + 10.0, (galley.size().y + 4.0).max(16.0));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if on {
        theme.accent_subtle
    } else if response.hovered() {
        theme.raised
    } else {
        Color32::TRANSPARENT
    };
    let radius = CornerRadius::same(theme.radius_small);
    ui.painter().rect_filled(rect, radius, fill);
    if response.hovered() {
        ui.painter()
            .rect_stroke(rect, radius, Stroke::new(1.0, theme.border), StrokeKind::Inside);
    }
    let pos = rect.center() - galley.size() * 0.5;
    ui.painter().galley(pos, galley, color);
    response.on_hover_text(tooltip)
}

pub fn keycap(ui: &mut Ui, text: &str) -> Response {
    let theme = CadTheme::current(ui.ctx());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), fonts::mono(10.5), theme.text_secondary);
    let size = galley.size() + vec2(8.0, 4.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(theme.radius_small), theme.raised);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(theme.radius_small),
        Stroke::new(1.0, theme.border),
        StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.min + vec2(4.0, 2.0), galley, theme.text_secondary);
    response
}

// ====================================================================== icon buttons

/// A square icon button for toolbars and tool palettes.
#[must_use = "call .show(ui)"]
pub struct IconButton<'a> {
    icon: &'a str,
    title: &'a str,
    description: &'a str,
    shortcut: Option<&'a str>,
    selected: bool,
    enabled: bool,
    size: f32,
}

impl<'a> IconButton<'a> {
    pub fn new(icon: &'a str, title: &'a str) -> Self {
        Self {
            icon,
            title,
            description: "",
            shortcut: None,
            selected: false,
            enabled: true,
            size: 26.0,
        }
    }

    pub fn description(mut self, text: &'a str) -> Self {
        self.description = text;
        self
    }

    pub fn shortcut(mut self, text: Option<&'a str>) -> Self {
        self.shortcut = text;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Edge length in points (26 for toolbars, 32 for tool palettes).
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let theme = CadTheme::current(ui.ctx());
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.size), sense);
        let (bg, fg) = if !self.enabled {
            (Color32::TRANSPARENT, theme.text_disabled)
        } else if self.selected {
            (theme.accent, theme.on_accent)
        } else if response.is_pointer_button_down_on() {
            (theme.accent_subtle, theme.text)
        } else if response.hovered() {
            (theme.raised, theme.text)
        } else {
            (Color32::TRANSPARENT, theme.text_secondary)
        };
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(theme.radius), bg);
        if response.has_focus() {
            painter.rect_stroke(
                rect.expand(1.0),
                CornerRadius::same(theme.radius + 1),
                Stroke::new(1.5, theme.focus),
                StrokeKind::Outside,
            );
        }
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            self.icon,
            fonts::icon(self.size * 0.58),
            fg,
        );
        rich_tooltip(response, self.title, self.description, self.shortcut)
    }
}

/// A thin vertical divider between toolbar groups.
pub fn toolbar_divider(ui: &mut Ui) {
    let theme = CadTheme::current(ui.ctx());
    let h = ui.spacing().interact_size.y;
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, h), Sense::hover());
    ui.painter().line_segment(
        [
            pos2(rect.center().x, rect.top() + 4.0),
            pos2(rect.center().x, rect.bottom() - 4.0),
        ],
        Stroke::new(1.0, theme.border),
    );
}

/// A thin horizontal divider between tool-palette groups.
pub fn palette_divider(ui: &mut Ui) {
    let theme = CadTheme::current(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    ui.painter().line_segment(
        [
            pos2(rect.left() + 8.0, rect.center().y),
            pos2(rect.right() - 8.0, rect.center().y),
        ],
        Stroke::new(1.0, theme.border),
    );
}

// ====================================================================== sections

/// A collapsible panel section with a chevron, an uppercase title and optional
/// header actions on the right (e.g. an "add" button). The open state persists.
pub fn section<R>(
    ui: &mut Ui,
    title: &str,
    default_open: bool,
    actions: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let theme = CadTheme::current(ui.ctx());
    let id = ui.make_persistent_id(("egui_cad_section", title));
    let mut open: bool = ui.data_mut(|d| *d.get_persisted_mut_or(id, default_open));
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    if response.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_persisted(id, open));
    }
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme.radius), theme.raised);
    }
    let chevron = if open {
        crate::icons::CARET_DOWN
    } else {
        crate::icons::CARET_RIGHT
    };
    let painter = ui.painter();
    painter.text(
        rect.left_center() + vec2(4.0, 0.0),
        Align2::LEFT_CENTER,
        chevron,
        fonts::icon(11.0),
        theme.text_secondary,
    );
    painter.text(
        rect.left_center() + vec2(20.0, 0.0),
        Align2::LEFT_CENTER,
        title.to_uppercase(),
        fonts::medium(10.5),
        theme.text_secondary,
    );
    // Header actions, right-aligned within the header row.
    let actions_rect = Rect::from_min_max(pos2(rect.right() - 80.0, rect.top()), rect.right_bottom());
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(actions_rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        actions,
    );
    if open {
        ui.add_space(2.0);
        let r = body(ui);
        ui.add_space(6.0);
        Some(r)
    } else {
        None
    }
}

/// Uppercase heading for a group of settings (non-collapsible).
pub fn section_header(ui: &mut Ui, title: &str) {
    let theme = CadTheme::current(ui.ctx());
    ui.add_space(4.0);
    ui.label(
        RichText::new(title.to_uppercase())
            .font(fonts::medium(10.5))
            .color(theme.text_secondary),
    );
    ui.add_space(2.0);
}

// ====================================================================== inputs

/// A segmented control (mutually exclusive options in one pill). Returns true when changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let theme = CadTheme::current(ui.ctx());
    let font = egui::FontId::proportional(12.0);
    let h = ui.spacing().interact_size.y;
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, label)| {
            ui.painter()
                .layout_no_wrap(label.to_string(), font.clone(), theme.text)
                .size()
                .x
                + 18.0
        })
        .collect();
    let total = widths.iter().sum::<f32>() + 4.0;
    let (rect, response) = ui.allocate_exact_size(vec2(total, h), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(theme.radius + 1), theme.bg);
    painter.rect_stroke(
        rect,
        CornerRadius::same(theme.radius + 1),
        Stroke::new(1.0, theme.border),
        StrokeKind::Inside,
    );
    let mut changed = false;
    let mut x = rect.left() + 2.0;
    for ((opt, label), w) in options.iter().zip(widths) {
        let seg = Rect::from_min_size(pos2(x, rect.top() + 2.0), vec2(w, h - 4.0));
        let resp = ui.interact(seg, response.id.with(label), Sense::click());
        let selected = *value == *opt;
        if selected {
            ui.painter()
                .rect_filled(seg, CornerRadius::same(theme.radius), theme.raised);
            ui.painter().rect_stroke(
                seg,
                CornerRadius::same(theme.radius),
                Stroke::new(1.0, theme.border_strong),
                StrokeKind::Inside,
            );
        } else if resp.hovered() {
            ui.painter().rect_filled(
                seg,
                CornerRadius::same(theme.radius),
                theme.raised.gamma_multiply(0.5),
            );
        }
        let color = if selected {
            theme.text
        } else {
            theme.text_secondary
        };
        ui.painter()
            .text(seg.center(), Align2::CENTER_CENTER, *label, font.clone(), color);
        if resp.clicked() && !selected {
            *value = *opt;
            changed = true;
        }
        x += w;
    }
    changed
}

/// An on/off switch. `on` is toggled when clicked.
pub fn toggle_switch(ui: &mut Ui, on: &mut bool) -> Response {
    let theme = CadTheme::current(ui.ctx());
    let (rect, mut response) = ui.allocate_exact_size(vec2(30.0, 16.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(response.id, *on);
    let track = if *on { theme.accent } else { theme.border_strong };
    ui.painter().rect_filled(rect, CornerRadius::same(8), track);
    let knob_x = egui::lerp(rect.left() + 8.0..=rect.right() - 8.0, t);
    let knob = if *on { theme.on_accent } else { theme.raised };
    ui.painter()
        .circle_filled(pos2(knob_x, rect.center().y), 6.0, knob);
    response
}

/// A labelled switch row: `[switch] label`. Clicking the label also toggles.
pub fn switch_row(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    ui.horizontal(|ui| {
        let switch = toggle_switch(ui, on);
        let text = ui.add(egui::Label::new(label).sense(Sense::click()));
        if text.clicked() {
            *on = !*on;
        }
        switch.union(text)
    })
    .inner
}

/// A search-field look-alike that opens a command palette when clicked.
pub fn search_button(ui: &mut Ui, placeholder: &str, shortcut: &str, width: f32) -> Response {
    let theme = CadTheme::current(ui.ctx());
    let h = ui.spacing().interact_size.y;
    let (rect, response) = ui.allocate_exact_size(vec2(width, h), Sense::click());
    let fill = if response.hovered() {
        theme.raised
    } else {
        theme.bg
    };
    let stroke = if response.hovered() {
        theme.border_strong
    } else {
        theme.border
    };
    let r = CornerRadius::same(theme.radius + 1);
    ui.painter().rect_filled(rect, r, fill);
    ui.painter()
        .rect_stroke(rect, r, Stroke::new(1.0, stroke), StrokeKind::Inside);
    ui.painter().text(
        rect.left_center() + vec2(9.0, 0.0),
        Align2::LEFT_CENTER,
        crate::icons::MAGNIFYING_GLASS,
        fonts::icon(13.0),
        theme.text_secondary,
    );
    ui.painter().text(
        rect.left_center() + vec2(27.0, 0.0),
        Align2::LEFT_CENTER,
        placeholder,
        fonts::ui(12.0),
        theme.text_secondary,
    );
    let key = ui
        .painter()
        .layout_no_wrap(shortcut.to_string(), fonts::mono(10.5), theme.text_secondary);
    let key_rect = Rect::from_min_size(
        pos2(
            rect.right() - key.size().x - 14.0,
            rect.center().y - key.size().y / 2.0 - 2.0,
        ),
        key.size() + vec2(8.0, 4.0),
    );
    ui.painter()
        .rect_filled(key_rect, CornerRadius::same(theme.radius_small), theme.raised);
    ui.painter().rect_stroke(
        key_rect,
        CornerRadius::same(theme.radius_small),
        Stroke::new(1.0, theme.border),
        StrokeKind::Inside,
    );
    ui.painter()
        .galley(key_rect.min + vec2(4.0, 2.0), key, theme.text_secondary);
    response
}

// ====================================================================== frames

/// Frame for docked side panels.
pub fn panel_frame(theme: &CadTheme) -> egui::Frame {
    egui::Frame::new()
        .fill(theme.panel)
        .inner_margin(Margin::symmetric(8, 6))
        .stroke(Stroke::new(1.0, theme.border_subtle))
}

/// Frame for thin bars (menu bar, status bar).
pub fn bar_frame(theme: &CadTheme) -> egui::Frame {
    egui::Frame::new()
        .fill(theme.panel)
        .inner_margin(Margin::symmetric(8, 3))
        .stroke(Stroke::new(1.0, theme.border_subtle))
}

/// Frame for controls floating over the canvas (view controls, HUDs).
pub fn floating_frame(theme: &CadTheme) -> egui::Frame {
    egui::Frame::new()
        .fill(theme.overlay.gamma_multiply(0.95))
        .inner_margin(Margin::same(3))
        .corner_radius(CornerRadius::same(theme.radius_large))
        .stroke(Stroke::new(1.0, theme.border))
        .shadow(theme.overlay_shadow())
}

// ====================================================================== empty state

/// A centred hint for an empty canvas: an icon, a title and `(keys, action)` rows.
pub fn empty_state(painter: &egui::Painter, rect: Rect, icon: &str, title: &str, hints: &[(&str, &str)]) {
    let theme = CadTheme::current(painter.ctx());
    let center = rect.center() - vec2(0.0, 30.0);
    painter.text(
        center - vec2(0.0, 36.0),
        Align2::CENTER_CENTER,
        icon,
        fonts::icon(34.0),
        theme.text_disabled,
    );
    painter.text(
        center,
        Align2::CENTER_CENTER,
        title,
        fonts::medium(15.0),
        theme.text_secondary,
    );
    let mut y = center.y + 26.0;
    for (keys, action) in hints {
        let key_galley = painter.layout_no_wrap(keys.to_string(), fonts::mono(11.0), theme.text_secondary);
        let action_galley = painter.layout_no_wrap(action.to_string(), fonts::ui(12.0), theme.text_secondary);
        let key_w = key_galley.size().x + 10.0;
        let total = key_w + 10.0 + action_galley.size().x;
        let left = center.x - total / 2.0;
        let key_rect = Rect::from_min_size(Pos2::new(left, y), vec2(key_w, 18.0));
        painter.rect_filled(key_rect, CornerRadius::same(theme.radius_small), theme.raised);
        painter.rect_stroke(
            key_rect,
            CornerRadius::same(theme.radius_small),
            Stroke::new(1.0, theme.border),
            StrokeKind::Inside,
        );
        painter.galley(key_rect.min + vec2(5.0, 2.5), key_galley, theme.text_secondary);
        painter.galley(
            pos2(key_rect.right() + 10.0, y + 1.5),
            action_galley,
            theme.text_secondary,
        );
        y += 26.0;
    }
}
