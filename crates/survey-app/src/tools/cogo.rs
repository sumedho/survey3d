//! Coordinate geometry tools: radiation, intersections and chainage/offset.
//!
//! Each creates survey points through one undoable, audited command; the
//! computation itself lives in [`survey_core::cogo`].

use egui_cad::snap::Pick;
use glam::{DVec2, DVec3};
use survey_core::cogo::{
    intersect_bearing_distance, intersect_bearings, intersect_distances, point_at_station, station_offset,
};
use survey_core::point_input::ResolvedPoint;
use survey_core::{Angle, Bearing, Conventions};
use survey_model::{ChangeSet, Entity, Polyline, SurveyPoint};

use super::{Preview, Tool, ToolCtx, ToolEvent, ToolOutcome, resolved};

pub const RADIATE_ID: &str = "radiate";
pub const CHAINAGE_ID: &str = "chainage";

/// Whether typed text looks like numbers (an observation or value), not a code or option.
fn is_numeric(text: &str) -> bool {
    text.trim()
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.'))
}

/// Splits on commas, or on whitespace when there are none.
fn fields(text: &str) -> Vec<&str> {
    if text.contains(',') {
        text.split(',').map(str::trim).collect()
    } else {
        text.split_whitespace().collect()
    }
}

/// A new point on the current layer.
fn new_point(ctx: &ToolCtx<'_>, name: String, position: DVec3, height_known: bool, code: &str) -> Entity {
    Entity::Point(SurveyPoint {
        name,
        position: if height_known {
            position
        } else {
            position.truncate().extend(0.0)
        },
        height_known,
        code: code.to_string(),
        description: String::new(),
        layer: ctx.doc.current_layer(),
    })
}

// ---------------------------------------------------------------- radiation

/// Radiation (side shots): set up on a station, orient to a backsight point or
/// bearing, then enter `angle distance [dZ]` observations. The angle is turned
/// clockwise from the backsight; `dZ` is the height difference from the station.
/// Non-numeric text sets the code for following points (`-` clears it).
#[derive(Default)]
pub struct RadiateTool {
    station: Option<ResolvedPoint>,
    backsight: Option<Bearing>,
    code: String,
    shots: Vec<DVec3>,
}

impl RadiateTool {
    fn observe(&mut self, ctx: &mut ToolCtx<'_>, text: &str) {
        let (Some(station), Some(bs)) = (self.station, self.backsight) else {
            return;
        };
        let conv = ctx.conventions();
        let parts = fields(text);
        if !(2..=3).contains(&parts.len()) {
            ctx.log
                .error("Enter: angle, distance[, dZ] — e.g. 45.3020, 25.31 or 45 30 20, 25.31, -0.42");
            return;
        }
        let parsed = (|| {
            let angle = conv.parse_angle(parts[0])?;
            let dist = conv.parse_length(parts[1])?;
            let dz = parts.get(2).map(|t| conv.parse_length(t)).transpose()?;
            Ok::<_, survey_core::ParseError>((angle, dist, dz))
        })();
        let (angle, dist, dz) = match parsed {
            Ok(v) => v,
            Err(e) => {
                ctx.log.error(e.to_string());
                return;
            }
        };
        let bearing = Bearing::new(Angle::from_radians(bs.radians() + angle.radians()).normalized());
        let plan = station.position.truncate() + bearing.unit_vector() * dist;
        let height_known = station.height_known && dz.is_some();
        let z = station.position.z + dz.unwrap_or(0.0);
        let name = ctx.doc.next_point_name();
        let point = new_point(ctx, name.clone(), plan.extend(z), height_known, &self.code);
        let conv = ctx.conventions();
        ctx.log.result(format!(
            "Point {name}: brg {}  dist {}  → {}",
            conv.format_bearing(bearing),
            conv.format_length(dist),
            conv.format_coord(plan.extend(z), height_known)
        ));
        self.shots.push(plan.extend(z));
        ctx.execute(ChangeSet::new(format!("Radiate point {name}")).insert_entity(point));
    }
}

impl Tool for RadiateTool {
    fn id(&self) -> &'static str {
        RADIATE_ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        let code = if self.code.is_empty() {
            String::new()
        } else {
            format!(" ({})", self.code)
        };
        match (self.station, self.backsight) {
            (None, _) => "Radiate — station (instrument) point:".into(),
            (Some(_), None) => "Radiate — backsight point or bearing:".into(),
            (Some(_), Some(_)) => format!("Radiate{code} — angle, distance[, dZ] [code]:"),
        }
    }

    fn wants_point(&self) -> bool {
        self.backsight.is_none()
    }

    fn claims_text(&self, conv: &Conventions, text: &str) -> bool {
        match (self.station, self.backsight) {
            (Some(_), None) => conv.parse_bearing(text).is_ok(),
            (Some(_), Some(_)) => is_numeric(text),
            _ => false,
        }
    }

    fn base_point(&self) -> Option<ResolvedPoint> {
        self.station
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) => {
                match self.station {
                    None => self.station = Some(resolved(&pick)),
                    // Clicks during observations do not re-orient the setup.
                    Some(_) if self.backsight.is_some() => {}
                    Some(st) => {
                        let d = pick.point - st.position;
                        match Bearing::from_delta(d.x, d.y) {
                            Some(b) => {
                                self.backsight = Some(b);
                                ctx.log.info(format!(
                                    "Backsight bearing {}",
                                    ctx.conventions().format_bearing(b)
                                ));
                            }
                            None => ctx.log.warn("The backsight must be away from the station"),
                        }
                    }
                }
                ToolOutcome::Continue
            }
            ToolEvent::Text(text) => {
                if self.station.is_some() && self.backsight.is_none() {
                    match ctx.conventions().parse_bearing(text) {
                        Ok(b) => self.backsight = Some(b),
                        Err(e) => ctx.log.error(e.to_string()),
                    }
                } else if self.backsight.is_some() && is_numeric(text) {
                    self.observe(ctx, text);
                } else if self.backsight.is_some() {
                    self.code = if text.trim() == "-" {
                        String::new()
                    } else {
                        text.trim().to_string()
                    };
                } else {
                    ctx.log.error(format!("Unknown option '{text}'"));
                }
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) | ToolEvent::Finish => ToolOutcome::Continue,
            ToolEvent::Cancel => {
                if self.backsight.take().is_some() || self.station.take().is_some() {
                    self.shots.clear();
                    ToolOutcome::Continue
                } else {
                    ToolOutcome::Done
                }
            }
        }
    }

    fn preview(&self, _conv: &Conventions, cursor: Option<&Pick>, out: &mut Preview) {
        let Some(st) = self.station else {
            return;
        };
        match self.backsight {
            None => {
                if let Some(c) = cursor {
                    out.rubber.push([st.position, c.point]);
                }
            }
            Some(bs) => {
                // Orientation ray, and the shots taken so far.
                let len = self
                    .shots
                    .iter()
                    .map(|p| p.truncate().distance(st.position.truncate()))
                    .fold(10.0, f64::max);
                let end = st.position + (bs.unit_vector() * len).extend(0.0);
                out.rubber.push([st.position, end]);
                for p in &self.shots {
                    out.rubber.push([st.position, *p]);
                }
            }
        }
    }
}

// ---------------------------------------------------------------- intersections

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntersectKind {
    BearingBearing,
    BearingDistance,
    DistanceDistance,
}

impl IntersectKind {
    pub fn id(self) -> &'static str {
        match self {
            IntersectKind::BearingBearing => "intersect_bb",
            IntersectKind::BearingDistance => "intersect_bd",
            IntersectKind::DistanceDistance => "intersect_dd",
        }
    }

    fn name(self) -> &'static str {
        match self {
            IntersectKind::BearingBearing => "Bearing–bearing",
            IntersectKind::BearingDistance => "Bearing–distance",
            IntersectKind::DistanceDistance => "Distance–distance",
        }
    }

    /// Whether the value from the first / second station is a bearing.
    fn bearing(self, second: bool) -> bool {
        match self {
            IntersectKind::BearingBearing => true,
            IntersectKind::BearingDistance => !second,
            IntersectKind::DistanceDistance => false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Value {
    Bearing(Bearing),
    Distance(f64),
}

/// Intersection of bearings and/or distances from two known points. Each value
/// can be typed or picked (the bearing or distance from the station to the pick).
/// When there are two solutions both are shown; pick near the one to keep.
pub struct IntersectTool {
    kind: IntersectKind,
    stations: Vec<ResolvedPoint>,
    values: Vec<Value>,
    solutions: Vec<DVec2>,
}

impl IntersectTool {
    pub fn new(kind: IntersectKind) -> Self {
        Self {
            kind,
            stations: Vec::new(),
            values: Vec::new(),
            solutions: Vec::new(),
        }
    }

    /// Whether the tool is waiting for the value from the latest station.
    fn wants_value(&self) -> bool {
        self.stations.len() > self.values.len()
    }

    fn value_is_bearing(&self) -> bool {
        self.kind.bearing(self.values.len() == 1)
    }

    fn parse_value(&self, conv: &Conventions, text: &str) -> Result<Value, String> {
        if self.value_is_bearing() {
            conv.parse_bearing(text)
                .map(Value::Bearing)
                .map_err(|e| e.to_string())
        } else {
            match conv.parse_length(text) {
                Ok(d) if d > 0.0 => Ok(Value::Distance(d)),
                Ok(_) => Err("The distance must be greater than zero".into()),
                Err(e) => Err(e.to_string()),
            }
        }
    }

    fn value_to(&self, p: DVec3) -> Option<Value> {
        let st = self.stations.last()?.position;
        let d = p - st;
        if self.value_is_bearing() {
            Bearing::from_delta(d.x, d.y).map(Value::Bearing)
        } else {
            let dist = d.truncate().length();
            (dist > 0.0).then_some(Value::Distance(dist))
        }
    }

    fn set_value(&mut self, ctx: &mut ToolCtx<'_>, value: Value) {
        self.values.push(value);
        if self.values.len() < 2 {
            return;
        }
        let (a, b) = (
            self.stations[0].position.truncate(),
            self.stations[1].position.truncate(),
        );
        self.solutions = match (self.values[0], self.values[1]) {
            (Value::Bearing(ba), Value::Bearing(bb)) => {
                intersect_bearings(a, ba, b, bb).into_iter().collect()
            }
            (Value::Bearing(ba), Value::Distance(db)) => intersect_bearing_distance(a, ba, b, db),
            (Value::Distance(da), Value::Distance(db)) => intersect_distances(a, da, b, db),
            (Value::Distance(_), Value::Bearing(_)) => Vec::new(),
        };
        match self.solutions.len() {
            0 => {
                ctx.log
                    .error("No intersection — the lines are parallel or the distances do not reach");
                self.reset();
            }
            1 => {
                let p = self.solutions[0];
                self.create(ctx, &[p]);
            }
            _ => ctx
                .log
                .info("Two solutions — pick near the one to keep, or type B for both"),
        }
    }

    fn create(&mut self, ctx: &mut ToolCtx<'_>, points: &[DVec2]) {
        let mut names = ctx.doc.point_names();
        let mut cs = ChangeSet::new(format!("{} intersection", self.kind.name()));
        for p in points {
            let name = names.next().unwrap_or_default();
            ctx.log.result(format!(
                "Point {name}: {}",
                ctx.conventions().format_coord(p.extend(0.0), false)
            ));
            cs.push_insert_entity(new_point(ctx, name, p.extend(0.0), false, ""));
        }
        ctx.execute(cs);
        self.reset();
    }

    fn reset(&mut self) {
        self.stations.clear();
        self.values.clear();
        self.solutions.clear();
    }
}

impl Tool for IntersectTool {
    fn id(&self) -> &'static str {
        self.kind.id()
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        let name = self.kind.name();
        if !self.solutions.is_empty() {
            return format!("{name} — pick near the solution to keep [Both]:");
        }
        let which = if self.stations.len() <= 1 && self.values.is_empty() {
            "first"
        } else {
            "second"
        };
        if self.wants_value() {
            let what = if self.value_is_bearing() {
                "bearing"
            } else {
                "distance"
            };
            format!("{name} — {what} from the {which} point (type or pick):")
        } else {
            format!("{name} — {which} known point:")
        }
    }

    fn claims_text(&self, conv: &Conventions, text: &str) -> bool {
        self.wants_value() && self.parse_value(conv, text).is_ok()
    }

    fn base_point(&self) -> Option<ResolvedPoint> {
        self.stations.last().copied()
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) => {
                if !self.solutions.is_empty() {
                    let c = pick.point.truncate();
                    if let Some(best) = self
                        .solutions
                        .iter()
                        .copied()
                        .min_by(|a, b| a.distance(c).total_cmp(&b.distance(c)))
                    {
                        self.create(ctx, &[best]);
                    }
                } else if self.wants_value() {
                    match self.value_to(pick.point) {
                        Some(v) => self.set_value(ctx, v),
                        None => ctx.log.warn("Pick a point away from the station"),
                    }
                } else {
                    self.stations.push(resolved(&pick));
                }
                ToolOutcome::Continue
            }
            ToolEvent::Text(text) => {
                let t = text.trim().to_ascii_lowercase();
                if self.solutions.len() > 1 && (t == "b" || t == "both") {
                    let both = self.solutions.clone();
                    self.create(ctx, &both);
                } else if self.wants_value() {
                    match self.parse_value(ctx.conventions(), text) {
                        Ok(v) => self.set_value(ctx, v),
                        Err(e) => ctx.log.error(e),
                    }
                } else {
                    ctx.log.error(format!("Unknown option '{text}'"));
                }
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) | ToolEvent::Finish => ToolOutcome::Continue,
            ToolEvent::Cancel => {
                if self.stations.is_empty() {
                    ToolOutcome::Done
                } else {
                    self.reset();
                    ToolOutcome::Continue
                }
            }
        }
    }

    fn preview(&self, _conv: &Conventions, cursor: Option<&Pick>, out: &mut Preview) {
        // Construction geometry for the values entered so far.
        for (st, v) in self.stations.iter().zip(&self.values) {
            let p = st.position.truncate().extend(0.0);
            match v {
                Value::Bearing(b) => {
                    let reach = self
                        .stations
                        .iter()
                        .map(|s| s.position.truncate().distance(p.truncate()))
                        .fold(50.0, f64::max)
                        * 2.0;
                    out.rubber.push([
                        p - (b.unit_vector() * reach).extend(0.0),
                        p + (b.unit_vector() * reach).extend(0.0),
                    ]);
                }
                Value::Distance(d) => {
                    if let Some(arc) = survey_core::arc::Arc::from_bulge(
                        p + DVec3::new(*d, 0.0, 0.0),
                        p - DVec3::new(*d, 0.0, 0.0),
                        1.0,
                    ) {
                        out.polylines.push(arc.tessellate(d * 1e-3));
                    }
                    if let Some(arc) = survey_core::arc::Arc::from_bulge(
                        p - DVec3::new(*d, 0.0, 0.0),
                        p + DVec3::new(*d, 0.0, 0.0),
                        1.0,
                    ) {
                        out.polylines.push(arc.tessellate(d * 1e-3));
                    }
                }
            }
        }
        out.points.extend(self.solutions.iter().map(|p| p.extend(0.0)));
        if let (true, Some(st), Some(c)) = (self.wants_value(), self.stations.last(), cursor) {
            out.rubber.push([st.position, c.point]);
        }
    }
}

// ---------------------------------------------------------------- chainage / offset

/// Chainage and offset along a polyline: pick the polyline, then pick points to
/// report their chainage and offset, or type `chainage[, offset]` to create a
/// point there. Offsets are positive to the right of the direction of travel.
#[derive(Default)]
pub struct ChainageTool {
    line: Option<Polyline>,
}

impl Tool for ChainageTool {
    fn id(&self) -> &'static str {
        CHAINAGE_ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        match self.line {
            None => "Chainage — pick a polyline (alignment):".into(),
            Some(_) => "Chainage — pick a point to report, or type chainage[, offset] to set out:".into(),
        }
    }

    fn wants_point(&self) -> bool {
        self.line.is_some()
    }

    fn claims_text(&self, _conv: &Conventions, text: &str) -> bool {
        self.line.is_some() && is_numeric(text)
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) => {
                match &self.line {
                    None => match ctx.hovered.and_then(|id| ctx.doc.entity(id)) {
                        Some(Entity::Polyline(l)) => {
                            self.line = Some(l.clone());
                            let (len, _, _) =
                                survey_core::arc::polyline_metrics(&l.vertices, &l.bulges, l.closed);
                            let conv = ctx.conventions();
                            ctx.log.info(format!(
                                "Alignment: chainage {} to {}",
                                conv.format_chainage(0.0),
                                conv.format_chainage(len)
                            ));
                        }
                        Some(_) => ctx.log.warn("Pick a polyline"),
                        None => {}
                    },
                    Some(l) => {
                        if let Some(so) =
                            station_offset(&l.vertices, &l.bulges, l.closed, pick.point.truncate())
                        {
                            let conv = ctx.conventions();
                            ctx.log.result(format!(
                                "Ch {}  Off {}",
                                conv.format_chainage(so.chainage),
                                conv.format_length(so.offset)
                            ));
                        }
                    }
                }
                ToolOutcome::Continue
            }
            ToolEvent::Text(text) => {
                let Some(l) = &self.line else {
                    return ToolOutcome::Continue;
                };
                let conv = ctx.conventions();
                let parts = fields(text);
                let parsed = (|| {
                    let ch = conv.parse_length(parts.first().copied().unwrap_or(""))?;
                    let off = parts.get(1).map(|t| conv.parse_length(t)).transpose()?;
                    Ok::<_, survey_core::ParseError>((ch, off.unwrap_or(0.0)))
                })();
                match parsed {
                    Ok((ch, off)) => match point_at_station(&l.vertices, &l.bulges, l.closed, ch, off) {
                        Some(p) => {
                            let name = ctx.doc.next_point_name();
                            let conv = ctx.conventions();
                            ctx.log.result(format!(
                                "Point {name}: Ch {}  Off {}  → {}",
                                conv.format_chainage(ch),
                                conv.format_length(off),
                                conv.format_coord(p, false)
                            ));
                            let point = new_point(ctx, name.clone(), p, false, "");
                            ctx.execute(ChangeSet::new(format!("Set out point {name}")).insert_entity(point));
                        }
                        None => ctx.log.error("That chainage is beyond the ends of the polyline"),
                    },
                    Err(e) => ctx.log.error(e.to_string()),
                }
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) | ToolEvent::Finish => ToolOutcome::Continue,
            ToolEvent::Cancel => {
                if self.line.take().is_some() {
                    ToolOutcome::Continue
                } else {
                    ToolOutcome::Done
                }
            }
        }
    }

    fn preview(&self, conv: &Conventions, cursor: Option<&Pick>, out: &mut Preview) {
        let (Some(l), Some(c)) = (&self.line, cursor) else {
            return;
        };
        if let Some(so) = station_offset(&l.vertices, &l.bulges, l.closed, c.point.truncate()) {
            out.rubber.push([so.foot, c.point]);
            out.points.push(so.foot);
            out.labels.push((
                c.point,
                DVec2::new(14.0, 14.0),
                format!(
                    "Ch {}  Off {}",
                    conv.format_chainage(so.chainage),
                    conv.format_length(so.offset)
                ),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_fields() {
        assert_eq!(fields("45.3020, 25.31"), vec!["45.3020", "25.31"]);
        assert_eq!(fields("45 30 20, 25.31, -0.4"), vec!["45 30 20", "25.31", "-0.4"]);
        assert_eq!(fields("45.3020 25.31"), vec!["45.3020", "25.31"]);
        assert!(is_numeric("-0.5, 3") && !is_numeric("KB") && !is_numeric(""));
    }
}
