//! Trim and extend polylines to boundary edges, and points where lines would
//! meet if extended (apparent intersections).

use egui_cad::snap::Pick;
use glam::{DVec2, DVec3};
use survey_core::Conventions;
use survey_core::point_input::PointInput;
use survey_core::trim::Segment;
use survey_model::{ChangeSet, Entity, EntityId};

use super::cogo::new_point;
use super::{Preview, Tool, ToolCtx, ToolEvent, ToolOutcome};
use crate::trim_extend::{apparent_intersection, extend_command, segment_near, trim_command};

pub const TRIM_ID: &str = "trim";
pub const EXTEND_ID: &str = "extend";
pub const APPARENT_ID: &str = "apparent";

/// Trim (`extend: false`) or extend: boundary edges first (the selection's
/// polylines, picked ones, or every line when none are chosen), then each
/// pick trims the part clicked or extends the end clicked near.
pub struct TrimExtendTool {
    extend: bool,
    boundaries: Vec<EntityId>,
    choosing: bool,
}

impl TrimExtendTool {
    pub fn new(extend: bool) -> Self {
        Self {
            extend,
            boundaries: Vec::new(),
            choosing: true,
        }
    }

    fn verb(&self) -> &'static str {
        if self.extend { "Extend" } else { "Trim" }
    }

    fn add_boundaries(&mut self, ctx: &mut ToolCtx<'_>, ids: impl IntoIterator<Item = EntityId>) {
        for id in ids {
            if matches!(ctx.doc.entity(id), Some(Entity::Polyline(_))) && !self.boundaries.contains(&id) {
                self.boundaries.push(id);
                ctx.selection.insert(id);
            }
        }
    }
}

impl Tool for TrimExtendTool {
    fn id(&self) -> &'static str {
        if self.extend { EXTEND_ID } else { TRIM_ID }
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        let verb = self.verb();
        if self.choosing {
            return format!(
                "{verb} — pick boundary lines, Enter when done ({}):",
                if self.boundaries.is_empty() {
                    "Enter now: every line is a boundary".to_string()
                } else {
                    format!("{} chosen", self.boundaries.len())
                }
            );
        }
        if self.extend {
            "Extend — click near the end of a line to extend (Enter to finish):".into()
        } else {
            "Trim — click the part of a line to cut away (Enter to finish):".into()
        }
    }

    fn activate(&mut self, ctx: &mut ToolCtx<'_>) -> ToolOutcome {
        let chosen: Vec<EntityId> = ctx.selection.ids().iter().copied().collect();
        self.add_boundaries(ctx, chosen);
        if !self.boundaries.is_empty() {
            self.choosing = false;
            ctx.log.info(format!(
                "Boundaries: the {} selected lines",
                self.boundaries.len()
            ));
        }
        ToolOutcome::Continue
    }

    fn wants_point(&self) -> bool {
        true
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) => {
                let Some(id) = ctx.hovered else {
                    ctx.log.warn("Click on a line");
                    return ToolOutcome::Continue;
                };
                if self.choosing {
                    self.add_boundaries(ctx, [id]);
                    return ToolOutcome::Continue;
                }
                let at = pick.point.truncate();
                let result = if self.extend {
                    extend_command(ctx.doc, id, &self.boundaries, at)
                } else {
                    trim_command(ctx.doc, id, &self.boundaries, at)
                };
                match result {
                    Ok((cs, summary)) => {
                        ctx.log.result(summary);
                        ctx.execute(cs);
                    }
                    Err(e) => ctx.log.error(e),
                }
                ToolOutcome::Continue
            }
            ToolEvent::Region(ids) if self.choosing => {
                self.add_boundaries(ctx, ids);
                ToolOutcome::Continue
            }
            ToolEvent::Finish if self.choosing => {
                self.choosing = false;
                ToolOutcome::Continue
            }
            ToolEvent::Text(_) | ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Finish | ToolEvent::Cancel => ToolOutcome::Done,
        }
    }

    fn preview(&self, _conv: &Conventions, _cursor: Option<&Pick>, _out: &mut Preview) {}
}

/// A point where two lines meet when extended: pick each line (the segment
/// clicked; an arc counts as its circle), or type each as two points
/// (`101 102`, or coordinates `1000,2000 1010,2000`).
#[derive(Default)]
pub struct ApparentTool {
    first: Option<Segment>,
}

/// A line typed as two points (names or coordinates), separated by spaces.
fn typed_line(ctx: &ToolCtx<'_>, conv: &Conventions, text: &str) -> Option<Result<Segment, String>> {
    let parts: Vec<&str> = text.split([' ', ';']).filter(|t| !t.is_empty()).collect();
    let [a, b] = parts[..] else { return None };
    let point = |t: &str| -> Option<DVec3> {
        if let Some((_, p)) = ctx.doc.find_point_by_name(t) {
            return Some(p.position);
        }
        match PointInput::parse(t, conv) {
            Ok(Some(input)) => input.resolve(None).ok().map(|r| r.position),
            _ => None,
        }
    };
    Some(match (point(a), point(b)) {
        (Some(p), Some(q)) if p.truncate().distance(q.truncate()) > 1e-9 => Ok((p, q, 0.0)),
        (Some(_), Some(_)) => Err("The two points are the same".into()),
        _ => Err(format!("Not points: {a} {b}")),
    })
}

impl ApparentTool {
    fn take(&mut self, ctx: &mut ToolCtx<'_>, line: Segment, near: DVec2) -> ToolOutcome {
        let Some(first) = self.first.take() else {
            self.first = Some(line);
            return ToolOutcome::Continue;
        };
        let Some(p) = apparent_intersection(&first, &line, near) else {
            ctx.log
                .error("Those lines never meet (parallel, or circles apart)");
            return ToolOutcome::Continue;
        };
        let name = ctx.doc.next_point_name();
        let point = new_point(ctx, name.clone(), p, false, "");
        ctx.log.result(format!(
            "Point {name} at the intersection: {}",
            ctx.conventions().format_coord(p, false)
        ));
        ctx.execute(ChangeSet::new(format!("Intersection point {name}")).insert_entity(point));
        ToolOutcome::Continue
    }
}

impl Tool for ApparentTool {
    fn id(&self) -> &'static str {
        APPARENT_ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        if self.first.is_none() {
            "Intersection — pick the first line, or type it as two points (101 102):".into()
        } else {
            "Intersection — pick the second line, or type it as two points:".into()
        }
    }

    fn wants_point(&self) -> bool {
        true
    }

    fn claims_text(&self, _conv: &Conventions, text: &str) -> bool {
        text.split([' ', ';']).filter(|t| !t.is_empty()).count() == 2
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) => {
                let at = pick.point.truncate();
                match ctx.hovered.and_then(|id| segment_near(ctx.doc, id, at)) {
                    Some(seg) => self.take(ctx, seg, at),
                    None => {
                        ctx.log.warn("Click on a line (or type it as two points)");
                        ToolOutcome::Continue
                    }
                }
            }
            ToolEvent::Text(text) => {
                let conv = ctx.conventions().clone();
                match typed_line(ctx, &conv, text) {
                    Some(Ok(seg)) => self.take(ctx, seg, seg.1.truncate()),
                    Some(Err(e)) => {
                        ctx.log.error(e);
                        ToolOutcome::Continue
                    }
                    None => {
                        ctx.log.error("Type a line as two points: 101 102");
                        ToolOutcome::Continue
                    }
                }
            }
            ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Finish | ToolEvent::Cancel => ToolOutcome::Done,
        }
    }

    fn preview(&self, _conv: &Conventions, _cursor: Option<&Pick>, out: &mut Preview) {
        if let Some((a, b, _)) = self.first {
            out.rubber.push([a, b]);
        }
    }
}
