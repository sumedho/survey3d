//! Bearing & distance entry (`bd`): walk a traverse or a lot boundary from a
//! start point by typing `bearing distance [dZ]` for each leg. Each leg creates
//! a point and moves on to it; the legs become one polyline. Sideshots (`s`)
//! take a point without moving on.
//!
//! A curved leg is given as a plan shows it, with `r=` radius and `l` or `r`
//! for the way it turns: by its chord (`bearing chord r=20 l`) or as a curve
//! leaving tangent to the leg before (`r=20 a=15.71 l`, with any two of
//! `r=` `a=` arc length, `c=` chord, `d=` delta and `t=` tangent length; `tb=`
//! gives the starting tangent bearing instead).
//!
//! A straight leg's direction can also be an angle: turned from the line
//! arriving at the current point (`a92.1500 25.31` clockwise, `al` anticlockwise)
//! or a deflection from that line's prolongation (`dr12.3000`, `dl`). The first
//! leg turns from a backsight set with `bs <point or bearing>`.
//!
//! `c` closes the figure on its start, the last leg being the one back to it,
//! and reports the misclosure, precision and area (the figure is not
//! adjusted); `j` joins back to the start with a computed closing leg instead.
//! Either way the lot can then be named to make it a parcel.

use egui_cad::snap::Pick;
use glam::{DVec2, DVec3};
use survey_core::arc::{Arc, end_direction, polyline_metrics};
use survey_core::point_input::{PointInput, ResolvedPoint};
use survey_core::solvers::{CurveInput, solve_curve};
use survey_core::{Angle, Bearing, Conventions};
use survey_model::{ChangeSet, Entity, ParcelRecord, Polyline};

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
    /// Length along the leg (the arc for a curve).
    distance: f64,
    /// The leg's bulge (0 when straight; see `survey_core::arc`).
    bulge: f64,
}

/// A closed figure waiting to be named as a parcel.
struct Closed {
    vertices: Vec<DVec3>,
    lot: String,
}

#[derive(Default)]
pub struct BdTool {
    start: Option<ResolvedPoint>,
    legs: Vec<Leg>,
    sideshot: bool,
    code: String,
    closed: Option<Closed>,
    /// Bearing from the start to its backsight, for angles on the first leg.
    backsight: Option<Bearing>,
}

/// How a leg's direction is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Turn2 {
    /// Clockwise from the back line.
    AngleRight,
    /// Anticlockwise from the back line.
    AngleLeft,
    /// Right of the line ahead (the arriving line prolonged).
    DeflectRight,
    DeflectLeft,
}

/// A turned angle or deflection at the start of `parts`: its kind, the angle
/// and how many fields it took (`a92.1500` or `a 92.1500`).
fn angle_form(conv: &Conventions, parts: &[&str]) -> Option<(Turn2, Angle, usize)> {
    const PREFIXES: [(&str, Turn2); 4] = [
        ("al", Turn2::AngleLeft),
        ("dr", Turn2::DeflectRight),
        ("dl", Turn2::DeflectLeft),
        ("a", Turn2::AngleRight),
    ];
    let first = parts.first()?.to_ascii_lowercase();
    for (prefix, kind) in PREFIXES {
        let Some(rest) = first.strip_prefix(prefix) else {
            continue;
        };
        if rest.is_empty() {
            let a = conv.parse_angle(parts.get(1)?).ok()?;
            return Some((kind, a, 2));
        }
        if let Ok(a) = conv.parse_angle(rest) {
            return Some((kind, a, 1));
        }
    }
    None
}

/// Which way a curve turns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Turn {
    Left,
    Right,
}

/// A curved leg, solved: where it ends and its shape.
#[derive(Debug, PartialEq)]
struct CurveLeg {
    end: DVec2,
    bulge: f64,
    radius: f64,
    length: f64,
    chord: f64,
    chord_bearing: Bearing,
    delta: f64,
    /// Fields after the curve's own (dZ and code).
    rest: Vec<String>,
}

/// Curve keys (`key=value`).
const CURVE_KEYS: [&str; 6] = ["r", "a", "c", "d", "t", "tb"];

fn curve_field(part: &str) -> Option<(&str, &str)> {
    let (k, v) = part.split_once('=')?;
    let k = k.trim();
    CURVE_KEYS
        .iter()
        .find(|c| c.eq_ignore_ascii_case(k))
        .map(|c| (*c, v.trim()))
}

fn is_curve(text: &str) -> bool {
    fields(text).iter().any(|p| curve_field(p).is_some())
}

/// Solves a curved leg leaving `from` (heading `incoming`, the direction the
/// figure arrives in, for tangent curves).
fn solve_curve_leg(
    conv: &Conventions,
    text: &str,
    from: DVec2,
    incoming: Option<DVec2>,
) -> Result<CurveLeg, String> {
    let parts = fields(text);
    let mut i = 0;
    // A leading bearing and distance is the chord.
    let mut chord_given = None;
    if parts.len() >= 2
        && curve_field(parts[0]).is_none()
        && curve_field(parts[1]).is_none()
        && let (Ok(b), Ok(d)) = (conv.parse_bearing(parts[0]), conv.parse_length(parts[1]))
    {
        chord_given = Some((b, d));
        i = 2;
    }
    let mut input = CurveInput::default();
    let mut turn = None;
    let mut major = false;
    let mut start_tangent = None;
    let mut rest = Vec::new();
    for part in &parts[i..] {
        if let Some((key, value)) = curve_field(part) {
            let length = || conv.parse_length(value).map_err(|e| format!("{key}=: {e}"));
            match key {
                "r" => input.radius = Some(length()?),
                "a" => input.length = Some(length()?),
                "c" => input.chord = Some(length()?),
                "t" => input.tangent = Some(length()?),
                "d" => {
                    let a = conv.parse_angle(value).map_err(|e| format!("d=: {e}"))?;
                    input.delta = Some(a.radians().abs());
                }
                _ => {
                    start_tangent = Some(conv.parse_bearing(value).map_err(|e| format!("tb=: {e}"))?);
                }
            }
            continue;
        }
        match part.to_ascii_lowercase().as_str() {
            "l" | "left" | "ccw" => turn = Some(Turn::Left),
            "r" | "right" | "cw" => turn = Some(Turn::Right),
            "major" => major = true,
            _ => rest.push((*part).to_string()),
        }
    }
    let turn = turn.ok_or("Say which way the curve turns: l (left) or r (right)")?;
    if let Some((_, chord)) = chord_given {
        if input.chord.is_some() {
            return Err("Give the chord once: as the bearing and distance, or as c=".into());
        }
        input.chord = Some(chord);
    }
    let known = [
        input.radius,
        input.delta,
        input.length,
        input.chord,
        input.tangent,
    ]
    .iter()
    .flatten()
    .count();
    if known != 2 {
        return Err(if chord_given.is_some() {
            "With a chord bearing and distance, give one more of r= a= d= t=".into()
        } else {
            "Give two of r= (radius) a= (arc) c= (chord) d= (delta) t= (tangent)".into()
        });
    }
    let mut solutions = solve_curve(input).map_err(|e| e.to_string())?;
    solutions.sort_by(|a, b| a.delta.total_cmp(&b.delta));
    let curve = if major {
        solutions.last()
    } else {
        solutions.first()
    }
    .copied()
    .ok_or("No curve has these values")?;
    let sign = if turn == Turn::Left { 1.0 } else { -1.0 };
    let chord_dir = match chord_given {
        Some((b, _)) => b.unit_vector(),
        None => {
            let tangent = match start_tangent {
                Some(b) => b.unit_vector(),
                None => incoming
                    .and_then(|d| d.try_normalize())
                    .ok_or("A tangent curve follows a leg (or give tb= for its starting tangent bearing)")?,
            };
            // The chord turns half the delta from the starting tangent.
            DVec2::from_angle(sign * curve.delta / 2.0).rotate(tangent)
        }
    };
    let end = from + chord_dir * curve.chord;
    let chord_bearing = Bearing::from_delta(chord_dir.x, chord_dir.y).ok_or("The chord has no length")?;
    Ok(CurveLeg {
        end,
        bulge: sign * (curve.delta / 4.0).tan(),
        radius: curve.radius,
        length: curve.length,
        chord: curve.chord,
        chord_bearing,
        delta: curve.delta,
        rest,
    })
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

    fn traverse(&self) -> impl Iterator<Item = &Leg> {
        self.legs.iter().filter(|l| !l.sideshot)
    }

    /// The figure's vertices: the start, then each traverse leg's end.
    fn figure(&self) -> Vec<DVec3> {
        let mut v: Vec<DVec3> = self.start.iter().map(|s| s.position).collect();
        v.extend(self.traverse().map(|l| l.position));
        v
    }

    /// The bulge of each figure segment.
    fn bulges(&self) -> Vec<f64> {
        self.traverse().map(|l| l.bulge).collect()
    }

    fn traverse_length(&self) -> f64 {
        self.traverse().map(|l| l.distance).sum()
    }

    /// The direction the figure arrives at the current point in.
    fn incoming(&self) -> Option<DVec2> {
        let v = self.figure();
        let b = self.bulges();
        let n = v.len();
        (n >= 2).then(|| end_direction(v[n - 2], v[n - 1], b[n - 2]))
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

    /// Fields after a leg's own: a number is dZ, anything else the code.
    fn take_rest(&mut self, conv: &Conventions, rest: &[&str]) -> Option<f64> {
        let mut dz = None;
        for part in rest {
            match conv.parse_length(part) {
                Ok(v) if dz.is_none() => dz = Some(v),
                _ => self.code = (*part).to_string(),
            }
        }
        dz
    }

    /// One `bearing distance [dZ] [code]` observation (or a curve) from the
    /// current point.
    fn leg(&mut self, ctx: &mut ToolCtx<'_>, text: &str) {
        let Some(from) = self.current() else { return };
        let conv = ctx.conventions().clone();
        let (plan, distance, bulge, dz, report) = if is_curve(text) {
            if self.sideshot {
                ctx.log
                    .error("A sideshot is a straight line: turn sideshots off (s) for a curve");
                return;
            }
            let curve = match solve_curve_leg(&conv, text, from.position.truncate(), self.incoming()) {
                Ok(c) => c,
                Err(e) => {
                    ctx.log.error(e);
                    return;
                }
            };
            let rest: Vec<&str> = curve.rest.iter().map(String::as_str).collect();
            if rest.len() > 2 {
                ctx.log.error("After the curve, only dZ and a code may follow");
                return;
            }
            let dz = self.take_rest(&conv, &rest);
            let report = format!(
                "R {}  arc {}  chord {} {}  Δ {} {}",
                conv.format_length(curve.radius),
                conv.format_length(curve.length),
                conv.format_bearing(curve.chord_bearing),
                conv.format_length(curve.chord),
                conv.format_angle(Angle::from_radians(curve.delta)),
                if curve.bulge > 0.0 { "left" } else { "right" },
            );
            (curve.end, curve.length, curve.bulge, dz, report)
        } else {
            let parts = fields(text);
            let (bearing, used, turned) = match angle_form(&conv, &parts) {
                Some((kind, angle, used)) => match self.turned_bearing(kind, angle) {
                    Ok(b) => (b, used, Some(angle)),
                    Err(e) => {
                        ctx.log.error(e);
                        return;
                    }
                },
                None => match conv.parse_bearing(parts.first().copied().unwrap_or_default()) {
                    Ok(b) => (b, 1, None),
                    Err(e) => {
                        ctx.log.error(e.to_string());
                        return;
                    }
                },
            };
            let rest = &parts[used..];
            if !(1..=3).contains(&rest.len()) {
                ctx.log.error(
                    "Enter: bearing distance[ dZ][ code] — e.g. 45.3020 25.31 — or an angle: a92.1500 25.31",
                );
                return;
            }
            let dist = match conv.parse_length(rest[0]) {
                Ok(d) => d,
                Err(e) => {
                    ctx.log.error(e.to_string());
                    return;
                }
            };
            let dz = self.take_rest(&conv, &rest[1..]);
            let report = format!(
                "{}brg {}  dist {}",
                turned.map_or_else(String::new, |a| format!("angle {}  ", conv.format_angle(a))),
                conv.format_bearing(bearing),
                conv.format_length(dist)
            );
            (
                from.position.truncate() + bearing.unit_vector() * dist,
                dist,
                0.0,
                dz,
                report,
            )
        };
        // Distances typed at ground go onto the grid × the line's combined factor.
        let plan = if ctx.doc.ground_distances() {
            let o = from.position.truncate();
            let k = ctx
                .doc
                .typed_to_grid(from.position, Some(plan.extend(from.position.z)));
            o + (plan - o) * k
        } else {
            plan
        };
        let height_known = from.height_known && dz.is_some();
        let position = plan.extend(from.position.z + dz.unwrap_or(0.0));
        let name = ctx.doc.next_point_name();
        let point = new_point(ctx, name.clone(), position, height_known, &self.code);
        ctx.execute(ChangeSet::new(format!("Bearing & distance point {name}")).insert_entity(point));
        ctx.log.result(format!(
            "{} {name}: {report}  → {}",
            if self.sideshot {
                "Sideshot"
            } else if bulge != 0.0 {
                "Curve to"
            } else {
                "Point"
            },
            conv.format_coord(position, height_known)
        ));
        self.legs.push(Leg {
            name,
            position,
            height_known,
            sideshot: self.sideshot,
            distance,
            bulge,
        });
    }

    /// The bearing of a leg turned `angle` from the line arriving at the
    /// current point (or from the backsight, before the first leg).
    fn turned_bearing(&self, kind: Turn2, angle: Angle) -> Result<Bearing, String> {
        let back = match self.incoming() {
            Some(d) => Bearing::from_delta(-d.x, -d.y),
            None => self.backsight,
        }
        .ok_or("An angle turns from the line before: give the first leg as a bearing, or set a backsight with bs <point or bearing>")?;
        let ahead = back.reverse().angle();
        Ok(Bearing::new(
            match kind {
                Turn2::AngleRight => back.angle() + angle,
                Turn2::AngleLeft => back.angle() - angle,
                Turn2::DeflectRight => ahead + angle,
                Turn2::DeflectLeft => ahead - angle,
            }
            .normalized(),
        ))
    }

    /// `bs <point or bearing>`: the backsight angles on the first leg turn from.
    fn set_backsight(&mut self, ctx: &mut ToolCtx<'_>, value: &str) {
        let Some(from) = self.current() else { return };
        let conv = ctx.conventions().clone();
        let bearing = match ctx.doc.find_point_by_name(value) {
            Some((_, p)) => {
                let d = p.position - from.position;
                Bearing::from_delta(d.x, d.y)
                    .ok_or_else(|| "The backsight is at the current point".to_string())
            }
            None => conv
                .parse_bearing(value)
                .map_err(|e| format!("Backsight: not a point or a bearing ({e})")),
        };
        match bearing {
            Ok(b) => {
                self.backsight = Some(b);
                ctx.log.info(format!("Backsight {}", conv.format_bearing(b)));
            }
            Err(e) => ctx.log.error(e),
        }
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
        let traverse = self.traverse().count();
        let Some(start) = self.start else { return };
        if traverse < 2 || (traverse < 3 && self.bulges().iter().all(|b| *b == 0.0)) {
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
        let bulges = self.bulges();
        let area = ctx
            .doc
            .shown_area(polyline_metrics(&vertices, &bulges, true).2.abs(), start.position);
        let conv = ctx.conventions().clone();
        // The misclosure as the legs were typed (ground or grid).
        let misclose = ctx.doc.shown_length(mis.length(), start.position, end);
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
        let mut cs = ChangeSet::new(format!("Close bearing & distance figure ({traverse} legs)"));
        if let Some((id, _)) = ctx.doc.find_point_by_name(&name) {
            cs = cs.delete_entity(id);
        }
        self.finish_closed(ctx, cs, vertices, bulges);
    }

    /// Joins back to the start with a computed closing leg, reported.
    fn join(&mut self, ctx: &mut ToolCtx<'_>) {
        let (Some(start), Some(cur)) = (self.start, self.current()) else {
            return;
        };
        let traverse = self.traverse().count();
        if traverse < 2 {
            ctx.log.warn("Join needs at least two legs to close with a third");
            return;
        }
        let back = (start.position - cur.position).truncate();
        let Some(bearing) = Bearing::from_delta(back.x, back.y) else {
            ctx.log.warn("Already at the start: close it with c");
            return;
        };
        let vertices = self.figure();
        let mut bulges = self.bulges();
        bulges.push(0.0);
        let (perimeter, _, area) = polyline_metrics(&vertices, &bulges, true);
        let conv = ctx.conventions().clone();
        let doc = ctx.doc;
        ctx.log.result(format!(
            "Closing leg: brg {}  dist {}  ({} legs, perimeter {}  area {})",
            conv.format_bearing(bearing),
            conv.format_length(doc.shown_distance(cur.position, start.position)),
            traverse + 1,
            conv.format_length(doc.shown_length(perimeter, start.position, cur.position)),
            conv.format_area(doc.shown_area(area.abs(), start.position)),
        ));
        let cs = ChangeSet::new(format!("Join bearing & distance figure ({} legs)", traverse + 1));
        self.finish_closed(ctx, cs, vertices, bulges);
    }

    /// Draws the closed figure (with `cs`) and asks for a lot name.
    fn finish_closed(
        &mut self,
        ctx: &mut ToolCtx<'_>,
        cs: ChangeSet,
        vertices: Vec<DVec3>,
        bulges: Vec<f64>,
    ) {
        let line = self.polyline(ctx, vertices.clone(), bulges, true);
        ctx.execute(cs.insert_entity(line));
        let lot = format!("Lot {}", crate::ui::parcels::next_lot_number(ctx.doc));
        self.reset();
        self.closed = Some(Closed { vertices, lot });
    }

    /// Makes the closed figure a parcel named `name`.
    fn make_parcel(&mut self, ctx: &mut ToolCtx<'_>, name: &str) {
        let Some(closed) = self.closed.take() else {
            return;
        };
        let name = if name.eq_ignore_ascii_case("y") {
            closed.lot
        } else {
            name.to_string()
        };
        // The newest closed polyline on these vertices is the one just drawn.
        let boundary = ctx
            .doc
            .entities()
            .filter_map(|(id, e)| match e {
                Entity::Polyline(l) if l.closed && l.vertices == closed.vertices => Some(id),
                _ => None,
            })
            .max();
        let Some(boundary) = boundary else {
            ctx.log.error("The closed figure is gone: draw it again");
            return;
        };
        ctx.execute(
            ChangeSet::new(format!("Create parcel {name}")).insert_record(ParcelRecord {
                name: name.clone(),
                boundary,
                ..Default::default()
            }),
        );
        ctx.log.result(format!("Parcel {name} created"));
    }

    fn polyline(&self, ctx: &ToolCtx<'_>, vertices: Vec<DVec3>, bulges: Vec<f64>, closed: bool) -> Entity {
        let heights_known =
            self.start.is_some_and(|s| s.height_known) && self.traverse().all(|l| l.height_known);
        let bulges = if bulges.iter().any(|b| *b != 0.0) {
            let mut b = bulges;
            b.resize(vertices.len(), 0.0);
            b
        } else {
            Vec::new()
        };
        Entity::Polyline(Polyline {
            name: String::new(),
            vertices,
            bulges,
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
            let line = self.polyline(ctx, vertices, self.bulges(), false);
            ctx.execute(ChangeSet::new("Bearing & distance linework").insert_entity(line));
        }
        self.reset();
    }

    fn reset(&mut self) {
        self.start = None;
        self.legs.clear();
        self.sideshot = false;
        self.closed = None;
        self.backsight = None;
    }
}

/// The value after `bs`.
fn backsight_arg(text: &str) -> Option<&str> {
    let (head, rest) = text.trim().split_once(char::is_whitespace)?;
    head.eq_ignore_ascii_case("bs")
        .then(|| rest.trim())
        .filter(|r| !r.is_empty())
}

/// Option keywords (exact, so codes in capitals are not taken for them).
fn is_option(text: &str) -> bool {
    matches!(text.trim(), "s" | "u" | "c" | "j")
}

impl Tool for BdTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn prompt(&self, conv: &Conventions) -> String {
        if let Some(closed) = &self.closed {
            return format!(
                "Closed — lot name to make it a parcel (y for {}, Enter to skip):",
                closed.lot
            );
        }
        let code = if self.code.is_empty() {
            String::new()
        } else {
            format!(" ({})", self.code)
        };
        let Some(start) = self.start else {
            return format!("Bearing & distance{code} — start point:");
        };
        let mode = if self.sideshot { " sideshots" } else { "" };
        let legs = self.traverse().count();
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
        format!(
            "Bearing & distance{code}{mode}{status} — bearing distance[ dZ][ code], angle (a/al/dr/dl…) distance, a curve (r=… l/r) / bs / s / u / c close / j join:"
        )
    }

    fn base_point(&self) -> Option<ResolvedPoint> {
        self.current()
    }

    fn wants_point(&self) -> bool {
        // While a lot name is asked for, typed text is the name.
        self.closed.is_none()
    }

    fn takes_codes(&self) -> bool {
        self.closed.is_none()
    }

    fn claims_text(&self, conv: &Conventions, text: &str) -> bool {
        if self.start.is_none() || self.closed.is_some() {
            return false;
        }
        if is_option(text) || is_curve(text) || backsight_arg(text).is_some() {
            return true;
        }
        let parts = fields(text);
        if let Some((_, _, used)) = angle_form(conv, &parts) {
            return parts.get(used).is_some_and(|d| conv.parse_length(d).is_ok());
        }
        parts.len() >= 2 && conv.parse_bearing(parts[0]).is_ok() && conv.parse_length(parts[1]).is_ok()
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        if self.closed.is_some() {
            match event {
                ToolEvent::Text(text) => {
                    // Coordinates start the next run instead.
                    let conv = ctx.conventions().clone();
                    if text.contains([',', '@'])
                        && let Ok(Some(input)) = PointInput::parse(text, &conv)
                        && let Ok(r) = input.resolve(None)
                    {
                        self.closed = None;
                        self.begin(
                            ctx,
                            &Pick {
                                point: r.position,
                                height_known: r.height_known,
                                snap: None,
                            },
                        );
                    } else {
                        self.make_parcel(ctx, text.trim());
                    }
                    return ToolOutcome::Continue;
                }
                ToolEvent::Finish => {
                    self.closed = None;
                    ctx.log
                        .info("No parcel made (select the boundary and run mkparcel to make one later)");
                    return ToolOutcome::Continue;
                }
                ToolEvent::Point(_) | ToolEvent::Region(_) => self.closed = None,
                ToolEvent::Cancel => {
                    self.closed = None;
                    return ToolOutcome::Done;
                }
            }
        }
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
                    "j" if self.start.is_some() => self.join(ctx),
                    t if self.start.is_some() && backsight_arg(t).is_some() => {
                        self.set_backsight(ctx, backsight_arg(t).unwrap_or_default());
                    }
                    t if self.start.is_some() && (is_curve(t) || fields(t).len() >= 2) => self.leg(ctx, t),
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
            let mut line = figure[..1].to_vec();
            for (w, b) in figure.windows(2).zip(self.bulges()) {
                match Arc::from_bulge(w[0], w[1], b) {
                    Some(arc) => line.extend(arc.tessellate(arc.radius * 1e-3).into_iter().skip(1)),
                    None => line.push(w[1]),
                }
            }
            out.polylines.push(line);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn conv() -> Conventions {
        Conventions {
            angle_format: survey_core::conventions::AngleFormat::DecimalDegrees,
            ..Default::default()
        }
    }

    fn close(a: DVec2, b: DVec2) -> bool {
        a.distance(b) < 1e-9
    }

    /// Heading north from the origin, a left-turning quarter circle of radius
    /// 10 ends at (-10, 10), whichever two elements give it.
    #[test]
    fn tangent_curves_from_any_two_elements() {
        let c = conv();
        let quarter = std::f64::consts::FRAC_PI_2 * 10.0;
        let chord = 200f64.sqrt();
        for text in [
            format!("r=10 a={quarter} l"),
            "r=10 d=90 l".to_string(),
            format!("r=10 c={chord} l"),
            "r=10 t=10 l".to_string(),
            format!("a={quarter} c={chord} left"),
        ] {
            let leg = solve_curve_leg(&c, &text, DVec2::ZERO, Some(DVec2::Y)).expect(&text);
            assert!(close(leg.end, DVec2::new(-10.0, 10.0)), "{text}: {:?}", leg.end);
            assert!(
                (leg.bulge - (std::f64::consts::FRAC_PI_8).tan()).abs() < 1e-9,
                "anticlockwise"
            );
            assert!((leg.length - quarter).abs() < 1e-9);
        }
        // Turning right ends at (10, 10), with a clockwise bulge.
        let leg = solve_curve_leg(&c, "r=10 d=90 r", DVec2::ZERO, Some(DVec2::Y)).expect("right");
        assert!(close(leg.end, DVec2::new(10.0, 10.0)) && leg.bulge < 0.0);
        // A starting tangent bearing replaces the leg before.
        let leg = solve_curve_leg(&c, "r=10 d=90 r tb=90", DVec2::ZERO, None).expect("tb");
        assert!(close(leg.end, DVec2::new(10.0, -10.0)), "{:?}", leg.end);
    }

    #[test]
    fn chord_curves_and_errors() {
        let c = conv();
        // Chord bearing 45 and chord √200 with radius 10: the quarter circle
        // through (10, 10), bulging left of the chord when turning right.
        let leg = solve_curve_leg(
            &c,
            &format!("45 {} r=10 r 1.5 KB", 200f64.sqrt()),
            DVec2::ZERO,
            None,
        )
        .expect("chord");
        assert!(close(leg.end, DVec2::new(10.0, 10.0)));
        assert!(
            (leg.delta - std::f64::consts::FRAC_PI_2).abs() < 1e-9,
            "the minor arc"
        );
        assert_eq!(leg.rest, vec!["1.5", "KB"]);
        let major = solve_curve_leg(
            &c,
            &format!("45 {} r=10 r major", 200f64.sqrt()),
            DVec2::ZERO,
            None,
        )
        .expect("major");
        assert!((major.delta - 1.5 * std::f64::consts::PI).abs() < 1e-9);
        assert!(
            solve_curve_leg(&c, "r=10 a=5", DVec2::ZERO, Some(DVec2::Y)).is_err(),
            "no turn"
        );
        assert!(
            solve_curve_leg(&c, "r=10 l", DVec2::ZERO, Some(DVec2::Y)).is_err(),
            "one element"
        );
        assert!(
            solve_curve_leg(&c, "r=10 a=5 l", DVec2::ZERO, None).is_err(),
            "no tangent"
        );
        assert!(
            solve_curve_leg(&c, "r=5 c=20 l", DVec2::ZERO, Some(DVec2::Y)).is_err(),
            "chord too long"
        );
        assert!(is_curve("r=10 a=5 l") && !is_curve("45 10"));
    }
}
