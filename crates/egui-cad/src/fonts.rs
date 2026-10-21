//! Bundled typography.
//!
//! * **Inter** (variable) for UI text — designed for screens, legible at 11–13 px.
//! * **Inter Medium** (the same file at weight 560) for headings and emphasis,
//!   available as [`medium`].
//! * **JetBrains Mono** for numbers and command input: every digit is the same
//!   width, so coordinates and distances line up in columns.
//! * **Phosphor** icons as a fallback in every family, so icon glyphs can be
//!   mixed into any text.
//!
//! All fonts are under the SIL Open Font License / MIT (see `assets/`).

use std::sync::Arc;

use egui::epaint::text::{FontTweak, VariationCoords};
use egui::{FontData, FontDefinitions, FontFamily, FontId, TextStyle};

const INTER: &[u8] = include_bytes!("../assets/Inter.ttf");
const MONO: &[u8] = include_bytes!("../assets/JetBrainsMono.ttf");
const ICON_BYTES: &[u8] = include_bytes!("../assets/Phosphor.ttf");

/// Family name for medium-weight UI text.
pub const MEDIUM: &str = "inter-medium";

/// Family for icon glyphs: Phosphor first. Inter also defines glyphs in the
/// Private Use Area, so icons must be drawn with this family to be reliable.
pub const ICONS: &str = "icons";

/// Icon font at `size`.
pub fn icon(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(ICONS.into()))
}

/// Medium-weight UI font at `size`.
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MEDIUM.into()))
}

/// Regular UI font at `size`.
pub fn ui(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Monospace (numbers, input) at `size`.
pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

fn weighted(bytes: &'static [u8], weight: f32) -> Arc<FontData> {
    let mut data = FontData::from_static(bytes);
    data.tweak = FontTweak {
        coords: VariationCoords::new([("wght", weight)]),
        ..Default::default()
    };
    Arc::new(data)
}

/// Installs the fonts once per context.
pub(crate) fn install(ctx: &egui::Context) {
    let id = egui::Id::unique("egui_cad_fonts_installed");
    if ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false) {
        return;
    }
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("inter".into(), weighted(INTER, 420.0));
    fonts.font_data.insert(MEDIUM.into(), weighted(INTER, 560.0));
    fonts.font_data.insert("mono".into(), weighted(MONO, 430.0));
    fonts
        .font_data
        .insert("icons".into(), Arc::new(FontData::from_static(ICON_BYTES)));

    // Keep egui's defaults (emoji etc.) as later fallbacks.
    let prop_fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mono_fallbacks = fonts
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    let chain = |first: &str, rest: &[String]| {
        let mut v = vec![first.to_string(), "icons".to_string()];
        v.extend(rest.iter().cloned());
        v
    };
    fonts
        .families
        .insert(FontFamily::Proportional, chain("inter", &prop_fallbacks));
    fonts
        .families
        .insert(FontFamily::Monospace, chain("mono", &mono_fallbacks));
    fonts
        .families
        .insert(FontFamily::Name(MEDIUM.into()), chain(MEDIUM, &prop_fallbacks));
    fonts.families.insert(
        FontFamily::Name(ICONS.into()),
        vec!["icons".into(), "inter".into()],
    );
    ctx.set_fonts(fonts);
    ctx.data_mut(|d| d.insert_temp(id, true));
}

/// Sizes for egui's built-in text styles.
pub(crate) fn configure_text_styles(style: &mut egui::Style) {
    use FontFamily::{Monospace, Name, Proportional};
    style.text_styles = [
        (TextStyle::Small, FontId::new(11.0, Proportional)),
        (TextStyle::Body, FontId::new(12.5, Proportional)),
        (TextStyle::Button, FontId::new(12.5, Proportional)),
        (TextStyle::Monospace, FontId::new(12.0, Monospace)),
        (TextStyle::Heading, FontId::new(15.0, Name(MEDIUM.into()))),
    ]
    .into();
}
