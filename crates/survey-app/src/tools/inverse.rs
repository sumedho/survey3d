use egui_cad::snap::Pick;
use survey_core::Conventions;
use survey_core::DVec2;
use survey_core::cogo::inverse;
use survey_core::point_input::ResolvedPoint;

use super::{Preview, Tool, ToolCtx, ToolEvent, ToolOutcome, resolved};

pub const ID: &str = "inverse";

/// Reports bearing, distance and height difference between successive points.
#[derive(Default)]
pub struct InverseTool {
    from: Option<ResolvedPoint>,
    /// The combined factor at the start point when the project shows ground
    /// distances (for the live label; the result uses the whole line's).
    ground: Option<f64>,
}

/// One-line summary of an inverse between two points; `ground` is the
/// combined factor when ground distances are shown.
pub fn describe(conv: &Conventions, from: ResolvedPoint, to: ResolvedPoint, ground: Option<f64>) -> String {
    let inv = inverse(from.position, to.position);
    let bearing = inv
        .bearing
        .map_or_else(|| "—".to_string(), |b| conv.format_bearing(b));
    let mut s = match ground {
        Some(k) => format!(
            "Brg {bearing}  Ground {}  (grid {}, factor {k:.8})",
            conv.format_length_with_unit(inv.horizontal / k),
            conv.format_length(inv.horizontal)
        ),
        None => format!(
            "Brg {bearing}  Dist {}",
            conv.format_length_with_unit(inv.horizontal)
        ),
    };
    if from.height_known && to.height_known {
        s.push_str(&format!(
            "  dZ {}  Slope {}",
            conv.format_length(inv.delta.z),
            conv.format_length(inv.slope)
        ));
        if let Some(g) = inv.grade {
            s.push_str(&format!("  Grade {:.3}%", g * 100.0));
        }
    }
    s
}

impl Tool for InverseTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        if self.from.is_none() {
            "Inverse — from point:".into()
        } else {
            "Inverse — to point (Enter to finish):".into()
        }
    }

    fn base_point(&self) -> Option<ResolvedPoint> {
        self.from
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) => {
                let to = resolved(&pick);
                let doc = ctx.doc;
                let ground = |a: ResolvedPoint, b: ResolvedPoint| {
                    doc.ground_distances()
                        .then(|| doc.line_factor(a.position, b.position))
                };
                if let Some(from) = self.from {
                    ctx.log
                        .result(describe(ctx.conventions(), from, to, ground(from, to)));
                }
                // Chain: the end of one inverse starts the next.
                self.from = Some(to);
                self.ground = ground(to, to);
                ToolOutcome::Continue
            }
            ToolEvent::Text(other) => {
                ctx.log.error(format!("Unknown option '{other}'"));
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) => ToolOutcome::Continue,
            // Enter or Escape ends the command (as the prompt says).
            ToolEvent::Finish | ToolEvent::Cancel => ToolOutcome::Done,
        }
    }

    fn preview(&self, conv: &Conventions, cursor: Option<&Pick>, out: &mut Preview) {
        if let (Some(from), Some(c)) = (self.from, cursor) {
            out.rubber.push([from.position, c.point]);
            out.labels.push((
                c.point,
                DVec2::new(14.0, 14.0),
                describe(conv, from, resolved(c), self.ground),
            ));
        }
    }
}
