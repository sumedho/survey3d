//! Per-user application settings (persisted by eframe, not stored in projects).

use egui_cad::constraint::OrthoSettings;
use egui_cad::snap::SnapSettings;
use egui_cad::{Accent, Density, ThemeMode};
use serde::{Deserialize, Serialize};
use survey_core::Conventions;
use survey_io::CsvLayout;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// Recorded in the audit trail for every edit.
    pub user_name: String,
    pub theme: ThemeMode,
    pub accent: Accent,
    pub density: Density,
    /// Show the prompt, live measurements and typed input next to the cursor (F12).
    pub dynamic_input: bool,
    /// Conventions given to new projects (each project stores its own).
    pub default_conventions: Conventions,
    pub snap: SnapSettings,
    pub ortho: OrthoSettings,
    pub show_grid: bool,
    /// Grid snap spacing in metres; `None` follows the visible grid's minor spacing.
    pub grid_snap_spacing: Option<f64>,
    /// Height of the working plane for picks that do not snap to anything.
    pub working_z: f64,
    pub csv_layout: CsvLayout,
    pub labels: LabelSettings,
}

/// Which point attributes are drawn next to points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LabelSettings {
    pub names: bool,
    pub codes: bool,
    pub heights: bool,
    pub size: f32,
}

impl Default for LabelSettings {
    fn default() -> Self {
        Self {
            names: true,
            codes: false,
            heights: false,
            size: 11.0,
        }
    }
}

impl LabelSettings {
    pub fn any(&self) -> bool {
        self.names || self.codes || self.heights
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            user_name: default_user_name(),
            theme: ThemeMode::Dark,
            accent: Accent::default(),
            density: Density::default(),
            dynamic_input: true,
            default_conventions: Conventions::default(),
            snap: SnapSettings::default(),
            ortho: OrthoSettings::default(),
            show_grid: true,
            grid_snap_spacing: None,
            working_z: 0.0,
            csv_layout: CsvLayout::from_letters("PENZC"),
            labels: LabelSettings::default(),
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
