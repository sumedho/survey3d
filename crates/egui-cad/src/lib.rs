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
pub mod command_palette;
pub mod constraint;
pub mod fonts;
pub mod hud;
pub mod icons;
pub mod labels;
pub mod layer_panel;
pub mod parsed_edit;
pub mod property_grid;
pub mod selection;
pub mod snap;
pub mod theme;
pub mod toasts;
pub mod viewport;

pub use theme::{Accent, CadTheme, Density, ThemeMode};

/// Installs the bundled fonts (also done by [`CadTheme::apply`]).
pub fn install_fonts(ctx: &egui::Context) {
    fonts::install(ctx);
}
