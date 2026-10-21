//! The survey3d application: input routing, tools and file handling.
//! Panel and viewport drawing live in [`layout`].

mod layout;

use cad_render::{GridSettings, Projection};
use egui::{Key, KeyboardShortcut, Modifiers};
use egui_cad::chrome::{bar_frame, panel_frame, section_header, tool_button};
use egui_cad::command_line::{CommandLine, CommandLineEvent, CommandLineState, Completion, LogKind};
use egui_cad::constraint::constrain_direction;
use egui_cad::layer_panel::{LayerAction, LayerRow, layer_panel};
use egui_cad::snap::{Pick, SnapEngine, SnapMode, SnapQuery, SnapSettings, draw_snap_glyph, free_point};
use egui_cad::viewport::{Viewport, ViewportState, draw_pickbox, draw_world_polyline, to_screen};
use egui_cad::{CadTheme, icons};
use glam::DVec3;
use survey_core::Aabb3;
use survey_core::point_input::PointInput;
use survey_model::{ChangeSet, Color, Command, Entity, EntityId, Layer, LayerId};
use survey_store::ProjectStore;

use crate::platform::{FileEvent, Platform};
use crate::project::{APP_VERSION, Project};
use crate::scene_build::{DocSnapProvider, SceneCache, color32, hit_test};
use crate::settings::AppSettings;
use crate::tools::{
    CoreModule, DomainModule, Preview, SELECT_TOOL_ID, Tool, ToolCtx, ToolEvent, ToolOutcome, ToolRegistry,
};
use crate::ui::audit_window::AuditWindow;
use crate::ui::csv_import::{CsvDialogResult, CsvImportDialog};
use crate::ui::properties::properties_panel;
use crate::ui::settings_window::SettingsWindow;

const SETTINGS_KEY: &str = "survey3d.settings";

/// Commands handled by the application itself (not tools).
const APP_COMMANDS: &[(&str, &str)] = &[
    ("undo", "Undo the last edit (Ctrl+Z)"),
    ("redo", "Redo (Ctrl+Shift+Z)"),
    ("ze", "Zoom to extents (Home)"),
    ("plan", "Lock to 2D plan view"),
    ("3d", "Free 3D view"),
    ("persp", "Toggle perspective / orthographic"),
    ("snap", "Toggle object snap (F3)"),
    ("ortho", "Toggle ortho constraint (F8)"),
    ("grid", "Toggle grid (F7)"),
    ("delete", "Delete selection (Del)"),
    ("import", "Import points from CSV/TXT"),
    ("save", "Save project (Ctrl+S)"),
    ("open", "Open project (Ctrl+O)"),
    ("new", "New project (Ctrl+N)"),
    ("audit", "Show the audit trail"),
    ("settings", "Open settings"),
    ("drafting", "Snap, ortho and grid settings"),
    ("help", "List commands"),
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
    snap_engine: SnapEngine,
    snap_override: Option<SnapMode>,
    command_line: CommandLineState,
    completions: Vec<Completion>,
    platform: Platform,
    /// Resolved cursor position this frame (snapped if possible).
    pick: Option<Pick>,
    hovered_entity: Option<EntityId>,
    csv_dialog: Option<CsvImportDialog>,
    settings_window: SettingsWindow,
    audit_window: AuditWindow,
    about_open: bool,
    confirm_discard: Option<Discard>,
    allowed_to_close: bool,
    title: String,
    shift_down: bool,
    /// Entity counts per layer, cached by document revision.
    layer_counts: (u64, std::collections::HashMap<LayerId, usize>),
}

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
        let theme = CadTheme::for_mode(settings.theme);
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
        completions.extend(APP_COMMANDS.iter().map(|(n, d)| Completion {
            name: (*n).to_string(),
            description: (*d).to_string(),
        }));
        completions.sort_by(|a, b| a.name.cmp(&b.name));

        let project = Project::new(&settings).expect("in-memory SQLite is available");
        let active_tool = (tools.get(SELECT_TOOL_ID).expect("select tool registered").create)();
        let mut viewport = ViewportState::new(1);
        viewport.camera.target = DVec3::ZERO;

        let mut command_line = CommandLineState::default();
        let module_names: Vec<&str> = modules.iter().map(|m| m.name()).collect();
        command_line.info(format!(
            "survey3d {APP_VERSION} — modules: {} — type 'help' for commands.",
            module_names.join(", ")
        ));

        Self {
            settings,
            theme,
            project,
            tools,
            active_tool,
            viewport,
            scene_cache: SceneCache::default(),
            snap_engine: SnapEngine::default(),
            snap_override: None,
            command_line,
            completions,
            platform: Platform::default(),
            pick: None,
            hovered_entity: None,
            csv_dialog: None,
            settings_window: SettingsWindow::default(),
            audit_window: AuditWindow::default(),
            about_open: false,
            confirm_discard: None,
            allowed_to_close: false,
            title: String::new(),
            shift_down: false,
            layer_counts: (0, Default::default()),
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

    fn set_tool(&mut self, id: &str) {
        let Some(create) = self.tools.get(id).map(|d| d.create) else {
            return;
        };
        // Give the current tool a chance to commit work in progress.
        if self.active_tool.id() != id {
            self.dispatch(ToolEvent::Cancel);
        }
        self.active_tool = create();
        self.snap_override = None;
    }

    /// Sends an event to the active tool and executes the commands it emits.
    fn dispatch(&mut self, event: ToolEvent<'_>) {
        let is_point = matches!(event, ToolEvent::Point(_));
        let mut commands: Vec<Box<dyn Command>> = Vec::new();
        let shift = self.shift_down;
        let outcome = {
            let p = &mut self.project;
            let mut ctx = ToolCtx::new(&p.doc, &mut p.selection, &mut self.command_line, &mut commands);
            ctx.hovered = self.hovered_entity;
            ctx.shift = shift;
            self.active_tool.on_event(&mut ctx, event)
        };
        for c in commands {
            self.execute(c);
        }
        if is_point {
            self.snap_override = None;
        }
        if outcome == ToolOutcome::Done
            && self.active_tool.id() != SELECT_TOOL_ID
            && let Some(desc) = self.tools.get(SELECT_TOOL_ID)
        {
            self.active_tool = (desc.create)();
        }
    }

    fn zoom_extents(&mut self) {
        let b = self.project.doc.bounds();
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

        if let Some(desc) = self.tools.find_command(text) {
            let id = desc.id;
            self.set_tool(id);
            return;
        }
        if self.run_app_command(ctx, text) {
            return;
        }
        if self.active_tool.id() != SELECT_TOOL_ID {
            self.dispatch(ToolEvent::Text(text));
        } else {
            self.command_line
                .error(format!("Unknown command '{text}' — type 'help'"));
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
            "grid" => self.settings.show_grid = !self.settings.show_grid,
            "delete" | "erase" => self.delete_selection(),
            "import" => self.platform.pick_csv(ctx),
            "save" => self.save(ctx),
            "open" => self.request_discard(ctx, Discard::Open),
            "new" => self.request_discard(ctx, Discard::New),
            "audit" => self.audit_window.open = true,
            "settings" | "options" => self.settings_window.open = true,
            "drafting" | "dsettings" => self.settings_window.open_at_drafting(),
            "help" | "?" => {
                let mut lines = vec!["Tools:".to_string()];
                for t in self.tools.tools() {
                    lines.push(format!("  {:<18} {}", t.commands.join(", "), t.description));
                }
                lines.push("Commands:".into());
                for (n, d) in APP_COMMANDS {
                    lines.push(format!("  {n:<18} {d}"));
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
            Discard::Open => self.platform.pick_project(ctx),
            Discard::Quit => {
                self.allowed_to_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn replace_project(&mut self, project: Project) {
        self.project = project;
        self.scene_cache.invalidate();
        self.set_tool(SELECT_TOOL_ID);
        self.zoom_extents();
    }

    fn open_store(&mut self, result: Result<ProjectStore, survey_store::StoreError>, name: String) {
        let project = result.and_then(|store| Project::from_store(store, name.clone(), &self.settings));
        match project {
            Ok(p) => {
                let n = p.doc.entity_count();
                self.replace_project(p);
                self.command_line.info(format!("Opened {name} ({n} objects)"));
            }
            Err(e) => self.command_line.error(format!("Could not open {name}: {e}")),
        }
    }

    fn save(&mut self, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            if self.project.has_file() {
                match self.project.save() {
                    Ok(()) => self
                        .command_line
                        .info(format!("Saved {}", self.project.display_name)),
                    Err(e) => self.command_line.error(format!("Save failed: {e}")),
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
            match self
                .project
                .save_to_bytes()
                .map_err(|e| e.to_string())
                .and_then(|bytes| crate::platform::download(&name, &bytes))
            {
                Ok(()) => {
                    self.project.display_name = name.clone();
                    self.command_line.info(format!("Downloaded {name}"));
                }
                Err(e) => self.command_line.error(format!("Save failed: {e}")),
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save_as(&mut self) {
        let default = file_stem(&self.project.doc.meta().name);
        if let Some(path) = self.platform.pick_save_path(&default) {
            match self.project.save_as(&path) {
                Ok(()) => self.command_line.info(format!("Saved {}", path.display())),
                Err(e) => self.command_line.error(format!("Save failed: {e}")),
            }
        }
    }

    fn handle_file_events(&mut self) {
        for event in self.platform.poll() {
            match event {
                #[cfg(target_arch = "wasm32")]
                FileEvent::ProjectBytes { name, bytes } => {
                    self.open_store(ProjectStore::from_bytes(&bytes), name);
                }
                #[cfg(not(target_arch = "wasm32"))]
                FileEvent::ProjectPath(path) => {
                    let name = crate::platform::file_name(&path);
                    self.open_store(ProjectStore::open_file(&path), name);
                }
                FileEvent::CsvBytes { name, bytes } => {
                    self.csv_dialog = Some(CsvImportDialog::new(
                        name,
                        bytes,
                        self.settings.csv_layout.clone(),
                    ));
                }
            }
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
        if self.theme.mode != self.settings.theme {
            self.theme = CadTheme::for_mode(self.settings.theme);
            self.theme.apply(&ctx);
        }
        self.project.history.set_user(self.settings.user_name.clone());
        self.handle_file_events();
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
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
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

        let (f3, f7, f8, home) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::F3),
                i.consume_key(Modifiers::NONE, Key::F7),
                i.consume_key(Modifiers::NONE, Key::F8),
                i.consume_key(Modifiers::NONE, Key::Home),
            )
        });
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
            if esc {
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
