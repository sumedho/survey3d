//! Building dimensions by right-angle legs (`bldg`): pick two corners along
//! the first wall, then type each wall's taped length, turning 90° right
//! (`5.20` or `r 5.20`) or left (`-5.20` or `l 5.20`) from the wall before,
//! or carrying straight on (`s 1.10`). `c` (or Enter) closes the outline
//! back to the first corner and reports the misclosure.

use egui_cad::snap::Pick;
use glam::{DVec2, DVec3};
use survey_core::Conventions;
use survey_core::arc::polyline_metrics;
use survey_core::point_input::ResolvedPoint;
use survey_model::{ChangeSet, Entity, Polyline};

use super::cogo::new_point;
use super::{Preview, Tool, ToolCtx, ToolEvent, ToolOutcome, resolved};

pub const ID: &str = "building";

#[derive(Default)]
pub struct BuildingTool {
    first: Option<ResolvedPoint>,
    /// The corners so far (the first two from picks, then from legs).
    corners: Vec<DVec3>,
    /// Also create a point at each corner.
    points: bool,
}

/// A typed leg: the turn from the wall before (+1 right, −1 left, 0
/// straight on) and the length.
fn leg(conv: &Conventions, text: &str) -> Option<(f64, f64)> {
    let t = text.trim().to_ascii_lowercase();
    let (turn, rest) = match t.split_at_checked(1) {
        Some(("r", rest)) => (1.0, rest),
        Some(("l", rest)) => (-1.0, rest),
        Some(("s", rest)) => (0.0, rest),
        _ => (
            if t.starts_with('-') { -1.0 } else { 1.0 },
            t.trim_start_matches(['-', '+']),
        ),
    };
    let length = conv.parse_length(rest.trim()).ok().filter(|l| *l > 0.0)?;
    Some((turn, length))
}

impl BuildingTool {
    fn direction(&self) -> Option<DVec2> {
        let n = self.corners.len();
        (n >= 2).then(|| {
            (self.corners[n - 1] - self.corners[n - 2])
                .truncate()
                .normalize_or_zero()
        })
    }

    fn add_leg(&mut self, ctx: &mut ToolCtx<'_>, turn: f64, length: f64) {
        let (Some(dir), Some(&last)) = (self.direction(), self.corners.last()) else {
            return;
        };
        // Right of the direction of travel is (y, −x).
        let right = DVec2::new(dir.y, -dir.x);
        let next_dir = if turn > 0.0 {
            right
        } else if turn < 0.0 {
            -right
        } else {
            dir
        };
        let p = last + (next_dir * length).extend(0.0);
        self.corners.push(p);
        let conv = ctx.conventions();
        ctx.log.result(format!(
            "Corner {}: {} {}  → {}",
            self.corners.len(),
            match turn {
                t if t > 0.0 => "right",
                t if t < 0.0 => "left",
                _ => "straight on",
            },
            conv.format_length(length),
            conv.format_coord(p, false)
        ));
    }

    fn close(&mut self, ctx: &mut ToolCtx<'_>) -> ToolOutcome {
        let n = self.corners.len();
        if n < 3 {
            ctx.log.warn("A building needs at least three corners");
            return ToolOutcome::Continue;
        }
        let conv = ctx.conventions().clone();
        let (start, last) = (self.corners[0], self.corners[n - 1]);
        let mis = (start - last).truncate();
        // The last wall should end on the first corner: the misclosure along
        // the last wall and square to it.
        let dir = self.direction().unwrap_or(DVec2::X);
        let (along, across) = (mis.dot(dir), mis.dot(DVec2::new(dir.y, -dir.x)));
        // A last wall ending at (or near) the first corner closes on it: its
        // end is the first corner, the misclosure reported.
        let mut corners = self.corners.clone();
        let last_wall = (last - self.corners[n - 2]).truncate().length();
        let closes = mis.length() < 0.5 * last_wall;
        if closes {
            corners.pop();
        }
        if corners.len() < 3 {
            ctx.log.warn("A building needs at least three corners");
            return ToolOutcome::Continue;
        }
        let (perimeter, _, area) = polyline_metrics(&corners, &[], true);
        if area.abs() < 1e-6 {
            ctx.log
                .error("The walls enclose no area: check the turns (right is positive, left negative)");
            return ToolOutcome::Continue;
        }
        let first_height = self.first.is_some_and(|f| f.height_known);
        ctx.log.result(format!(
            "Building closed: {} corners, misclose {} (along the last wall {}, square to it {}), perimeter {}, area {}",
            corners.len(),
            conv.format_length(mis.length()),
            conv.format_length(along),
            conv.format_length(across),
            conv.format_length(perimeter),
            conv.format_area(area.abs()),
        ));
        let mut cs = ChangeSet::new(format!("Building by right angles ({} corners)", corners.len()))
            .insert_entity(Entity::Polyline(Polyline {
                name: "Building".into(),
                vertices: corners.clone(),
                bulges: Vec::new(),
                closed: true,
                code: String::new(),
                layer: ctx.doc.current_layer(),
                heights_known: first_height,
            }));
        if self.points {
            let mut names = ctx.doc.point_names();
            for c in &corners {
                cs.push_insert_entity(new_point(
                    ctx,
                    names.next().unwrap_or_default(),
                    *c,
                    first_height,
                    "",
                ));
            }
        }
        ctx.execute(cs);
        if !closes {
            ctx.log.warn(format!(
                "The last wall ends {} from the first corner, so a closing side was added: check for a missed wall or a wrong turn (undo removes the building)",
                conv.format_length(mis.length())
            ));
        }
        self.first = None;
        self.corners.clear();
        ToolOutcome::Continue
    }
}

impl Tool for BuildingTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        let points = if self.points { " [points on]" } else { "" };
        match self.corners.len() {
            0 => format!("Building{points} — first corner:"),
            1 => format!("Building{points} — second corner, along the first wall:"),
            n => format!(
                "Building{points} — wall {}: length turning right (5.2 or r 5.2), left (-5.2 or l 5.2), straight on (s 5.2) / u undo / p points / c close:",
                n
            ),
        }
    }

    fn wants_point(&self) -> bool {
        self.corners.len() < 2
    }

    fn claims_text(&self, conv: &Conventions, text: &str) -> bool {
        self.corners.len() >= 2 && leg(conv, text).is_some()
    }

    fn base_point(&self) -> Option<ResolvedPoint> {
        self.corners.last().map(|p| ResolvedPoint {
            position: *p,
            height_known: self.first.is_some_and(|f| f.height_known),
        })
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) if self.corners.len() < 2 => {
                let p = resolved(&pick);
                match self.corners.first() {
                    None => {
                        self.first = Some(p);
                        self.corners.push(p.position);
                    }
                    Some(a) if a.truncate().distance(p.position.truncate()) < 1e-6 => {
                        ctx.log.warn("The second corner must be away from the first");
                    }
                    // Corners keep the first corner's height.
                    Some(a) => {
                        let z = a.z;
                        let len = a.truncate().distance(p.position.truncate());
                        self.corners.push(p.position.truncate().extend(z));
                        let conv = ctx.conventions();
                        ctx.log.info(format!("First wall {}", conv.format_length(len)));
                    }
                }
                ToolOutcome::Continue
            }
            ToolEvent::Point(_) | ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Text(text) => {
                let conv = ctx.conventions().clone();
                match text.trim().to_ascii_lowercase().as_str() {
                    "u" | "undo" => {
                        if self.corners.len() > 2 {
                            self.corners.pop();
                        } else {
                            self.corners.clear();
                            self.first = None;
                        }
                        ToolOutcome::Continue
                    }
                    "p" => {
                        self.points = !self.points;
                        ToolOutcome::Continue
                    }
                    "c" | "close" => self.close(ctx),
                    _ => {
                        match leg(&conv, text) {
                            Some((turn, length)) if self.corners.len() >= 2 => {
                                self.add_leg(ctx, turn, length)
                            }
                            Some(_) => ctx.log.warn("Pick the first two corners first"),
                            None => ctx.log.error(format!(
                                "'{text}' is not a wall length (5.2, -5.2, r 5.2, l 5.2, s 5.2)"
                            )),
                        }
                        ToolOutcome::Continue
                    }
                }
            }
            ToolEvent::Finish if self.corners.len() >= 3 => self.close(ctx),
            ToolEvent::Finish | ToolEvent::Cancel => {
                if self.corners.is_empty() {
                    ToolOutcome::Done
                } else {
                    self.corners.clear();
                    self.first = None;
                    ToolOutcome::Continue
                }
            }
        }
    }

    fn preview(&self, conv: &Conventions, cursor: Option<&Pick>, out: &mut Preview) {
        if self.corners.len() >= 2 {
            out.polylines.push(self.corners.clone());
            // The closing side, dashed.
            if let (Some(&a), Some(&z)) = (self.corners.first(), self.corners.last()) {
                out.rubber.push([z, a]);
                out.labels.push((
                    z,
                    DVec2::new(14.0, 14.0),
                    format!(
                        "to close {}",
                        conv.format_length(a.truncate().distance(z.truncate()))
                    ),
                ));
            }
        } else if let (Some(&a), Some(c)) = (self.corners.first(), cursor) {
            out.rubber.push([a, c.point]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_legs() {
        let c = Conventions::default();
        assert_eq!(leg(&c, "5.2"), Some((1.0, 5.2)));
        assert_eq!(leg(&c, "-5.2"), Some((-1.0, 5.2)));
        assert_eq!(leg(&c, "L 3"), Some((-1.0, 3.0)));
        assert_eq!(leg(&c, "r3"), Some((1.0, 3.0)));
        assert_eq!(leg(&c, "s 1.5"), Some((0.0, 1.5)));
        assert_eq!(leg(&c, "c"), None);
        assert_eq!(leg(&c, "0"), None);
    }
}
