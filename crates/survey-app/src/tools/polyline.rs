use egui_cad::snap::Pick;
use survey_core::Conventions;
use survey_core::point_input::ResolvedPoint;
use survey_model::{ChangeSet, Entity, Polyline};

use super::{Preview, Tool, ToolCtx, ToolEvent, ToolOutcome, resolved};

pub const ID: &str = "polyline";

/// Draws polylines. Enter or right-click finishes, `c` closes, `u` removes the last vertex.
/// After finishing, the tool is ready for the next polyline; Escape with no vertices exits.
#[derive(Default)]
pub struct PolylineTool {
    vertices: Vec<ResolvedPoint>,
}

impl Tool for PolylineTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        match self.vertices.len() {
            0 => "Polyline — first point:".into(),
            1 => "Next point [Undo]:".into(),
            2 => "Next point [Undo] (Enter to finish):".into(),
            _ => "Next point [Close/Undo] (Enter to finish):".into(),
        }
    }

    fn base_point(&self) -> Option<ResolvedPoint> {
        self.vertices.last().copied()
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) => {
                self.vertices.push(resolved(&pick));
                ToolOutcome::Continue
            }
            ToolEvent::Text(text) => {
                match text.trim().to_ascii_lowercase().as_str() {
                    "c" | "close" if self.vertices.len() >= 3 => self.commit(ctx, true),
                    "c" | "close" => ctx.log.warn("Need at least 3 points to close"),
                    "u" | "undo" => {
                        self.vertices.pop();
                    }
                    other => ctx.log.error(format!("Unknown option '{other}'")),
                }
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Finish => {
                self.commit(ctx, false);
                ToolOutcome::Continue
            }
            ToolEvent::Cancel => {
                if self.vertices.is_empty() {
                    ToolOutcome::Done
                } else {
                    self.commit(ctx, false);
                    ToolOutcome::Continue
                }
            }
        }
    }

    fn preview(&self, _conv: &Conventions, cursor: Option<&Pick>, out: &mut Preview) {
        if self.vertices.len() >= 2 {
            out.polylines
                .push(self.vertices.iter().map(|v| v.position).collect());
        }
        if let (Some(last), Some(c)) = (self.vertices.last(), cursor) {
            out.rubber.push([last.position, c.point]);
        }
    }
}

impl PolylineTool {
    fn commit(&mut self, ctx: &mut ToolCtx<'_>, closed: bool) {
        let vertices: Vec<_> = std::mem::take(&mut self.vertices)
            .into_iter()
            .map(|v| v.position)
            .collect();
        if vertices.len() < 2 {
            return;
        }
        let n = vertices.len();
        let line = Polyline {
            name: String::new(),
            vertices,
            bulges: Vec::new(),
            closed,
            code: String::new(),
            layer: ctx.doc.current_layer(),
        };
        ctx.log.result(format!(
            "Polyline: {n} vertices{}",
            if closed { ", closed" } else { "" }
        ));
        ctx.execute(
            ChangeSet::new(format!("Add polyline ({n} vertices)")).insert_entity(Entity::Polyline(line)),
        );
    }
}
