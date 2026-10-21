//! Transient notifications stacked in a corner of a rect (e.g. "Saved",
//! "Imported 120 points"). They fade out on their own; errors linger longer.

use egui::{Align2, CornerRadius, Rect, Stroke, StrokeKind, pos2, vec2};

use crate::fonts;
use crate::icons;
use crate::theme::CadTheme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
struct Toast {
    kind: ToastKind,
    text: String,
    /// `None` until first shown (so the lifetime starts when it is visible).
    shown_at: Option<f64>,
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
}

const MAX_VISIBLE: usize = 4;

impl Toasts {
    pub fn push(&mut self, kind: ToastKind, text: impl Into<String>) {
        self.items.push(Toast {
            kind,
            text: text.into(),
            shown_at: None,
        });
        if self.items.len() > MAX_VISIBLE {
            self.items.remove(0);
        }
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.push(ToastKind::Info, text);
    }

    pub fn success(&mut self, text: impl Into<String>) {
        self.push(ToastKind::Success, text);
    }

    pub fn warning(&mut self, text: impl Into<String>) {
        self.push(ToastKind::Warning, text);
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.push(ToastKind::Error, text);
    }

    /// Draws the toasts in the bottom-right corner of `rect`.
    pub fn show(&mut self, painter: &egui::Painter, rect: Rect) {
        let ctx = painter.ctx().clone();
        let theme = CadTheme::current(&ctx);
        let now = ctx.input(|i| i.time);
        self.items
            .retain(|t| t.shown_at.is_none_or(|s| now - s < lifetime(t.kind)));
        let mut bottom = rect.bottom() - 52.0;
        for toast in self.items.iter_mut().rev() {
            let shown = *toast.shown_at.get_or_insert(now);
            let age = now - shown;
            let life = lifetime(toast.kind);
            let alpha = (((life - age) / 0.4).clamp(0.0, 1.0) * (age / 0.15).clamp(0.0, 1.0)) as f32;
            let (icon, color) = match toast.kind {
                ToastKind::Info => (icons::INFO, theme.info),
                ToastKind::Success => (icons::CHECK_CIRCLE, theme.success),
                ToastKind::Warning => (icons::WARNING_CIRCLE, theme.warning),
                ToastKind::Error => (icons::X_CIRCLE, theme.error),
            };
            let galley = painter.layout(toast.text.clone(), fonts::ui(12.5), theme.text, 320.0);
            let size = vec2(galley.size().x + 44.0, galley.size().y.max(16.0) + 16.0);
            let r = Rect::from_min_size(pos2(rect.right() - size.x - 16.0, bottom - size.y), size);
            let shadow = theme
                .overlay_shadow()
                .as_shape(r, CornerRadius::same(theme.radius_large));
            painter.add(egui::Shape::Rect(shadow).with_alpha(alpha));
            painter.rect_filled(
                r,
                CornerRadius::same(theme.radius_large),
                theme.overlay.gamma_multiply(alpha),
            );
            painter.rect_stroke(
                r,
                CornerRadius::same(theme.radius_large),
                Stroke::new(1.0, theme.border.gamma_multiply(alpha)),
                StrokeKind::Inside,
            );
            // Status colour as a left accent bar and icon.
            painter.rect_filled(
                Rect::from_min_size(r.min + vec2(0.0, 6.0), vec2(3.0, r.height() - 12.0)),
                CornerRadius::same(2),
                color.gamma_multiply(alpha),
            );
            painter.text(
                pos2(r.left() + 18.0, r.center().y),
                Align2::CENTER_CENTER,
                icon,
                fonts::icon(15.0),
                color.gamma_multiply(alpha),
            );
            painter.galley(
                pos2(r.left() + 32.0, r.center().y - galley.size().y / 2.0),
                galley,
                theme.text.gamma_multiply(alpha),
            );
            bottom = r.top() - 8.0;
        }
        if !self.items.is_empty() {
            ctx.request_repaint();
        }
    }
}

fn lifetime(kind: ToastKind) -> f64 {
    match kind {
        ToastKind::Error | ToastKind::Warning => 6.0,
        ToastKind::Info | ToastKind::Success => 3.0,
    }
}

trait WithAlpha {
    fn with_alpha(self, alpha: f32) -> Self;
}

impl WithAlpha for egui::Shape {
    fn with_alpha(self, alpha: f32) -> Self {
        match self {
            egui::Shape::Rect(mut r) => {
                r.fill = r.fill.gamma_multiply(alpha);
                egui::Shape::Rect(r)
            }
            other => other,
        }
    }
}
