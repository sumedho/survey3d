//! The survey3d application: input routing, tools and file handling.
//! The interface is drawn by the submodules: [`menus`], [`panels`], [`viewport`],
//! [`overlays`] and [`dialogs`].

mod dialogs;
mod menus;
mod overlays;
mod panels;
mod viewport;

use cad_render::{GridSettings, Projection};
use egui::{Key, KeyboardShortcut, Modifiers};
use egui_cad::chrome::{bar_frame, panel_frame, section};
use egui_cad::command_line::{CommandLine, CommandLineEvent, Completion, LogKind};
use egui_cad::command_palette::{CommandPalette, PaletteItem};
use egui_cad::constraint::constrain_direction;
use egui_cad::layer_panel::{LayerAction, LayerRow, layer_panel};
use egui_cad::snap::{
    Pick, SnapEngine, SnapHit, SnapMode, SnapQuery, SnapSettings, draw_snap_glyph, free_point,
};
use egui_cad::toasts::Toasts;
use egui_cad::viewport::ViewPreset;
use egui_cad::viewport::{Viewport, ViewportState, draw_pickbox, draw_world_polyline, to_screen};
use egui_cad::{CadTheme, ThemeMode, icons};
use glam::DVec3;
use survey_core::Aabb3;
use survey_model::{
    AlignmentRecord, ChangeSet, Color, Entity, EntityId, Layer, LayerId, ParcelRecord, SheetRecord,
};
use survey_store::ProjectStore;

use crate::cloud_view::CloudSceneCache;
use crate::platform::{FileEvent, FileKind, Platform, export};
use crate::project::{APP_VERSION, Project};
use crate::scene_build::{DocSnapProvider, SceneCache, SelectionCache, color32, hit_test};
use crate::session::{Event, Panel, Session, Toast};
use crate::settings::AppSettings;
use crate::surface_view::{SurfaceIndexes, SurfaceSceneCache};
use crate::tools::grips::Grip;
use crate::tools::{Preview, SELECT_TOOL_ID, ToolEvent};
use crate::ui::alignments::AlignmentsWindow;
use crate::ui::audit_window::AuditWindow;
use crate::ui::calculator::{CalcTab, CalculatorWindow};
use crate::ui::calibration_window::{CalibrationAction, CalibrationWindow};
use crate::ui::closure_window::{ClosureAction, ClosureWindow};
use crate::ui::clouds::{CloudAction, CloudPanel};
use crate::ui::csv_import::{CsvDialogResult, CsvImportDialog};
use crate::ui::feature_codes::{CodesAction, FeatureCodesWindow};
use crate::ui::legend::LegendWindow;
use crate::ui::network::{NetworkAction, NetworkWindow};
use crate::ui::parcels::ParcelsWindow;
use crate::ui::points_table::{PointsAction, PointsWindow};
use crate::ui::projection_calc::{ProjectionAction, ProjectionWindow};
use crate::ui::properties::properties_panel;
use crate::ui::reports::{ReportAction, ReportsWindow};
use crate::ui::resection_window::{ResectionAction, ResectionWindow};
use crate::ui::sections::SectionsWindow;
use crate::ui::settings_window::SettingsWindow;
use crate::ui::sheets::{PlotFormat, SheetsWindow, ViewInfo};
use crate::ui::stakeout::StakeoutWindow;
use crate::ui::stockpile::StockpileWindow;
use crate::ui::subdivide_window::{SubdivideAction, SubdivideWindow};
use crate::ui::surfaces::{SurfaceAction, SurfaceWindow, SurfaceWindowAction};
use crate::ui::traverse_window::{TraverseAction, TraverseWindow};
use crate::ui::volumes::VolumesWindow;

const SETTINGS_KEY: &str = "survey3d.settings";

/// A command handled by the application itself (not a tool).
struct AppCommand {
    /// Command-line name.
    name: &'static str,
    title: &'static str,
    category: &'static str,
    icon: &'static str,
    /// A modifier shortcut (formatted per platform) …
    shortcut: Option<KeyboardShortcut>,
    /// … or a plain key such as `F3`.
    key: Option<&'static str>,
}

const fn cmd(
    name: &'static str,
    title: &'static str,
    category: &'static str,
    icon: &'static str,
) -> AppCommand {
    AppCommand {
        name,
        title,
        category,
        icon,
        shortcut: None,
        key: None,
    }
}

const fn with_shortcut(mut c: AppCommand, modifiers: Modifiers, key: Key) -> AppCommand {
    c.shortcut = Some(KeyboardShortcut::new(modifiers, key));
    c
}

const fn with_key(mut c: AppCommand, key: &'static str) -> AppCommand {
    c.key = Some(key);
    c
}

const SHIFT_COMMAND: Modifiers = Modifiers {
    alt: false,
    ctrl: false,
    shift: true,
    mac_cmd: false,
    command: true,
};

const APP_COMMANDS: &[AppCommand] = &[
    with_shortcut(
        cmd("undo", "Undo", "Edit", icons::ARROW_U_UP_LEFT),
        Modifiers::COMMAND,
        Key::Z,
    ),
    with_shortcut(
        cmd("redo", "Redo", "Edit", icons::ARROW_U_UP_RIGHT),
        SHIFT_COMMAND,
        Key::Z,
    ),
    with_key(cmd("delete", "Delete selection", "Edit", icons::TRASH), "Del"),
    cmd(
        "explode",
        "Explode polylines into segments",
        "Modify",
        icons::ARROWS_SPLIT,
    ),
    cmd(
        "join",
        "Join polylines that meet end to end",
        "Modify",
        icons::ARROWS_MERGE,
    ),
    cmd(
        "reverse",
        "Reverse polyline direction",
        "Modify",
        icons::ARROWS_LEFT_RIGHT,
    ),
    cmd("points", "Points table", "View", icons::TABLE),
    with_key(cmd("ze", "Zoom extents", "View", icons::FRAME_CORNERS), "Home"),
    cmd("plan", "Plan view (2D lock)", "View", icons::MAP_TRIFOLD),
    cmd("3d", "Free 3D view", "View", icons::CUBE),
    cmd("iso", "Isometric view", "View", icons::CUBE_FOCUS),
    with_key(
        cmd("pivot", "Set the centre of rotation", "View", icons::TARGET),
        "Shift+C",
    ),
    cmd("persp", "Toggle perspective", "View", icons::PERSPECTIVE),
    with_key(cmd("grid", "Toggle grid", "View", icons::GRID_FOUR), "F7"),
    with_key(cmd("snap", "Toggle object snap", "Drafting", icons::MAGNET), "F3"),
    with_key(
        cmd("gridsnap", "Toggle snap to grid", "Drafting", icons::DOTS_NINE),
        "F9",
    ),
    with_key(
        cmd("ortho", "Toggle ortho / polar", "Drafting", icons::ANGLE),
        "F8",
    ),
    with_key(
        cmd("hud", "Toggle dynamic input", "Drafting", icons::CROSSHAIR),
        "F12",
    ),
    with_shortcut(
        cmd("new", "New project", "File", icons::FILE_PLUS),
        Modifiers::COMMAND,
        Key::N,
    ),
    with_shortcut(
        cmd("open", "Open project", "File", icons::FOLDER_OPEN),
        Modifiers::COMMAND,
        Key::O,
    ),
    with_shortcut(
        cmd("save", "Save project", "File", icons::FLOPPY_DISK),
        Modifiers::COMMAND,
        Key::S,
    ),
    cmd("import", "Import points (CSV/TXT)", "File", icons::FILE_CSV),
    cmd("dxfin", "Import DXF", "File", icons::FILE_ARROW_UP),
    cmd("dxfout", "Export DXF", "File", icons::EXPORT),
    cmd("traverse", "Traverse (reduce & adjust)", "COGO", icons::STEPS),
    cmd(
        "closure",
        "Figure closure & missing values",
        "COGO",
        icons::POLYGON,
    ),
    cmd(
        "subdivide",
        "Subdivide area (parallel or swing line)",
        "COGO",
        icons::SCISSORS,
    ),
    cmd("resection", "Resection / free station", "COGO", icons::ARROWS_IN),
    cmd(
        "calc",
        "COGO calculator (curve & triangle solutions)",
        "COGO",
        icons::CALCULATOR,
    ),
    cmd("average", "Average selected points", "COGO", icons::DOTS_NINE),
    cmd(
        "surface",
        "Create surface (TIN) from points and breaklines",
        "Surface",
        icons::MAP_TRIFOLD,
    ),
    cmd(
        "volume",
        "Volumes (surface to surface or level)",
        "Surface",
        icons::CUBE,
    ),
    cmd(
        "sections",
        "Sections (long and cross sections through surfaces)",
        "Surface",
        icons::CHART_LINE,
    ),
    cmd(
        "parcels",
        "Parcels (lots, areas, plan closure)",
        "Cadastral",
        icons::POLYGON,
    ),
    cmd(
        "alignments",
        "Alignments (chainage, elements, set-out)",
        "Construction",
        icons::PATH,
    ),
    cmd(
        "mkalign",
        "Alignment from the selected polyline",
        "Construction",
        icons::SIGNPOST,
    ),
    cmd(
        "stakeout",
        "Stake-out sheet (angles and distances from a station)",
        "Construction",
        icons::TARGET,
    ),
    cmd(
        "stockpile",
        "Stockpile volume (toe, base, tonnes)",
        "Mining",
        icons::TRIANGLE,
    ),
    cmd(
        "cloudin",
        "Import point cloud (LAS, XYZ)",
        "File",
        icons::DOTS_NINE,
    ),
    cmd(
        "rawin",
        "Import raw data (Leica GSI, Trimble JobXML)",
        "File",
        icons::FILE_TEXT,
    ),
    cmd(
        "sheets",
        "Plan sheets (title block, grid, legend; print to PDF/SVG at scale)",
        "File",
        icons::MAP_TRIFOLD,
    ),
    cmd(
        "plotpdf",
        "Export the current plan sheet to PDF",
        "File",
        icons::EXPORT,
    ),
    cmd(
        "gpkg",
        "Export GeoPackage (points, lines, parcels)",
        "File",
        icons::FILE_TEXT,
    ),
    cmd(
        "netadj",
        "Least-squares network adjustment",
        "Survey",
        icons::POLYGON,
    ),
    cmd(
        "projcalc",
        "Projection calculator (lat/long, grid, scale factors)",
        "Project",
        icons::GLOBE,
    ),
    cmd(
        "sitecal",
        "Site calibration (fit GNSS points to local control)",
        "Survey",
        icons::MAP_PIN,
    ),
    cmd(
        "mkparcel",
        "Parcels from selected closed polylines",
        "Cadastral",
        icons::SQUARE_HALF,
    ),
    cmd(
        "parcelxml",
        "Export parcels to LandXML",
        "Cadastral",
        icons::EXPORT,
    ),
    cmd(
        "landxmlin",
        "Import LandXML surfaces",
        "Surface",
        icons::FILE_ARROW_UP,
    ),
    cmd(
        "landxmlout",
        "Export surfaces to LandXML",
        "Surface",
        icons::EXPORT,
    ),
    cmd(
        "contours",
        "Contours of the active surface to polylines",
        "Surface",
        icons::PATH,
    ),
    cmd(
        "legend",
        "Legend (feature code symbols and line styles)",
        "View",
        icons::SWATCHES,
    ),
    cmd(
        "accuracy",
        "Accuracy limits (misclosure checks)",
        "Project",
        icons::SHIELD_CHECK,
    ),
    cmd(
        "codes",
        "Feature codes (field-to-finish)",
        "COGO",
        icons::BOOK_OPEN,
    ),
    cmd(
        "linework",
        "Process field-to-finish linework",
        "COGO",
        icons::MAGIC_WAND,
    ),
    cmd(
        "list",
        "List selection (bearings, distances, coordinates)",
        "COGO",
        icons::LIST_BULLETS,
    ),
    cmd("reports", "Reports", "Project", icons::FILE_TEXT),
    cmd("audit", "Audit trail", "Project", icons::SHIELD_CHECK),
    cmd("conventions", "Conventions", "Project", icons::RULER),
    cmd("drafting", "Drafting settings", "Settings", icons::MAGNET),
    cmd("labels", "Label & display settings", "Settings", icons::TAG),
    cmd("theme", "Toggle light / dark theme", "Settings", icons::PALETTE),
    cmd("settings", "Settings", "Settings", icons::GEAR),
    cmd("help", "List commands", "Help", icons::KEYBOARD),
];

/// An action that would discard unsaved changes and needs confirmation.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Discard {
    New,
    Open,
    Quit,
}

pub struct SurveyApp {
    settings: AppSettings,
    theme: CadTheme,
    session: Session,
    viewport: ViewportState,
    scene_cache: SceneCache,
    selection_cache: SelectionCache,
    surface_cache: SurfaceSceneCache,
    cloud_cache: CloudSceneCache,
    cloud_panel: CloudPanel,
    surface_indexes: SurfaceIndexes,
    /// The next viewport click sets the centre of rotation.
    picking_pivot: bool,
    surface_window: SurfaceWindow,
    volumes_window: VolumesWindow,
    sections_window: SectionsWindow,
    parcels_window: ParcelsWindow,
    alignments_window: AlignmentsWindow,
    stakeout_window: StakeoutWindow,
    stockpile_window: StockpileWindow,
    projection_window: ProjectionWindow,
    network_window: NetworkWindow,
    snap_engine: SnapEngine,
    completions: Vec<Completion>,
    platform: Platform,
    /// Resolved cursor position this frame (snapped if possible).
    pick: Option<Pick>,
    /// Grips of the selection, cached by `(document revision, selection revision)`.
    grips: (Option<(u64, u64)>, Vec<Grip>),
    csv_dialog: Option<CsvImportDialog>,
    palette: CommandPalette,
    toasts: Toasts,
    settings_window: SettingsWindow,
    audit_window: AuditWindow,
    traverse_window: TraverseWindow,
    closure_window: ClosureWindow,
    calculator: CalculatorWindow,
    legend: LegendWindow,
    sheets_window: SheetsWindow,
    subdivide_window: SubdivideWindow,
    resection_window: ResectionWindow,
    calibration_window: CalibrationWindow,
    codes_window: FeatureCodesWindow,
    reports: ReportsWindow,
    points_window: PointsWindow,
    #[cfg(not(target_arch = "wasm32"))]
    copc_window: crate::ui::copc::CopcWindow,
    #[cfg(not(target_arch = "wasm32"))]
    cloud_streamer: crate::cloud_stream::CloudStreamer,
    about_open: bool,
    confirm_discard: Option<Discard>,
    allowed_to_close: bool,
    title: String,
    /// Entity counts per layer, cached by document revision.
    layer_counts: (u64, std::collections::HashMap<LayerId, usize>),
    /// A crash-recovery snapshot waiting for the user to accept or discard it.
    #[cfg(target_arch = "wasm32")]
    recovery: Option<Vec<u8>>,
    #[cfg(target_arch = "wasm32")]
    autosave: Autosave,
}

/// Camera state saved with each project so it reopens where it was left.
#[derive(serde::Serialize, serde::Deserialize)]
struct SavedView {
    target: [f64; 3],
    distance: f64,
    yaw: f64,
    pitch: f64,
    plan_locked: bool,
    perspective: bool,
}

/// Web crash-recovery snapshots of unsaved work.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct Autosave {
    /// `(document revision, pending audit entries)` last written.
    written: Option<(u64, usize)>,
    last_ms: f64,
    /// Set once the startup recovery check has finished, so a fresh session
    /// never overwrites a snapshot before the user has decided about it.
    ready: bool,
}

#[cfg(target_arch = "wasm32")]
const AUTOSAVE_INTERVAL_MS: f64 = 15_000.0;

impl SurveyApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let render_state = cc
            .wgpu_render_state
            .as_ref()
            .expect("survey3d requires the wgpu renderer");
        cad_render::install(render_state);
        egui_cad::install_fonts(&cc.egui_ctx);

        let settings: AppSettings = cc
            .storage
            .and_then(|s| eframe::get_value(s, SETTINGS_KEY))
            .unwrap_or_default();
        let theme = CadTheme::new(settings.theme, settings.accent, settings.density);
        theme.apply(&cc.egui_ctx);

        let project = Project::new(&settings).expect("in-memory SQLite is available");
        let mut session = Session::new(project);
        let mut completions: Vec<Completion> = session
            .tools
            .tools()
            .iter()
            .flat_map(|t| {
                t.commands.iter().map(|c| Completion {
                    name: (*c).to_string(),
                    description: t.description.to_string(),
                })
            })
            .collect();
        completions.extend(APP_COMMANDS.iter().map(|c| Completion {
            name: c.name.to_string(),
            description: c.title.to_string(),
        }));
        completions.sort_by(|a, b| a.name.cmp(&b.name));

        let mut viewport = ViewportState::new(1);
        viewport.camera.target = DVec3::ZERO;

        let platform = Platform::default();
        let mut module_names: Vec<&str> = session.module_names().collect();
        module_names.extend(
            crate::settings::Module::ALL
                .into_iter()
                .filter(|m| settings.modules.is_on(*m))
                .map(|m| m.name()),
        );
        let banner = format!(
            "survey3d {APP_VERSION} — modules: {} — type 'help' for commands.",
            module_names.join(", ")
        );
        session.log.info(banner);

        #[cfg(target_arch = "wasm32")]
        platform.check_recovery(&cc.egui_ctx);

        Self {
            settings,
            theme,
            session,
            viewport,
            scene_cache: SceneCache::default(),
            selection_cache: SelectionCache::default(),
            surface_cache: SurfaceSceneCache::default(),
            cloud_cache: CloudSceneCache::default(),
            cloud_panel: CloudPanel::default(),
            surface_indexes: SurfaceIndexes::default(),
            picking_pivot: false,
            surface_window: SurfaceWindow::default(),
            volumes_window: VolumesWindow::default(),
            sections_window: SectionsWindow::default(),
            parcels_window: ParcelsWindow::default(),
            alignments_window: AlignmentsWindow::default(),
            stakeout_window: StakeoutWindow::default(),
            stockpile_window: StockpileWindow::default(),
            projection_window: ProjectionWindow::default(),
            network_window: NetworkWindow::default(),
            snap_engine: SnapEngine::default(),
            completions,
            platform,
            pick: None,
            grips: (None, Vec::new()),
            csv_dialog: None,
            palette: CommandPalette::default(),
            toasts: Toasts::default(),
            settings_window: SettingsWindow::default(),
            audit_window: AuditWindow::default(),
            traverse_window: TraverseWindow::default(),
            closure_window: ClosureWindow::default(),
            calculator: CalculatorWindow::default(),
            legend: LegendWindow::default(),
            sheets_window: SheetsWindow::default(),
            subdivide_window: SubdivideWindow::default(),
            resection_window: ResectionWindow::default(),
            calibration_window: CalibrationWindow::default(),
            codes_window: FeatureCodesWindow::default(),
            reports: ReportsWindow::default(),
            points_window: PointsWindow::default(),
            #[cfg(not(target_arch = "wasm32"))]
            copc_window: crate::ui::copc::CopcWindow::default(),
            #[cfg(not(target_arch = "wasm32"))]
            cloud_streamer: crate::cloud_stream::CloudStreamer::default(),
            about_open: false,
            confirm_discard: None,
            allowed_to_close: false,
            title: String::new(),
            layer_counts: (0, Default::default()),
            #[cfg(target_arch = "wasm32")]
            recovery: None,
            #[cfg(target_arch = "wasm32")]
            autosave: Autosave::default(),
        }
    }

    // ------------------------------------------------------------------ session

    /// Shows what the session asked for: toasts, reports, panels, view changes.
    fn handle_session_events(&mut self, ctx: &egui::Context) {
        let events = self.session.take_events();
        if !events.is_empty() {
            ctx.request_repaint();
        }
        for event in events {
            match event {
                Event::Toast(Toast::Success, text) => self.toasts.success(text),
                Event::Toast(Toast::Warning, text) => self.toasts.warning(text),
                Event::Report { title, text } => self.reports.add(title, text),
                Event::ZoomExtents => self.zoom_extents(),
                Event::Open(Panel::Parcels) => self.parcels_window.open = true,
                Event::Open(Panel::Alignments) => self.alignments_window.open = true,
                Event::Open(Panel::FeatureCodes) => self.codes_window.open(),
                Event::Open(Panel::Network { source }) => {
                    self.network_window.open_with(&self.session.project.doc);
                    self.network_window.select_only(source);
                }
                Event::SelectCloud(id) => self.cloud_panel.active = Some(id),
            }
        }
    }

    fn submit(&mut self, ctx: &egui::Context, text: String) {
        if !self.session.submit(&text) && !self.run_app_command(ctx, text.trim()) {
            self.session.submit_unhandled(text.trim());
        }
    }

    fn apply_surface_action(&mut self, action: SurfaceAction) {
        match action {
            SurfaceAction::New => self.surface_window.create(&self.session.project.doc),
            SurfaceAction::Edit(id) => self.surface_window.edit(&self.session.project.doc, id),
            SurfaceAction::Delete(id) => self.session.delete_surface(id),
            SurfaceAction::Style(id, style) => self.session.set_surface_style(id, style),
            SurfaceAction::Rebuild(id) => self.session.rebuild_surface(id),
        }
    }

    fn apply_surface_window(&mut self, action: SurfaceWindowAction) {
        match action {
            SurfaceWindowAction::Create(surface) => self.session.create_surface(surface),
            SurfaceWindowAction::Update(id, surface) => self.session.update_surface(id, surface),
        }
    }

    fn apply_cloud_action(&mut self, action: CloudAction) {
        match action {
            CloudAction::Style(id, style) => self.session.set_cloud_style(id, style),
            CloudAction::Delete(id) => self.session.delete_cloud(id),
            CloudAction::Thin(id, spacing) => self.session.thin_cloud(id, spacing),
            CloudAction::Surface(id, cell, thin, max_edge) => {
                self.session.surface_from_cloud(id, cell, thin, max_edge);
            }
        }
    }

    /// Writes the active surface (or every surface when none is active) as LandXML.
    fn export_landxml(&mut self) {
        let Some((stem, xml)) = self.session.surfaces_landxml() else {
            return;
        };
        match export(&self.platform, FileKind::LandXml, &stem, xml.as_bytes()) {
            Ok(Some(where_)) => self.session.log.result(format!("Exported surfaces to {where_}")),
            Ok(None) => {}
            Err(e) => self.session.log.error(e),
        }
    }

    fn export_parcels_landxml(&mut self) {
        let Some(xml) = self.session.parcels_landxml() else {
            return;
        };
        match export(&self.platform, FileKind::LandXml, "parcels", xml.as_bytes()) {
            Ok(Some(where_)) => self.session.log.result(format!("Exported parcels to {where_}")),
            Ok(None) => {}
            Err(e) => self.session.log.error(e),
        }
    }

    fn import_code_sheet(&mut self, name: &str, bytes: &[u8]) {
        match survey_io::code_sheet::read_code_sheet(bytes) {
            Ok((codes, warnings)) => {
                let n = self.codes_window.merge(&self.session.project.doc, codes);
                self.session
                    .log
                    .info(format!("Read {n} feature codes from {name} — Apply to keep them"));
                for w in warnings {
                    self.session.log.warn(w);
                }
            }
            Err(e) => self.session.log.error(format!("Could not read {name}: {e}")),
        }
    }

    fn zoom_extents(&mut self) {
        // A new framing starts orbiting about the view again.
        self.viewport.pivot = None;
        let doc = &self.session.project.doc;
        let b = doc
            .surfaces()
            .filter(|(_, s)| s.style.visible)
            .fold(doc.bounds(), |b, (_, s)| b.union(&s.tin.bounds()));
        let b = doc
            .clouds()
            .filter(|(_, c)| c.style.visible)
            .fold(b, |b, (_, c)| b.union(&c.full_bounds()));
        if b.is_empty() {
            self.viewport.camera.target = DVec3::ZERO;
            self.viewport.camera.distance = 100.0;
        } else {
            self.viewport.camera.zoom_to_bounds(&b);
        }
    }

    fn toggle_plan(&mut self) {
        let locked = !self.viewport.plan_locked;
        self.viewport.set_plan_locked(locked);
        self.session
            .log
            .info(if locked { "Plan view (2D lock)" } else { "3D view" });
    }

    fn toggle_projection(&mut self) {
        if self.viewport.plan_locked {
            self.session.log.warn("Perspective is only available in 3D view");
            return;
        }
        let next = match self.viewport.camera.projection {
            Projection::Perspective => Projection::Orthographic,
            Projection::Orthographic => Projection::Perspective,
        };
        self.viewport.set_projection(next);
    }

    fn toggle_snap(&mut self) {
        self.settings.snap.enabled = !self.settings.snap.enabled;
        let state = if self.settings.snap.enabled { "on" } else { "off" };
        self.session.log.info(format!("Snap {state}"));
    }

    fn toggle_grid_snap(&mut self) {
        self.settings.snap.toggle(SnapMode::Grid);
        let state = if self.settings.snap.is_on(SnapMode::Grid) {
            "on"
        } else {
            "off"
        };
        self.session.log.info(format!("Grid snap {state}"));
    }

    fn toggle_ortho(&mut self) {
        self.settings.ortho.enabled = !self.settings.ortho.enabled;
        let state = if self.settings.ortho.enabled { "on" } else { "off" };
        self.session.log.info(format!("Ortho {state}"));
    }

    // ------------------------------------------------------------------ command line

    /// Everything searchable in the command palette.
    fn palette_items(&self, ctx: &egui::Context) -> Vec<PaletteItem> {
        let mut items: Vec<PaletteItem> = self
            .session
            .tools
            .tools()
            .iter()
            .map(|t| PaletteItem {
                id: format!("tool:{}", t.id),
                title: t.name.to_string(),
                category: format!("{} tool", t.group.label()),
                description: t.description.to_string(),
                shortcut: Some(t.commands[0].to_string()),
                icon: t.icon,
            })
            .collect();
        items.extend(APP_COMMANDS.iter().map(|c| {
            PaletteItem {
                id: format!("cmd:{}", c.name),
                title: c.title.to_string(),
                category: c.category.to_string(),
                description: format!("Command: {}", c.name),
                shortcut: c
                    .shortcut
                    .map(|s| ctx.format_shortcut(&s))
                    .or(c.key.map(str::to_string)),
                icon: c.icon,
            }
        }));
        items
    }

    fn run_palette_choice(&mut self, ctx: &egui::Context, id: &str) {
        if let Some(tool) = id.strip_prefix("tool:") {
            self.session.set_tool(tool);
        } else if let Some(name) = id.strip_prefix("cmd:") {
            self.session.log.log(LogKind::Command, format!("Command: {name}"));
            self.run_app_command(ctx, name);
        }
    }

    /// Session commands, then view, file and window commands.
    fn run_app_command(&mut self, ctx: &egui::Context, text: &str) -> bool {
        // The application's help also lists its own (window, view, file) commands.
        if matches!(text.to_ascii_lowercase().as_str(), "help" | "?") {
            let mut lines = vec!["Tools:".to_string()];
            for t in self.session.tools.tools() {
                lines.push(format!("  {:<18} {}", t.commands.join(", "), t.description));
            }
            lines.push("Commands:".into());
            for c in APP_COMMANDS {
                lines.push(format!("  {:<18} {}", c.name, c.title));
            }
            lines.push(crate::session::POINT_INPUT_HELP.into());
            self.session.log.info(lines.join("\n"));
            return true;
        }
        if self.session.run_command(text) {
            return true;
        }
        match text.to_ascii_lowercase().as_str() {
            "ze" | "zoomextents" | "extents" => self.zoom_extents(),
            "plan" | "2d" => {
                if !self.viewport.plan_locked {
                    self.toggle_plan();
                }
            }
            "3d" => {
                if self.viewport.plan_locked {
                    self.toggle_plan();
                }
            }
            "persp" | "perspective" => self.toggle_projection(),
            "pivot" | "orbitcentre" | "orbitcenter" => self.arm_pivot(),
            "snap" | "osnap" => self.toggle_snap(),
            "ortho" => self.toggle_ortho(),
            "gridsnap" | "snapgrid" => self.toggle_grid_snap(),
            "grid" => self.settings.show_grid = !self.settings.show_grid,
            "import" => self.platform.pick_file(ctx, FileKind::Points),
            "dxfin" | "importdxf" => self.platform.pick_file(ctx, FileKind::Dxf),
            "dxfout" | "exportdxf" => self.export_dxf(),
            "labels" => self.settings_window.open_at_display(),
            "save" => self.save(ctx),
            "open" => self.request_discard(ctx, Discard::Open),
            "new" => self.request_discard(ctx, Discard::New),
            "audit" => self.audit_window.open = true,
            "traverse" | "trav" => self.traverse_window.open = true,
            "closure" | "clo" => match self.session.selected_polylines().first() {
                Some((_, line)) => self.closure_window.load_polyline(&self.session.project.doc, line),
                None => self.closure_window.open = true,
            },
            "subdivide" | "subdiv" => {
                let parcel = self
                    .session
                    .selected_polylines()
                    .into_iter()
                    .find(|(_, l)| l.closed);
                match parcel {
                    Some((id, line)) => {
                        self.subdivide_window.load(id, &line);
                    }
                    None => self.subdivide_window.open = true,
                }
            }
            "resection" | "resect" | "freestation" => self.resection_window.open = true,
            "calc" | "calculator" | "curve" => self.calculator.open_at(CalcTab::Curve),
            "triangle" => self.calculator.open_at(CalcTab::Triangle),
            "legend" => self.legend.open = true,
            "sheets" | "sheet" | "layout" => self.sheets_window.open = true,
            "plotpdf" | "plot" | "print" => self.plot_current_sheet(),
            "surface" | "tin" => self.surface_window.create(&self.session.project.doc),
            "parcels" => self.parcels_window.open = true,
            "alignments" | "align" => self.alignments_window.open = true,
            "stakeout" | "setoutsheet" => self.stakeout_window.open = true,
            "stockpile" => {
                let toe = self
                    .session
                    .selected_polylines()
                    .into_iter()
                    .find(|(_, l)| l.closed)
                    .map(|(id, _)| id);
                self.stockpile_window
                    .open_with(&self.session.project.doc, self.session.active_surface, toe);
            }
            "parcelxml" => self.export_parcels_landxml(),
            "cloudin" | "importcloud" | "las" => self.platform.pick_file(ctx, FileKind::PointCloud),
            #[cfg(not(target_arch = "wasm32"))]
            "copc" | "copcin" => self.copc_window.open(),
            "rawin" | "gsi" | "jobxml" => self.platform.pick_file(ctx, FileKind::RawData),
            "gpkg" | "geopackage" => self.export_geopackage(),
            "landxmlin" | "importxml" => self.platform.pick_file(ctx, FileKind::LandXml),
            "landxmlout" | "exportxml" => self.export_landxml(),
            "sections" | "section" | "profile" => {
                let line = self.session.selected_polylines().first().map(|(id, _)| *id);
                self.sections_window.open_with(line);
            }
            "volume" | "volumes" | "vol" => self
                .volumes_window
                .open(&self.session.project.doc, self.session.active_surface),
            "projcalc" | "projection" | "geo" => self.projection_window.open = true,
            "sitecal" | "calibrate" | "calibration" | "localise" | "localize" => {
                self.calibration_window.open = true
            }
            "netadj" | "adjust" | "lsa" => self.network_window.open_with(&self.session.project.doc),
            "accuracy" | "tolerances" => self.settings_window.open_at_accuracy(),
            "codes" | "featurecodes" => self.codes_window.open(),
            "reports" => self.reports.open = true,
            "points" | "pts" | "table" => self.points_window.open = true,
            "settings" | "options" => self.settings_window.open = true,
            "drafting" | "dsettings" => self.settings_window.open_at_drafting(),
            "conventions" => self.settings_window.open_at_conventions(),
            "iso" | "isometric" => {
                self.viewport.apply_preset(ViewPreset::Isometric);
            }
            "hud" | "dyn" => {
                self.settings.dynamic_input = !self.settings.dynamic_input;
                let state = if self.settings.dynamic_input { "on" } else { "off" };
                self.session.log.info(format!("Dynamic input {state}"));
            }
            "theme" => {
                self.settings.theme = match self.settings.theme {
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::Dark,
                };
            }
            _ => return false,
        }
        true
    }

    // ------------------------------------------------------------------ files

    fn request_discard(&mut self, ctx: &egui::Context, action: Discard) {
        if self.session.project.is_dirty() {
            self.confirm_discard = Some(action);
        } else {
            self.perform_discard(ctx, action);
        }
    }

    fn perform_discard(&mut self, ctx: &egui::Context, action: Discard) {
        match action {
            Discard::New => match Project::new(&self.settings) {
                Ok(p) => {
                    self.replace_project(p);
                    self.session.log.info("New project");
                }
                Err(e) => self.session.log.error(format!("Could not create project: {e}")),
            },
            Discard::Open => self.platform.pick_file(ctx, FileKind::Project),
            Discard::Quit => {
                self.allowed_to_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn replace_project(&mut self, project: Project) {
        self.viewport.pivot = None;
        self.picking_pivot = false;
        self.session.replace_project(project);
        self.scene_cache.invalidate();
        self.selection_cache.invalidate();
        self.surface_cache.invalidate();
        self.cloud_cache.invalidate();
        // Cloud ids restart with each project: drop selections keyed by them.
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.cloud_streamer = Default::default();
        }
        self.cloud_panel.active = None;
        self.surface_indexes.invalidate();
        // Revisions restart with each document, so drop caches keyed by them.
        self.layer_counts = (u64::MAX, Default::default());
        if !self.restore_view() {
            self.zoom_extents();
        }
    }

    /// Restores the camera saved with the project; `false` if there is none.
    fn restore_view(&mut self) -> bool {
        let Some(view) = self
            .session
            .project
            .store
            .read_view_state()
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str::<SavedView>(&json).ok())
        else {
            return false;
        };
        let vp = &mut self.viewport;
        vp.set_plan_locked(view.plan_locked);
        let cam = &mut vp.camera;
        cam.target = DVec3::from_array(view.target);
        cam.distance = view.distance;
        cam.yaw = view.yaw;
        cam.pitch = view.pitch;
        cam.projection = if view.perspective && !view.plan_locked {
            Projection::Perspective
        } else {
            Projection::Orthographic
        };
        true
    }

    /// Writes the camera into the project store (display state, not audited).
    fn store_view(&self) {
        let cam = &self.viewport.camera;
        let view = SavedView {
            target: cam.target.to_array(),
            distance: cam.distance,
            yaw: cam.yaw,
            pitch: cam.pitch,
            plan_locked: self.viewport.plan_locked,
            perspective: cam.projection == Projection::Perspective,
        };
        if let Ok(json) = serde_json::to_string(&view)
            && let Err(e) = self.session.project.store.write_view_state(&json)
        {
            log::warn!("could not store view: {e}");
        }
    }

    fn export_geopackage(&mut self) {
        let stem = file_stem(&self.session.project.doc.meta().name);
        let result = survey_store::geopackage::export_geopackage(&self.session.project.doc)
            .map_err(|e| e.to_string())
            .and_then(|(bytes, s)| {
                export(&self.platform, FileKind::GeoPackage, &stem, &bytes)
                    .map(|w| w.map(|w| (w, s)))
                    .map_err(|e| e.to_string())
            });
        match result {
            Ok(Some((where_, s))) => {
                let msg = format!(
                    "Exported GeoPackage: {} points, {} lines, {} parcels",
                    s.points, s.lines, s.parcels
                );
                self.session.log.info(format!("{msg} — {where_}"));
                self.toasts.success(msg);
            }
            Ok(None) => {}
            Err(e) => self.session.log.error(format!("GeoPackage export failed: {e}")),
        }
    }

    /// What the viewport shows, for placing plan sheets.
    pub(crate) fn view_info(&self) -> ViewInfo {
        let cam = &self.viewport.camera;
        let bounds = (self.session.project.doc.entity_count() > 0).then(|| {
            let b = self.session.project.doc.bounds();
            (b.min.truncate(), b.max.truncate())
        });
        let mut up = cam.yaw.to_degrees().rem_euclid(360.0);
        if up > 180.0 {
            up -= 360.0;
        }
        ViewInfo {
            centre: cam.target.truncate(),
            up_bearing: if cam.is_plan() { up } else { 0.0 },
            extent: cam.viewport * cam.world_per_pixel(),
            bounds,
        }
    }

    /// Point labels on plan sheets: the global label settings.
    pub(crate) fn plan_labels(&self) -> survey_core::style::LabelSet {
        survey_core::style::LabelSet {
            name: self.settings.labels.names,
            code: self.settings.labels.codes,
            height: self.settings.labels.heights,
            description: false,
        }
    }

    /// Exports a plan sheet as PDF or SVG.
    pub(crate) fn export_sheet(&mut self, sheet: &survey_model::SheetRecord, format: PlotFormat) {
        let plot = crate::plan::build_plot(&self.session.project.doc, sheet, self.plan_labels());
        let (kind, bytes, what) = match format {
            PlotFormat::Pdf => (FileKind::Pdf, survey_io::plot::write_pdf(&plot), "PDF"),
            PlotFormat::Svg => (
                FileKind::Svg,
                survey_io::plot::write_svg(&plot).into_bytes(),
                "SVG",
            ),
        };
        let stem = file_stem(&format!(
            "{} {}",
            self.session.project.doc.meta().name,
            sheet.name
        ));
        match export(&self.platform, kind, &stem, &bytes) {
            Ok(Some(where_)) => {
                self.session.log.info(format!(
                    "Exported {} ({}) to {where_}",
                    sheet.name,
                    sheet.scale_label()
                ));
                self.toasts.success(format!("Exported {what}"));
            }
            Ok(None) => {}
            Err(e) => self.session.log.error(format!("{what} export failed: {e}")),
        }
    }

    /// The sheet chosen in the sheets window, else the first.
    fn current_sheet(&self) -> Option<&survey_model::SheetRecord> {
        let sheets = self.session.project.doc.sheets();
        self.sheets_window
            .current
            .and_then(|id| sheets.iter().find(|s| s.id == id))
            .or(sheets.first())
            .map(|s| &s.record)
    }

    /// Exports the sheet open in the sheets window (or the first) to PDF.
    fn plot_current_sheet(&mut self) {
        let Some(sheet) = self.current_sheet().cloned() else {
            self.sheets_window.open = true;
            self.session
                .log
                .warn("There are no plan sheets yet: frame the area and press “New from view”");
            return;
        };
        self.export_sheet(&sheet, PlotFormat::Pdf);
    }

    fn export_dxf(&mut self) {
        // Symbols and text are sized for the current plan sheet's scale.
        let (text, opts) = self.session.dxf(self.current_sheet());
        let stem = file_stem(&self.session.project.doc.meta().name);
        match export(&self.platform, FileKind::Dxf, &stem, text.as_bytes()) {
            Ok(Some(where_)) => {
                self.session.log.info(format!(
                    "Exported DXF: {where_} (symbols and names sized for 1:{})",
                    opts.scale
                ));
                self.toasts.success("Exported DXF");
            }
            Ok(None) => {}
            Err(e) => self.session.log.error(format!("DXF export failed: {e}")),
        }
    }

    /// Opens a project file (e.g. one named on the command line).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_path(&mut self, path: &std::path::Path) {
        let name = crate::platform::file_name(path);
        self.open_store(ProjectStore::open_file(path), name);
    }

    fn open_store(&mut self, result: Result<ProjectStore, survey_store::StoreError>, name: String) {
        let project = result.and_then(|store| Project::from_store(store, name.clone(), &self.settings));
        match project {
            Ok(p) => {
                let n = p.doc.entity_count();
                self.replace_project(p);
                #[cfg(not(target_arch = "wasm32"))]
                self.resolve_clouds();
                self.session.log.info(format!("Opened {name} ({n} objects)"));
                self.toasts.success(format!("Opened {name}"));
            }
            Err(e) => {
                self.session.log.error(format!("Could not open {name}: {e}"));
                self.toasts.error(format!("Could not open {name}"));
            }
        }
    }

    /// Re-reads the resident working set of every out-of-core (COPC) cloud
    /// from disk after a project opens. The project file holds only references,
    /// so without this the clouds would be empty. A missing file is reported
    /// and left for the user to re-link; it does not fail the open.
    #[cfg(not(target_arch = "wasm32"))]
    fn resolve_clouds(&mut self) {
        // A light overview so the cloud is visible immediately and zoom-extents
        // is meaningful; the streamer refines it to the view from the first frame.
        const INITIAL_PREVIEW: usize = 2_000_000;
        let jobs: Vec<(survey_model::PointCloudId, std::path::PathBuf, String)> = self
            .session
            .project
            .doc
            .clouds()
            .filter_map(|(id, c)| c.source.as_ref().map(|s| (id, s.path.clone(), c.name.clone())))
            .collect();
        for (id, path, name) in jobs {
            let Some(found) = self.resolve_cloud_path(&path) else {
                self.session.log.warn(format!(
                    "Point cloud {name}: {} not found. Re-link it from File → Point clouds (COPC).",
                    path.display()
                ));
                continue;
            };
            match survey_io::copc::read_copc_coarse(&found, INITIAL_PREVIEW) {
                Ok((raw, _)) => {
                    self.session
                        .project
                        .doc
                        .set_cloud_resident(id, survey_model::Resident::new(&raw.points, raw.colors));
                }
                Err(e) => self.session.log.warn(format!(
                    "Point cloud {name}: could not read {}: {e}",
                    found.display()
                )),
            }
        }
    }

    /// Drives view-dependent streaming of out-of-core clouds: takes in
    /// finished selections, requests fresh ones for the current view, and
    /// returns what the renderer should draw. Called once per frame.
    #[cfg(not(target_arch = "wasm32"))]
    fn stream_clouds(&mut self, ctx: &egui::Context) -> Option<std::sync::Arc<cad_render::CloudFrame>> {
        self.cloud_streamer.drain();
        let any = self
            .session
            .project
            .doc
            .clouds()
            .any(|(_, c)| c.is_out_of_core() && c.style.visible);
        if !any {
            return None;
        }
        let cam = &self.viewport.camera;
        let vp = cam.viewport;
        let resolution = cam.world_per_pixel();
        // The view's frustum through a screen rectangle 1.5x the viewport, so
        // a small pan or orbit is already covered. Unlike a box on the target
        // plane, it reaches an oblique view's horizon.
        let margin = vp * 0.25;
        let frustum = cam.rect_frustum(-margin, vp + margin);
        let eye = (cam.projection == cad_render::Projection::Perspective).then(|| (cam.eye(), cam.distance));
        let key = crate::cloud_stream::ViewKey {
            target: cam.target,
            eye: cam.eye(),
            span: resolution * vp.length(),
            resolution,
        };
        let mut jobs: Vec<(survey_model::PointCloudId, std::path::PathBuf)> = Vec::new();
        for (id, c) in self.session.project.doc.clouds() {
            if !c.style.visible {
                continue;
            }
            let Some(src) = &c.source else { continue };
            if !frustum.intersects_aabb(&src.full_bounds) {
                continue; // the view does not see this cloud
            }
            if let Some(path) = self.resolve_cloud_path(&src.path) {
                jobs.push((id, path));
            }
        }
        for (id, path) in jobs {
            let view = survey_io::copc::ViewQuery {
                frustum: frustum.planes().to_vec(),
                world_per_pixel: resolution,
                eye,
                spacing_px: crate::cloud_stream::SPACING_PX,
            };
            self.cloud_streamer.request(id, path, view, key);
        }
        if self.cloud_streamer.is_busy() {
            ctx.request_repaint();
        }
        self.cloud_streamer
            .frame(&self.session.project.doc, self.settings.cloud_edl)
    }

    /// Finds a referenced COPC file: at its stored path, or by file name in the
    /// data directory when it has moved.
    #[cfg(not(target_arch = "wasm32"))]
    fn resolve_cloud_path(&self, path: &std::path::Path) -> Option<std::path::PathBuf> {
        if path.exists() {
            return Some(path.to_path_buf());
        }
        let name = path.file_name()?;
        let candidate = self.settings.data_dir.as_ref()?.join(name);
        candidate.exists().then_some(candidate)
    }

    fn save(&mut self, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            if self.session.project.has_file() {
                self.store_view();
                match self.session.project.save() {
                    Ok(()) => {
                        self.session
                            .log
                            .info(format!("Saved {}", self.session.project.display_name));
                        self.toasts.success("Saved");
                    }
                    Err(e) => {
                        self.session.log.error(format!("Save failed: {e}"));
                        self.toasts.error("Save failed — see the command line");
                    }
                }
            } else {
                self.save_as();
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
            let name = format!(
                "{}.{}",
                file_stem(&self.session.project.doc.meta().name),
                survey_store::FILE_EXTENSION
            );
            self.store_view();
            match self
                .session
                .project
                .save_to_bytes()
                .map_err(|e| e.to_string())
                .and_then(|bytes| crate::platform::download(&name, &bytes))
            {
                Ok(()) => {
                    self.session.project.display_name = name.clone();
                    self.session.log.info(format!("Downloaded {name}"));
                    self.toasts.success(format!("Saved {name}"));
                    // The download is now the user's copy; drop the recovery snapshot.
                    self.autosave.written = None;
                    wasm_bindgen_futures::spawn_local(async {
                        let _ = crate::platform::opfs::remove(crate::platform::opfs::AUTOSAVE).await;
                    });
                }
                Err(e) => {
                    self.session.log.error(format!("Save failed: {e}"));
                    self.toasts.error("Save failed — see the command line");
                }
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save_as(&mut self) {
        let default = file_stem(&self.session.project.doc.meta().name);
        if let Some(path) = self.platform.pick_save_path(FileKind::Project, &default) {
            self.store_view();
            match self.session.project.save_as(&path) {
                Ok(()) => {
                    self.session.log.info(format!("Saved {}", path.display()));
                    self.toasts.success("Saved");
                }
                Err(e) => {
                    self.session.log.error(format!("Save failed: {e}"));
                    self.toasts.error("Save failed — see the command line");
                }
            }
        }
    }

    fn handle_file_events(&mut self) {
        for event in self.platform.poll() {
            match event {
                FileEvent::Picked { kind, name, bytes } => match kind {
                    FileKind::Project => self.open_store(ProjectStore::from_bytes(&bytes), name),
                    FileKind::Points => {
                        self.csv_dialog = Some(CsvImportDialog::new(
                            name,
                            bytes,
                            self.settings.csv_layout.clone(),
                        ));
                    }
                    FileKind::Dxf => self.session.import_dxf(&name, &bytes),
                    FileKind::Codes => self.import_code_sheet(&name, &bytes),
                    FileKind::LandXml => self.session.import_landxml(&name, &bytes),
                    FileKind::PointCloud => self.session.import_cloud(&name, &bytes),
                    FileKind::RawData => self.session.import_raw(&name, &bytes),
                    // Reports and GeoPackages are only saved, never opened.
                    FileKind::Report | FileKind::GeoPackage | FileKind::Pdf | FileKind::Svg => {}
                },
                #[cfg(not(target_arch = "wasm32"))]
                FileEvent::ProjectPath(path) => self.open_path(&path),
                #[cfg(target_arch = "wasm32")]
                FileEvent::Recovery { bytes } => {
                    self.recovery = bytes;
                    self.autosave.ready = true;
                }
            }
        }
    }

    /// Periodically snapshots unsaved work to browser storage (web).
    #[cfg(target_arch = "wasm32")]
    fn autosave(&mut self, ctx: &egui::Context) {
        let now = js_sys::Date::now();
        let key = (
            self.session.project.doc.revision(),
            self.session.project.history.pending_audit().len(),
        );
        let due = now - self.autosave.last_ms >= AUTOSAVE_INTERVAL_MS;
        // Only real edits (which always produce audit records) are worth recovering.
        let edited = !self.session.project.history.pending_audit().is_empty();
        let blocked = !self.autosave.ready || self.recovery.is_some();
        if blocked || !edited || self.autosave.written == Some(key) || !due {
            if edited {
                ctx.request_repaint_after(std::time::Duration::from_secs(5));
            }
            return;
        }
        self.autosave.last_ms = now;
        self.store_view();
        match self.session.project.snapshot() {
            Ok(bytes) => {
                self.autosave.written = Some(key);
                wasm_bindgen_futures::spawn_local(async move {
                    if let Err(e) =
                        crate::platform::opfs::write(crate::platform::opfs::AUTOSAVE, &bytes).await
                    {
                        log::warn!("autosave failed: {e:?}");
                    }
                });
            }
            Err(e) => log::warn!("autosave snapshot failed: {e}"),
        }
    }
}

fn file_stem(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.trim().is_empty() {
        "project".into()
    } else {
        s.trim().to_string()
    }
}

// ====================================================================== frame

impl eframe::App for SurveyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let s = &self.settings;
        if (self.theme.mode, self.theme.accent_family, self.theme.density) != (s.theme, s.accent, s.density) {
            self.theme = CadTheme::new(s.theme, s.accent, s.density);
            self.theme.apply(&ctx);
        }
        self.session
            .project
            .history
            .set_user(self.settings.user_name.clone());
        self.handle_file_events();
        #[cfg(target_arch = "wasm32")]
        self.autosave(&ctx);
        self.handle_close_request(&ctx);
        self.handle_shortcuts(&ctx);
        self.update_title(&ctx);

        let theme = self.theme.clone();
        egui::Panel::top("menu_bar")
            .frame(bar_frame(&theme))
            .show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status_bar")
            .frame(bar_frame(&theme))
            .show(ui, |ui| self.status_bar(ui));
        egui::Panel::bottom("command_line")
            .frame(panel_frame(&theme))
            .resizable(false)
            .show(ui, |ui| self.command_line_ui(ui));
        egui::Panel::left("tool_palette")
            .frame(panel_frame(&theme))
            .exact_size(46.0)
            .resizable(false)
            .show(ui, |ui| self.tool_palette(ui));
        egui::Panel::right("side_panel")
            .frame(panel_frame(&theme))
            .resizable(true)
            .default_size(300.0)
            .size_range(220.0..=520.0)
            .show(ui, |ui| self.side_panel(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewport_ui(ui));

        self.dialogs(&ctx);
        self.handle_session_events(&ctx);
        if self.palette.is_open() {
            let items = self.palette_items(&ctx);
            if let Some(id) = self.palette.show(&ctx, &items) {
                self.run_palette_choice(&ctx, &id);
            }
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
    }
}

impl SurveyApp {
    fn update_title(&mut self, ctx: &egui::Context) {
        let dirty = if self.session.project.is_dirty() {
            " •"
        } else {
            ""
        };
        let title = format!("{}{dirty} — survey3d", self.session.project.display_name);
        if title != self.title {
            #[cfg(not(target_arch = "wasm32"))]
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            #[cfg(target_arch = "wasm32")]
            {
                let _ = ctx; // the web backend ignores viewport title commands
                if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                    doc.set_title(&title);
                }
            }
            self.title = title;
        }
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allowed_to_close
            && self.session.project.is_dirty()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_discard = Some(Discard::Quit);
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let sc = |m: Modifiers, k: Key| KeyboardShortcut::new(m, k);
        let shift_cmd = Modifiers::COMMAND | Modifiers::SHIFT;
        let (palette, f12) = ctx.input_mut(|i| {
            (
                i.consume_shortcut(&sc(Modifiers::COMMAND, Key::K)),
                i.consume_key(Modifiers::NONE, Key::F12),
            )
        });
        if palette {
            self.palette.toggle();
        }
        if f12 {
            self.run_app_command(ctx, "hud");
        }
        // Shift+C picks the centre of rotation, unless it is the start of
        // typed input or a feature code (codes are often capitals).
        let typing = !self.session.log.input.trim().is_empty() || self.session.active_tool.takes_codes();
        if !typing
            && ctx.input_mut(|i| {
                // Text typed earlier in the same frame has not reached the
                // command line yet: Shift+C is then part of that typing.
                let typed_before = i
                    .events
                    .iter()
                    .take_while(|e| {
                        !matches!(
                            e,
                            egui::Event::Key {
                                key: Key::C,
                                pressed: true,
                                ..
                            }
                        )
                    })
                    .any(|e| matches!(e, egui::Event::Text(t) if !t.trim().is_empty() && t != "C"));
                if typed_before {
                    return false;
                }
                // The key event, or (when a text field has focus, some
                // platforms send only text) a "C" typed with Shift held.
                let shifted_c = i.modifiers.shift
                    && i.events
                        .iter()
                        .any(|e| matches!(e, egui::Event::Text(t) if t == "C"));
                let hit = i.consume_key(Modifiers::SHIFT, Key::C) || shifted_c;
                if hit {
                    // Its typed "C" must not reach the command line.
                    i.events
                        .retain(|e| !matches!(e, egui::Event::Text(t) if t == "C"));
                }
                hit
            })
        {
            self.arm_pivot();
        }
        if self.palette.is_open() {
            return; // the palette owns the keyboard while open
        }
        let (undo, redo, redo_y, save, save_as, open, new) = ctx.input_mut(|i| {
            (
                i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Z)),
                i.consume_shortcut(&sc(shift_cmd, Key::Z)),
                i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Y)),
                i.consume_shortcut(&sc(Modifiers::COMMAND, Key::S)),
                i.consume_shortcut(&sc(shift_cmd, Key::S)),
                i.consume_shortcut(&sc(Modifiers::COMMAND, Key::O)),
                i.consume_shortcut(&sc(Modifiers::COMMAND, Key::N)),
            )
        });
        if undo {
            self.session.undo();
        }
        if redo || redo_y {
            self.session.redo();
        }
        if save {
            self.save(ctx);
        }
        if save_as {
            #[cfg(not(target_arch = "wasm32"))]
            self.save_as();
            #[cfg(target_arch = "wasm32")]
            self.save(ctx);
        }
        if open {
            self.request_discard(ctx, Discard::Open);
        }
        if new {
            self.request_discard(ctx, Discard::New);
        }

        let (f3, f7, f8, f9, home) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::F3),
                i.consume_key(Modifiers::NONE, Key::F7),
                i.consume_key(Modifiers::NONE, Key::F8),
                i.consume_key(Modifiers::NONE, Key::F9),
                i.consume_key(Modifiers::NONE, Key::Home),
            )
        });
        if f9 {
            self.toggle_grid_snap();
        }
        if f3 {
            self.toggle_snap();
        }
        if f7 {
            self.settings.show_grid = !self.settings.show_grid;
        }
        if f8 {
            self.toggle_ortho();
        }
        if home {
            self.zoom_extents();
        }

        // Keys that only apply when no text field has focus.
        if ctx.memory(|m| m.focused().is_none()) {
            let (esc, enter, del, tab) = ctx.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::Escape),
                    i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::Delete)
                        || i.consume_key(Modifiers::NONE, Key::Backspace),
                    i.consume_key(Modifiers::NONE, Key::Tab),
                )
            });
            if esc && self.settings_window.open {
                // Escape dismisses the settings panel first.
                self.settings_window.open = false;
            } else if esc && self.picking_pivot {
                self.picking_pivot = false;
                self.session.log.info("Centre of rotation unchanged");
            } else if esc {
                self.session.log.input.clear();
                self.session.snap_override = None;
                self.session.dispatch(ToolEvent::Cancel);
            }
            if enter {
                match self.session.log.take_pending_input() {
                    Some(text) => self.submit(ctx, text),
                    None => self.session.dispatch(ToolEvent::Finish),
                }
            }
            if del {
                self.session.delete_selection();
            }
            if tab {
                self.snap_engine.cycle_next();
            }
        }
        self.session.pointer.shift = ctx.input(|i| i.modifiers.shift);
    }
}
