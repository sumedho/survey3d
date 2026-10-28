//! The editing session: a project, its tools and the command line, without any
//! user interface. The application drives it from the GUI; scripts and agents
//! drive it directly with [`Session::run`].
//!
//! Operations log what they did to [`Session::log`] and leave anything the
//! interface should show (a toast, a report, a panel to open) in the event queue
//! for the caller to [`take`](Session::take_events).

mod edit;
mod geoid;
#[cfg(not(target_arch = "wasm32"))]
mod images;
#[cfg(not(target_arch = "wasm32"))]
pub use images::control_report;

pub use edit::layer_change;
pub use geoid::{GEOID_HELP, geoid_files};
mod io;
mod points;

use egui_cad::command_line::{CommandLineState, LogKind};
use egui_cad::snap::{Pick, SnapMode};
use survey_core::point_input::PointInput;
use survey_model::{Command, EntityId, PointCloudId, SurfaceId};

use crate::project::Project;
use crate::tools::grips::Grip;
use crate::tools::{
    CoreModule, DomainModule, SELECT_TOOL_ID, Tool, ToolCtx, ToolEvent, ToolOutcome, ToolRegistry,
};

/// Something for the interface to show or do after an operation.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A short notification (also logged).
    Toast(Toast, String),
    /// A text report (listing, set-out table…).
    Report {
        title: String,
        text: String,
    },
    /// New content the view should frame.
    ZoomExtents,
    Open(Panel),
    /// A point cloud to make current in the clouds panel.
    SelectCloud(PointCloudId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toast {
    Success,
    Warning,
}

/// Panels an operation may ask to open.
#[derive(Clone, Debug, PartialEq)]
pub enum Panel {
    Parcels,
    Alignments,
    FeatureCodes,
    /// The raw data editor, on this file.
    RawData {
        name: String,
    },
}

/// Other names for session commands.
const COMMAND_ALIASES: &[(&str, &str)] = &[
    ("?", "help"),
    ("erase", "delete"),
    ("avg", "average"),
    ("li", "list"),
    ("f2f", "linework"),
    ("parcel", "mkparcel"),
];

/// The session command `name` (lowercase) stands for, if any.
fn canonical_command(name: &str) -> Option<&'static str> {
    SESSION_COMMANDS
        .iter()
        .map(|(n, _)| *n)
        .find(|n| *n == name)
        .or_else(|| COMMAND_ALIASES.iter().find(|(a, _)| *a == name).map(|(_, n)| *n))
}

/// How points can be typed, for help text.
pub const POINT_INPUT_HELP: &str =
    "Point input: E,N[,Z]  @dE,dN[,dZ]  @dist<bearing  or a point name; Enter (empty) finishes";

/// Pointer state the select and grip tools use (set by the viewport each frame).
#[derive(Clone, Copy, Debug, Default)]
pub struct Pointer {
    pub hovered: Option<EntityId>,
    pub hovered_grip: Option<Grip>,
    pub shift: bool,
}

/// Commands the session handles itself (the application adds view, file and
/// window commands). Name and description, for help and completion.
pub const SESSION_COMMANDS: &[(&str, &str)] = &[
    ("help", "List tools and commands"),
    ("undo", "Undo"),
    ("redo", "Redo"),
    ("delete", "Delete selection"),
    ("explode", "Explode polylines into segments"),
    ("join", "Join polylines that meet end to end"),
    ("reverse", "Reverse polyline direction"),
    ("average", "Average selected points"),
    ("list", "List selection (bearings, distances, coordinates)"),
    ("linework", "Process field-to-finish linework"),
    ("mkparcel", "Parcels from selected closed polylines"),
    ("mkalign", "Alignment from the selected polyline"),
    ("contours", "Contours of the active surface to polylines"),
    (
        "reduce",
        "Reduce raw data to coded points (reduce <file> when there are several)",
    ),
    (
        "renumber",
        "Renumber points: renumber 1001 (selection), renumber +1000, renumber 1-50 1001",
    ),
    (
        "raise",
        "Raise or lower the selection's heights: raise 0.15, raise -0.05",
    ),
    (
        "geoid",
        "Geoid separation at the selection; geoid ortho / geoid ellip convert heights",
    ),
];

pub struct Session {
    pub project: Project,
    pub tools: ToolRegistry,
    pub active_tool: Box<dyn Tool>,
    pub log: CommandLineState,
    /// One-shot snap mode typed for the next point (`end`, `mid`…).
    pub snap_override: Option<SnapMode>,
    /// The surface surface commands act on.
    pub active_surface: Option<SurfaceId>,
    pub pointer: Pointer,
    modules: Vec<Box<dyn DomainModule>>,
    events: Vec<Event>,
    /// Folder the project's geoid model is read from.
    geoid_dir: Option<std::path::PathBuf>,
    geoid: geoid::GeoidState,
    /// Image tile caches (desktop).
    #[cfg(not(target_arch = "wasm32"))]
    pub imagery: crate::imagery::Imagery,
}

impl Session {
    pub fn new(project: Project) -> Self {
        let mut tools = ToolRegistry::default();
        // Domain modules (cadastral, construction, mining) register here as they are added.
        let modules: Vec<Box<dyn DomainModule>> = vec![Box::new(CoreModule)];
        for m in &modules {
            m.register(&mut tools);
        }
        let active_tool = (tools.get(SELECT_TOOL_ID).expect("select tool registered").create)();
        Self {
            project,
            tools,
            active_tool,
            log: CommandLineState::default(),
            snap_override: None,
            active_surface: None,
            pointer: Pointer::default(),
            modules,
            events: Vec::new(),
            geoid_dir: None,
            geoid: Default::default(),
            #[cfg(not(target_arch = "wasm32"))]
            imagery: Default::default(),
        }
    }

    pub fn module_names(&self) -> impl Iterator<Item = &str> {
        self.modules.iter().map(|m| m.name())
    }

    /// Switches to another project (the caller resets its views).
    pub fn replace_project(&mut self, project: Project) {
        self.project = project;
        self.active_surface = None;
        self.set_tool(SELECT_TOOL_ID);
        self.sync_geoid();
        #[cfg(not(target_arch = "wasm32"))]
        self.sync_imagery();
    }

    /// Events since the last call, oldest first.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    fn emit(&mut self, event: Event) {
        self.events.push(event);
    }

    /// Logs `text` as a result and asks for a success toast.
    fn success(&mut self, text: String) {
        self.log.result(text.clone());
        self.emit(Event::Toast(Toast::Success, text));
    }

    fn report(&mut self, title: impl Into<String>, text: impl Into<String>) {
        self.emit(Event::Report {
            title: title.into(),
            text: text.into(),
        });
    }

    // ------------------------------------------------------------------ editing

    pub fn execute(&mut self, command: Box<dyn Command>) {
        let p = &mut self.project;
        let dropped = p.history.dropped();
        p.history.execute(command, &mut p.doc);
        let doc = &p.doc;
        p.selection.retain(|id| doc.contains(id));
        // The coordinate system may have changed its geoid model, and images come and go.
        self.sync_geoid();
        #[cfg(not(target_arch = "wasm32"))]
        self.sync_imagery();
        let p = &mut self.project;
        let now = p.history.dropped() - dropped;
        // Step-count trimming is routine; say so when memory forced it.
        if now > 0 && p.history.memory_used() > survey_model::UNDO_MEMORY_LIMIT / 2 {
            self.log.info(format!(
                "Undo history is using {} MB: the {now} oldest undo step{} were let go",
                p.history.memory_used() / (1024 * 1024),
                if now == 1 { "" } else { "s" }
            ));
        }
    }

    /// Executes a command built elsewhere (a window or importer) and logs its summary.
    pub fn apply(&mut self, command: Box<dyn Command>, summary: impl Into<String>) {
        self.execute(command);
        self.log.result(summary);
    }

    /// As [`Self::apply`], with a toast as well.
    pub fn apply_with_toast(&mut self, command: Box<dyn Command>, summary: impl Into<String>) {
        self.execute(command);
        self.success(summary.into());
    }

    pub fn undo(&mut self) {
        let p = &mut self.project;
        match p.history.undo(&mut p.doc) {
            Some(label) => self.log.info(format!("Undo: {label}")),
            None => self.log.warn("Nothing to undo"),
        }
        let doc = &p.doc;
        p.selection.retain(|id| doc.contains(id));
        self.sync_geoid();
        #[cfg(not(target_arch = "wasm32"))]
        self.sync_imagery();
    }

    pub fn redo(&mut self) {
        let p = &mut self.project;
        match p.history.redo(&mut p.doc) {
            Some(label) => self.log.info(format!("Redo: {label}")),
            None => self.log.warn("Nothing to redo"),
        }
        let doc = &p.doc;
        p.selection.retain(|id| doc.contains(id));
        self.sync_geoid();
        #[cfg(not(target_arch = "wasm32"))]
        self.sync_imagery();
    }

    // ------------------------------------------------------------------ tools

    pub fn set_tool(&mut self, id: &str) {
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
    pub fn dispatch(&mut self, event: ToolEvent<'_>) {
        let is_point = matches!(event, ToolEvent::Point(_));
        self.with_tool(|tool, ctx| tool.on_event(ctx, event));
        if is_point {
            self.snap_override = None;
        }
    }

    /// Runs `f` on the active tool with a [`ToolCtx`], then executes the commands
    /// it emitted and returns to the select tool if it finished.
    pub fn with_tool(&mut self, f: impl FnOnce(&mut dyn Tool, &mut ToolCtx<'_>) -> ToolOutcome) {
        let mut commands: Vec<Box<dyn Command>> = Vec::new();
        let outcome = {
            let p = &mut self.project;
            let mut ctx = ToolCtx::new(&p.doc, &mut p.selection, &mut self.log, &mut commands);
            ctx.hovered = self.pointer.hovered;
            ctx.hovered_grip = self.pointer.hovered_grip;
            ctx.shift = self.pointer.shift;
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

    /// The active tool's prompt.
    pub fn prompt(&self) -> String {
        self.active_tool.prompt(self.project.doc.conventions())
    }

    // ------------------------------------------------------------------ command line

    /// Runs one line of command-line input exactly as typed in the application
    /// (tool names, point names, coordinates, tool options, session commands).
    pub fn run(&mut self, text: &str) {
        if !self.submit(text) && !self.run_command(text.trim()) {
            self.submit_unhandled(text.trim());
        }
    }

    /// Routes typed input to the active tool (values, points, codes) or starts a
    /// tool. Returns `false` for anything else, which the caller tries as a
    /// command ([`Self::run_command`] and its own) before [`Self::submit_unhandled`].
    pub fn submit(&mut self, text: &str) -> bool {
        let text = text.trim();
        let prompt = self.prompt();
        self.log.log(LogKind::Command, format!("{prompt} {text}"));
        if text.is_empty() {
            self.dispatch(ToolEvent::Finish);
            return true;
        }

        // Values the tool asks for (a radius, a bearing, an observation…) go straight to it.
        if self.active_tool.claims_text(self.project.doc.conventions(), text) {
            self.dispatch(ToolEvent::Text(text));
            return true;
        }

        if self.active_tool.wants_point() {
            if let Some(mode) = SnapMode::from_code(text) {
                self.snap_override = Some(mode);
                self.log
                    .info(format!("Snap override: {} (next point)", mode.label()));
                return true;
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
                return true;
            }
            match PointInput::parse(text, self.project.doc.conventions()) {
                Ok(Some(input)) => {
                    match input.resolve(self.active_tool.base_point()) {
                        Ok(r) => self.dispatch(ToolEvent::Point(Pick {
                            point: r.position,
                            height_known: r.height_known,
                            snap: None,
                        })),
                        Err(e) => self.log.error(e.to_string()),
                    }
                    return true;
                }
                Err(e) => {
                    self.log.error(e.to_string());
                    return true;
                }
                Ok(None) => {}
            }
        }

        // A feature code wins over a command of the same name while coding points.
        if self.active_tool.takes_codes() && self.project.doc.meta().feature_codes.feature_for(text).is_some()
        {
            self.dispatch(ToolEvent::Text(text));
            return true;
        }
        if let Some(desc) = self.tools.find_command(text) {
            let id = desc.id;
            self.set_tool(id);
            return true;
        }
        false
    }

    /// Input that is neither a tool, a point nor a command: an option for the
    /// active tool, or an error.
    pub fn submit_unhandled(&mut self, text: &str) {
        if self.active_tool.id() != SELECT_TOOL_ID || self.active_tool.wants_point() {
            self.dispatch(ToolEvent::Text(text));
        } else {
            self.log.error(format!("Unknown command '{text}' — type 'help'"));
        }
    }

    /// Tools, the session's commands and the point input forms, one per line.
    pub fn help_lines(&self) -> Vec<String> {
        let mut lines = vec!["Tools:".to_string()];
        for t in self.tools.tools() {
            lines.push(format!("  {:<18} {}", t.commands.join(", "), t.description));
        }
        lines.push("Commands:".into());
        for (name, title) in SESSION_COMMANDS {
            lines.push(format!("  {name:<18} {title}"));
        }
        lines.push(POINT_INPUT_HELP.into());
        lines
    }

    /// Runs a session command ([`SESSION_COMMANDS`] and aliases); `false` if `text` is not one.
    /// A command ends the active tool first (as starting another tool does).
    pub fn run_command(&mut self, text: &str) -> bool {
        // Commands that take arguments.
        if let Some((head, args)) = text.trim().split_once(char::is_whitespace) {
            let head = head.to_ascii_lowercase();
            if matches!(head.as_str(), "reduce" | "renumber" | "raise" | "geoid") {
                if self.active_tool.id() != SELECT_TOOL_ID {
                    self.set_tool(SELECT_TOOL_ID);
                }
                let args = args.trim();
                let result = match head.as_str() {
                    "reduce" => self.reduce_raw(args),
                    "renumber" => self.renumber(args),
                    "geoid" => self.geoid_command(args),
                    _ => self.raise_selection(args),
                };
                if let Err(e) = result {
                    self.log.error(e);
                }
                return true;
            }
        }
        let lower = text.to_ascii_lowercase();
        let Some(command) = canonical_command(&lower) else {
            return false;
        };
        if command == "help" {
            let help = self.help_lines().join("\n");
            self.log.info(help);
            return true;
        }
        if self.active_tool.id() != SELECT_TOOL_ID {
            self.set_tool(SELECT_TOOL_ID);
        }
        match command {
            "undo" => self.undo(),
            "redo" => self.redo(),
            "delete" => self.delete_selection(),
            "explode" => self.explode_selection(),
            "join" => self.join_selection(),
            "reverse" => self.reverse_selection(),
            "average" => self.average_selection(),
            "list" => self.list_selection(),
            "linework" => {
                let library = self.project.doc.meta().feature_codes.clone();
                let only_selected = !self.project.selection.is_empty();
                self.process_linework(library, only_selected, false);
            }
            "mkparcel" => self.parcels_from_selection(),
            "mkalign" => self.alignment_from_selection(),
            "contours" => self.contours_to_polylines(),
            "reduce" => {
                if let Err(e) = self.reduce_raw("") {
                    self.log.error(e);
                }
            }
            "renumber" => self.log.info(format!("Usage: {}", points::RENUMBER_HELP)),
            "geoid" => {
                if let Err(e) = self.geoid_command("") {
                    self.log.error(e);
                }
            }
            "raise" => self.log.info(
                "Usage: raise <amount> — raise 0.150, or raise -0.05 to lower, the selection's heights",
            ),
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests;
