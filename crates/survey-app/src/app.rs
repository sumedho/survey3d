//! The survey3d application: input routing, tools and file handling.
//! Panel and viewport drawing live in [`layout`].

mod layout;

use cad_render::{GridSettings, Projection};
use egui::{Key, KeyboardShortcut, Modifiers};
use egui_cad::chrome::{bar_frame, panel_frame, section};
use egui_cad::command_line::{CommandLine, CommandLineEvent, CommandLineState, Completion, LogKind};
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
use survey_core::point_input::PointInput;
use survey_model::{ChangeSet, Color, Command, Entity, EntityId, Layer, LayerId, SurfaceId};
use survey_store::ProjectStore;

use crate::import_command::{ImportEntities, LayerRef};
use crate::interchange::{dxf_export, dxf_import};
use crate::platform::{FileEvent, FileKind, Platform, export};
use crate::project::{APP_VERSION, Project};
use crate::scene_build::{DocSnapProvider, SceneCache, color32, hit_test};
use crate::settings::AppSettings;
use crate::surface_view::{SurfaceIndexes, SurfaceSceneCache};
use crate::tools::grips::Grip;
use crate::tools::{
    CoreModule, DomainModule, Preview, SELECT_TOOL_ID, Tool, ToolCtx, ToolEvent, ToolOutcome, ToolRegistry,
};
use crate::ui::audit_window::AuditWindow;
use crate::ui::calculator::{CalcTab, CalculatorWindow};
use crate::ui::closure_window::{ClosureAction, ClosureWindow};
use crate::ui::csv_import::{CsvDialogResult, CsvImportDialog};
use crate::ui::feature_codes::{CodesAction, FeatureCodesWindow};
use crate::ui::legend::LegendWindow;
use crate::ui::parcels::ParcelsWindow;
use crate::ui::points_table::{PointsAction, PointsWindow};
use crate::ui::properties::properties_panel;
use crate::ui::reports::{ReportAction, ReportsWindow};
use crate::ui::resection_window::{ResectionAction, ResectionWindow};
use crate::ui::sections::SectionsWindow;
use crate::ui::settings_window::SettingsWindow;
use crate::ui::subdivide_window::{SubdivideAction, SubdivideWindow};
use crate::ui::surfaces::{SurfaceAction, SurfaceWindow, SurfaceWindowAction, build_from};
use crate::ui::traverse_window::{TraverseAction, TraverseWindow};
use crate::ui::volumes::VolumesWindow;

const SETTINGS_KEY: &str = "survey3d.settings";

/// Polyline ends closer than this (plan, metres) are joined.
const JOIN_TOLERANCE: f64 = 0.001;

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
    project: Project,
    tools: ToolRegistry,
    active_tool: Box<dyn Tool>,
    viewport: ViewportState,
    scene_cache: SceneCache,
    surface_cache: SurfaceSceneCache,
    surface_indexes: SurfaceIndexes,
    surface_window: SurfaceWindow,
    volumes_window: VolumesWindow,
    sections_window: SectionsWindow,
    parcels_window: ParcelsWindow,
    active_surface: Option<SurfaceId>,
    snap_engine: SnapEngine,
    snap_override: Option<SnapMode>,
    command_line: CommandLineState,
    completions: Vec<Completion>,
    platform: Platform,
    /// Resolved cursor position this frame (snapped if possible).
    pick: Option<Pick>,
    hovered_entity: Option<EntityId>,
    /// Grips of the selection, cached by `(document revision, selection revision)`.
    grips: (Option<(u64, u64)>, Vec<Grip>),
    hovered_grip: Option<Grip>,
    csv_dialog: Option<CsvImportDialog>,
    palette: CommandPalette,
    toasts: Toasts,
    settings_window: SettingsWindow,
    audit_window: AuditWindow,
    traverse_window: TraverseWindow,
    closure_window: ClosureWindow,
    calculator: CalculatorWindow,
    legend: LegendWindow,
    subdivide_window: SubdivideWindow,
    resection_window: ResectionWindow,
    codes_window: FeatureCodesWindow,
    reports: ReportsWindow,
    points_window: PointsWindow,
    about_open: bool,
    confirm_discard: Option<Discard>,
    allowed_to_close: bool,
    title: String,
    shift_down: bool,
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

        let mut tools = ToolRegistry::default();
        // Domain modules (cadastral, construction, mining) register here as they are added.
        let modules: Vec<Box<dyn DomainModule>> = vec![Box::new(CoreModule)];
        for m in &modules {
            m.register(&mut tools);
        }
        let mut completions: Vec<Completion> = tools
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

        let project = Project::new(&settings).expect("in-memory SQLite is available");
        let active_tool = (tools.get(SELECT_TOOL_ID).expect("select tool registered").create)();
        let mut viewport = ViewportState::new(1);
        viewport.camera.target = DVec3::ZERO;

        let platform = Platform::default();
        let mut command_line = CommandLineState::default();
        let mut module_names: Vec<&str> = modules.iter().map(|m| m.name()).collect();
        module_names.extend(
            crate::settings::Module::ALL
                .into_iter()
                .filter(|m| settings.modules.is_on(*m))
                .map(|m| m.name()),
        );
        command_line.info(format!(
            "survey3d {APP_VERSION} — modules: {} — type 'help' for commands.",
            module_names.join(", ")
        ));

        #[cfg(target_arch = "wasm32")]
        platform.check_recovery(&cc.egui_ctx);

        Self {
            settings,
            theme,
            project,
            tools,
            active_tool,
            viewport,
            scene_cache: SceneCache::default(),
            surface_cache: SurfaceSceneCache::default(),
            surface_indexes: SurfaceIndexes::default(),
            surface_window: SurfaceWindow::default(),
            volumes_window: VolumesWindow::default(),
            sections_window: SectionsWindow::default(),
            parcels_window: ParcelsWindow::default(),
            active_surface: None,
            snap_engine: SnapEngine::default(),
            snap_override: None,
            command_line,
            completions,
            platform,
            pick: None,
            hovered_entity: None,
            grips: (None, Vec::new()),
            hovered_grip: None,
            csv_dialog: None,
            palette: CommandPalette::default(),
            toasts: Toasts::default(),
            settings_window: SettingsWindow::default(),
            audit_window: AuditWindow::default(),
            traverse_window: TraverseWindow::default(),
            closure_window: ClosureWindow::default(),
            calculator: CalculatorWindow::default(),
            legend: LegendWindow::default(),
            subdivide_window: SubdivideWindow::default(),
            resection_window: ResectionWindow::default(),
            codes_window: FeatureCodesWindow::default(),
            reports: ReportsWindow::default(),
            points_window: PointsWindow::default(),
            about_open: false,
            confirm_discard: None,
            allowed_to_close: false,
            title: String::new(),
            shift_down: false,
            layer_counts: (0, Default::default()),
            #[cfg(target_arch = "wasm32")]
            recovery: None,
            #[cfg(target_arch = "wasm32")]
            autosave: Autosave::default(),
        }
    }

    // ------------------------------------------------------------------ editing

    fn execute(&mut self, command: Box<dyn Command>) {
        let p = &mut self.project;
        p.history.execute(command, &mut p.doc);
        let doc = &p.doc;
        p.selection.retain(|id| doc.contains(id));
    }

    fn undo(&mut self) {
        let p = &mut self.project;
        match p.history.undo(&mut p.doc) {
            Some(label) => self.command_line.info(format!("Undo: {label}")),
            None => self.command_line.warn("Nothing to undo"),
        }
        let doc = &p.doc;
        p.selection.retain(|id| doc.contains(id));
    }

    fn redo(&mut self) {
        let p = &mut self.project;
        match p.history.redo(&mut p.doc) {
            Some(label) => self.command_line.info(format!("Redo: {label}")),
            None => self.command_line.warn("Nothing to redo"),
        }
        let doc = &p.doc;
        p.selection.retain(|id| doc.contains(id));
    }

    fn delete_selection(&mut self) {
        let ids: Vec<EntityId> = self.project.selection.ids().iter().copied().collect();
        let ids: Vec<EntityId> = ids
            .into_iter()
            .filter(|id| self.project.doc.is_entity_editable(*id))
            .collect();
        if ids.is_empty() {
            return;
        }
        let mut cs = ChangeSet::new(format!("Delete {} objects", ids.len()));
        for id in &ids {
            cs.push_delete_entity(*id);
        }
        self.execute(Box::new(cs));
        self.command_line.info(format!("Deleted {} objects", ids.len()));
    }

    /// Editable selected polylines.
    fn selected_polylines(&self) -> Vec<(EntityId, survey_model::Polyline)> {
        let doc = &self.project.doc;
        self.project
            .selection
            .ids()
            .iter()
            .filter(|id| doc.is_entity_editable(**id))
            .filter_map(|id| match doc.entity(*id) {
                Some(Entity::Polyline(l)) => Some((*id, l.clone())),
                _ => None,
            })
            .collect()
    }

    fn explode_selection(&mut self) {
        let lines = self.selected_polylines();
        if lines.is_empty() {
            self.command_line.warn("Select the polylines to explode first");
            return;
        }
        let mut cs = ChangeSet::new(format!("Explode {} polylines", lines.len()));
        let mut parts = 0;
        for (id, l) in &lines {
            cs.push_delete_entity(*id);
            for part in l.explode() {
                cs.push_insert_entity(Entity::Polyline(part));
                parts += 1;
            }
        }
        self.execute(Box::new(cs));
        self.command_line.result(format!(
            "Exploded {} polylines into {parts} segments",
            lines.len()
        ));
    }

    fn join_selection(&mut self) {
        let lines = self.selected_polylines();
        if lines.len() < 2 {
            self.command_line.warn("Select two or more polylines to join");
            return;
        }
        let polylines: Vec<_> = lines.iter().map(|(_, l)| l.clone()).collect();
        let joined = survey_model::join_polylines(&polylines, JOIN_TOLERANCE);
        if joined.len() == lines.len() {
            self.command_line
                .warn("None of the selected polylines meet end to end");
            return;
        }
        let mut cs = ChangeSet::new(format!("Join {} polylines", lines.len()));
        for (id, _) in &lines {
            cs.push_delete_entity(*id);
        }
        for l in &joined {
            cs.push_insert_entity(Entity::Polyline(l.clone()));
        }
        self.execute(Box::new(cs));
        self.command_line
            .result(format!("Joined {} polylines into {}", lines.len(), joined.len()));
    }

    fn reverse_selection(&mut self) {
        let lines = self.selected_polylines();
        if lines.is_empty() {
            self.command_line.warn("Select the polylines to reverse first");
            return;
        }
        let mut cs = ChangeSet::new(format!("Reverse {} polylines", lines.len()));
        for (id, mut l) in lines {
            l.reverse();
            cs.push_update_entity(id, Entity::Polyline(l));
        }
        self.execute(Box::new(cs));
    }

    /// Adds a report describing the selection: polyline legs (bearing, distance,
    /// arcs) and point coordinates.
    fn list_selection(&mut self) {
        let doc = &self.project.doc;
        let conv = doc.conventions();
        if self.project.selection.is_empty() {
            self.command_line.warn("Select objects to list first");
            return;
        }
        let mut text = String::new();
        for id in self.project.selection.ids() {
            match doc.entity(*id) {
                Some(Entity::Point(p)) => {
                    text.push_str(&format!(
                        "POINT {:<10} {}  {}  {}\n",
                        p.name,
                        conv.format_coord(p.position, p.height_known),
                        p.code,
                        p.description
                    ));
                }
                Some(Entity::Polyline(l)) => {
                    text.push_str(&crate::listing::polyline_listing(l, conv));
                }
                None => {}
            }
        }
        let n = self.project.selection.len();
        self.reports
            .add(format!("Listing ({})", crate::tools::objects(n)), text);
    }

    fn apply_surface_action(&mut self, action: SurfaceAction) {
        match action {
            SurfaceAction::New => self.surface_window.create(&self.project.doc),
            SurfaceAction::Edit(id) => self.surface_window.edit(&self.project.doc, id),
            SurfaceAction::Delete(id) => {
                let name = self
                    .project
                    .doc
                    .surface(id)
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                self.execute(Box::new(
                    ChangeSet::new(format!("Delete surface {name}")).delete_surface(id),
                ));
            }
            SurfaceAction::Style(id, style) => {
                let name = self
                    .project
                    .doc
                    .surface(id)
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                self.execute(Box::new(
                    ChangeSet::new(format!("Surface {name} display")).set_surface_style(id, style),
                ));
            }
            SurfaceAction::Rebuild(id) => {
                let doc = &self.project.doc;
                let Some(s) = doc.surface(id) else {
                    return;
                };
                let Some(def) = &s.definition else {
                    return;
                };
                match build_from(doc, def) {
                    Ok(tin) => {
                        let summary = format!(
                            "Surface {}: {} points, {} triangles",
                            s.name,
                            tin.vertices.len(),
                            tin.triangles.len()
                        );
                        let mut surface = s.clone();
                        surface.tin = tin;
                        let label = format!("Rebuild surface {}", s.name);
                        self.execute(Box::new(ChangeSet::new(label).update_surface(id, surface)));
                        self.command_line.result(summary);
                    }
                    Err(e) => self.command_line.error(format!("Surface {}: {e}", s.name)),
                }
            }
        }
    }

    /// The surface commands act on: the active one, else the only one.
    fn target_surface(&mut self) -> Option<SurfaceId> {
        let doc = &self.project.doc;
        if let Some(id) = self.active_surface.filter(|id| doc.surface(*id).is_some()) {
            return Some(id);
        }
        let mut ids = doc.surfaces().map(|(id, _)| id);
        match (ids.next(), ids.next()) {
            (Some(id), None) => Some(id),
            (None, _) => {
                self.command_line
                    .warn("Create a surface first (Surface → Create surface…)");
                None
            }
            _ => {
                self.command_line
                    .warn("Select a surface in the Surfaces panel first");
                None
            }
        }
    }

    /// Adds the active surface's contours (at its contour interval) as polylines.
    fn contours_to_polylines(&mut self) {
        let Some(id) = self.target_surface() else {
            return;
        };
        let doc = &self.project.doc;
        let Some(s) = doc.surface(id) else {
            return;
        };
        let contours = crate::surface_view::surface_contours(s);
        if contours.is_empty() {
            self.command_line.warn("No contours at this interval");
            return;
        }
        let mut cmd = ImportEntities::new(format!("Contours of {}", s.name));
        let mut layer = |name: &str, color: Color| match doc.layer_by_name(name) {
            Some(l) => LayerRef::Existing(l.id),
            None => cmd.add_layer(name.to_string(), color, true),
        };
        let minor = layer("CONTOUR MINOR", Color::rgb(160, 130, 100));
        let major = layer("CONTOUR MAJOR", Color::rgb(200, 160, 110));
        let n = contours.len();
        for c in contours {
            let line = survey_model::Polyline {
                name: format!("Contour {}", doc.conventions().format_length(c.z)),
                vertices: c.points,
                bulges: Vec::new(),
                closed: c.closed,
                code: String::new(),
                layer: doc.current_layer(),
            };
            cmd.add(if c.major { major } else { minor }, Entity::Polyline(line));
        }
        let summary = format!("Contours: added {n} polylines from {}", s.name);
        self.execute(Box::new(cmd));
        self.command_line.result(summary);
    }

    fn apply_surface_window(&mut self, action: SurfaceWindowAction) {
        match action {
            SurfaceWindowAction::Create(surface) => {
                let summary = format!(
                    "Surface {}: {} points, {} triangles",
                    surface.name,
                    surface.tin.vertices.len(),
                    surface.tin.triangles.len()
                );
                let cmd = ChangeSet::new(format!("Create surface {}", surface.name)).insert_surface(surface);
                self.execute(Box::new(cmd));
                // New surfaces get the highest id.
                self.active_surface = self.project.doc.surfaces().last().map(|(id, _)| id);
                self.command_line.result(summary.clone());
                self.toasts.success(summary);
            }
            SurfaceWindowAction::Update(id, surface) => {
                let label = format!("Rebuild surface {}", surface.name);
                self.execute(Box::new(ChangeSet::new(label).update_surface(id, surface)));
            }
        }
    }

    /// Creates the mean of the selected points and reports their spread.
    fn average_selection(&mut self) {
        let doc = &self.project.doc;
        let points: Vec<(DVec3, bool)> = self
            .project
            .selection
            .ids()
            .iter()
            .filter_map(|id| match doc.entity(*id) {
                Some(Entity::Point(p)) => Some((p.position, p.height_known)),
                _ => None,
            })
            .collect();
        if points.len() < 2 {
            self.command_line.warn("Select two or more points to average");
            return;
        }
        let Some(avg) = survey_core::cogo::average(&points) else {
            return;
        };
        let conv = doc.conventions();
        let name = doc.next_point_name();
        let len = |v: f64| conv.format_length(v);
        let (ea, nb) = conv.horizontal_labels();
        let mut spread = format!(
            "spread {} plan (sd {ea} {} {nb} {})",
            len(avg.max_plan),
            len(avg.sd.x),
            len(avg.sd.y)
        );
        if avg.height_known {
            spread.push_str(&format!(
                ", {} height (sd {})",
                len(avg.max_height),
                len(avg.sd.z)
            ));
        }
        let summary = format!(
            "Average of {} points → {name}: {}; {spread}",
            points.len(),
            conv.format_coord(avg.position, avg.height_known)
        );
        let point = survey_model::SurveyPoint {
            name: name.clone(),
            position: avg.position,
            height_known: avg.height_known,
            code: "AVG".into(),
            description: format!("Average of {} points", points.len()),
            layer: doc.current_layer(),
        };
        self.execute(Box::new(
            ChangeSet::new(format!("Average point {name}")).insert_entity(Entity::Point(point)),
        ));
        self.command_line.result(summary);
    }

    /// Stores `library` in the project if it differs (one undoable change).
    fn apply_feature_codes(&mut self, library: survey_core::field_to_finish::FeatureLibrary) {
        if self.project.doc.meta().feature_codes == library {
            return;
        }
        let mut meta = self.project.doc.meta().clone();
        meta.feature_codes = library;
        self.execute(Box::new(ChangeSet::new("Edit feature codes").set_meta(meta)));
    }

    fn process_linework(
        &mut self,
        library: survey_core::field_to_finish::FeatureLibrary,
        only_selected: bool,
        assign_layers: bool,
    ) {
        if library.codes.is_empty() {
            self.command_line
                .warn("The feature code library is empty — open it with 'codes'");
            self.codes_window.open();
            return;
        }
        self.apply_feature_codes(library);
        let selection = self.project.selection.ids().clone();
        let options = crate::field_to_finish::LineworkOptions {
            only: only_selected.then_some(&selection),
            assign_layers,
        };
        let (cmd, summary) = crate::field_to_finish::build_linework(
            &self.project.doc,
            &self.project.doc.meta().feature_codes,
            &options,
        );
        if !cmd.is_empty() {
            self.execute(Box::new(cmd));
        }
        let text = summary.describe();
        self.command_line.result(text.clone());
        if summary.unknown.is_empty() {
            self.toasts.success(format!("{} strings", summary.lines));
        } else {
            self.toasts
                .warning("Some codes are not in the library — see the command line");
        }
    }

    fn import_landxml(&mut self, name: &str, bytes: &[u8]) {
        match survey_io::landxml::read_surfaces(bytes) {
            Ok((surfaces, warnings)) => {
                let n = surfaces.len();
                let mut cmd = ChangeSet::new(format!("Import LandXML {name}"));
                for s in surfaces {
                    // Keep names unique in the project.
                    let mut surface_name = s.name.clone();
                    let mut k = 2;
                    while self.project.doc.surface_by_name(&surface_name).is_some() {
                        surface_name = format!("{} ({k})", s.name);
                        k += 1;
                    }
                    cmd = cmd.insert_surface(survey_model::Surface {
                        name: surface_name,
                        definition: None,
                        tin: s.tin,
                        style: survey_model::SurfaceStyle::default(),
                    });
                }
                self.execute(Box::new(cmd));
                self.zoom_extents();
                let summary = format!(
                    "Imported {n} surface{} from {name}",
                    if n == 1 { "" } else { "s" }
                );
                self.command_line.result(summary.clone());
                self.toasts.success(summary);
                for w in warnings {
                    self.command_line.warn(w);
                }
            }
            Err(e) => self.command_line.error(format!("Could not read {name}: {e}")),
        }
    }

    /// Writes the active surface (or every surface when none is active) as LandXML.
    fn export_landxml(&mut self) {
        let doc = &self.project.doc;
        let chosen: Vec<(&str, &survey_core::tin::Tin)> =
            match self.active_surface.and_then(|id| doc.surface(id)) {
                Some(s) => vec![(s.name.as_str(), &s.tin)],
                None => doc.surfaces().map(|(_, s)| (s.name.as_str(), &s.tin)).collect(),
            };
        if chosen.is_empty() {
            self.command_line.warn("There are no surfaces to export");
            return;
        }
        let xml = survey_io::landxml::write_surfaces(&doc.meta().name, &chosen);
        let stem = if chosen.len() == 1 {
            chosen[0].0.to_string()
        } else {
            "surfaces".into()
        };
        let n = chosen.len();
        match export(&self.platform, FileKind::LandXml, &stem, xml.as_bytes()) {
            Ok(Some(where_)) => self
                .command_line
                .result(format!("Exported {n} surface(s) to {where_}")),
            Ok(None) => {}
            Err(e) => self.command_line.error(e),
        }
    }

    /// Makes a parcel of each selected closed polyline that is not one already.
    fn parcels_from_selection(&mut self) {
        let doc = &self.project.doc;
        let existing: Vec<EntityId> = doc.meta().parcels.iter().map(|p| p.boundary).collect();
        let lines: Vec<EntityId> = self
            .selected_polylines()
            .into_iter()
            .filter(|(id, l)| l.closed && !existing.contains(id))
            .map(|(id, _)| id)
            .collect();
        if lines.is_empty() {
            self.command_line
                .warn("Select closed polylines (lot boundaries) that are not parcels yet");
            return;
        }
        // Continue the "Lot n" numbering.
        let mut next = doc
            .meta()
            .parcels
            .iter()
            .filter_map(|p| {
                p.name
                    .strip_prefix("Lot ")
                    .and_then(|n| n.trim().parse::<u32>().ok())
            })
            .max()
            .unwrap_or(0)
            + 1;
        let mut meta = doc.meta().clone();
        let n = lines.len();
        for id in lines {
            let name = match doc.entity(id) {
                Some(Entity::Polyline(l)) if !l.name.trim().is_empty() => l.name.trim().to_string(),
                _ => {
                    let name = format!("Lot {next}");
                    next += 1;
                    name
                }
            };
            meta.parcels.push(survey_model::ParcelRecord {
                name,
                boundary: id,
                ..Default::default()
            });
        }
        self.execute(Box::new(
            ChangeSet::new(format!("Create {n} parcels")).set_meta(meta),
        ));
        self.parcels_window.open = true;
        self.command_line
            .result(format!("Created {n} parcel{}", if n == 1 { "" } else { "s" }));
    }

    fn export_parcels_landxml(&mut self) {
        let doc = &self.project.doc;
        let geoms: Vec<(String, String, f64, &survey_model::Polyline)> = doc
            .meta()
            .parcels
            .iter()
            .filter_map(|p| {
                let l = crate::ui::parcels::boundary(doc, p)?;
                let (_, _, area) = survey_core::arc::polyline_metrics(&l.vertices, &l.bulges, true);
                Some((p.name.clone(), p.plan.clone(), area.abs(), l))
            })
            .collect();
        if geoms.is_empty() {
            self.command_line.warn("There are no parcels to export");
            return;
        }
        let parcels: Vec<survey_io::landxml::ParcelGeometry<'_>> = geoms
            .iter()
            .map(|(name, plan, area, l)| survey_io::landxml::ParcelGeometry {
                name,
                description: plan,
                area: *area,
                vertices: &l.vertices,
                bulges: &l.bulges,
            })
            .collect();
        let xml = survey_io::landxml::write_parcels(&doc.meta().name, &parcels);
        let n = parcels.len();
        match export(&self.platform, FileKind::LandXml, "parcels", xml.as_bytes()) {
            Ok(Some(where_)) => self
                .command_line
                .result(format!("Exported {n} parcels to {where_}")),
            Ok(None) => {}
            Err(e) => self.command_line.error(e),
        }
    }

    fn import_code_sheet(&mut self, name: &str, bytes: &[u8]) {
        match survey_io::code_sheet::read_code_sheet(bytes) {
            Ok((codes, warnings)) => {
                let n = self.codes_window.merge(&self.project.doc, codes);
                self.command_line
                    .info(format!("Read {n} feature codes from {name} — Apply to keep them"));
                for w in warnings {
                    self.command_line.warn(w);
                }
            }
            Err(e) => self.command_line.error(format!("Could not read {name}: {e}")),
        }
    }

    fn set_tool(&mut self, id: &str) {
        let Some(create) = self.tools.get(id).map(|d| d.create) else {
            return;
        };
        // Give the current tool a chance to commit work in progress. (Not the select
        // tool: cancelling it clears the selection the next tool may work on.)
        if self.active_tool.id() != id && self.active_tool.id() != SELECT_TOOL_ID {
            self.dispatch(ToolEvent::Cancel);
        }
        self.active_tool = create();
        self.snap_override = None;
        self.with_tool(|tool, ctx| tool.activate(ctx));
    }

    /// Sends an event to the active tool and executes the commands it emits.
    fn dispatch(&mut self, event: ToolEvent<'_>) {
        let is_point = matches!(event, ToolEvent::Point(_));
        self.with_tool(|tool, ctx| tool.on_event(ctx, event));
        if is_point {
            self.snap_override = None;
        }
    }

    /// Runs `f` on the active tool with a [`ToolCtx`], then executes the commands
    /// it emitted and returns to the select tool if it finished.
    fn with_tool(&mut self, f: impl FnOnce(&mut dyn Tool, &mut ToolCtx<'_>) -> ToolOutcome) {
        let mut commands: Vec<Box<dyn Command>> = Vec::new();
        let outcome = {
            let p = &mut self.project;
            let mut ctx = ToolCtx::new(&p.doc, &mut p.selection, &mut self.command_line, &mut commands);
            ctx.hovered = self.hovered_entity;
            ctx.hovered_grip = self.hovered_grip;
            ctx.shift = self.shift_down;
            f(self.active_tool.as_mut(), &mut ctx)
        };
        for c in commands {
            self.execute(c);
        }
        if outcome == ToolOutcome::Done
            && self.active_tool.id() != SELECT_TOOL_ID
            && let Some(desc) = self.tools.get(SELECT_TOOL_ID)
        {
            self.active_tool = (desc.create)();
        }
    }

    fn zoom_extents(&mut self) {
        let doc = &self.project.doc;
        let b = doc
            .surfaces()
            .filter(|(_, s)| s.style.visible)
            .fold(doc.bounds(), |b, (_, s)| b.union(&s.tin.bounds()));
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
        self.command_line
            .info(if locked { "Plan view (2D lock)" } else { "3D view" });
    }

    fn toggle_projection(&mut self) {
        if self.viewport.plan_locked {
            self.command_line.warn("Perspective is only available in 3D view");
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
        self.command_line.info(format!("Snap {state}"));
    }

    fn toggle_grid_snap(&mut self) {
        self.settings.snap.toggle(SnapMode::Grid);
        let state = if self.settings.snap.is_on(SnapMode::Grid) {
            "on"
        } else {
            "off"
        };
        self.command_line.info(format!("Grid snap {state}"));
    }

    fn toggle_ortho(&mut self) {
        self.settings.ortho.enabled = !self.settings.ortho.enabled;
        let state = if self.settings.ortho.enabled { "on" } else { "off" };
        self.command_line.info(format!("Ortho {state}"));
    }

    // ------------------------------------------------------------------ command line

    fn submit(&mut self, ctx: &egui::Context, text: String) {
        let text = text.trim();
        let prompt = self.active_tool.prompt(self.project.doc.conventions());
        self.command_line
            .log(LogKind::Command, format!("{prompt} {text}"));
        if text.is_empty() {
            self.dispatch(ToolEvent::Finish);
            return;
        }

        // Values the tool asks for (a radius, a bearing, an observation…) go straight to it.
        if self.active_tool.claims_text(self.project.doc.conventions(), text) {
            self.dispatch(ToolEvent::Text(text));
            return;
        }

        if self.active_tool.wants_point() {
            if let Some(mode) = SnapMode::from_code(text) {
                self.snap_override = Some(mode);
                self.command_line
                    .info(format!("Snap override: {} (next point)", mode.label()));
                return;
            }
            // A bare point name picks that point.
            if !text.contains([',', ' ', '@'])
                && let Some((_, p)) = self.project.doc.find_point_by_name(text)
            {
                let pick = Pick {
                    point: p.position,
                    height_known: p.height_known,
                    snap: None,
                };
                self.dispatch(ToolEvent::Point(pick));
                return;
            }
            match PointInput::parse(text, self.project.doc.conventions()) {
                Ok(Some(input)) => {
                    match input.resolve(self.active_tool.base_point()) {
                        Ok(r) => self.dispatch(ToolEvent::Point(Pick {
                            point: r.position,
                            height_known: r.height_known,
                            snap: None,
                        })),
                        Err(e) => self.command_line.error(e.to_string()),
                    }
                    return;
                }
                Err(e) => {
                    self.command_line.error(e.to_string());
                    return;
                }
                Ok(None) => {}
            }
        }

        // A feature code wins over a command of the same name while coding points.
        if self.active_tool.takes_codes() && self.project.doc.meta().feature_codes.feature_for(text).is_some()
        {
            self.dispatch(ToolEvent::Text(text));
            return;
        }
        if let Some(desc) = self.tools.find_command(text) {
            let id = desc.id;
            self.set_tool(id);
            return;
        }
        if self.run_app_command(ctx, text) {
            return;
        }
        if self.active_tool.id() != SELECT_TOOL_ID || self.active_tool.wants_point() {
            self.dispatch(ToolEvent::Text(text));
        } else {
            self.command_line
                .error(format!("Unknown command '{text}' — type 'help'"));
        }
    }

    /// Everything searchable in the command palette.
    fn palette_items(&self, ctx: &egui::Context) -> Vec<PaletteItem> {
        let mut items: Vec<PaletteItem> = self
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
            self.set_tool(tool);
        } else if let Some(name) = id.strip_prefix("cmd:") {
            self.command_line
                .log(LogKind::Command, format!("Command: {name}"));
            self.run_app_command(ctx, name);
        }
    }

    fn run_app_command(&mut self, ctx: &egui::Context, text: &str) -> bool {
        match text.to_ascii_lowercase().as_str() {
            "undo" => self.undo(),
            "redo" => self.redo(),
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
            "snap" | "osnap" => self.toggle_snap(),
            "ortho" => self.toggle_ortho(),
            "gridsnap" | "snapgrid" => self.toggle_grid_snap(),
            "grid" => self.settings.show_grid = !self.settings.show_grid,
            "delete" | "erase" => self.delete_selection(),
            "explode" => self.explode_selection(),
            "join" => self.join_selection(),
            "reverse" => self.reverse_selection(),
            "import" => self.platform.pick_file(ctx, FileKind::Points),
            "dxfin" | "importdxf" => self.platform.pick_file(ctx, FileKind::Dxf),
            "dxfout" | "exportdxf" => self.export_dxf(),
            "labels" => self.settings_window.open_at_display(),
            "save" => self.save(ctx),
            "open" => self.request_discard(ctx, Discard::Open),
            "new" => self.request_discard(ctx, Discard::New),
            "audit" => self.audit_window.open = true,
            "traverse" | "trav" => self.traverse_window.open = true,
            "closure" | "clo" => match self.selected_polylines().first() {
                Some((_, line)) => self.closure_window.load_polyline(&self.project.doc, line),
                None => self.closure_window.open = true,
            },
            "subdivide" | "subdiv" => {
                let parcel = self.selected_polylines().into_iter().find(|(_, l)| l.closed);
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
            "average" | "avg" => self.average_selection(),
            "legend" => self.legend.open = true,
            "surface" | "tin" => self.surface_window.create(&self.project.doc),
            "contours" => self.contours_to_polylines(),
            "parcels" => self.parcels_window.open = true,
            "mkparcel" | "parcel" => self.parcels_from_selection(),
            "parcelxml" => self.export_parcels_landxml(),
            "landxmlin" | "importxml" => self.platform.pick_file(ctx, FileKind::LandXml),
            "landxmlout" | "exportxml" => self.export_landxml(),
            "sections" | "section" | "profile" => {
                let line = self.selected_polylines().first().map(|(id, _)| *id);
                self.sections_window.open_with(line);
            }
            "volume" | "volumes" | "vol" => self.volumes_window.open(&self.project.doc, self.active_surface),
            "accuracy" | "tolerances" => self.settings_window.open_at_accuracy(),
            "codes" | "featurecodes" => self.codes_window.open(),
            "linework" | "f2f" => {
                let library = self.project.doc.meta().feature_codes.clone();
                let only_selected = !self.project.selection.is_empty();
                self.process_linework(library, only_selected, false);
            }
            "reports" => self.reports.open = true,
            "points" | "pts" | "table" => self.points_window.open = true,
            "list" | "li" => self.list_selection(),
            "settings" | "options" => self.settings_window.open = true,
            "drafting" | "dsettings" => self.settings_window.open_at_drafting(),
            "conventions" => self.settings_window.open_at_conventions(),
            "iso" | "isometric" => {
                self.viewport.apply_preset(ViewPreset::Isometric);
            }
            "hud" | "dyn" => {
                self.settings.dynamic_input = !self.settings.dynamic_input;
                let state = if self.settings.dynamic_input { "on" } else { "off" };
                self.command_line.info(format!("Dynamic input {state}"));
            }
            "theme" => {
                self.settings.theme = match self.settings.theme {
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::Dark,
                };
            }
            "help" | "?" => {
                let mut lines = vec!["Tools:".to_string()];
                for t in self.tools.tools() {
                    lines.push(format!("  {:<18} {}", t.commands.join(", "), t.description));
                }
                lines.push("Commands:".into());
                for c in APP_COMMANDS {
                    lines.push(format!("  {:<18} {}", c.name, c.title));
                }
                lines.push("Point input: E,N[,Z]  @dE,dN[,dZ]  @dist<bearing  or a point name".into());
                self.command_line.info(lines.join("\n"));
            }
            _ => return false,
        }
        true
    }

    // ------------------------------------------------------------------ files

    fn request_discard(&mut self, ctx: &egui::Context, action: Discard) {
        if self.project.is_dirty() {
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
                    self.command_line.info("New project");
                }
                Err(e) => self.command_line.error(format!("Could not create project: {e}")),
            },
            Discard::Open => self.platform.pick_file(ctx, FileKind::Project),
            Discard::Quit => {
                self.allowed_to_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn replace_project(&mut self, project: Project) {
        self.project = project;
        self.scene_cache.invalidate();
        self.surface_cache.invalidate();
        self.surface_indexes.invalidate();
        self.active_surface = None;
        // Revisions restart with each document, so drop caches keyed by them.
        self.layer_counts = (u64::MAX, Default::default());
        self.set_tool(SELECT_TOOL_ID);
        if !self.restore_view() {
            self.zoom_extents();
        }
    }

    /// Restores the camera saved with the project; `false` if there is none.
    fn restore_view(&mut self) -> bool {
        let Some(view) = self
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
            && let Err(e) = self.project.store.write_view_state(&json)
        {
            log::warn!("could not store view: {e}");
        }
    }

    fn export_dxf(&mut self) {
        let text = survey_io::dxf::write_dxf(&dxf_export(&self.project.doc));
        let stem = file_stem(&self.project.doc.meta().name);
        match export(&self.platform, FileKind::Dxf, &stem, text.as_bytes()) {
            Ok(Some(where_)) => {
                self.command_line.info(format!("Exported DXF: {where_}"));
                self.toasts.success("Exported DXF");
            }
            Ok(None) => {}
            Err(e) => self.command_line.error(format!("DXF export failed: {e}")),
        }
    }

    fn import_dxf(&mut self, name: &str, bytes: &[u8]) {
        match survey_io::dxf::read_dxf(bytes) {
            Ok(drawing) => {
                let cmd = dxf_import(&self.project.doc, &drawing, name);
                let n = cmd.len();
                self.execute(Box::new(cmd));
                self.command_line
                    .info(format!("Imported {n} objects from {name}"));
                self.toasts.success(format!("Imported {n} objects from {name}"));
                for w in &drawing.warnings {
                    self.command_line.warn(w.clone());
                }
                self.zoom_extents();
            }
            Err(e) => self.command_line.error(format!("Could not import {name}: {e}")),
        }
    }

    fn open_store(&mut self, result: Result<ProjectStore, survey_store::StoreError>, name: String) {
        let project = result.and_then(|store| Project::from_store(store, name.clone(), &self.settings));
        match project {
            Ok(p) => {
                let n = p.doc.entity_count();
                self.replace_project(p);
                self.command_line.info(format!("Opened {name} ({n} objects)"));
                self.toasts.success(format!("Opened {name}"));
            }
            Err(e) => {
                self.command_line.error(format!("Could not open {name}: {e}"));
                self.toasts.error(format!("Could not open {name}"));
            }
        }
    }

    fn save(&mut self, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            if self.project.has_file() {
                self.store_view();
                match self.project.save() {
                    Ok(()) => {
                        self.command_line
                            .info(format!("Saved {}", self.project.display_name));
                        self.toasts.success("Saved");
                    }
                    Err(e) => {
                        self.command_line.error(format!("Save failed: {e}"));
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
                file_stem(&self.project.doc.meta().name),
                survey_store::FILE_EXTENSION
            );
            self.store_view();
            match self
                .project
                .save_to_bytes()
                .map_err(|e| e.to_string())
                .and_then(|bytes| crate::platform::download(&name, &bytes))
            {
                Ok(()) => {
                    self.project.display_name = name.clone();
                    self.command_line.info(format!("Downloaded {name}"));
                    self.toasts.success(format!("Saved {name}"));
                    // The download is now the user's copy; drop the recovery snapshot.
                    self.autosave.written = None;
                    wasm_bindgen_futures::spawn_local(async {
                        let _ = crate::platform::opfs::remove(crate::platform::opfs::AUTOSAVE).await;
                    });
                }
                Err(e) => {
                    self.command_line.error(format!("Save failed: {e}"));
                    self.toasts.error("Save failed — see the command line");
                }
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save_as(&mut self) {
        let default = file_stem(&self.project.doc.meta().name);
        if let Some(path) = self.platform.pick_save_path(FileKind::Project, &default) {
            self.store_view();
            match self.project.save_as(&path) {
                Ok(()) => {
                    self.command_line.info(format!("Saved {}", path.display()));
                    self.toasts.success("Saved");
                }
                Err(e) => {
                    self.command_line.error(format!("Save failed: {e}"));
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
                    FileKind::Dxf => self.import_dxf(&name, &bytes),
                    FileKind::Codes => self.import_code_sheet(&name, &bytes),
                    FileKind::LandXml => self.import_landxml(&name, &bytes),
                    // Reports are only saved, never opened.
                    FileKind::Report => {}
                },
                #[cfg(not(target_arch = "wasm32"))]
                FileEvent::ProjectPath(path) => {
                    let name = crate::platform::file_name(&path);
                    self.open_store(ProjectStore::open_file(&path), name);
                }
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
            self.project.doc.revision(),
            self.project.history.pending_audit().len(),
        );
        let due = now - self.autosave.last_ms >= AUTOSAVE_INTERVAL_MS;
        // Only real edits (which always produce audit records) are worth recovering.
        let edited = !self.project.history.pending_audit().is_empty();
        let blocked = !self.autosave.ready || self.recovery.is_some();
        if blocked || !edited || self.autosave.written == Some(key) || !due {
            if edited {
                ctx.request_repaint_after(std::time::Duration::from_secs(5));
            }
            return;
        }
        self.autosave.last_ms = now;
        self.store_view();
        match self.project.snapshot() {
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
        self.project.history.set_user(self.settings.user_name.clone());
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
        let dirty = if self.project.is_dirty() { " •" } else { "" };
        let title = format!("{}{dirty} — survey3d", self.project.display_name);
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
        if ctx.input(|i| i.viewport().close_requested()) && !self.allowed_to_close && self.project.is_dirty()
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
            self.undo();
        }
        if redo || redo_y {
            self.redo();
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
            } else if esc {
                self.command_line.input.clear();
                self.snap_override = None;
                self.dispatch(ToolEvent::Cancel);
            }
            if enter {
                match self.command_line.take_pending_input() {
                    Some(text) => self.submit(ctx, text),
                    None => self.dispatch(ToolEvent::Finish),
                }
            }
            if del {
                self.delete_selection();
            }
            if tab {
                self.snap_engine.cycle_next();
            }
        }
        self.shift_down = ctx.input(|i| i.modifiers.shift);
    }
}
