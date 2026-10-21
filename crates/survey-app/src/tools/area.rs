use egui_cad::snap::Pick;
use glam::DVec3;
use survey_core::Conventions;
use survey_core::DVec2;
use survey_core::arc::polyline_metrics;
use survey_model::Entity;

use super::{Preview, Tool, ToolCtx, ToolEvent, ToolOutcome};

pub const ID: &str = "area";

/// Area and perimeter of selected polylines, or of a figure picked corner by corner.
#[derive(Default)]
pub struct AreaTool {
    corners: Vec<DVec3>,
}

fn describe(conv: &Conventions, area: f64, perimeter: f64) -> String {
    format!(
        "Area {}  Perimeter {}",
        conv.format_area(area),
        conv.format_length_with_unit(perimeter)
    )
}

/// Plan area and perimeter of a closed figure through `corners`.
fn measure(corners: &[DVec3]) -> (f64, f64) {
    let (perimeter, _, area) = polyline_metrics(corners, &[], true);
    (area.abs(), perimeter)
}

impl Tool for AreaTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        if self.corners.is_empty() {
            "Area — first corner (or select polylines first):".into()
        } else {
            "Area — next corner [Undo] (Enter to finish):".into()
        }
    }

    fn activate(&mut self, ctx: &mut ToolCtx<'_>) -> ToolOutcome {
        let conv = ctx.conventions().clone();
        let mut lines = Vec::new();
        for id in ctx.selection.ids() {
            if let Some(Entity::Polyline(l)) = ctx.doc.entity(*id) {
                let (len, _, area) = polyline_metrics(&l.vertices, &l.bulges, true);
                let name = if l.name.is_empty() { "Polyline" } else { &l.name };
                let note = if l.closed {
                    ""
                } else {
                    " (open; closed with a straight line)"
                };
                lines.push((
                    area.abs(),
                    len,
                    format!("{name}{note}: {}", describe(&conv, area.abs(), len)),
                ));
            }
        }
        if lines.is_empty() {
            return ToolOutcome::Continue;
        }
        let (total, perimeter): (f64, f64) = lines.iter().fold((0.0, 0.0), |(a, p), l| (a + l.0, p + l.1));
        let many = lines.len() > 1;
        for (_, _, text) in lines {
            ctx.log.result(text);
        }
        if many {
            ctx.log
                .result(format!("Total: {}", describe(&conv, total, perimeter)));
        }
        ToolOutcome::Done
    }

    fn claims_text(&self, _conv: &Conventions, text: &str) -> bool {
        matches!(text.trim().to_ascii_lowercase().as_str(), "u" | "undo")
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) => {
                self.corners.push(pick.point);
                ToolOutcome::Continue
            }
            ToolEvent::Text(t) if matches!(t.trim().to_ascii_lowercase().as_str(), "u" | "undo") => {
                self.corners.pop();
                ToolOutcome::Continue
            }
            ToolEvent::Text(other) => {
                ctx.log.error(format!("Unknown option '{other}'"));
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Finish => {
                if self.corners.len() >= 3 {
                    let (area, perimeter) = measure(&self.corners);
                    ctx.log.result(format!(
                        "{} corners: {}",
                        self.corners.len(),
                        describe(ctx.conventions(), area, perimeter)
                    ));
                } else if !self.corners.is_empty() {
                    ctx.log.error("An area needs at least three corners");
                }
                if self.corners.is_empty() {
                    ToolOutcome::Done
                } else {
                    self.corners.clear();
                    ToolOutcome::Continue
                }
            }
            ToolEvent::Cancel => {
                if self.corners.is_empty() {
                    ToolOutcome::Done
                } else {
                    self.corners.clear();
                    ToolOutcome::Continue
                }
            }
        }
    }

    fn preview(&self, conv: &Conventions, cursor: Option<&Pick>, out: &mut Preview) {
        if self.corners.is_empty() {
            return;
        }
        let mut ring = self.corners.clone();
        if let Some(c) = cursor {
            ring.push(c.point);
        }
        out.polylines.push(ring.clone());
        if ring.len() >= 2 {
            out.rubber.push([ring[ring.len() - 1], ring[0]]);
        }
        if ring.len() >= 3
            && let Some(c) = cursor
        {
            let (area, perimeter) = measure(&ring);
            out.labels
                .push((c.point, DVec2::new(14.0, 14.0), describe(conv, area, perimeter)));
        }
    }
}
