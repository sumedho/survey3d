use egui_cad::snap::Pick;
use survey_core::Conventions;
use survey_core::arc::Arc;
use survey_core::point_input::ResolvedPoint;
use survey_model::{ChangeSet, Entity, Polyline};

use super::{Preview, Tool, ToolCtx, ToolEvent, ToolOutcome, resolved};

pub const ID: &str = "arc";

/// Three-point arc: start, any point on the arc, end. The result is a
/// two-vertex polyline whose segment carries the arc's bulge.
#[derive(Default)]
pub struct ArcTool {
    points: Vec<ResolvedPoint>,
}

impl Tool for ArcTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        match self.points.len() {
            0 => "Arc — start point:".into(),
            1 => "Arc — point on the arc:".into(),
            _ => "Arc — end point:".into(),
        }
    }

    fn base_point(&self) -> Option<ResolvedPoint> {
        self.points.last().copied()
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) => {
                self.points.push(resolved(&pick));
                if self.points.len() == 3 {
                    let [a, m, b] = [self.points[0], self.points[1], self.points[2]];
                    self.points.clear();
                    match Arc::bulge_through(a.position, m.position, b.position) {
                        Some(bulge) => commit(ctx, a, b, bulge),
                        None => ctx.log.error("The three points are in a straight line"),
                    }
                }
                ToolOutcome::Continue
            }
            ToolEvent::Text(other) => {
                ctx.log.error(format!("Unknown option '{other}'"));
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Finish | ToolEvent::Cancel => {
                if self.points.is_empty() {
                    ToolOutcome::Done
                } else {
                    self.points.clear();
                    ToolOutcome::Continue
                }
            }
        }
    }

    fn preview(&self, _conv: &Conventions, cursor: Option<&Pick>, out: &mut Preview) {
        let Some(c) = cursor else {
            return;
        };
        match self.points.as_slice() {
            [a] => out.rubber.push([a.position, c.point]),
            [a, m] => match Arc::bulge_through(a.position, m.position, c.point)
                .and_then(|b| Arc::from_bulge(a.position, c.point, b))
            {
                Some(arc) => out.polylines.push(arc.tessellate(arc.radius * 1e-3)),
                None => out.rubber.push([a.position, c.point]),
            },
            _ => {}
        }
    }
}

fn commit(ctx: &mut ToolCtx<'_>, a: ResolvedPoint, b: ResolvedPoint, bulge: f64) {
    let conv = ctx.conventions();
    if let Some(arc) = Arc::from_bulge(a.position, b.position, bulge) {
        ctx.log.result(format!(
            "Arc: radius {}  length {}",
            conv.format_length_with_unit(arc.radius),
            conv.format_length_with_unit(arc.length_2d())
        ));
    }
    let line = Polyline {
        name: String::new(),
        vertices: vec![a.position, b.position],
        bulges: vec![bulge, 0.0],
        closed: false,
        code: String::new(),
        layer: ctx.doc.current_layer(),
    };
    ctx.execute(ChangeSet::new("Add arc").insert_entity(Entity::Polyline(line)));
}
