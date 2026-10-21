//! Traverse reduction and adjustment.
//!
//! A traverse starts at a known station with a known backsight bearing and runs
//! through legs observed either as horizontal angles (turned clockwise from the
//! backsight) or as bearings, with horizontal distances and optional height
//! differences. When it closes on a known point the linear misclosure is
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

use crate::angle::{Angle, Bearing};
use crate::conventions::Conventions;

/// How a leg's direction was observed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Direction {
    /// Horizontal angle turned clockwise from the backsight at the leg's start.
    Angle(Angle),
    /// Bearing of the leg.
    Bearing(Bearing),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Leg {
    /// Name of the station at the end of the leg.
    pub to: String,
    pub direction: Direction,
    /// Horizontal distance.
    pub distance: f64,
    /// Height difference from the leg's start to its end.
    pub dz: Option<f64>,
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
}

#[derive(Clone, Debug, PartialEq, Error)]
pub enum TraverseError {
    #[error("the traverse has no legs")]
    NoLegs,
    #[error("angle observations need a backsight bearing at the start")]
    NoBacksight,
    #[error("leg {0} has no length")]
    ZeroDistance(usize),
}

/// One computed station.
#[derive(Clone, Debug, PartialEq)]
pub struct Station {
    pub name: String,
    /// (Angle-adjusted) bearing of the leg arriving at this station.
    pub bearing: Bearing,
    pub distance: f64,
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
    if let Some(i) = t.legs.iter().position(|l| l.distance <= 0.0) {
        return Err(TraverseError::ZeroDistance(i + 1));
    }
    let angles = t
        .legs
        .iter()
        .filter(|l| matches!(l.direction, Direction::Angle(_)))
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
            let b = match leg.direction {
                Direction::Angle(a) => Bearing::new(back.angle() + a + Angle::from_radians(correction)),
                Direction::Bearing(b) => b,
            };
            out.push(b);
            back = b.reverse();
        }
        (out, back)
    };

    let (mut legs_brg, last_back) = bearings(0.0);
    let mut angular_misclosure = None;
    let mut angles_adjusted = 0;
    if let Some((known, observed)) = t.closing.and_then(|c| c.angle_check) {
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
    for (leg, b) in t.legs.iter().zip(&legs_brg) {
        let d = b.unit_vector() * leg.distance;
        p += d;
        length += leg.distance;
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
    let heights_known = t.start_height_known && t.legs.iter().all(|l| l.dz.is_some());
    let mut z = t.start.z;
    let mut zs = Vec::with_capacity(t.legs.len());
    for leg in &t.legs {
        z += leg.dz.unwrap_or(0.0);
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
            distance: leg.distance,
            unadjusted: origin + rel[i],
            position: (origin + rel[i] + corrections[i]).extend(if heights_known { zs[i] } else { 0.0 }),
            height_known: heights_known,
        })
        .collect();

    Ok(TraverseResult {
        stations,
        length,
        angular_misclosure,
        angles_adjusted,
        linear_misclosure,
        vertical_misclosure,
        adjustment: t.adjustment,
    })
}

/// A plain-text report of a computed traverse.
pub fn report(t: &Traverse, r: &TraverseResult, conv: &Conventions) -> String {
    let mut s = String::new();
    let len = |m: f64| conv.format_length(m);
    let _ = writeln!(s, "TRAVERSE from {}", t.start_name);
    let _ = writeln!(s, "Start  {}", conv.format_coord(t.start, t.start_height_known));
    if let Some(bs) = t.backsight {
        let _ = writeln!(s, "Backsight bearing  {}", conv.format_bearing(bs));
    }
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "{:<10} {:>16} {:>12}   Coordinates",
        "Station", "Bearing", "Distance"
    );
    for st in &r.stations {
        let _ = writeln!(
            s,
            "{:<10} {:>16} {:>12}   {}",
            st.name,
            conv.format_bearing(st.bearing),
            len(st.distance),
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
        let _ = writeln!(s, "Angular misclosure  {}", conv.format_angle(a));
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
            let _ = writeln!(s, "Adjustment          {}", r.adjustment.label());
        }
        None => {
            let _ = writeln!(s, "Open traverse — no misclosure check");
        }
    }
    if let Some(v) = r.vertical_misclosure {
        let _ = writeln!(s, "Vertical misclosure {}", conv.format_length_with_unit(v));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leg(to: &str, direction: Direction, distance: f64) -> Leg {
        Leg {
            to: to.into(),
            direction,
            distance,
            dz: None,
        }
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
                    to: "2".into(),
                    direction: Direction::Bearing(Bearing::from_degrees(0.0)),
                    distance: 50.0,
                    dz: Some(1.5),
                },
                Leg {
                    to: "3".into(),
                    direction: Direction::Bearing(Bearing::from_degrees(90.0)),
                    distance: 30.0,
                    dz: Some(-0.5),
                },
            ],
            closing: None,
            adjustment: Adjustment::Bowditch,
            adjust_angles: false,
        };
        let r = compute(&t).expect("open traverse");
        assert!(r.linear_misclosure.is_none() && r.precision().is_none());
        assert_eq!(r.stations[1].position, DVec3::new(1030.0, 2050.0, 11.0));
        assert!(r.stations[1].height_known);
        let text = report(&t, &r, &Conventions::default());
        assert!(text.contains("Open traverse"), "{text}");
        assert!(
            compute(&Traverse {
                legs: Vec::new(),
                ..t.clone()
            })
            .is_err()
        );
    }
}
