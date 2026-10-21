//! Coordinate geometry (COGO) computations.

use glam::{DVec2, DVec3};

use crate::angle::Bearing;
use crate::arc::{Arc, bulge_segments};
use crate::geom::line_intersection_2d;
use crate::offset::{circle_circle, line_circle};

/// Result of an inverse computation between two points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inverse {
    /// `None` when the points are horizontally coincident.
    pub bearing: Option<Bearing>,
    pub horizontal: f64,
    /// `to - from`.
    pub delta: DVec3,
    /// 3D distance; only meaningful when both heights are known.
    pub slope: f64,
    /// `dz / horizontal`; `None` when the horizontal distance is zero.
    pub grade: Option<f64>,
}

/// Bearing, distance and height difference from `from` to `to`.
pub fn inverse(from: DVec3, to: DVec3) -> Inverse {
    let delta = to - from;
    let horizontal = delta.truncate().length();
    Inverse {
        bearing: Bearing::from_delta(delta.x, delta.y),
        horizontal,
        delta,
        slope: delta.length(),
        grade: (horizontal > 0.0).then(|| delta.z / horizontal),
    }
}

/// Radiates a point from `from` along `bearing` for a horizontal `distance`.
pub fn polar(from: DVec2, bearing: Bearing, distance: f64) -> DVec2 {
    from + bearing.unit_vector() * distance
}

// ---------------------------------------------------------------- intersections
//
// Computed relative to the first station so large grid coordinates keep full precision.

/// Intersection of the lines through `a` on `brg_a` and through `b` on `brg_b`;
/// `None` if they are parallel.
pub fn intersect_bearings(a: DVec2, brg_a: Bearing, b: DVec2, brg_b: Bearing) -> Option<DVec2> {
    let (da, db) = (brg_a.unit_vector(), brg_b.unit_vector());
    let rb = b - a;
    let (t, _) = line_intersection_2d(DVec2::ZERO, da, rb, rb + db)?;
    Some(a + da * t)
}

/// Points on the line through `a` on `brg` at horizontal `distance` from `b`
/// (zero, one or two), those ahead along the bearing first.
pub fn intersect_bearing_distance(a: DVec2, brg: Bearing, b: DVec2, distance: f64) -> Vec<DVec2> {
    let d = brg.unit_vector();
    let mut pts = line_circle(DVec2::ZERO, d, b - a, distance);
    pts.sort_by(|p, q| q.dot(d).total_cmp(&p.dot(d)));
    pts.into_iter().map(|p| a + p).collect()
}

/// Points at `dist_a` from `a` and `dist_b` from `b` (zero, one or two): the one
/// to the right of `a → b` first.
pub fn intersect_distances(a: DVec2, dist_a: f64, b: DVec2, dist_b: f64) -> Vec<DVec2> {
    let rb = b - a;
    let mut pts = circle_circle(DVec2::ZERO, dist_a, rb, dist_b);
    pts.sort_by(|p, q| rb.perp_dot(*p).total_cmp(&rb.perp_dot(*q)));
    pts.into_iter().map(|p| a + p).collect()
}

// ---------------------------------------------------------------- chainage & offset

/// A point's position relative to a polyline (an alignment or string).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StationOffset {
    /// Plan distance along the polyline to the foot of the perpendicular.
    pub chainage: f64,
    /// Plan distance from the polyline: positive to the **right** of the direction of travel.
    pub offset: f64,
    /// Index of the segment the foot lies on.
    pub segment: usize,
    /// The foot point on the polyline (with interpolated height).
    pub foot: DVec3,
}

/// Projects `p` onto the nearest part of a polyline (plan).
pub fn station_offset(vertices: &[DVec3], bulges: &[f64], closed: bool, p: DVec2) -> Option<StationOffset> {
    let mut best: Option<(f64, StationOffset)> = None;
    let mut start = 0.0;
    for (i, (a, b, bulge)) in bulge_segments(vertices, bulges, closed).enumerate() {
        let (length, t, foot, offset) = match Arc::from_bulge(a, b, bulge) {
            Some(arc) => {
                let d = p - arc.center;
                let t = (d.length_squared() > 0.0)
                    .then(|| arc.param_of_angle(d.y.atan2(d.x)))
                    .flatten()
                    .unwrap_or_else(|| {
                        let (s, e) = (arc.point_at(0.0), arc.point_at(1.0));
                        if s.truncate().distance(p) <= e.truncate().distance(p) {
                            0.0
                        } else {
                            1.0
                        }
                    });
                let foot = arc.point_at(t);
                // An anticlockwise arc has its centre on the left, so outside is right.
                let side = (d.length() - arc.radius).signum() * arc.sweep.signum();
                (arc.length_2d(), t, foot, side * foot.truncate().distance(p))
            }
            None => {
                let dir = (b - a).truncate();
                let len = dir.length();
                let t = if len > 0.0 {
                    ((p - a.truncate()).dot(dir) / (len * len)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let foot = a + (b - a) * t;
                let side = if dir.perp_dot(p - foot.truncate()) > 0.0 {
                    -1.0
                } else {
                    1.0
                };
                (len, t, foot, side * foot.truncate().distance(p))
            }
        };
        let candidate = StationOffset {
            chainage: start + length * t,
            offset,
            segment: i,
            foot,
        };
        if best.is_none_or(|(d, _)| offset.abs() < d) {
            best = Some((offset.abs(), candidate));
        }
        start += length;
    }
    best.map(|(_, so)| so)
}

/// The point at `chainage` along a polyline and `offset` to the right of it (plan);
/// `None` beyond either end. Heights are interpolated along the polyline.
pub fn point_at_station(
    vertices: &[DVec3],
    bulges: &[f64],
    closed: bool,
    chainage: f64,
    offset: f64,
) -> Option<DVec3> {
    let eps = 1e-9 * (1.0 + chainage.abs());
    if chainage < -eps {
        return None;
    }
    let mut start = 0.0;
    for (a, b, bulge) in bulge_segments(vertices, bulges, closed) {
        let (length, point) = match Arc::from_bulge(a, b, bulge) {
            Some(arc) => {
                let len = arc.length_2d();
                let t = ((chainage - start) / len).clamp(0.0, 1.0);
                let foot = arc.point_at(t);
                let outward = (foot.truncate() - arc.center) / arc.radius;
                (len, foot + (outward * offset * arc.sweep.signum()).extend(0.0))
            }
            None => {
                let dir = (b - a).truncate();
                let len = dir.length();
                let t = if len > 0.0 {
                    ((chainage - start) / len).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let right = if len > 0.0 {
                    DVec2::new(dir.y, -dir.x) / len
                } else {
                    DVec2::ZERO
                };
                (len, a + (b - a) * t + (right * offset).extend(0.0))
            }
        };
        if chainage <= start + length + eps {
            return Some(point);
        }
        start += length;
    }
    None
}

// ---------------------------------------------------------------- averaging

/// The mean of repeated observations of one point, with their spread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Average {
    /// Mean plan position; the height is the mean of the known heights (0 if none).
    pub position: DVec3,
    pub height_known: bool,
    /// Largest plan distance of an observation from the mean.
    pub max_plan: f64,
    /// Largest height difference from the mean (known heights only).
    pub max_height: f64,
    /// Sample standard deviations of easting, northing and height.
    pub sd: DVec3,
}

/// Averages points given as `(position, height known)`; `None` when empty.
pub fn average(points: &[(DVec3, bool)]) -> Option<Average> {
    let first = points.first()?.0;
    let n = points.len() as f64;
    let plan: DVec2 = points.iter().map(|(p, _)| (*p - first).truncate()).sum::<DVec2>() / n;
    let heights: Vec<f64> = points.iter().filter(|(_, k)| *k).map(|(p, _)| p.z).collect();
    let z = (!heights.is_empty()).then(|| heights.iter().sum::<f64>() / heights.len() as f64);
    let mean = first + plan.extend(0.0);
    let sd = |values: &mut dyn Iterator<Item = f64>, count: usize| {
        if count < 2 {
            0.0
        } else {
            (values.map(|v| v * v).sum::<f64>() / (count - 1) as f64).sqrt()
        }
    };
    let rel: Vec<DVec2> = points.iter().map(|(p, _)| (*p - mean).truncate()).collect();
    Some(Average {
        position: mean.truncate().extend(z.unwrap_or(0.0)),
        height_known: z.is_some(),
        max_plan: rel.iter().map(|r| r.length()).fold(0.0, f64::max),
        max_height: z.map_or(0.0, |z| heights.iter().map(|h| (h - z).abs()).fold(0.0, f64::max)),
        sd: DVec3::new(
            sd(&mut rel.iter().map(|r| r.x), rel.len()),
            sd(&mut rel.iter().map(|r| r.y), rel.len()),
            z.map_or(0.0, |z| sd(&mut heights.iter().map(|h| h - z), heights.len())),
        ),
    })
}

// ---------------------------------------------------------------- fillet

/// A circular fillet replacing the corner between two straight lines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fillet {
    /// Tangent point on the incoming line.
    pub start: DVec2,
    /// Tangent point on the outgoing line.
    pub end: DVec2,
    /// Bulge of the arc from `start` to `end` (positive anticlockwise).
    pub bulge: f64,
    /// Distance from the corner to each tangent point.
    pub tangent: f64,
}

/// Fillet of `radius` at `corner` between the lines `prev → corner → next`
/// (plan); `None` when the lines are straight on or turn back, or a line is too
/// short for the tangent length.
pub fn fillet(prev: DVec2, corner: DVec2, next: DVec2, radius: f64) -> Option<Fillet> {
    let (a, b) = (corner - prev, next - corner);
    let (la, lb) = (a.length(), b.length());
    if radius <= 0.0 || la <= 0.0 || lb <= 0.0 {
        return None;
    }
    let (ua, ub) = (a / la, b / lb);
    let deflection = ua.perp_dot(ub).atan2(ua.dot(ub));
    if deflection.abs() < 1e-9 || deflection.abs() > std::f64::consts::PI - 1e-9 {
        return None;
    }
    let tangent = radius * (deflection.abs() / 2.0).tan();
    if tangent > la * (1.0 + 1e-12) || tangent > lb * (1.0 + 1e-12) {
        return None;
    }
    Some(Fillet {
        start: corner - ua * tangent,
        end: corner + ub * tangent,
        bulge: (deflection / 4.0).tan(),
        tangent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_and_polar_are_consistent() {
        let a = DVec3::new(512_345.678, 6_912_345.678, 10.0);
        let b = DVec3::new(512_445.678, 6_912_245.678, 12.0);
        let inv = inverse(a, b);
        let bearing = inv.bearing.expect("distinct");
        assert!((bearing.degrees() - 135.0).abs() < 1e-9);
        assert!((inv.horizontal - 100.0 * 2f64.sqrt()).abs() < 1e-9);
        assert!((inv.grade.expect("grade") - 2.0 / inv.horizontal).abs() < 1e-12);
        let back = polar(a.truncate(), bearing, inv.horizontal);
        assert!((back - b.truncate()).length() < 1e-8);
    }

    #[test]
    fn coincident_points() {
        let inv = inverse(DVec3::ONE, DVec3::new(1.0, 1.0, 5.0));
        assert!(inv.bearing.is_none() && inv.grade.is_none());
        assert_eq!(inv.slope, 4.0);
    }

    #[test]
    fn intersections() {
        let o = DVec2::new(512_000.0, 6_912_000.0);
        // Bearing–bearing: 45° from the origin, 315° from (100, 0) → (50, 50).
        let p = intersect_bearings(
            o,
            Bearing::from_degrees(45.0),
            o + DVec2::new(100.0, 0.0),
            Bearing::from_degrees(315.0),
        )
        .expect("not parallel");
        assert!((p - (o + DVec2::new(50.0, 50.0))).length() < 1e-8);
        assert!(
            intersect_bearings(
                o,
                Bearing::from_degrees(10.0),
                o + DVec2::X,
                Bearing::from_degrees(190.0)
            )
            .is_none()
        );
        // Bearing–distance: due east from the origin, 5 from (4, 3) → (8, 0) ahead and (0, 0) behind.
        let pts = intersect_bearing_distance(o, Bearing::from_degrees(90.0), o + DVec2::new(4.0, 3.0), 5.0);
        assert_eq!(pts.len(), 2);
        assert!(
            (pts[0] - (o + DVec2::new(8.0, 0.0))).length() < 1e-8,
            "{:?}",
            pts[0] - o
        );
        // Distance–distance: equilateral triangle; the solution right of a→b (south) first.
        let pts = intersect_distances(o, 5.0, o + DVec2::new(5.0, 0.0), 5.0);
        assert_eq!(pts.len(), 2);
        assert!(pts[0].y < o.y && pts[1].y > o.y);
        assert!(intersect_distances(o, 1.0, o + DVec2::new(5.0, 0.0), 1.0).is_empty());
    }

    #[test]
    fn chainage_and_offset() {
        let o = DVec3::new(300_000.0, 5_000_000.0, 0.0);
        // East 10, then a quarter circle (r = 10) turning left to (20, 10).
        let verts = [o, o + DVec3::new(10.0, 0.0, 0.0), o + DVec3::new(20.0, 10.0, 0.0)];
        let bulges = [0.0, (std::f64::consts::PI / 8.0).tan()];
        // 2 m south of the line at chainage 4 → right offset.
        let so = station_offset(&verts, &bulges, false, o.truncate() + DVec2::new(4.0, -2.0)).expect("so");
        assert!((so.chainage - 4.0).abs() < 1e-9 && (so.offset - 2.0).abs() < 1e-9 && so.segment == 0);
        // Inside the left-turning curve is left (negative).
        let arc_len = 10.0 * std::f64::consts::FRAC_PI_2;
        let mid = point_at_station(&verts, &bulges, false, 10.0 + arc_len / 2.0, -3.0).expect("on arc");
        let back = station_offset(&verts, &bulges, false, mid.truncate()).expect("so");
        assert!((back.chainage - (10.0 + arc_len / 2.0)).abs() < 1e-9, "{back:?}");
        assert!((back.offset + 3.0).abs() < 1e-9 && back.segment == 1);
        let centre = o.truncate() + DVec2::new(10.0, 10.0);
        assert!((mid.truncate().distance(centre) - 7.0).abs() < 1e-9);
        assert!(point_at_station(&verts, &bulges, false, 10.0 + arc_len + 1.0, 0.0).is_none());
        let end = point_at_station(&verts, &bulges, false, 10.0 + arc_len, 0.0).expect("end");
        assert!((end - verts[2]).length() < 1e-9);
    }

    #[test]
    fn fillet_corner() {
        let o = DVec2::new(400_000.0, 6_000_000.0);
        // East then north: a left turn of 90°; radius 5 → tangent 5.
        let f = fillet(o, o + DVec2::new(20.0, 0.0), o + DVec2::new(20.0, 20.0), 5.0).expect("fillet");
        assert!((f.tangent - 5.0).abs() < 1e-12);
        assert!((f.start - (o + DVec2::new(15.0, 0.0))).length() < 1e-9);
        assert!((f.end - (o + DVec2::new(20.0, 5.0))).length() < 1e-9);
        assert!((f.bulge - (std::f64::consts::PI / 8.0).tan()).abs() < 1e-12);
        let arc = Arc::from_bulge(f.start.extend(0.0), f.end.extend(0.0), f.bulge).expect("arc");
        assert!((arc.radius - 5.0).abs() < 1e-9);
        // Too large for the lines, or no corner at all.
        assert!(fillet(o, o + DVec2::new(20.0, 0.0), o + DVec2::new(20.0, 20.0), 25.0).is_none());
        assert!(fillet(o, o + DVec2::X, o + DVec2::new(2.0, 0.0), 1.0).is_none());
    }

    #[test]
    fn averaging() {
        let o = DVec3::new(600_000.0, 7_000_000.0, 0.0);
        let pts = [
            (o + DVec3::new(0.010, 0.0, 10.0), true),
            (o + DVec3::new(-0.010, 0.0, 10.02), true),
            (o + DVec3::new(0.0, 0.006, 0.0), false),
        ];
        let a = average(&pts).expect("points");
        assert!((a.position - (o + DVec3::new(0.0, 0.002, 10.01))).length() < 1e-9);
        assert!(a.height_known);
        assert!((a.max_plan - 0.010_198_039).abs() < 1e-6);
        assert!((a.max_height - 0.01).abs() < 1e-9);
        assert!(a.sd.x > 0.0 && a.sd.z > 0.0);
        assert!(average(&[]).is_none());
    }
}
