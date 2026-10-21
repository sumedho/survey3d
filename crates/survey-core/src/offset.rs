//! Parallel offsets of polylines whose segments may be arcs.
//!
//! Each segment is offset on its own (lines move sideways, arcs change radius
//! about the same centre) and neighbours are then trimmed or extended to meet.
//! Where offset segments no longer meet (e.g. an arc pulled away from a line),
//! a straight connector is inserted. Vertex heights are kept.

use glam::{DVec2, DVec3};

use crate::arc::{Arc, STRAIGHT_BULGE, bulge_segments};
use crate::geom::line_intersection_2d;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OffsetError {
    /// Fewer than two distinct vertices.
    Degenerate,
    /// The offset would shrink this arc segment to nothing (offset ≥ radius on the inside).
    ArcCollapses { segment: usize },
}

impl std::fmt::Display for OffsetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OffsetError::Degenerate => write!(f, "the polyline has no length"),
            OffsetError::ArcCollapses { segment } => write!(
                f,
                "segment {} is an arc smaller than the offset distance",
                segment + 1
            ),
        }
    }
}

impl std::error::Error for OffsetError {}

/// Joins farther than this many offset distances from the original vertex are bevelled
/// instead of extended (avoids spikes at very sharp angles).
const MAX_MITER: f64 = 50.0;

#[derive(Clone, Copy, Debug)]
enum Seg {
    Line { a: DVec3, b: DVec3 },
    Arc { arc: Arc, a: DVec3, b: DVec3 },
}

impl Seg {
    fn a(&self) -> DVec3 {
        match self {
            Seg::Line { a, .. } | Seg::Arc { a, .. } => *a,
        }
    }

    fn b(&self) -> DVec3 {
        match self {
            Seg::Line { b, .. } | Seg::Arc { b, .. } => *b,
        }
    }
}

/// Offsets a polyline by `distance` to the **right** of its direction of travel
/// (negative distances offset to the left). Returns the new vertices and bulges.
pub fn offset_polyline(
    vertices: &[DVec3],
    bulges: &[f64],
    closed: bool,
    distance: f64,
) -> Result<(Vec<DVec3>, Vec<f64>), OffsetError> {
    let mut segs = Vec::new();
    // Original vertex at the end of each offset segment (for heights and miter limits).
    let mut corners = Vec::new();
    for (i, (a, b, bulge)) in bulge_segments(vertices, bulges, closed).enumerate() {
        let seg = match Arc::from_bulge(a, b, bulge) {
            Some(arc) => {
                let radius = arc.radius + distance * arc.sweep.signum();
                if radius <= arc.radius * 1e-9 {
                    return Err(OffsetError::ArcCollapses { segment: i });
                }
                let moved =
                    |p: DVec3| (arc.center + (p.truncate() - arc.center) / arc.radius * radius).extend(p.z);
                Seg::Arc {
                    arc: Arc { radius, ..arc },
                    a: moved(a),
                    b: moved(b),
                }
            }
            None => {
                let dir = (b - a).truncate();
                let len = dir.length();
                if len == 0.0 {
                    continue; // repeated vertex
                }
                let shift = (DVec2::new(dir.y, -dir.x) / len * distance).extend(0.0);
                Seg::Line {
                    a: a + shift,
                    b: b + shift,
                }
            }
        };
        segs.push(seg);
        corners.push(b);
    }
    let n = segs.len();
    if n == 0 {
        return Err(OffsetError::Degenerate);
    }

    let mut starts: Vec<DVec3> = segs.iter().map(Seg::a).collect();
    let mut ends: Vec<DVec3> = segs.iter().map(Seg::b).collect();
    // A straight connector follows segment `i` when its neighbours do not meet.
    let mut bevel = vec![false; n];
    let joins = if closed { n } else { n - 1 };
    let tolerance = 1e-9 * (1.0 + distance.abs());
    for i in 0..joins {
        let j = (i + 1) % n;
        let corner = corners[i];
        let (p, q) = (ends[i].truncate(), starts[j].truncate());
        let joined = if p.distance(q) <= tolerance {
            Some((p + q) * 0.5)
        } else {
            meet(&segs[i], &segs[j], (p + q) * 0.5)
                .filter(|x| x.distance(corner.truncate()) <= MAX_MITER * distance.abs())
        };
        match joined {
            Some(x) => {
                ends[i] = x.extend(corner.z);
                starts[j] = x.extend(corner.z);
            }
            None => bevel[i] = true,
        }
    }

    let mut out_v = Vec::with_capacity(n + 1);
    let mut out_b = Vec::with_capacity(n + 1);
    for (i, seg) in segs.iter().enumerate() {
        out_v.push(starts[i]);
        out_b.push(match seg {
            Seg::Line { .. } => 0.0,
            Seg::Arc { arc, .. } => arc.bulge_between(starts[i].truncate(), ends[i].truncate()),
        });
        let last = i == n - 1;
        if bevel[i] || (last && !closed) {
            out_v.push(ends[i]);
            out_b.push(0.0);
        }
    }
    if out_b.iter().all(|b| b.abs() < STRAIGHT_BULGE) {
        out_b.clear();
    }
    Ok((out_v, out_b))
}

/// Where the (extended) offset segments `s` and `t` meet, nearest `near`.
fn meet(s: &Seg, t: &Seg, near: DVec2) -> Option<DVec2> {
    let candidates: Vec<DVec2> = match (s, t) {
        (Seg::Line { a, b }, Seg::Line { a: c, b: d }) => {
            let (a, b) = (a.truncate(), b.truncate());
            line_intersection_2d(a - near, b - near, c.truncate() - near, d.truncate() - near)
                .map(|(u, _)| a + (b - a) * u)
                .into_iter()
                .collect()
        }
        (Seg::Line { a, b }, Seg::Arc { arc, .. }) | (Seg::Arc { arc, .. }, Seg::Line { a, b }) => {
            line_circle(a.truncate(), b.truncate(), arc.center, arc.radius)
        }
        (Seg::Arc { arc: p, .. }, Seg::Arc { arc: q, .. }) => {
            circle_circle(p.center, p.radius, q.center, q.radius)
        }
    };
    candidates
        .into_iter()
        .min_by(|x, y| x.distance_squared(near).total_cmp(&y.distance_squared(near)))
}

/// Intersections of the infinite line through `a`, `b` with a full circle.
pub(crate) fn line_circle(a: DVec2, b: DVec2, centre: DVec2, radius: f64) -> Vec<DVec2> {
    let d = b - a;
    let f = a - centre;
    let qa = d.dot(d);
    if qa == 0.0 {
        return Vec::new();
    }
    let (qb, qc) = (2.0 * f.dot(d), f.dot(f) - radius * radius);
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return Vec::new();
    }
    let s = disc.sqrt();
    let mut out = vec![a + d * ((-qb - s) / (2.0 * qa))];
    if s > 0.0 {
        out.push(a + d * ((-qb + s) / (2.0 * qa)));
    }
    out
}

/// Intersections of two full circles (none when concentric).
pub(crate) fn circle_circle(c0: DVec2, r0: f64, c1: DVec2, r1: f64) -> Vec<DVec2> {
    let d = c1 - c0;
    let dist = d.length();
    if dist == 0.0 || dist > r0 + r1 || dist < (r0 - r1).abs() {
        return Vec::new();
    }
    let a = (r0 * r0 - r1 * r1 + dist * dist) / (2.0 * dist);
    let h = (r0 * r0 - a * a).max(0.0).sqrt();
    let base = c0 + d * (a / dist);
    let off = d.perp() * (h / dist);
    if h == 0.0 {
        vec![base]
    } else {
        vec![base + off, base - off]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arc::polyline_metrics;
    use std::f64::consts::PI;

    fn v(x: f64, y: f64) -> DVec3 {
        DVec3::new(x, y, 0.0)
    }

    #[test]
    fn square_grows_outwards_to_the_right() {
        let o = DVec3::new(500_000.0, 7_000_000.0, 12.5);
        let square: Vec<DVec3> = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
            .iter()
            .map(|&(x, y)| o + v(x, y))
            .collect();
        // Anticlockwise, so the right is the outside.
        let (verts, bulges) = offset_polyline(&square, &[], true, 1.0).expect("offset");
        assert_eq!(verts.len(), 4);
        assert!(bulges.is_empty());
        assert!(
            (verts[0] - (o + v(-1.0, -1.0))).length() < 1e-9,
            "{:?}",
            verts[0] - o
        );
        assert!((verts[2] - (o + v(11.0, 11.0))).length() < 1e-9);
        assert_eq!(verts[1].z, 12.5);
        let (_, _, area) = polyline_metrics(&verts, &bulges, true);
        assert!((area - 144.0).abs() < 1e-6);
        let (inner, _) = offset_polyline(&square, &[], true, -2.0).expect("offset");
        let (_, _, area) = polyline_metrics(&inner, &[], true);
        assert!((area - 36.0).abs() < 1e-6);
    }

    #[test]
    fn circle_changes_radius() {
        // r = 5 anticlockwise circle.
        let verts = [v(0.0, 0.0), v(10.0, 0.0)];
        let (out, bulges) = offset_polyline(&verts, &[1.0, 1.0], true, 1.0).expect("offset");
        let (_, _, area) = polyline_metrics(&out, &bulges, true);
        assert!((area - 36.0 * PI).abs() < 1e-9, "{area}");
        assert_eq!(
            offset_polyline(&verts, &[1.0, 1.0], true, -5.0),
            Err(OffsetError::ArcCollapses { segment: 0 })
        );
    }

    #[test]
    fn open_polyline_with_tangent_arc() {
        // East 10 m, then a quarter circle (r = 10) turning left, ending heading north.
        let verts = [v(0.0, 0.0), v(10.0, 0.0), v(20.0, 10.0)];
        let bulge = (PI / 8.0).tan();
        let (out, bulges) = offset_polyline(&verts, &[0.0, bulge], false, -2.0).expect("offset");
        // Offset to the left by 2: the line moves north, the arc radius shrinks to 8.
        assert_eq!(out.len(), 3);
        assert!((out[0] - v(0.0, 2.0)).length() < 1e-9);
        assert!((out[1] - v(10.0, 2.0)).length() < 1e-9);
        assert!((out[2] - v(18.0, 10.0)).length() < 1e-9);
        let arc = Arc::from_bulge(out[1], out[2], bulges[1]).expect("arc");
        assert!((arc.radius - 8.0).abs() < 1e-9);
    }

    #[test]
    fn concave_corner_is_trimmed_and_gap_is_bevelled() {
        // A right angle turning left; offsetting left trims the inside corner.
        let l = [v(0.0, 0.0), v(10.0, 0.0), v(10.0, 10.0)];
        let (out, _) = offset_polyline(&l, &[], false, -1.0).expect("offset");
        assert_eq!(out, vec![v(0.0, 1.0), v(9.0, 1.0), v(9.0, 10.0)]);
        // A line into an arc that turns away: offsetting outward leaves a gap → connector.
        let verts = [v(0.0, 0.0), v(10.0, 0.0), v(10.0, 10.0)];
        let (out, bulges) = offset_polyline(&verts, &[0.0, -1.0], false, -1.0).expect("offset");
        assert!(out.len() >= 3);
        assert_eq!(bulges.len(), out.len());
    }
}
