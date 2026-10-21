//! Heads-up display next to the cursor ("dynamic input" in AutoCAD terms):
//! the active prompt, live measurements and whatever the user is typing, so
//! attention can stay on the drawing instead of the command line.

use egui::{CornerRadius, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};

use crate::fonts;
use crate::theme::CadTheme;

/// One line of the HUD.
#[derive(Clone, Debug)]
pub enum HudLine {
    /// The command prompt (secondary text).
    Prompt(String),
    /// A live value such as a bearing/distance (monospace).
    Value(String),
    /// Text the user is typing (accent, with a caret).
    Input(String),
}

/// Draws the HUD offset from `cursor`, kept inside `bounds`.
pub fn draw_cursor_hud(painter: &egui::Painter, bounds: Rect, cursor: Pos2, lines: &[HudLine]) {
    if lines.is_empty() {
        return;
    }
    let theme = CadTheme::current(painter.ctx());
    let galleys: Vec<_> = lines
        .iter()
        .map(|line| match line {
            HudLine::Prompt(t) => painter.layout_no_wrap(t.clone(), fonts::ui(11.5), theme.text_secondary),
            HudLine::Value(t) => painter.layout_no_wrap(t.clone(), fonts::mono(11.5), theme.text),
            HudLine::Input(t) => painter.layout_no_wrap(format!("{t}▏"), fonts::mono(12.0), theme.accent),
        })
        .collect();
    let pad = vec2(8.0, 5.0);
    let gap = 2.0;
    let width = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max);
    let height = galleys.iter().map(|g| g.size().y).sum::<f32>() + gap * (galleys.len() - 1) as f32;
    let size = vec2(width, height) + pad * 2.0;
    // Below-right of the cursor, flipped when it would leave the viewport.
    let mut min = cursor + vec2(18.0, 20.0);
    if min.x + size.x > bounds.right() - 4.0 {
        min.x = cursor.x - 18.0 - size.x;
    }
    if min.y + size.y > bounds.bottom() - 4.0 {
        min.y = cursor.y - 20.0 - size.y;
    }
    let rect = Rect::from_min_size(min, size);
    let r = CornerRadius::same(theme.radius + 1);
    painter.rect_filled(rect, r, theme.overlay.gamma_multiply(0.92));
    painter.rect_stroke(rect, r, Stroke::new(1.0, theme.border), StrokeKind::Inside);
    let mut y = rect.top() + pad.y;
    for g in galleys {
        let h = g.size().y;
        painter.galley(pos2(rect.left() + pad.x, y), g, theme.text);
        y += h + gap;
    }
}
