//! Traverse reduction and adjustment.
//!
//! A traverse starts at a known station with a known backsight bearing and runs
//! through legs observed as horizontal angles (turned clockwise or anticlockwise
//! from the backsight, or deflections from the prolonged back line) or as
//! bearings, with horizontal or slope distances (zenith angle, instrument and
//! target heights) and optional height differences. Distances can be scaled
//! (e.g. ground to grid). When it closes on a known point the linear misclosure is
//! reported and distributed by the Bowditch (compass) or Transit rule; when a
//! closing angle onto a known bearing is observed, the angular misclosure is
//! reported and can be distributed equally among the angles first.
//!
//! All arithmetic runs relative to the start station, so grid coordinates keep
//! full precision.

use std::fmt::Write as _;

use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::accuracy::{Check, Tolerances};
use crate::angle::{Angle, Bearing};
use crate::conventions::Conventions;

/// How a leg's direction was observed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Direction {
    /// Horizontal angle turned clockwise from the backsight at the leg's start.
    Angle(Angle),
    /// Horizontal angle turned anticlockwise from the backsight.
    AngleLeft(Angle),
    /// Deflection from the prolonged back line: positive right, negative left.
    Deflection(Angle),
    /// Bearing of the leg.
    Bearing(Bearing),
}

impl Direction {
    /// The equivalent clockwise angle from the backsight; `None` for bearings.
    pub fn angle_right(self) -> Option<Angle> {
        match self {
            Direction::Angle(a) => Some(a),
            Direction::AngleLeft(a) => Some(-a),
            Direction::Deflection(d) => Some(Angle::from_degrees(180.0) + d),
            Direction::Bearing(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Leg {
    /// Name of the station at the end of the leg.
    pub to: String,
    pub direction: Direction,
    /// Horizontal distance, or slope distance when a zenith angle is given.
    pub distance: f64,
    /// Zenith angle of a slope distance (90° is level).
    #[serde(default)]
    pub zenith: Option<Angle>,
    /// Height difference from the leg's start to its end (ignored with a zenith angle).
    pub dz: Option<f64>,
    /// Instrument height at the leg's start (used with a zenith angle).
    #[serde(default)]
    pub instrument_height: f64,
    /// Target height at the leg's end (used with a zenith angle).
    #[serde(default)]
    pub target_height: f64,
}

impl Leg {
    pub fn new(to: impl Into<String>, direction: Direction, distance: f64) -> Self {
        Self {
            to: to.into(),
            direction,
            distance,
            zenith: None,
            dz: None,
            instrument_height: 0.0,
            target_height: 0.0,
        }
    }

    /// Horizontal distance (before scaling) and ground height difference.
    pub fn reduce(&self) -> (f64, Option<f64>) {
        match self.zenith {
            Some(z) => {
                let (s, c) = z.radians().sin_cos();
                (
                    self.distance * s,
                    Some(self.distance * c + self.instrument_height - self.target_height),
                )
            }
            None => (self.distance, self.dz),
        }
    }
}

/// Known control the traverse closes on.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Closing {
    /// Known plan position of the last station.
    pub point: DVec2,
    /// Known height of the last station.
    pub height: Option<f64>,
    /// Known bearing from the last station to a closing reference, and the angle
    /// observed there from the last backsight to that reference.
    pub angle_check: Option<(Bearing, Angle)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Adjustment {
    /// Report misclosures only.
    None,
    /// Corrections proportional to cumulative leg length (compass rule).
    #[default]
    Bowditch,
    /// Easting and northing corrections proportional to cumulative |ΔE| and |ΔN|.
    Transit,
}

impl Adjustment {
    pub const ALL: [Adjustment; 3] = [Adjustment::None, Adjustment::Bowditch, Adjustment::Transit];

    pub fn label(self) -> &'static str {
        match self {
            Adjustment::None => "None",
            Adjustment::Bowditch => "Bowditch",
            Adjustment::Transit => "Transit",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Traverse {
    pub start_name: String,
    pub start: DVec3,
    pub start_height_known: bool,
    /// Bearing from the start station to its backsight (needed for angle observations).
    pub backsight: Option<Bearing>,
    pub legs: Vec<Leg>,
    pub closing: Option<Closing>,
    pub adjustment: Adjustment,
    /// Distribute the angular misclosure equally among the observed angles.
    pub adjust_angles: bool,
    /// Applied to every horizontal distance (e.g. a combined ground-to-grid factor).
    pub scale_factor: f64,
}

#[derive(Clone, Debug, PartialEq, Error)]
pub enum TraverseError {
    #[error("The traverse has no legs")]
    NoLegs,
    #[error("Angle observations need a backsight bearing at the start")]
    NoBacksight,
    #[error("Leg {0} has no length")]
    ZeroDistance(usize),
    #[error("The scale factor must be positive")]
    BadScale,
}

/// One computed station.
#[derive(Clone, Debug, PartialEq)]
pub struct Station {
    pub name: String,
    /// (Angle-adjusted) bearing of the leg arriving at this station.
    pub bearing: Bearing,
    /// Reduced horizontal distance (scaled).
    pub distance: f64,
    /// Ground height difference of the arriving leg, when known.
    pub dz: Option<f64>,
    /// Coordinates before linear adjustment.
    pub unadjusted: DVec2,
    /// Final coordinates (height is 0 when unknown).
    pub position: DVec3,
    pub height_known: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TraverseResult {
    pub stations: Vec<Station>,
    /// Total horizontal length.
    pub length: f64,
    /// Computed minus known closing bearing (before angle adjustment).
    pub angular_misclosure: Option<Angle>,
    /// Number of observed angles, including the closing angle.
    pub angles: usize,
    /// Number of angles the angular misclosure was distributed over (0 = not adjusted).
    pub angles_adjusted: usize,
    /// Computed minus known closing position (after angle adjustment, before linear adjustment).
    pub linear_misclosure: Option<DVec2>,
    /// Computed minus known closing height.
    pub vertical_misclosure: Option<f64>,
    pub adjustment: Adjustment,
}

impl TraverseResult {
    /// Precision as the `N` of `1 : N` (length over linear misclosure); `None` if
    /// not closed or if it closes perfectly.
    pub fn precision(&self) -> Option<f64> {
        let m = self.linear_misclosure?.length();
        (m > 0.0).then(|| self.length / m)
    }
}

pub fn compute(t: &Traverse) -> Result<TraverseResult, TraverseError> {
    if t.legs.is_empty() {
        return Err(TraverseError::NoLegs);
    }
    if !t.scale_factor.is_finite() || t.scale_factor <= 0.0 {
        return Err(TraverseError::BadScale);
    }
    let reduced: Vec<(f64, Option<f64>)> = t
        .legs
        .iter()
        .map(|l| {
            let (h, dz) = l.reduce();
            (h * t.scale_factor, dz)
        })
        .collect();
    if let Some(i) = reduced.iter().position(|(h, _)| *h <= 0.0) {
        return Err(TraverseError::ZeroDistance(i + 1));
    }
    let angles = t
        .legs
        .iter()
        .filter(|l| l.direction.angle_right().is_some())
        .count();
    if angles > 0 && t.backsight.is_none() {
        return Err(TraverseError::NoBacksight);
    }

    // Bearings with `correction` added to every observed angle (corrections
    // accumulate along the traverse because each bearing carries the previous ones).
    let bearings = |correction: f64| -> (Vec<Bearing>, Bearing) {
        let mut back = t.backsight.unwrap_or_default();
        let mut out = Vec::with_capacity(t.legs.len());
        for leg in &t.legs {
            let b = match (leg.direction, leg.direction.angle_right()) {
                (Direction::Bearing(b), _) => b,
                (_, Some(a)) => Bearing::new(back.angle() + a + Angle::from_radians(correction)),
                (_, None) => unreachable!("only bearings have no angle"),
            };
            out.push(b);
            back = b.reverse();
        }
        (out, back)
    };

    let (mut legs_brg, last_back) = bearings(0.0);
    let mut angular_misclosure = None;
    let mut angles_adjusted = 0;
    let closing_angle = t.closing.and_then(|c| c.angle_check);
    if let Some((known, observed)) = closing_angle {
        let computed = Bearing::new(last_back.angle() + observed);
        let mis = (computed.angle() - known.angle()).normalized_signed();
        angular_misclosure = Some(mis);
        // Only when every direction is an observed angle does an equal share make sense.
        if t.adjust_angles && angles == t.legs.len() {
            angles_adjusted = angles + 1;
            let correction = -mis.radians() / angles_adjusted as f64;
            legs_brg = bearings(correction).0;
        }
    }

    // Provisional coordinates relative to the start.
    let mut rel = Vec::with_capacity(t.legs.len());
    let mut p = DVec2::ZERO;
    let mut length = 0.0;
    let mut cum = Vec::with_capacity(t.legs.len());
    let (mut cum_e, mut cum_n) = (Vec::new(), Vec::new());
    let (mut sum_e, mut sum_n) = (0.0, 0.0);
    for ((h, _), b) in reduced.iter().zip(&legs_brg) {
        let d = b.unit_vector() * *h;
        p += d;
        length += h;
        sum_e += d.x.abs();
        sum_n += d.y.abs();
        rel.push(p);
        cum.push(length);
        cum_e.push(sum_e);
        cum_n.push(sum_n);
    }

    let origin = t.start.truncate();
    let linear_misclosure = t.closing.map(|c| p - (c.point - origin));
    let corrections: Vec<DVec2> = match (linear_misclosure, t.adjustment) {
        (Some(m), Adjustment::Bowditch) => cum.iter().map(|c| -m * (c / length)).collect(),
        (Some(m), Adjustment::Transit) => cum_e
            .iter()
            .zip(&cum_n)
            .map(|(e, n)| {
                DVec2::new(
                    if sum_e > 0.0 { -m.x * e / sum_e } else { 0.0 },
                    if sum_n > 0.0 { -m.y * n / sum_n } else { 0.0 },
                )
            })
            .collect(),
        _ => vec![DVec2::ZERO; rel.len()],
    };

    // Heights: only when the start height and every dZ are known.
    let heights_known = t.start_height_known && reduced.iter().all(|(_, dz)| dz.is_some());
    let mut z = t.start.z;
    let mut zs = Vec::with_capacity(t.legs.len());
    for (_, dz) in &reduced {
        z += dz.unwrap_or(0.0);
        zs.push(z);
    }
    let vertical_misclosure = match t.closing.and_then(|c| c.height) {
        Some(h) if heights_known => Some(z - h),
        _ => None,
    };
    if let Some(vm) = vertical_misclosure
        && t.adjustment != Adjustment::None
    {
        for (zi, c) in zs.iter_mut().zip(&cum) {
            *zi -= vm * c / length;
        }
    }

    let stations = t
        .legs
        .iter()
        .enumerate()
        .map(|(i, leg)| Station {
            name: leg.to.clone(),
            bearing: legs_brg[i],
            distance: reduced[i].0,
            dz: reduced[i].1,
            unadjusted: origin + rel[i],
            position: (origin + rel[i] + corrections[i]).extend(if heights_known { zs[i] } else { 0.0 }),
            height_known: heights_known,
        })
        .collect();

    Ok(TraverseResult {
        stations,
        length,
        angular_misclosure,
        angles: angles + usize::from(closing_angle.is_some()),
        angles_adjusted,
        linear_misclosure,
        vertical_misclosure,
        adjustment: t.adjustment,
    })
}

/// A plain-text field-book style report: observations, reduced legs, stations,
/// misclosures and checks against the accuracy limits.
pub fn report(t: &Traverse, r: &TraverseResult, tolerances: &Tolerances, conv: &Conventions) -> String {
    let mut s = String::new();
    let len = |m: f64| conv.format_length(m);
    let pass = |c: Check| if c.pass { "PASS" } else { "FAIL" };
    let _ = writeln!(s, "TRAVERSE from {}", t.start_name);
    let _ = writeln!(s, "Start  {}", conv.format_coord(t.start, t.start_height_known));
    if let Some(bs) = t.backsight {
        let _ = writeln!(s, "Backsight bearing  {}", conv.format_bearing(bs));
    }
    if t.scale_factor != 1.0 {
        let _ = writeln!(s, "Scale factor       {:.8}", t.scale_factor);
    }
    let _ = writeln!(s);
    let _ = writeln!(s, "OBSERVATIONS");
    let _ = writeln!(
        s,
        "{:<10} {:<6} {:>16} {:>12} {:>14} {:>8} {:>8}",
        "To", "Type", "Direction", "Distance", "Zenith", "HI", "HT"
    );
    for leg in &t.legs {
        let (kind, value) = match leg.direction {
            Direction::Angle(a) => ("AR", conv.format_angle(a)),
            Direction::AngleLeft(a) => ("AL", conv.format_angle(a)),
            Direction::Deflection(d) => (
                if d.radians() < 0.0 { "DL" } else { "DR" },
                conv.format_angle(Angle::from_radians(d.radians().abs())),
            ),
            Direction::Bearing(b) => ("BRG", conv.format_bearing(b)),
        };
        let (zenith, hi, ht) = match leg.zenith {
            Some(z) => (
                conv.format_angle(z),
                len(leg.instrument_height),
                len(leg.target_height),
            ),
            None => (String::new(), String::new(), String::new()),
        };
        let _ = writeln!(
            s,
            "{:<10} {:<6} {:>16} {:>12} {:>14} {:>8} {:>8}",
            leg.to,
            kind,
            value,
            len(leg.distance),
            zenith,
            hi,
            ht
        );
    }
    let _ = writeln!(s);
    let _ = writeln!(s, "STATIONS");
    let _ = writeln!(
        s,
        "{:<10} {:>16} {:>12} {:>9}   Coordinates",
        "Station", "Bearing", "Hz dist", "dZ"
    );
    for st in &r.stations {
        let _ = writeln!(
            s,
            "{:<10} {:>16} {:>12} {:>9}   {}",
            st.name,
            conv.format_bearing(st.bearing),
            len(st.distance),
            st.dz.map(len).unwrap_or_default(),
            conv.format_coord(st.position, st.height_known)
        );
    }
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "Length              {}",
        conv.format_length_with_unit(r.length)
    );
    if let Some(a) = r.angular_misclosure {
        let _ = writeln!(
            s,
            "Angular misclosure  {}  ({} angles)",
            conv.format_angle(a),
            r.angles
        );
        if let Some(c) = tolerances.check_angular(a, r.angles) {
            let _ = writeln!(
                s,
                "  allowed           {}  {}",
                conv.format_angle(Angle::from_radians(c.allowed)),
                pass(c)
            );
        }
        if r.angles_adjusted > 0 {
            let _ = writeln!(
                s,
                "                    distributed equally over {} angles",
                r.angles_adjusted
            );
        }
    }
    match r.linear_misclosure {
        Some(m) => {
            let (ea, nb) = conv.horizontal_labels();
            let (ma, mb) = conv.horizontal_values(m.extend(0.0));
            let _ = writeln!(
                s,
                "Linear misclosure   {} (d{ea} {}  d{nb} {})",
                conv.format_length_with_unit(m.length()),
                len(ma),
                len(mb)
            );
            if let Some(b) = Bearing::from_delta(m.x, m.y) {
                let _ = writeln!(s, "Misclosure bearing  {}", conv.format_bearing(b));
            }
            match r.precision() {
                Some(n) => {
                    let _ = writeln!(s, "Precision           1 : {:.0}", n);
                }
                None => {
                    let _ = writeln!(s, "Precision           closes exactly");
                }
            }
            if let Some(c) = tolerances.check_linear(m.length(), r.length) {
                let _ = writeln!(
                    s,
                    "  allowed           {}  {}",
                    conv.format_length_with_unit(c.allowed),
                    pass(c)
                );
            }
            let _ = writeln!(s, "Adjustment          {}", r.adjustment.label());
        }
        None => {
            let _ = writeln!(s, "Open traverse — no misclosure check");
        }
    }
    if let Some(v) = r.vertical_misclosure {
        let _ = writeln!(s, "Vertical misclosure {}", conv.format_length_with_unit(v));
        if let Some(c) = tolerances.check_vertical(v, r.length) {
            let _ = writeln!(
                s,
                "  allowed           {}  {}",
                conv.format_length_with_unit(c.allowed),
                pass(c)
            );
        }
    }
    s
}

// ---------------------------------------------------------------- stored traverses

/// A direction reference: a point by name or a fixed bearing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Reference {
    Point(String),
    Bearing(Bearing),
}

/// A traverse as entered, saved with the project. Stations are referred to by
/// name, so recomputing picks up any change to the control points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TraverseRecord {
    pub name: String,
    pub start: String,
    pub backsight: Option<Reference>,
    pub legs: Vec<Leg>,
    pub close_on: Option<String>,
    pub closing_reference: Option<Reference>,
    pub closing_angle: Option<Angle>,
    pub adjustment: Adjustment,
    pub adjust_angles: bool,
    pub scale_factor: f64,
}

impl Default for TraverseRecord {
    fn default() -> Self {
        Self {
            name: String::new(),
            start: String::new(),
            backsight: None,
            legs: Vec::new(),
            close_on: None,
            closing_reference: None,
            closing_angle: None,
            adjustment: Adjustment::Bowditch,
            adjust_angles: true,
            scale_factor: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leg(to: &str, direction: Direction, distance: f64) -> Leg {
        Leg::new(to, direction, distance)
    }

    /// A 100 m square loop A → B (east) → C → D → A turning right at every corner,
    /// so each angle clockwise from the backsight to the foresight is 270°.
    fn square(err_angle_sec: f64, err_dist: f64) -> Traverse {
        let o = DVec3::new(500_000.0, 7_000_000.0, 0.0);
        let a = |deg: f64| Direction::Angle(Angle::from_degrees(deg));
        let e = err_angle_sec / 3600.0;
        Traverse {
            start_name: "A".into(),
            start: o,
            start_height_known: false,
            // From A the backsight D is due south; clockwise from south to east is 270°.
            backsight: Some(Bearing::from_degrees(180.0)),
            legs: vec![
                leg("B", a(270.0 + e), 100.0 + err_dist),
                leg("C", a(270.0), 100.0),
                leg("D", a(270.0), 100.0),
                leg("A", a(270.0), 100.0),
            ],
            closing: Some(Closing {
                point: o.truncate(),
                height: None,
                // Back at A: from the backsight D, 270° clockwise onto B (known bearing 90°).
                angle_check: Some((Bearing::from_degrees(90.0), Angle::from_degrees(270.0))),
            }),
            adjustment: Adjustment::Bowditch,
            adjust_angles: true,
            scale_factor: 1.0,
        }
    }

    #[test]
    fn perfect_loop_closes() {
        let r = compute(&square(0.0, 0.0)).expect("traverse");
        assert!(r.linear_misclosure.expect("closed").length() < 1e-9);
        assert!(r.angular_misclosure.expect("checked").radians().abs() < 1e-12);
        let b = &r.stations[0];
        assert!((b.bearing.degrees() - 90.0).abs() < 1e-9);
        assert!((b.position.truncate() - DVec2::new(500_100.0, 7_000_000.0)).length() < 1e-9);
        assert_eq!(r.length, 400.0);
    }

    #[test]
    fn angular_misclosure_is_distributed() {
        // 10" error in the first angle; 5 angles share it (4 legs + closing angle).
        let r = compute(&square(10.0, 0.0)).expect("traverse");
        let mis = r.angular_misclosure.expect("checked").degrees() * 3600.0;
        assert!((mis - 10.0).abs() < 1e-6, "{mis}");
        assert_eq!(r.angles_adjusted, 5);
        // Each angle loses 2": the first bearing is 90°00'08", the last 0°00'02",
        // and the closing angle then lands exactly on the known 90°.
        let sec = |b: Bearing| b.angle().normalized_signed().degrees() * 3600.0;
        assert!((sec(r.stations[0].bearing) - (90.0 * 3600.0 + 8.0)).abs() < 1e-6);
        assert!((sec(r.stations[3].bearing) - 2.0).abs() < 1e-6);
        // After adjustment the closing bearing check is exact, so the loop almost closes
        // (the residual comes only from the one-angle error being smeared).
        let lin = r.linear_misclosure.expect("closed").length();
        assert!(lin < 0.01, "{lin}");
    }

    #[test]
    fn bowditch_distributes_by_length() {
        let mut t = square(0.0, 0.04); // first leg measured 40 mm long
        let r = compute(&t).expect("traverse");
        let m = r.linear_misclosure.expect("closed");
        assert!((m - DVec2::new(0.04, 0.0)).length() < 1e-9, "{m:?}");
        assert!((r.precision().expect("precision") - 400.04 / 0.04).abs() < 1e-6);
        // Final station lands exactly on the known point; B takes 1/4 of the correction.
        let last = r.stations.last().expect("stations").position.truncate();
        assert!((last - t.start.truncate()).length() < 1e-9);
        let b = r.stations[0].position.truncate() - t.start.truncate();
        assert!((b.x - (100.04 - 0.04 * 100.04 / 400.04)).abs() < 1e-9);

        t.adjustment = Adjustment::Transit;
        let r = compute(&t).expect("traverse");
        let last = r.stations.last().expect("stations").position.truncate();
        assert!((last - t.start.truncate()).length() < 1e-9);
    }

    #[test]
    fn bearings_and_heights() {
        let o = DVec3::new(1000.0, 2000.0, 10.0);
        let t = Traverse {
            start_name: "1".into(),
            start: o,
            start_height_known: true,
            backsight: None,
            legs: vec![
                Leg {
                    dz: Some(1.5),
                    ..leg("2", Direction::Bearing(Bearing::from_degrees(0.0)), 50.0)
                },
                Leg {
                    dz: Some(-0.5),
                    ..leg("3", Direction::Bearing(Bearing::from_degrees(90.0)), 30.0)
                },
            ],
            closing: None,
            adjustment: Adjustment::Bowditch,
            adjust_angles: false,
            scale_factor: 1.0,
        };
        let r = compute(&t).expect("open traverse");
        assert!(r.linear_misclosure.is_none() && r.precision().is_none());
        assert_eq!(r.stations[1].position, DVec3::new(1030.0, 2050.0, 11.0));
        assert!(r.stations[1].height_known);
        let text = report(&t, &r, &Tolerances::default(), &Conventions::default());
        assert!(text.contains("Open traverse"), "{text}");
        assert!(
            compute(&Traverse {
                legs: Vec::new(),
                ..t.clone()
            })
            .is_err()
        );
    }

    #[test]
    fn angle_forms_are_equivalent() {
        // 270° right = 90° left = deflection 90° right.
        let base = compute(&square(0.0, 0.0)).expect("traverse");
        for form in [
            Direction::AngleLeft(Angle::from_degrees(90.0)),
            Direction::Deflection(Angle::from_degrees(90.0)),
        ] {
            let mut t = square(0.0, 0.0);
            for l in &mut t.legs {
                l.direction = form;
            }
            let r = compute(&t).expect("traverse");
            for (a, b) in r.stations.iter().zip(&base.stations) {
                assert!((a.position - b.position).length() < 1e-9);
            }
            assert_eq!(r.angles, 5);
        }
        // A left deflection turns the other way: south-east → east is 45° left.
        let d = Direction::Deflection(Angle::from_degrees(-45.0))
            .angle_right()
            .expect("angle");
        assert!((d.degrees() - 135.0).abs() < 1e-12);
    }

    #[test]
    fn slope_distances_and_scale() {
        let o = DVec3::new(1000.0, 2000.0, 50.0);
        // 100 m slope at zenith 60°: 86.603 horizontal, 50 up; HI 1.5, HT 2.0.
        let mut t = Traverse {
            start_name: "1".into(),
            start: o,
            start_height_known: true,
            backsight: None,
            legs: vec![Leg {
                zenith: Some(Angle::from_degrees(60.0)),
                instrument_height: 1.5,
                target_height: 2.0,
                ..leg("2", Direction::Bearing(Bearing::from_degrees(90.0)), 100.0)
            }],
            closing: None,
            adjustment: Adjustment::None,
            adjust_angles: false,
            scale_factor: 1.0,
        };
        let r = compute(&t).expect("traverse");
        let st = &r.stations[0];
        let h = 100.0 * 3f64.sqrt() / 2.0;
        assert!((st.distance - h).abs() < 1e-9);
        assert!((st.position.z - (50.0 + 50.0 + 1.5 - 2.0)).abs() < 1e-9);
        assert!(st.height_known);
        t.scale_factor = 0.9996;
        let r = compute(&t).expect("traverse");
        assert!((r.stations[0].distance - h * 0.9996).abs() < 1e-9);
        t.scale_factor = 0.0;
        assert_eq!(compute(&t), Err(TraverseError::BadScale));
    }

    #[test]
    fn report_checks_tolerances() {
        let t = square(10.0, 0.04);
        let r = compute(&t).expect("traverse");
        let tol = Tolerances {
            angular_seconds: Some(5.0),
            min_precision: Some(5000.0),
            ..Default::default()
        };
        let text = report(&t, &r, &tol, &Conventions::default());
        // 10" over 5 angles allows 11.2": pass; 1:~10000 against 1:5000: pass.
        assert!(text.contains("(5 angles)") && !text.contains("FAIL"), "{text}");
        let strict = Tolerances {
            angular_seconds: Some(4.0),
            ..tol
        };
        assert!(report(&t, &r, &strict, &Conventions::default()).contains("FAIL"));
        assert!(text.contains("OBSERVATIONS") && text.contains("AR"));
    }
}
