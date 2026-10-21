//! Colour tokens and egui style for a compact, CAD-style interface.

use egui::{Color32, CornerRadius, Stroke, Vec2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    Dark,
    Light,
}

/// Design tokens shared by every widget in the crate.
#[derive(Clone, Debug, PartialEq)]
pub struct CadTheme {
    pub mode: ThemeMode,
    /// Window / app background behind panels.
    pub bg: Color32,
    pub panel: Color32,
    /// Raised surfaces: inputs, hovered rows, chips.
    pub surface: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub viewport_bg: Color32,
    pub selection: Color32,
    pub hover: Color32,
    pub snap: Color32,
    pub preview: Color32,
    pub warning: Color32,
    pub error: Color32,
    pub success: Color32,
    pub axis_x: Color32,
    pub axis_y: Color32,
    pub axis_z: Color32,
    pub grid_minor: Color32,
    pub grid_major: Color32,
}

impl CadTheme {
    pub fn dark() -> Self {
        Self {
            mode: ThemeMode::Dark,
            bg: Color32::from_rgb(0x12, 0x14, 0x18),
            panel: Color32::from_rgb(0x1a, 0x1d, 0x22),
            surface: Color32::from_rgb(0x24, 0x28, 0x2f),
            border: Color32::from_rgb(0x2f, 0x34, 0x3c),
            text: Color32::from_rgb(0xd8, 0xdd, 0xe3),
            text_dim: Color32::from_rgb(0x8a, 0x93, 0x9f),
            accent: Color32::from_rgb(0xf2, 0xa3, 0x3a),
            accent_text: Color32::from_rgb(0x1a, 0x12, 0x05),
            viewport_bg: Color32::from_rgb(0x0e, 0x10, 0x13),
            selection: Color32::from_rgb(0xff, 0xb8, 0x4d),
            hover: Color32::from_rgb(0x7d, 0xd3, 0xfc),
            snap: Color32::from_rgb(0x4a, 0xde, 0x80),
            preview: Color32::from_rgb(0x9c, 0xc8, 0xff),
            warning: Color32::from_rgb(0xfb, 0xbf, 0x24),
            error: Color32::from_rgb(0xf8, 0x71, 0x71),
            success: Color32::from_rgb(0x4a, 0xde, 0x80),
            axis_x: Color32::from_rgb(0xef, 0x5b, 0x5b),
            axis_y: Color32::from_rgb(0x5b, 0xd1, 0x7a),
            axis_z: Color32::from_rgb(0x5b, 0x9b, 0xef),
            grid_minor: Color32::from_rgba_unmultiplied(0x6b, 0x75, 0x82, 50),
            grid_major: Color32::from_rgba_unmultiplied(0x7d, 0x88, 0x96, 105),
        }
    }

    pub fn light() -> Self {
        Self {
            mode: ThemeMode::Light,
            bg: Color32::from_rgb(0xe4, 0xe7, 0xeb),
            panel: Color32::from_rgb(0xf4, 0xf5, 0xf7),
            surface: Color32::from_rgb(0xff, 0xff, 0xff),
            border: Color32::from_rgb(0xcf, 0xd4, 0xdb),
            text: Color32::from_rgb(0x1e, 0x23, 0x2a),
            text_dim: Color32::from_rgb(0x5f, 0x68, 0x74),
            accent: Color32::from_rgb(0xc9, 0x6f, 0x00),
            accent_text: Color32::WHITE,
            viewport_bg: Color32::from_rgb(0xfb, 0xfb, 0xfc),
            selection: Color32::from_rgb(0xd9, 0x6c, 0x00),
            hover: Color32::from_rgb(0x02, 0x84, 0xc7),
            snap: Color32::from_rgb(0x16, 0xa3, 0x4a),
            preview: Color32::from_rgb(0x25, 0x63, 0xeb),
            warning: Color32::from_rgb(0xb4, 0x53, 0x09),
            error: Color32::from_rgb(0xdc, 0x26, 0x26),
            success: Color32::from_rgb(0x16, 0xa3, 0x4a),
            axis_x: Color32::from_rgb(0xdc, 0x26, 0x26),
            axis_y: Color32::from_rgb(0x16, 0xa3, 0x4a),
            axis_z: Color32::from_rgb(0x25, 0x63, 0xeb),
            grid_minor: Color32::from_rgba_unmultiplied(0x5f, 0x68, 0x74, 40),
            grid_major: Color32::from_rgba_unmultiplied(0x5f, 0x68, 0x74, 90),
        }
    }

    pub fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::dark(),
            ThemeMode::Light => Self::light(),
        }
    }

    /// Straight RGBA in 0..1 for the renderer.
    pub fn rgba_f32(c: Color32) -> [f32; 4] {
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        [r, g, b, a].map(|v| f32::from(v) / 255.0)
    }

    /// Applies the theme to egui and stores it for [`CadTheme::current`].
    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = match self.mode {
            ThemeMode::Dark => egui::Visuals::dark(),
            ThemeMode::Light => egui::Visuals::light(),
        };
        let radius = CornerRadius::same(3);
        visuals.panel_fill = self.panel;
        visuals.window_fill = self.panel;
        visuals.window_stroke = Stroke::new(1.0, self.border);
        visuals.window_corner_radius = CornerRadius::same(6);
        visuals.menu_corner_radius = CornerRadius::same(4);
        visuals.extreme_bg_color = self.bg;
        visuals.faint_bg_color = self.surface;
        visuals.code_bg_color = self.surface;
        visuals.text_edit_bg_color = Some(self.bg);
        visuals.hyperlink_color = self.accent;
        visuals.warn_fg_color = self.warning;
        visuals.error_fg_color = self.error;
        visuals.selection.bg_fill = self.accent.gamma_multiply(0.35);
        visuals.selection.stroke = Stroke::new(1.0, self.accent);
        visuals.striped = true;

        let w = &mut visuals.widgets;
        w.noninteractive.bg_fill = self.panel;
        w.noninteractive.weak_bg_fill = self.panel;
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        w.inactive.bg_fill = self.surface;
        w.inactive.weak_bg_fill = self.surface;
        w.inactive.bg_stroke = Stroke::new(1.0, self.border);
        w.inactive.fg_stroke = Stroke::new(1.0, self.text);
        w.hovered.bg_fill = self.surface.gamma_multiply(1.25);
        w.hovered.weak_bg_fill = self.surface.gamma_multiply(1.25);
        w.hovered.bg_stroke = Stroke::new(1.0, self.accent.gamma_multiply(0.7));
        w.hovered.fg_stroke = Stroke::new(1.5, self.text);
        w.active.bg_fill = self.accent.gamma_multiply(0.5);
        w.active.weak_bg_fill = self.accent.gamma_multiply(0.5);
        w.active.bg_stroke = Stroke::new(1.0, self.accent);
        w.active.fg_stroke = Stroke::new(1.5, self.text);
        w.open.weak_bg_fill = self.surface;
        for v in [
            &mut w.noninteractive,
            &mut w.inactive,
            &mut w.hovered,
            &mut w.active,
            &mut w.open,
        ] {
            v.corner_radius = radius;
            v.expansion = 0.0;
        }

        ctx.set_visuals(visuals);
        ctx.global_style_mut(|style| {
            let s = &mut style.spacing;
            s.item_spacing = Vec2::new(6.0, 4.0);
            s.button_padding = Vec2::new(6.0, 3.0);
            s.interact_size = Vec2::new(36.0, 22.0);
            s.window_margin = egui::Margin::same(8);
            s.menu_margin = egui::Margin::same(4);
            s.indent = 14.0;
            s.combo_width = 120.0;
            for (text_style, size) in [
                (egui::TextStyle::Body, 13.0),
                (egui::TextStyle::Button, 13.0),
                (egui::TextStyle::Small, 11.0),
                (egui::TextStyle::Monospace, 12.5),
                (egui::TextStyle::Heading, 15.0),
            ] {
                if let Some(font) = style.text_styles.get_mut(&text_style) {
                    font.size = size;
                }
            }
        });
        ctx.data_mut(|d| d.insert_temp(Self::id(), self.clone()));
    }

    /// The theme last applied with [`CadTheme::apply`] (dark if none).
    pub fn current(ctx: &egui::Context) -> Self {
        ctx.data(|d| d.get_temp(Self::id())).unwrap_or_else(Self::dark)
    }

    fn id() -> egui::Id {
        egui::Id::unique("egui_cad_theme")
    }
}
