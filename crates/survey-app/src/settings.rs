//! Per-user application settings (persisted by eframe, not stored in projects).

use egui_cad::ThemeMode;
use egui_cad::constraint::OrthoSettings;
use egui_cad::snap::SnapSettings;
use serde::{Deserialize, Serialize};
use survey_core::Conventions;
use survey_io::CsvLayout;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// Recorded in the audit trail for every edit.
    pub user_name: String,
    pub theme: ThemeMode,
    /// Conventions given to new projects (each project stores its own).
    pub default_conventions: Conventions,
    pub snap: SnapSettings,
    pub ortho: OrthoSettings,
    pub show_grid: bool,
    /// Height of the working plane for picks that do not snap to anything.
    pub working_z: f64,
    pub csv_layout: CsvLayout,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            user_name: default_user_name(),
            theme: ThemeMode::Dark,
            default_conventions: Conventions::default(),
            snap: SnapSettings::default(),
            ortho: OrthoSettings::default(),
            show_grid: true,
            working_z: 0.0,
            csv_layout: CsvLayout::from_letters("PENZC"),
        }
    }
}

fn default_user_name() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "user".into())
    }
    #[cfg(target_arch = "wasm32")]
    {
        "web user".into()
    }
}
