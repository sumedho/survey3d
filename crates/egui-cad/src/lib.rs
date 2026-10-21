//! Reusable CAD widgets for egui.
//!
//! This crate knows about CAD concepts (viewports, snapping, layers, command
//! entry) but nothing about surveying: applications supply geometry through
//! [`snap::SnapProvider`] and domain-specific parsing/formatting through
//! [`parsed_edit::ParsedEdit`] closures.
//!
//! Start with [`theme::CadTheme::apply`] and [`install_fonts`], then compose
//! [`viewport::Viewport`], [`command_line::CommandLine`], [`property_grid`],
//! [`layer_panel`] and the [`chrome`] helpers.

pub mod chrome;
pub mod command_line;
pub mod constraint;
pub mod icons;
pub mod labels;
pub mod layer_panel;
pub mod parsed_edit;
pub mod property_grid;
pub mod selection;
pub mod snap;
pub mod theme;
pub mod viewport;

pub use theme::{CadTheme, ThemeMode};

/// Registers the bundled icon font as a fallback for proportional text.
pub fn install_fonts(ctx: &egui::Context) {
    const NAME: &str = "phosphor";
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        NAME.into(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/Phosphor.ttf"
        ))),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let keys = fonts.families.entry(family).or_default();
        // Second, so it never shadows the text font.
        keys.insert(keys.len().min(1), NAME.into());
    }
    ctx.set_fonts(fonts);
}
