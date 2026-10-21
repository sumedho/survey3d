//! Design tokens and the egui style derived from them.
//!
//! The palette follows established practice for professional tools:
//! * **Layered surfaces, not shadows, in dark mode.** Higher elevation is
//!   lighter: backdrop → panels → raised controls → overlays (menus, tooltips).
//!   Light mode keeps white overlays and uses soft shadows instead.
//! * **No pure black or pure white** for large areas or body text, which
//!   reduces glare and halation over long sessions.
//! * **One desaturated accent**, used sparingly for the active tool, focus and
//!   primary state. Status colours are reserved for status.
//! * **Semantic tokens**: widgets ask for "panel" or "text_secondary", never a
//!   literal colour, so themes, accents and density can change in one place.

use egui::{Color32, CornerRadius, Margin, Shadow, Stroke, Vec2};
use serde::{Deserialize, Serialize};

use crate::fonts;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    Dark,
    Light,
}

/// Accent colour family.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Accent {
    #[default]
    Amber,
    Blue,
    Teal,
}

impl Accent {
    pub const ALL: [Accent; 3] = [Accent::Amber, Accent::Blue, Accent::Teal];

    pub fn label(self) -> &'static str {
        match self {
            Accent::Amber => "Amber",
            Accent::Blue => "Blue",
            Accent::Teal => "Teal",
        }
    }

    /// (accent, hover, pressed, text-on-accent) for a mode. Dark mode uses
    /// lighter, less saturated tones so the accent does not vibrate.
    fn tones(self, mode: ThemeMode) -> (Color32, Color32, Color32, Color32) {
        let rgb = Color32::from_rgb;
        match (self, mode) {
            (Accent::Amber, ThemeMode::Dark) => (
                rgb(0xF2, 0xA8, 0x3B),
                rgb(0xF6, 0xBD, 0x67),
                rgb(0xD9, 0x91, 0x28),
                rgb(0x1E, 0x14, 0x05),
            ),
            (Accent::Amber, ThemeMode::Light) => (
                rgb(0xA8, 0x5E, 0x00),
                rgb(0xB8, 0x6A, 0x05),
                rgb(0x8E, 0x4F, 0x00),
                Color32::WHITE,
            ),
            (Accent::Blue, ThemeMode::Dark) => (
                rgb(0x6A, 0xAE, 0xF5),
                rgb(0x8C, 0xC2, 0xF8),
                rgb(0x4F, 0x95, 0xE0),
                rgb(0x06, 0x14, 0x24),
            ),
            (Accent::Blue, ThemeMode::Light) => (
                rgb(0x1F, 0x6F, 0xD1),
                rgb(0x2E, 0x80, 0xE3),
                rgb(0x18, 0x5B, 0xAE),
                Color32::WHITE,
            ),
            (Accent::Teal, ThemeMode::Dark) => (
                rgb(0x4C, 0xC9, 0xB5),
                rgb(0x72, 0xD8, 0xC7),
                rgb(0x34, 0xAE, 0x9B),
                rgb(0x04, 0x1C, 0x18),
            ),
            (Accent::Teal, ThemeMode::Light) => (
                rgb(0x0D, 0x7A, 0x6E),
                rgb(0x12, 0x8C, 0x7E),
                rgb(0x0A, 0x66, 0x5C),
                Color32::WHITE,
            ),
        }
    }
}

/// Spacing scale. Both densities sit on a 2/4 px grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Density {
    #[default]
    Compact,
    Comfortable,
}

impl Density {
    pub const ALL: [Density; 2] = [Density::Compact, Density::Comfortable];

    pub fn label(self) -> &'static str {
        match self {
            Density::Compact => "Compact",
            Density::Comfortable => "Comfortable",
        }
    }

    /// Height of an interactive row (buttons, inputs, list rows).
    pub fn row_height(self) -> f32 {
        match self {
            Density::Compact => 22.0,
            Density::Comfortable => 26.0,
        }
    }

    fn item_spacing(self) -> Vec2 {
        match self {
            Density::Compact => Vec2::new(6.0, 4.0),
            Density::Comfortable => Vec2::new(8.0, 6.0),
        }
    }

    fn button_padding(self) -> Vec2 {
        match self {
            Density::Compact => Vec2::new(8.0, 3.0),
            Density::Comfortable => Vec2::new(10.0, 5.0),
        }
    }
}

/// Design tokens shared by every widget in the crate.
#[derive(Clone, Debug, PartialEq)]
pub struct CadTheme {
    pub mode: ThemeMode,
    pub accent_family: Accent,
    pub density: Density,

    // Surfaces, from lowest to highest elevation.
    /// Backdrop behind everything (gaps between panels).
    pub bg: Color32,
    /// Docked panels, bars.
    pub panel: Color32,
    /// Inputs, hovered rows, cards inside panels.
    pub raised: Color32,
    /// Menus, popups, tooltips, floating toolbars.
    pub overlay: Color32,
    /// The 3D/plan canvas.
    pub viewport_bg: Color32,

    // Lines.
    pub border_subtle: Color32,
    pub border: Color32,
    pub border_strong: Color32,

    // Text.
    pub text: Color32,
    pub text_secondary: Color32,
    pub text_disabled: Color32,

    // Accent.
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_pressed: Color32,
    /// Background for selected rows / active toggles.
    pub accent_subtle: Color32,
    pub on_accent: Color32,
    pub focus: Color32,

    // Status.
    pub info: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,

    // Canvas.
    pub selection: Color32,
    pub hover: Color32,
    pub snap: Color32,
    pub preview: Color32,
    pub axis_x: Color32,
    pub axis_y: Color32,
    pub axis_z: Color32,
    pub grid_minor: Color32,
    pub grid_major: Color32,

    pub radius_small: u8,
    pub radius: u8,
    pub radius_large: u8,
}

impl CadTheme {
    pub fn new(mode: ThemeMode, accent: Accent, density: Density) -> Self {
        let rgb = Color32::from_rgb;
        let (accent_c, accent_hover, accent_pressed, on_accent) = accent.tones(mode);
        match mode {
            ThemeMode::Dark => Self {
                mode,
                accent_family: accent,
                density,
                bg: rgb(0x13, 0x15, 0x19),
                panel: rgb(0x1B, 0x1E, 0x23),
                raised: rgb(0x24, 0x28, 0x2F),
                overlay: rgb(0x2B, 0x30, 0x38),
                viewport_bg: rgb(0x10, 0x12, 0x15),
                border_subtle: rgb(0x27, 0x2B, 0x32),
                border: rgb(0x34, 0x39, 0x42),
                border_strong: rgb(0x4A, 0x50, 0x5B),
                text: rgb(0xE3, 0xE6, 0xEA),
                text_secondary: rgb(0xA0, 0xA8, 0xB3),
                text_disabled: rgb(0x68, 0x6F, 0x7A),
                accent: accent_c,
                accent_hover,
                accent_pressed,
                accent_subtle: mix(rgb(0x1B, 0x1E, 0x23), accent_c, 0.16),
                on_accent,
                focus: accent_c.gamma_multiply(0.8),
                info: rgb(0x6C, 0xB2, 0xF0),
                success: rgb(0x5F, 0xCF, 0x8A),
                warning: rgb(0xF2, 0xC1, 0x4E),
                error: rgb(0xF0, 0x74, 0x74),
                selection: rgb(0xFF, 0xB8, 0x4D),
                hover: rgb(0x7D, 0xD3, 0xFC),
                snap: rgb(0x4A, 0xDE, 0x80),
                preview: rgb(0x9C, 0xC8, 0xFF),
                axis_x: rgb(0xE8, 0x60, 0x60),
                axis_y: rgb(0x5B, 0xCC, 0x7A),
                axis_z: rgb(0x5B, 0x9B, 0xEF),
                grid_minor: Color32::from_rgba_unmultiplied(0x70, 0x7A, 0x88, 40),
                grid_major: Color32::from_rgba_unmultiplied(0x82, 0x8C, 0x9A, 95),
                radius_small: 3,
                radius: 4,
                radius_large: 8,
            },
            ThemeMode::Light => Self {
                mode,
                accent_family: accent,
                density,
                bg: rgb(0xE6, 0xE8, 0xEC),
                panel: rgb(0xF4, 0xF5, 0xF7),
                raised: rgb(0xFF, 0xFF, 0xFF),
                overlay: rgb(0xFF, 0xFF, 0xFF),
                viewport_bg: rgb(0xFA, 0xFA, 0xFB),
                border_subtle: rgb(0xE1, 0xE4, 0xE8),
                border: rgb(0xD0, 0xD5, 0xDC),
                border_strong: rgb(0xAE, 0xB5, 0xBF),
                text: rgb(0x1C, 0x21, 0x27),
                text_secondary: rgb(0x52, 0x5B, 0x67),
                text_disabled: rgb(0x9A, 0xA1, 0xAB),
                accent: accent_c,
                accent_hover,
                accent_pressed,
                accent_subtle: mix(rgb(0xF4, 0xF5, 0xF7), accent_c, 0.14),
                on_accent,
                focus: accent_c.gamma_multiply(0.7),
                info: rgb(0x1F, 0x6F, 0xD1),
                success: rgb(0x15, 0x8A, 0x45),
                warning: rgb(0xA8, 0x5B, 0x00),
                error: rgb(0xC9, 0x2A, 0x2A),
                selection: rgb(0xD9, 0x6C, 0x00),
                hover: rgb(0x02, 0x84, 0xC7),
                snap: rgb(0x16, 0xA3, 0x4A),
                preview: rgb(0x25, 0x63, 0xEB),
                axis_x: rgb(0xD0, 0x2B, 0x2B),
                axis_y: rgb(0x15, 0x96, 0x45),
                axis_z: rgb(0x23, 0x5F, 0xD9),
                grid_minor: Color32::from_rgba_unmultiplied(0x55, 0x5E, 0x6A, 34),
                grid_major: Color32::from_rgba_unmultiplied(0x55, 0x5E, 0x6A, 80),
                radius_small: 3,
                radius: 4,
                radius_large: 8,
            },
        }
    }

    pub fn dark() -> Self {
        Self::new(ThemeMode::Dark, Accent::default(), Density::default())
    }

    pub fn light() -> Self {
        Self::new(ThemeMode::Light, Accent::default(), Density::default())
    }

    pub fn for_mode(mode: ThemeMode) -> Self {
        Self::new(mode, Accent::default(), Density::default())
    }

    /// Straight RGBA in 0..1 for the renderer.
    pub fn rgba_f32(c: Color32) -> [f32; 4] {
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        [r, g, b, a].map(|v| f32::from(v) / 255.0)
    }

    /// Shadow for floating surfaces (menus, popups, overlays on the canvas).
    pub fn overlay_shadow(&self) -> Shadow {
        match self.mode {
            ThemeMode::Dark => Shadow {
                offset: [0, 4],
                blur: 16,
                spread: 0,
                color: Color32::from_black_alpha(110),
            },
            ThemeMode::Light => Shadow {
                offset: [0, 4],
                blur: 14,
                spread: 0,
                color: Color32::from_black_alpha(36),
            },
        }
    }

    /// Applies the theme to egui and stores it for [`CadTheme::current`].
    /// Also installs the bundled fonts (only the first time).
    pub fn apply(&self, ctx: &egui::Context) {
        fonts::install(ctx);
        let mut visuals = match self.mode {
            ThemeMode::Dark => egui::Visuals::dark(),
            ThemeMode::Light => egui::Visuals::light(),
        };
        let r = CornerRadius::same(self.radius);
        visuals.panel_fill = self.panel;
        visuals.window_fill = self.overlay;
        visuals.window_stroke = Stroke::new(1.0, self.border);
        visuals.window_corner_radius = CornerRadius::same(self.radius_large);
        visuals.window_shadow = self.overlay_shadow();
        visuals.popup_shadow = self.overlay_shadow();
        visuals.menu_corner_radius = CornerRadius::same(self.radius + 2);
        visuals.extreme_bg_color = self.bg;
        visuals.faint_bg_color = match self.mode {
            ThemeMode::Dark => self.panel.gamma_multiply(1.12),
            ThemeMode::Light => self.bg.gamma_multiply(1.02),
        };
        visuals.code_bg_color = self.raised;
        visuals.text_edit_bg_color = Some(match self.mode {
            ThemeMode::Dark => self.bg,
            ThemeMode::Light => self.raised,
        });
        visuals.hyperlink_color = self.accent;
        visuals.warn_fg_color = self.warning;
        visuals.error_fg_color = self.error;
        visuals.selection.bg_fill = self.accent_subtle;
        visuals.selection.stroke = Stroke::new(1.0, self.accent);
        visuals.text_cursor.stroke = Stroke::new(1.5, self.accent);
        visuals.striped = true;
        visuals.slider_trailing_fill = true;
        visuals.indent_has_left_vline = false;
        visuals.collapsing_header_frame = false;
        visuals.override_text_color = None;

        let w = &mut visuals.widgets;
        // Static content: labels, separators, frames.
        w.noninteractive.bg_fill = self.panel;
        w.noninteractive.weak_bg_fill = self.panel;
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.border_subtle);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        // Resting controls.
        w.inactive.bg_fill = self.raised;
        w.inactive.weak_bg_fill = self.raised;
        w.inactive.bg_stroke = Stroke::new(1.0, self.border);
        w.inactive.fg_stroke = Stroke::new(1.0, self.text);
        // Hover: one step up in elevation, stronger border.
        let hover_fill = match self.mode {
            ThemeMode::Dark => self.overlay,
            ThemeMode::Light => Color32::from_rgb(0xEE, 0xF0, 0xF3),
        };
        w.hovered.bg_fill = hover_fill;
        w.hovered.weak_bg_fill = hover_fill;
        w.hovered.bg_stroke = Stroke::new(1.0, self.border_strong);
        w.hovered.fg_stroke = Stroke::new(1.0, self.text);
        // Pressed / dragging: accent.
        w.active.bg_fill = self.accent_subtle;
        w.active.weak_bg_fill = self.accent_subtle;
        w.active.bg_stroke = Stroke::new(1.0, self.accent);
        w.active.fg_stroke = Stroke::new(1.0, self.text);
        // Open combo boxes / menus.
        w.open.bg_fill = self.overlay;
        w.open.weak_bg_fill = self.overlay;
        w.open.bg_stroke = Stroke::new(1.0, self.border_strong);
        w.open.fg_stroke = Stroke::new(1.0, self.text);
        for v in [
            &mut w.noninteractive,
            &mut w.inactive,
            &mut w.hovered,
            &mut w.active,
            &mut w.open,
        ] {
            v.corner_radius = r;
            v.expansion = 0.0;
        }

        ctx.set_visuals(visuals);
        let density = self.density;
        ctx.global_style_mut(|style| {
            let s = &mut style.spacing;
            s.item_spacing = density.item_spacing();
            s.button_padding = density.button_padding();
            s.interact_size = Vec2::new(36.0, density.row_height());
            s.window_margin = Margin::same(10);
            s.menu_margin = Margin::same(4);
            s.indent = 14.0;
            s.combo_width = 120.0;
            s.icon_width = 14.0;
            s.icon_width_inner = 8.0;
            s.icon_spacing = 6.0;
            s.scroll = egui::style::ScrollStyle::thin();
            style.interaction.tooltip_delay = 0.35;
            style.interaction.selectable_labels = false;
            fonts::configure_text_styles(style);
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

/// Opaque linear mix of two colours in sRGB space (`t = 0` → `a`).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relative luminance per WCAG 2.1.
    fn luminance(c: Color32) -> f64 {
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_meets_wcag_aa_on_every_surface() {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            for accent in Accent::ALL {
                let t = CadTheme::new(mode, accent, Density::Compact);
                for surface in [t.bg, t.panel, t.raised, t.overlay, t.viewport_bg] {
                    let body = contrast(t.text, surface);
                    let secondary = contrast(t.text_secondary, surface);
                    assert!(body >= 7.0, "{mode:?} body text {body:.2}");
                    assert!(secondary >= 4.5, "{mode:?} secondary text {secondary:.2}");
                }
                let on = contrast(t.on_accent, t.accent);
                let ui = contrast(t.accent, t.panel);
                assert!(on >= 4.5, "{mode:?} {accent:?} text on accent {on:.2}");
                assert!(ui >= 3.0, "{mode:?} {accent:?} accent vs panel {ui:.2}");
            }
        }
    }

    #[test]
    fn dark_surfaces_get_lighter_with_elevation() {
        let t = CadTheme::dark();
        let order = [t.viewport_bg, t.bg, t.panel, t.raised, t.overlay];
        for w in order.windows(2) {
            assert!(luminance(w[0]) < luminance(w[1]));
        }
        assert_ne!(t.bg, Color32::BLACK);
        assert_ne!(t.text, Color32::WHITE);
    }
}
