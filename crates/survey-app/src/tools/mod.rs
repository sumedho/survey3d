//! Interactive tools and the registry that domain modules extend.
//!
//! A [`Tool`] is a small state machine fed with [`ToolEvent`]s (picked points,
//! typed text, finish/cancel). Tools never mutate the document directly: they
//! emit [`Command`]s through [`ToolCtx`], which the app executes through the
//! undo history (and therefore the audit trail).

mod arc;
mod circle;
mod inverse;
mod point;
mod polyline;
mod select;

use egui_cad::command_line::CommandLineState;
use egui_cad::icons;
use egui_cad::snap::Pick;
use glam::DVec3;
use survey_core::point_input::ResolvedPoint;
use survey_core::{Conventions, DVec2};
use survey_model::{Command, Document, EntityId, Selection};

pub use select::SELECT_TOOL_ID;

/// Input delivered to the active tool.
#[derive(Clone, Debug)]
pub enum ToolEvent<'a> {
    /// A picked (possibly snapped) or typed point.
    Point(Pick),
    /// Typed text that was not a point (tool options such as `close` or `undo`).
    Text(&'a str),
    /// Entities chosen with a window/crossing box.
    Region(Vec<EntityId>),
    /// Enter / right-click: finish the current operation.
    Finish,
    /// Escape.
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolOutcome {
    /// Keep the tool active.
    Continue,
    /// The tool is finished; the app returns to the default (select) tool.
    Done,
}

/// What a tool may read and emit while handling an event.
pub struct ToolCtx<'a> {
    pub doc: &'a Document,
    pub selection: &'a mut Selection,
    pub log: &'a mut CommandLineState,
    /// Entity under the cursor (for selection).
    pub hovered: Option<EntityId>,
    /// Shift held (add/toggle selection).
    pub shift: bool,
    commands: &'a mut Vec<Box<dyn Command>>,
}

impl<'a> ToolCtx<'a> {
    pub fn new(
        doc: &'a Document,
        selection: &'a mut Selection,
        log: &'a mut CommandLineState,
        commands: &'a mut Vec<Box<dyn Command>>,
    ) -> Self {
        Self {
            doc,
            selection,
            log,
            hovered: None,
            shift: false,
            commands,
        }
    }

    pub fn conventions(&self) -> &Conventions {
        self.doc.conventions()
    }

    /// Queues a command; the app executes it after the event is handled.
    pub fn execute(&mut self, command: impl Command + 'static) {
        self.commands.push(Box::new(command));
    }
}

/// Transient geometry drawn over the viewport while a tool is active.
#[derive(Default)]
pub struct Preview {
    pub polylines: Vec<Vec<DVec3>>,
    /// Rubber-band lines (drawn dashed).
    pub rubber: Vec<[DVec3; 2]>,
    /// Text annotations anchored at world points, offset in pixels.
    pub labels: Vec<(DVec3, DVec2, String)>,
}

pub trait Tool {
    /// Registry id of this tool.
    fn id(&self) -> &'static str;
    /// Prompt shown on the command line.
    fn prompt(&self, conv: &Conventions) -> String;
    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome;
    /// Whether clicks and typed coordinates are treated as point input (enables snapping).
    fn wants_point(&self) -> bool {
        true
    }
    /// Whether a typed plain number should be passed as text (e.g. a radius)
    /// rather than looked up as a point name.
    fn wants_distance(&self) -> bool {
        false
    }
    /// The last point entered, for relative input, ortho and perpendicular snaps.
    fn base_point(&self) -> Option<ResolvedPoint> {
        None
    }
    fn preview(&self, _conv: &Conventions, _cursor: Option<&Pick>, _out: &mut Preview) {}
}

/// Static description of a tool, used for the palette, command names and creation.
#[derive(Clone)]
pub struct ToolDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    /// Command-line names; the first is canonical. Avoid single letters: they
    /// would shadow tool options such as `c` (close) or `u` (undo).
    pub commands: &'static [&'static str],
    pub icon: &'static str,
    pub description: &'static str,
    pub create: fn() -> Box<dyn Tool>,
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: Vec<ToolDescriptor>,
}

impl ToolRegistry {
    pub fn register(&mut self, tool: ToolDescriptor) {
        debug_assert!(self.get(tool.id).is_none(), "duplicate tool id {}", tool.id);
        self.tools.push(tool);
    }

    pub fn tools(&self) -> &[ToolDescriptor] {
        &self.tools
    }

    pub fn get(&self, id: &str) -> Option<&ToolDescriptor> {
        self.tools.iter().find(|t| t.id == id)
    }

    pub fn find_command(&self, name: &str) -> Option<&ToolDescriptor> {
        let name = name.trim().to_ascii_lowercase();
        self.tools.iter().find(|t| t.commands.contains(&name.as_str()))
    }
}

/// A bundle of tools (and, later, panels and reports) for one domain.
/// Cadastral, construction and mining modules plug in through this trait.
pub trait DomainModule {
    fn name(&self) -> &'static str;
    fn register(&self, tools: &mut ToolRegistry);
}

/// The general-purpose CAD/COGO tools every project gets.
pub struct CoreModule;

impl DomainModule for CoreModule {
    fn name(&self) -> &'static str {
        "Core"
    }

    fn register(&self, tools: &mut ToolRegistry) {
        tools.register(ToolDescriptor {
            id: SELECT_TOOL_ID,
            name: "Select",
            commands: &["select", "sel"],
            icon: icons::CURSOR,
            description: "Select objects (Shift toggles)",
            create: || Box::new(select::SelectTool),
        });
        tools.register(ToolDescriptor {
            id: point::ID,
            name: "Point",
            commands: &["point", "pt", "po"],
            icon: icons::CROSSHAIR,
            description: "Create survey points by picking or typing coordinates",
            create: || Box::<point::PointTool>::default(),
        });
        tools.register(ToolDescriptor {
            id: polyline::ID,
            name: "Polyline",
            commands: &["polyline", "pl", "line"],
            icon: icons::POLYGON,
            description: "Draw a polyline (string); Enter finishes, C closes, U undoes a vertex",
            create: || Box::<polyline::PolylineTool>::default(),
        });
        tools.register(ToolDescriptor {
            id: arc::ID,
            name: "Arc",
            commands: &["arc", "ar"],
            icon: icons::ARC,
            description: "Arc through three points: start, a point on the arc, end",
            create: || Box::<arc::ArcTool>::default(),
        });
        tools.register(ToolDescriptor {
            id: circle::ID,
            name: "Circle",
            commands: &["circle", "ci"],
            icon: icons::CIRCLE,
            description: "Circle from its centre and a radius (pick or type)",
            create: || Box::<circle::CircleTool>::default(),
        });
        tools.register(ToolDescriptor {
            id: inverse::ID,
            name: "Inverse",
            commands: &["inverse", "inv", "dist", "di"],
            icon: icons::RULER,
            description: "Bearing, distance and height difference between points",
            create: || Box::<inverse::InverseTool>::default(),
        });
    }
}

/// Converts a pick into the point form used for relative input.
pub(crate) fn resolved(pick: &Pick) -> ResolvedPoint {
    ResolvedPoint {
        position: pick.point,
        height_known: pick.height_known,
    }
}
