use egui_cad::snap::Pick;
use survey_core::Conventions;
use survey_core::point_input::ResolvedPoint;
use survey_model::{ChangeSet, Entity};

use super::grips::{Grip, apply_grip, delete_vertex};
use super::{Preview, Tool, ToolCtx, ToolEvent, ToolOutcome};

pub const SELECT_TOOL_ID: &str = "select";

/// Click to select the object under the cursor; Shift toggles; clicking empty space clears.
/// Clicking a grip of a selected object makes it *hot*: the next pick (snapped,
/// typed or relative to the grip) moves it.
#[derive(Default)]
pub struct SelectTool {
    /// The grip being moved and its object as it was when picked.
    hot: Option<(Grip, Entity)>,
}

impl Tool for SelectTool {
    fn id(&self) -> &'static str {
        SELECT_TOOL_ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        match &self.hot {
            Some((grip, Entity::Polyline(_))) if matches!(grip.kind, super::grips::GripKind::Vertex(_)) => {
                format!("{} — new position [Delete vertex]:", grip.describe())
            }
            Some((grip, _)) => format!("{} — new position:", grip.describe()),
            None => "Command:".into(),
        }
    }

    fn wants_point(&self) -> bool {
        self.hot.is_some()
    }

    fn base_point(&self) -> Option<ResolvedPoint> {
        self.hot.as_ref().map(|(g, _)| ResolvedPoint {
            position: g.position,
            height_known: true,
        })
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) => {
                if let Some((grip, entity)) = self.hot.take() {
                    if let Some(edited) = apply_grip(&entity, &grip, &pick) {
                        let label = format!("{} {}", grip.describe(), entity.name())
                            .trim()
                            .to_string();
                        ctx.execute(ChangeSet::new(label).update_entity(grip.entity, edited));
                    }
                    return ToolOutcome::Continue;
                }
                if let Some(grip) = ctx.hovered_grip
                    && let Some(entity) = ctx.doc.entity(grip.entity)
                {
                    self.hot = Some((grip, entity.clone()));
                    return ToolOutcome::Continue;
                }
                match (ctx.hovered, ctx.shift) {
                    (Some(id), true) => ctx.selection.toggle(id),
                    (Some(id), false) => ctx.selection.set([id]),
                    (None, false) => ctx.selection.clear(),
                    (None, true) => {}
                }
                ToolOutcome::Continue
            }
            ToolEvent::Region(ids) => {
                if ctx.shift {
                    for id in ids {
                        ctx.selection.insert(id);
                    }
                } else {
                    ctx.selection.set(ids);
                }
                ToolOutcome::Continue
            }
            ToolEvent::Text(text) => {
                match (&self.hot, text.trim().to_ascii_lowercase().as_str()) {
                    (Some((grip, entity)), "d" | "del" | "delete") => match delete_vertex(entity, grip) {
                        Some(edited) => {
                            ctx.execute(ChangeSet::new("Delete vertex").update_entity(grip.entity, edited));
                            self.hot = None;
                        }
                        None => ctx.log.warn("A polyline needs at least two vertices"),
                    },
                    (Some(_), other) => ctx.log.error(format!("Unknown option '{other}'")),
                    (None, _) => {}
                }
                ToolOutcome::Continue
            }
            ToolEvent::Cancel => {
                if self.hot.take().is_none() {
                    ctx.selection.clear();
                }
                ToolOutcome::Continue
            }
            ToolEvent::Finish => {
                self.hot = None;
                ToolOutcome::Continue
            }
        }
    }

    fn preview(&self, _conv: &Conventions, cursor: Option<&Pick>, out: &mut Preview) {
        let (Some((grip, entity)), Some(c)) = (&self.hot, cursor) else {
            return;
        };
        out.rubber.push([grip.position, c.point]);
        match apply_grip(entity, grip, c) {
            Some(Entity::Polyline(l)) => {
                let tolerance = 1e-3 * (1.0 + grip.position.truncate().distance(c.point.truncate()));
                out.polylines.extend(l.tessellate(tolerance));
            }
            Some(Entity::Point(p)) => out.points.push(p.position),
            None => {}
        }
    }
}
