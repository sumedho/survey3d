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
    /// Picks that do not snap to an object take their height from a visible surface.
    pub surface_heights: bool,
    /// Domain modules whose menus are shown.
    pub modules: ModuleSettings,
    /// Height of the working plane for picks that do not snap to anything.
    pub working_z: f64,
    pub csv_layout: CsvLayout,
    pub labels: LabelSettings,
    /// Folder where COPC point clouds are kept and where conversions are
    /// written (native only; see the COPC window).
    pub data_dir: Option<std::path::PathBuf>,
    /// Eye-dome lighting on point clouds: shades depth edges so shapes read.
    pub cloud_edl: bool,
}

/// A domain module that adds its own menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Module {
    Cadastral,
    Construction,
    Mining,
}

impl Module {
    pub const ALL: [Module; 3] = [Module::Cadastral, Module::Construction, Module::Mining];

    pub fn name(self) -> &'static str {
        match self {
            Module::Cadastral => "Cadastral",
            Module::Construction => "Construction",
            Module::Mining => "Mining",
        }
    }
}

/// Which domain modules are switched on (all by default).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModuleSettings {
    pub cadastral: bool,
    pub construction: bool,
    pub mining: bool,
}

impl Default for ModuleSettings {
    fn default() -> Self {
        Self {
            cadastral: true,
            construction: true,
            mining: true,
        }
    }
}

impl ModuleSettings {
    pub fn get_mut(&mut self, m: Module) -> &mut bool {
        match m {
            Module::Cadastral => &mut self.cadastral,
            Module::Construction => &mut self.construction,
            Module::Mining => &mut self.mining,
        }
    }

    pub fn is_on(&self, m: Module) -> bool {
        match m {
            Module::Cadastral => self.cadastral,
            Module::Construction => self.construction,
            Module::Mining => self.mining,
        }
    }
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
            surface_heights: false,
            modules: ModuleSettings::default(),
            working_z: 0.0,
            csv_layout: CsvLayout::from_letters("PENZC"),
            labels: LabelSettings::default(),
            data_dir: None,
            cloud_edl: true,
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
