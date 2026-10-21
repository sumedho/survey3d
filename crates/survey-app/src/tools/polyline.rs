use egui_cad::snap::Pick;
use glam::DVec3;
use survey_core::Conventions;
use survey_core::arc::{Arc, end_direction, tangent_bulge};
use survey_core::point_input::ResolvedPoint;
use survey_model::{ChangeSet, Entity, Polyline};

use super::{Preview, Tool, ToolCtx, ToolEvent, ToolOutcome, resolved};

pub const ID: &str = "polyline";

/// Draws polylines. Enter or right-click finishes, `c` closes, `u` removes the last vertex.
/// `a` switches to arc segments (tangent to the previous segment; `s` gives a
/// second point for a three-point arc instead) and `l` back to straight lines.
/// After finishing, the tool is ready for the next polyline; Escape with no vertices exits.
#[derive(Default)]
pub struct PolylineTool {
    vertices: Vec<ResolvedPoint>,
    /// Bulge of each finished segment (one fewer than the vertices).
    bulges: Vec<f64>,
    arc_mode: bool,
    /// The next arc segment is a three-point arc (`s`).
    three_point: bool,
    /// Second point of a three-point arc segment, once picked.
    through: Option<ResolvedPoint>,
}

impl PolylineTool {
    /// Direction of travel at the last vertex, if there is a previous segment.
    fn tangent(&self) -> Option<glam::DVec2> {
        let n = self.vertices.len();
        (n >= 2).then(|| {
            end_direction(
                self.vertices[n - 2].position,
                self.vertices[n - 1].position,
                self.bulges[n - 2],
            )
        })
    }

    /// Whether the next arc segment needs a second point (no tangent to follow).
    fn needs_through(&self) -> bool {
        self.three_point || self.tangent().is_none()
    }

    /// Bulge of the next segment from the last vertex to `to`; `Err` when no arc fits.
    fn next_bulge(&self, to: DVec3) -> Result<f64, &'static str> {
        let Some(last) = self.vertices.last() else {
            return Ok(0.0);
        };
        if !self.arc_mode {
            return Ok(0.0);
        }
        if self.needs_through() {
            let m = self.through.ok_or("Pick the second point first")?;
            return Arc::bulge_through(last.position, m.position, to)
                .ok_or("The three points are in a straight line");
        }
        let tangent = self.tangent().ok_or("No previous segment")?;
        tangent_bulge(last.position.truncate(), tangent, to.truncate())
            .ok_or("That point is directly behind the tangent")
    }

    fn commit(&mut self, ctx: &mut ToolCtx<'_>, closing_bulge: Option<f64>) {
        let vertices: Vec<DVec3> = std::mem::take(&mut self.vertices)
            .into_iter()
            .map(|v| v.position)
            .collect();
        let mut bulges = std::mem::take(&mut self.bulges);
        self.through = None;
        self.three_point = false;
        if vertices.len() < 2 {
            return;
        }
        let closed = closing_bulge.is_some();
        bulges.push(closing_bulge.unwrap_or(0.0));
        if bulges.iter().all(|b| *b == 0.0) {
            bulges.clear();
        }
        let n = vertices.len();
        let line = Polyline {
            name: String::new(),
            vertices,
            bulges,
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

    fn close(&mut self, ctx: &mut ToolCtx<'_>) {
        let Some(first) = self.vertices.first().map(|v| v.position) else {
            return;
        };
        let has_arcs = self.bulges.iter().any(|b| *b != 0.0) || self.arc_mode;
        if self.vertices.len() < 3 && !has_arcs {
            ctx.log.warn("Need at least 3 points to close");
            return;
        }
        let bulge = if self.arc_mode && !self.needs_through() {
            self.next_bulge(first)
        } else {
            Ok(0.0)
        };
        match bulge {
            Ok(b) => self.commit(ctx, Some(b)),
            Err(e) => ctx.log.error(e),
        }
    }
}

impl Tool for PolylineTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        let n = self.vertices.len();
        if n == 0 {
            return "Polyline — first point:".into();
        }
        if self.arc_mode {
            if self.needs_through() {
                return match self.through {
                    None => "Arc — second point (on the arc) [Line/Undo]:".into(),
                    Some(_) => "Arc — end point:".into(),
                };
            }
            return "Arc — end point [Line/Second pt/Close/Undo] (Enter to finish):".into();
        }
        match n {
            1 => "Next point [Arc/Undo]:".into(),
            2 => "Next point [Arc/Undo] (Enter to finish):".into(),
            _ => "Next point [Arc/Close/Undo] (Enter to finish):".into(),
        }
    }

    fn base_point(&self) -> Option<ResolvedPoint> {
        self.vertices.last().copied()
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) => {
                let p = resolved(&pick);
                if self.vertices.is_empty() {
                    self.vertices.push(p);
                } else if self.arc_mode && self.needs_through() && self.through.is_none() {
                    self.through = Some(p);
                } else {
                    match self.next_bulge(p.position) {
                        Ok(b) => {
                            self.bulges.push(b);
                            self.vertices.push(p);
                            self.through = None;
                            self.three_point = false;
                        }
                        Err(e) => ctx.log.error(e),
                    }
                }
                ToolOutcome::Continue
            }
            ToolEvent::Text(text) => {
                match text.trim().to_ascii_lowercase().as_str() {
                    "c" | "close" => self.close(ctx),
                    "a" | "arc" => {
                        self.arc_mode = true;
                        self.through = None;
                    }
                    "l" | "line" => {
                        self.arc_mode = false;
                        self.three_point = false;
                        self.through = None;
                    }
                    "s" | "second" if self.arc_mode => {
                        self.three_point = true;
                        self.through = None;
                    }
                    "u" | "undo" => {
                        if self.through.take().is_none() {
                            self.vertices.pop();
                            self.bulges.truncate(self.vertices.len().saturating_sub(1));
                        }
                    }
                    other => ctx.log.error(format!("Unknown option '{other}'")),
                }
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Finish => {
                self.commit(ctx, None);
                ToolOutcome::Continue
            }
            ToolEvent::Cancel => {
                if self.vertices.is_empty() {
                    ToolOutcome::Done
                } else {
                    self.commit(ctx, None);
                    ToolOutcome::Continue
                }
            }
        }
    }

    fn preview(&self, _conv: &Conventions, cursor: Option<&Pick>, out: &mut Preview) {
        if self.vertices.len() >= 2 {
            let mut bulges = self.bulges.clone();
            bulges.push(0.0);
            let line = Polyline {
                name: String::new(),
                vertices: self.vertices.iter().map(|v| v.position).collect(),
                bulges,
                closed: false,
                code: String::new(),
                layer: survey_model::LayerId(0),
            };
            out.polylines.extend(line.tessellate(1e-3));
        }
        let (Some(last), Some(c)) = (self.vertices.last(), cursor) else {
            return;
        };
        if let Some(m) = self.through {
            out.points.push(m.position);
        }
        let arc = self
            .arc_mode
            .then(|| self.next_bulge(c.point).ok())
            .flatten()
            .and_then(|b| Arc::from_bulge(last.position, c.point, b));
        match arc {
            Some(arc) => out.polylines.push(arc.tessellate(arc.radius * 1e-3)),
            None => out.rubber.push([self.through.unwrap_or(*last).position, c.point]),
        }
    }
}
