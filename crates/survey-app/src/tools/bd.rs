//! Bearing & distance entry (`bd`): walk a traverse or a lot boundary from a
//! start point by typing `bearing distance [dZ]` for each leg. Each leg creates
//! a point and moves on to it; the legs become one polyline. `c` closes the
//! figure on its start and reports the misclosure, precision and area (the
//! figure is not adjusted). Sideshots (`s`) take a point without moving on.

use egui_cad::snap::Pick;
use glam::DVec3;
use survey_core::point_input::ResolvedPoint;
use survey_core::{Bearing, Conventions};
use survey_model::{ChangeSet, Entity, Polyline};

use super::cogo::{fields, new_point};
use super::{Preview, Tool, ToolCtx, ToolEvent, ToolOutcome, resolved};

pub const ID: &str = "bd";

/// A point taken from the current point.
struct Leg {
    /// The point created for it.
    name: String,
    position: DVec3,
    height_known: bool,
    /// Taken without moving on (not part of the figure).
    sideshot: bool,
    distance: f64,
}

#[derive(Default)]
pub struct BdTool {
    start: Option<ResolvedPoint>,
    legs: Vec<Leg>,
    sideshot: bool,
    code: String,
}

impl BdTool {
    /// The point legs are taken from: the last traverse leg's end, or the start.
    fn current(&self) -> Option<ResolvedPoint> {
        self.legs
            .iter()
            .rev()
            .find(|l| !l.sideshot)
            .map(|l| ResolvedPoint {
                position: l.position,
                height_known: l.height_known,
            })
            .or(self.start)
    }

    /// The figure's vertices: the start, then each traverse leg's end.
    fn figure(&self) -> Vec<DVec3> {
        let mut v: Vec<DVec3> = self.start.iter().map(|s| s.position).collect();
        v.extend(self.legs.iter().filter(|l| !l.sideshot).map(|l| l.position));
        v
    }

    fn traverse_length(&self) -> f64 {
        self.legs.iter().filter(|l| !l.sideshot).map(|l| l.distance).sum()
    }

    /// Sets the start point, creating a point there unless one exists.
    fn begin(&mut self, ctx: &mut ToolCtx<'_>, pick: &Pick) {
        let start = resolved(pick);
        let existing = ctx
            .doc
            .entities()
            .any(|(_, e)| matches!(e, Entity::Point(p) if p.position.truncate().distance(start.position.truncate()) < 1e-6));
        if !existing {
            let name = ctx.doc.next_point_name();
            let point = new_point(ctx, name.clone(), start.position, start.height_known, &self.code);
            ctx.execute(ChangeSet::new(format!("Bearing & distance start {name}")).insert_entity(point));
            ctx.log.info(format!("Start point {name}"));
        }
        self.start = Some(start);
    }

    /// One `bearing distance [dZ] [code]` observation from the current point.
    fn leg(&mut self, ctx: &mut ToolCtx<'_>, text: &str) {
        let Some(from) = self.current() else { return };
        let conv = ctx.conventions();
        let parts = fields(text);
        if !(2..=4).contains(&parts.len()) {
            ctx.log
                .error("Enter: bearing distance[ dZ][ code] — e.g. 45.3020 25.31 or N45.3020E,25.31");
            return;
        }
        let parsed = (|| {
            let bearing = conv.parse_bearing(parts[0])?;
            let dist = conv.parse_length(parts[1])?;
            Ok::<_, survey_core::ParseError>((bearing, dist))
        })();
        let (bearing, dist) = match parsed {
            Ok(v) => v,
            Err(e) => {
                ctx.log.error(e.to_string());
                return;
            }
        };
        // A third field is dZ when numeric, else a code (as is a fourth).
        let mut dz = None;
        for part in &parts[2..] {
            match conv.parse_length(part) {
                Ok(v) if dz.is_none() => dz = Some(v),
                _ => self.code = part.to_string(),
            }
        }
        let plan = from.position.truncate() + bearing.unit_vector() * dist;
        let height_known = from.height_known && dz.is_some();
        let position = plan.extend(from.position.z + dz.unwrap_or(0.0));
        let name = ctx.doc.next_point_name();
        let point = new_point(ctx, name.clone(), position, height_known, &self.code);
        ctx.execute(ChangeSet::new(format!("Bearing & distance point {name}")).insert_entity(point));
        let conv = ctx.conventions();
        ctx.log.result(format!(
            "{} {name}: brg {}  dist {}  → {}",
            if self.sideshot { "Sideshot" } else { "Point" },
            conv.format_bearing(bearing),
            conv.format_length(dist),
            conv.format_coord(position, height_known)
        ));
        self.legs.push(Leg {
            name,
            position,
            height_known,
            sideshot: self.sideshot,
            distance: dist,
        });
    }

    /// Removes the last leg and its point.
    fn undo(&mut self, ctx: &mut ToolCtx<'_>) {
        let Some(leg) = self.legs.pop() else {
            ctx.log.warn("No leg to undo");
            return;
        };
        if let Some((id, _)) = ctx.doc.find_point_by_name(&leg.name) {
            ctx.execute(ChangeSet::new(format!("Undo leg to {}", leg.name)).delete_entity(id));
        }
        ctx.log.info(format!("Removed {}", leg.name));
    }

    /// The last leg returned to the start: report the misclosure, replace its
    /// end point by the start, and draw the closed figure.
    fn close(&mut self, ctx: &mut ToolCtx<'_>) {
        let traverse = self.legs.iter().filter(|l| !l.sideshot).count();
        let Some(start) = self.start else { return };
        if traverse < 3 {
            ctx.log
                .warn("Close needs at least three legs (the last one back to the start)");
            return;
        }
        let Some(last) = self.legs.iter().rposition(|l| !l.sideshot) else {
            return;
        };
        let end = self.legs[last].position;
        let perimeter = self.traverse_length();
        let mis = (end - start.position).truncate();
        // The figure without the closing leg's end (it is the start corner).
        let mut vertices = self.figure();
        vertices.pop();
        let area = polygon_area(&vertices);
        let conv = ctx.conventions().clone();
        let misclose = mis.length();
        let precision = if misclose > 1e-9 {
            format!("1:{:.0}", perimeter / misclose)
        } else {
            "exact".to_string()
        };
        let direction =
            Bearing::from_delta(mis.x, mis.y).map_or_else(|| "—".to_string(), |b| conv.format_bearing(b));
        ctx.log.result(format!(
            "Closed {traverse} legs: misclose {} brg {direction} ({precision})  perimeter {}  area {}",
            conv.format_length(misclose),
            conv.format_length(perimeter),
            conv.format_area(area),
        ));
        let name = self.legs[last].name.clone();
        let mut cs = ChangeSet::new(format!("Close bearing & distance figure ({} legs)", traverse));
        if let Some((id, _)) = ctx.doc.find_point_by_name(&name) {
            cs = cs.delete_entity(id);
        }
        cs = cs.insert_entity(self.polyline(ctx, vertices, true));
        ctx.execute(cs);
        ctx.log
            .info("Closed boundary drawn — select it and run mkparcel to make it a parcel");
        self.reset();
    }

    fn polyline(&self, ctx: &ToolCtx<'_>, vertices: Vec<DVec3>, closed: bool) -> Entity {
        let heights_known = self.start.is_some_and(|s| s.height_known)
            && self.legs.iter().filter(|l| !l.sideshot).all(|l| l.height_known);
        Entity::Polyline(Polyline {
            name: String::new(),
            vertices,
            bulges: Vec::new(),
            closed,
            code: self.code.clone(),
            layer: ctx.doc.current_layer(),
            heights_known,
        })
    }

    /// Ends the current run, drawing its legs as an open polyline.
    fn finish_run(&mut self, ctx: &mut ToolCtx<'_>) {
        let vertices = self.figure();
        if vertices.len() >= 2 {
            let line = self.polyline(ctx, vertices, false);
            ctx.execute(ChangeSet::new("Bearing & distance linework").insert_entity(line));
        }
        self.reset();
    }

    fn reset(&mut self) {
        self.start = None;
        self.legs.clear();
        self.sideshot = false;
    }
}

/// Plan area of a polygon (shoelace).
fn polygon_area(v: &[DVec3]) -> f64 {
    let n = v.len();
    (0..n)
        .map(|i| {
            let (a, b) = (v[i].truncate(), v[(i + 1) % n].truncate());
            a.perp_dot(b)
        })
        .sum::<f64>()
        .abs()
        * 0.5
}

/// Option keywords (exact, so codes in capitals are not taken for them).
fn is_option(text: &str) -> bool {
    matches!(text.trim(), "s" | "u" | "c")
}

impl Tool for BdTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn prompt(&self, conv: &Conventions) -> String {
        let code = if self.code.is_empty() {
            String::new()
        } else {
            format!(" ({})", self.code)
        };
        let Some(start) = self.start else {
            return format!("Bearing & distance{code} — start point:");
        };
        let mode = if self.sideshot { " sideshots" } else { "" };
        let legs = self.legs.iter().filter(|l| !l.sideshot).count();
        let status = match self.current() {
            Some(cur) if legs > 0 => {
                let back = (cur.position - start.position).truncate().length();
                format!(
                    " [{legs} legs, {}; {} from start]",
                    conv.format_length(self.traverse_length()),
                    conv.format_length(back)
                )
            }
            _ => String::new(),
        };
        format!("Bearing & distance{code}{mode}{status} — bearing distance[ dZ][ code] / s / u / c:")
    }

    fn base_point(&self) -> Option<ResolvedPoint> {
        self.current()
    }

    fn takes_codes(&self) -> bool {
        true
    }

    fn claims_text(&self, conv: &Conventions, text: &str) -> bool {
        if self.start.is_none() {
            return false;
        }
        if is_option(text) {
            return true;
        }
        let parts = fields(text);
        parts.len() >= 2 && conv.parse_bearing(parts[0]).is_ok() && conv.parse_length(parts[1]).is_ok()
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) => {
                // A point after legs starts a new run from it.
                if self.start.is_some() {
                    self.finish_run(ctx);
                }
                self.begin(ctx, &pick);
                ToolOutcome::Continue
            }
            ToolEvent::Text(text) => {
                match text.trim() {
                    "s" if self.start.is_some() => {
                        self.sideshot = !self.sideshot;
                        ctx.log.info(if self.sideshot {
                            "Sideshots: points are taken without moving on"
                        } else {
                            "Traverse: each leg moves on to its point"
                        });
                    }
                    "u" if self.start.is_some() => self.undo(ctx),
                    "c" if self.start.is_some() => self.close(ctx),
                    t if self.start.is_some() && fields(t).len() >= 2 => self.leg(ctx, t),
                    t => {
                        self.code = if t == "-" { String::new() } else { t.to_string() };
                        ctx.log.info(if self.code.is_empty() {
                            "Code cleared".to_string()
                        } else {
                            format!("Code set to {}", self.code)
                        });
                    }
                }
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Finish => {
                if self.start.is_some() {
                    self.finish_run(ctx);
                    ToolOutcome::Continue
                } else {
                    ToolOutcome::Done
                }
            }
            ToolEvent::Cancel => {
                self.finish_run(ctx);
                ToolOutcome::Done
            }
        }
    }

    fn preview(&self, _conv: &Conventions, _cursor: Option<&Pick>, out: &mut Preview) {
        let figure = self.figure();
        if figure.len() >= 2 {
            out.polylines.push(figure.clone());
        }
        if let Some(cur) = self.current() {
            out.points.push(cur.position);
            // Each sideshot as a ray from the point it was taken at.
            let mut at = self.start.map(|s| s.position);
            for l in &self.legs {
                if l.sideshot {
                    if let Some(a) = at {
                        out.rubber.push([a, l.position]);
                    }
                } else {
                    at = Some(l.position);
                }
            }
            // Back to the start, while the figure is open.
            if let Some(start) = self.start
                && figure.len() >= 3
            {
                out.rubber.push([cur.position, start.position]);
            }
        }
    }
}
