//! Construction helpers: fillet a polyline corner, divide a polyline into
//! parts or intervals, and points along a line.

use egui_cad::snap::Pick;
use glam::{DVec2, DVec3};
use survey_core::arc::polyline_metrics;
use survey_core::cogo::{fillet, point_at_station};
use survey_core::point_input::ResolvedPoint;
use survey_core::{Bearing, Conventions};
use survey_model::{ChangeSet, Entity, EntityId, Polyline};

use super::cogo::{fields, is_numeric, new_point};
use super::{Preview, Tool, ToolCtx, ToolEvent, ToolOutcome, resolved};

pub const FILLET_ID: &str = "fillet";
pub const DIVIDE_ID: &str = "divide";
pub const ALONG_ID: &str = "along";

// ---------------------------------------------------------------- fillet

/// Rounds polyline corners: type the radius, then click near corners.
#[derive(Default)]
pub struct FilletTool {
    radius: Option<f64>,
}

/// The polyline with corner `i` replaced by a fillet, or why it cannot be.
fn filleted(l: &Polyline, i: usize, radius: f64) -> Result<Polyline, &'static str> {
    let n = l.vertices.len();
    let interior = if l.closed { n >= 3 } else { i > 0 && i + 1 < n };
    if !interior {
        return Err("Pick an interior corner");
    }
    let (ip, inext) = ((i + n - 1) % n, (i + 1) % n);
    let bulges = l.padded_bulges();
    if bulges[ip] != 0.0 || bulges[i] != 0.0 {
        return Err("Both sides of the corner must be straight");
    }
    let (p, c, q) = (l.vertices[ip], l.vertices[i], l.vertices[inext]);
    let f = fillet(p.truncate(), c.truncate(), q.truncate(), radius)
        .ok_or("The lines are too short for that radius, or there is no corner")?;
    // Heights interpolated along each line.
    let along = |a: DVec3, b: DVec3, t: f64| a + (b - a) * t;
    let ta = 1.0 - f.tangent / (c - p).truncate().length();
    let tb = f.tangent / (q - c).truncate().length();
    let start = along(p, c, ta);
    let end = along(c, q, tb);
    let mut out = l.clone();
    out.vertices.splice(i..=i, [start, end]);
    let mut b = bulges;
    b.splice(i..=i, [f.bulge, 0.0]);
    out.bulges = b;
    Ok(out)
}

/// Index of the polyline vertex nearest `p` (plan).
fn nearest_vertex(l: &Polyline, p: DVec2) -> Option<usize> {
    l.vertices
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.truncate().distance(p).total_cmp(&b.1.truncate().distance(p)))
        .map(|(i, _)| i)
}

impl Tool for FilletTool {
    fn id(&self) -> &'static str {
        FILLET_ID
    }

    fn prompt(&self, conv: &Conventions) -> String {
        match self.radius {
            None => "Fillet — radius:".into(),
            Some(r) => format!(
                "Fillet (R {}) — click near a polyline corner, or type a new radius:",
                conv.format_length(r)
            ),
        }
    }

    fn wants_point(&self) -> bool {
        false
    }

    fn claims_text(&self, conv: &Conventions, text: &str) -> bool {
        conv.parse_length(text).is_ok()
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Text(text) => {
                match ctx.conventions().parse_length(text) {
                    Ok(r) if r > 0.0 => self.radius = Some(r),
                    Ok(_) => ctx.log.error("The radius must be greater than zero"),
                    Err(e) => ctx.log.error(e.to_string()),
                }
                ToolOutcome::Continue
            }
            ToolEvent::Point(pick) => {
                let Some(radius) = self.radius else {
                    ctx.log.warn("Type the fillet radius first");
                    return ToolOutcome::Continue;
                };
                let Some((id, l)) = hovered_polyline(ctx) else {
                    return ToolOutcome::Continue;
                };
                let Some(i) = nearest_vertex(&l, pick.point.truncate()) else {
                    return ToolOutcome::Continue;
                };
                match filleted(&l, i, radius) {
                    Ok(new) => {
                        ctx.log.result(format!(
                            "Fillet R {} at corner {}",
                            ctx.conventions().format_length(radius),
                            i + 1
                        ));
                        ctx.execute(ChangeSet::new("Fillet corner").update_entity(id, Entity::Polyline(new)));
                    }
                    Err(e) => ctx.log.error(e),
                }
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) | ToolEvent::Finish | ToolEvent::Cancel => ToolOutcome::Done,
        }
    }

    fn preview(&self, _conv: &Conventions, _cursor: Option<&Pick>, _out: &mut Preview) {}
}

fn hovered_polyline(ctx: &mut ToolCtx<'_>) -> Option<(EntityId, Polyline)> {
    let id = ctx.hovered?;
    match ctx.doc.entity(id) {
        Some(Entity::Polyline(l)) if ctx.doc.is_entity_editable(id) => Some((id, l.clone())),
        Some(Entity::Polyline(_)) => {
            ctx.log.warn("That polyline is on a locked layer");
            None
        }
        Some(_) => {
            ctx.log.warn("Pick a polyline");
            None
        }
        None => None,
    }
}

// ---------------------------------------------------------------- divide

/// Points along a polyline: pick it, then type a number of equal parts (`5`)
/// or an interval (`@20`).
#[derive(Default)]
pub struct DivideTool {
    line: Option<Polyline>,
}

impl Tool for DivideTool {
    fn id(&self) -> &'static str {
        DIVIDE_ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        match self.line {
            None => "Divide — pick a polyline:".into(),
            Some(_) => "Divide — number of equal parts, or @interval (e.g. 5 or @20):".into(),
        }
    }

    fn wants_point(&self) -> bool {
        false
    }

    fn claims_text(&self, _conv: &Conventions, text: &str) -> bool {
        self.line.is_some() && (is_numeric(text) || text.trim().starts_with('@'))
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(_) if self.line.is_none() => {
                if let Some((_, l)) = hovered_polyline(ctx) {
                    let (len, _, _) = polyline_metrics(&l.vertices, &l.bulges, l.closed);
                    ctx.log.info(format!(
                        "Polyline length {}",
                        ctx.conventions().format_length_with_unit(len)
                    ));
                    self.line = Some(l);
                }
                ToolOutcome::Continue
            }
            ToolEvent::Text(text) => {
                let Some(l) = self.line.clone() else {
                    return ToolOutcome::Continue;
                };
                let conv = ctx.conventions().clone();
                let (len, _, _) = polyline_metrics(&l.vertices, &l.bulges, l.closed);
                let t = text.trim();
                let chainages: Result<Vec<f64>, String> = if let Some(interval) = t.strip_prefix('@') {
                    conv.parse_length(interval.trim())
                        .map_err(|e| e.to_string())
                        .and_then(|d| {
                            if d <= 0.0 {
                                return Err("The interval must be greater than zero".into());
                            }
                            let n = (len / d).floor() as usize;
                            Ok((1..=n)
                                .map(|k| k as f64 * d)
                                .filter(|c| *c < len - 1e-9)
                                .collect())
                        })
                } else {
                    t.parse::<usize>()
                        .map_err(|_| format!("'{t}' is not a whole number of parts"))
                        .and_then(|n| {
                            if n < 2 {
                                return Err("Divide into at least two parts".into());
                            }
                            Ok((1..n).map(|k| len * k as f64 / n as f64).collect())
                        })
                };
                match chainages {
                    Ok(chs) if chs.is_empty() => ctx
                        .log
                        .warn("No points: the interval is longer than the polyline"),
                    Ok(chs) => {
                        let heights = l.vertices.iter().any(|v| v.z != 0.0);
                        let mut names = ctx.doc.point_names();
                        let mut cs = ChangeSet::new(format!("Divide polyline ({} points)", chs.len()));
                        for ch in &chs {
                            if let Some(p) = point_at_station(&l.vertices, &l.bulges, l.closed, *ch, 0.0) {
                                let name = names.next().unwrap_or_default();
                                cs.push_insert_entity(new_point(ctx, name, p, heights, "DIV"));
                            }
                        }
                        ctx.log.result(format!("Divide: created {} points", chs.len()));
                        ctx.execute(cs);
                        self.line = None;
                    }
                    Err(e) => ctx.log.error(e),
                }
                ToolOutcome::Continue
            }
            ToolEvent::Point(_) | ToolEvent::Region(_) | ToolEvent::Finish => ToolOutcome::Continue,
            ToolEvent::Cancel => {
                if self.line.take().is_some() {
                    ToolOutcome::Continue
                } else {
                    ToolOutcome::Done
                }
            }
        }
    }
}

// ---------------------------------------------------------------- along

/// Points along a line: pick the start and a point giving the direction, then
/// type `distance[, offset]` (offsets positive right; negative distances go back).
#[derive(Default)]
pub struct AlongTool {
    from: Option<ResolvedPoint>,
    to: Option<ResolvedPoint>,
}

impl AlongTool {
    fn place(&self, distance: f64, offset: f64) -> Option<(DVec3, bool)> {
        let (a, b) = (self.from?, self.to?);
        let d = (b.position - a.position).truncate();
        let len = d.length();
        if len <= 0.0 {
            return None;
        }
        let u = d / len;
        let right = DVec2::new(u.y, -u.x);
        let plan = a.position.truncate() + u * distance + right * offset;
        let heights = a.height_known && b.height_known;
        let z = if heights {
            a.position.z + (b.position.z - a.position.z) * distance / len
        } else {
            0.0
        };
        Some((plan.extend(z), heights))
    }
}

impl Tool for AlongTool {
    fn id(&self) -> &'static str {
        ALONG_ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        match (self.from, self.to) {
            (None, _) => "Along — start point:".into(),
            (Some(_), None) => "Along — point giving the direction:".into(),
            _ => "Along — distance[, offset] (Enter to finish):".into(),
        }
    }

    fn base_point(&self) -> Option<ResolvedPoint> {
        self.from
    }

    fn claims_text(&self, _conv: &Conventions, text: &str) -> bool {
        self.to.is_some() && is_numeric(text)
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) => {
                let p = resolved(&pick);
                match (self.from, self.to) {
                    (None, _) => self.from = Some(p),
                    (Some(a), None) => {
                        if (p.position - a.position).truncate().length() > 0.0 {
                            self.to = Some(p);
                            let d = (p.position - a.position).truncate();
                            if let Some(b) = Bearing::from_delta(d.x, d.y) {
                                let conv = ctx.conventions();
                                ctx.log.info(format!(
                                    "Line {}  {}",
                                    conv.format_bearing(b),
                                    conv.format_length_with_unit(d.length())
                                ));
                            }
                        } else {
                            ctx.log.warn("Pick a different point for the direction");
                        }
                    }
                    _ => ctx.log.warn("Type distance[, offset]"),
                }
                ToolOutcome::Continue
            }
            ToolEvent::Text(text) => {
                let conv = ctx.conventions().clone();
                let parts = fields(text);
                let parsed = (|| {
                    let d = conv.parse_length(parts.first().copied().unwrap_or(""))?;
                    let o = parts.get(1).map(|t| conv.parse_length(t)).transpose()?;
                    Ok::<_, survey_core::ParseError>((d, o.unwrap_or(0.0)))
                })();
                match parsed.map(|(d, o)| (d, o, self.place(d, o))) {
                    Ok((d, o, Some((p, heights)))) => {
                        let name = ctx.doc.next_point_name();
                        ctx.log.result(format!(
                            "Point {name}: {} along, {} offset → {}",
                            conv.format_length(d),
                            conv.format_length(o),
                            conv.format_coord(p, heights)
                        ));
                        let point = new_point(ctx, name.clone(), p, heights, "");
                        ctx.execute(ChangeSet::new(format!("Point {name} along line")).insert_entity(point));
                    }
                    Ok((_, _, None)) => {}
                    Err(e) => ctx.log.error(e.to_string()),
                }
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Finish | ToolEvent::Cancel => {
                if self.from.is_some() {
                    self.from = None;
                    self.to = None;
                    ToolOutcome::Continue
                } else {
                    ToolOutcome::Done
                }
            }
        }
    }

    fn preview(&self, _conv: &Conventions, cursor: Option<&Pick>, out: &mut Preview) {
        match (self.from, self.to, cursor) {
            (Some(a), None, Some(c)) => out.rubber.push([a.position, c.point]),
            (Some(a), Some(b), _) => {
                out.rubber.push([a.position, b.position]);
                out.points.push(a.position);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use survey_model::LayerId;

    fn line(points: &[(f64, f64)], closed: bool) -> Polyline {
        Polyline {
            name: String::new(),
            vertices: points.iter().map(|(x, y)| DVec3::new(*x, *y, 0.0)).collect(),
            bulges: Vec::new(),
            closed,
            code: String::new(),
            layer: LayerId(0),
            heights_known: true,
        }
    }

    #[test]
    fn fillet_replaces_the_corner() {
        let l = line(&[(0.0, 0.0), (20.0, 0.0), (20.0, 20.0)], false);
        let f = filleted(&l, 1, 5.0).expect("fillet");
        assert_eq!(f.vertices.len(), 4);
        assert!((f.vertices[1] - DVec3::new(15.0, 0.0, 0.0)).length() < 1e-9);
        assert!((f.vertices[2] - DVec3::new(20.0, 5.0, 0.0)).length() < 1e-9);
        assert!(f.bulges[1] > 0.0 && f.bulges[0] == 0.0 && f.bulges[2] == 0.0);
        assert!(filleted(&l, 0, 5.0).is_err());
        // Closed polylines can round the first corner.
        let sq = line(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], true);
        let f = filleted(&sq, 0, 2.0).expect("fillet");
        assert_eq!(f.vertices.len(), 5);
        assert_eq!(f.bulges.len(), 5);
    }
}
