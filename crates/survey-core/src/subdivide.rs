//! Area subdivision: cutting a given area off a parcel with a line parallel to
//! one of its edges, or with a line swung about a fixed (hinge) point.
//!
//! Parcels are plan polygons (arcs tessellated by the caller). The cut is found
//! by bisection on the area of the polygon clipped to one side of the line,
//! which works for any simple polygon; the cut line itself is reported as the
//! part(s) of the line inside the parcel. Work relative to a nearby origin to
//! keep grid coordinates precise.

use glam::DVec2;
use thiserror::Error;

use crate::angle::Bearing;

#[derive(Clone, Debug, PartialEq, Error)]
pub enum SubdivideError {
    #[error("The parcel needs at least three corners")]
    TooFew,
    #[error("The parcel has no area")]
    NoArea,
    #[error("The area to cut off must be between zero and the parcel's area")]
    AreaOutOfRange,
    #[error("Edge {0} does not exist or has no length")]
    BadEdge(usize),
    #[error("No line through the hinge point cuts off that area")]
    NoSolution,
}

/// One way of dividing a parcel.
#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    /// The part(s) of the cut line inside the parcel (one for convex parcels).
    pub segments: Vec<(DVec2, DVec2)>,
    /// The parcel cut off (its area equals the requested area).
    pub part: Vec<DVec2>,
    /// The rest of the parcel.
    pub remainder: Vec<DVec2>,
    pub part_area: f64,
    pub remainder_area: f64,
}

/// Signed shoelace area (positive anticlockwise).
pub fn signed_area(poly: &[DVec2]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| poly[i].perp_dot(poly[(i + 1) % n])).sum::<f64>() / 2.0
}

/// The part of `poly` where `(x - p)·n <= offset` (Sutherland–Hodgman against one
/// half-plane; non-convex parcels may gain zero-width bridges along the line,
/// which do not change the area).
pub fn clip(poly: &[DVec2], p: DVec2, n: DVec2, offset: f64) -> Vec<DVec2> {
    let side = |x: DVec2| (x - p).dot(n) - offset;
    let mut out = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (sa, sb) = (side(a), side(b));
        if sa <= 0.0 {
            out.push(a);
        }
        if (sa < 0.0 && sb > 0.0) || (sa > 0.0 && sb < 0.0) {
            out.push(a + (b - a) * (sa / (sa - sb)));
        }
    }
    out
}

/// Parts of the line `(x - p)·n = offset` inside `poly`.
fn chords(poly: &[DVec2], p: DVec2, n: DVec2, offset: f64) -> Vec<(DVec2, DVec2)> {
    let dir = DVec2::new(-n.y, n.x);
    let base = p + n * offset;
    let side = |x: DVec2| (x - p).dot(n) - offset;
    let mut ts = Vec::new();
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (sa, sb) = (side(a), side(b));
        // Half-open test so a line through a vertex is counted once.
        if (sa < 0.0) != (sb < 0.0) {
            let x = a + (b - a) * (sa / (sa - sb));
            ts.push((x - base).dot(dir));
        }
    }
    ts.sort_by(f64::total_cmp);
    ts.chunks_exact(2)
        .map(|c| (base + dir * c[0], base + dir * c[1]))
        .filter(|(a, b)| a.distance(*b) > 1e-9)
        .collect()
}

fn validate(poly: &[DVec2], area: f64) -> Result<f64, SubdivideError> {
    if poly.len() < 3 {
        return Err(SubdivideError::TooFew);
    }
    let total = signed_area(poly).abs();
    if total <= 1e-12 {
        return Err(SubdivideError::NoArea);
    }
    if !(area > 0.0 && area < total) {
        return Err(SubdivideError::AreaOutOfRange);
    }
    Ok(total)
}

fn cut(poly: &[DVec2], p: DVec2, n: DVec2, offset: f64) -> Cut {
    let part = clip(poly, p, n, offset);
    let remainder = clip(poly, p, -n, -offset);
    Cut {
        segments: chords(poly, p, n, offset),
        part_area: signed_area(&part).abs(),
        remainder_area: signed_area(&remainder).abs(),
        part,
        remainder,
    }
}

/// Cuts `area` off the side of the parcel along `edge` (from corner `edge` to
/// the next) with a line parallel to that edge.
pub fn parallel(poly: &[DVec2], edge: usize, area: f64) -> Result<Cut, SubdivideError> {
    let total = validate(poly, area)?;
    if edge >= poly.len() {
        return Err(SubdivideError::BadEdge(edge + 1));
    }
    let (a, b) = (poly[edge], poly[(edge + 1) % poly.len()]);
    let d = b - a;
    if d.length() <= 1e-12 {
        return Err(SubdivideError::BadEdge(edge + 1));
    }
    // Inward normal: interior is left of each edge when anticlockwise.
    let left = DVec2::new(-d.y, d.x) / d.length();
    let n = if signed_area(poly) > 0.0 { left } else { -left };
    // `n` points into the parcel, so the strip next to the edge is (x-a)·n ≤ t.
    let far = poly.iter().map(|v| (*v - a).dot(n)).fold(0.0, f64::max);
    let area_at = |t: f64| signed_area(&clip(poly, a, n, t)).abs();
    let (mut lo, mut hi) = (0.0, far);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if area_at(mid) < area {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo <= 1e-12 * (1.0 + far) {
            break;
        }
    }
    let result = cut(poly, a, n, 0.5 * (lo + hi));
    debug_assert!((result.part_area + result.remainder_area - total).abs() < 1e-6 * total.max(1.0));
    Ok(result)
}

/// Cuts `area` off the parcel with a line through `hinge`. Returns every
/// distinct solution: usually two, one keeping the area on each side.
pub fn swing(poly: &[DVec2], hinge: DVec2, area: f64) -> Result<Vec<(Bearing, Cut)>, SubdivideError> {
    validate(poly, area)?;
    // Area to the left of the line through the hinge on bearing θ.
    let normal = |theta: f64| {
        let (s, c) = theta.sin_cos();
        // Direction (s, c); its right-hand normal, so `≤ 0` is the left side.
        DVec2::new(c, -s)
    };
    let left_area = |theta: f64| signed_area(&clip(poly, hinge, normal(theta), 0.0)).abs();
    const STEPS: usize = 720;
    let step = std::f64::consts::TAU / STEPS as f64;
    let mut out: Vec<(Bearing, Cut)> = Vec::new();
    let mut prev = left_area(0.0) - area;
    for i in 1..=STEPS {
        let theta = i as f64 * step;
        let cur = left_area(theta) - area;
        if prev == 0.0 || (prev < 0.0) != (cur < 0.0) {
            let (mut lo, mut hi) = (theta - step, theta);
            let f_lo = prev;
            for _ in 0..100 {
                let mid = 0.5 * (lo + hi);
                let f = left_area(mid) - area;
                if (f < 0.0) == (f_lo < 0.0) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let theta = 0.5 * (lo + hi);
            let c = cut(poly, hinge, normal(theta), 0.0);
            let keeps = !c.segments.is_empty() && (c.part_area - area).abs() <= 1e-6 * area.max(1.0);
            let bearing = Bearing::new(crate::Angle::from_radians(theta));
            if keeps
                && !out
                    .iter()
                    .any(|(b, _)| (b.angle() - bearing.angle()).normalized_signed().radians().abs() < 1e-7)
            {
                out.push((bearing, c));
            }
        }
        prev = cur;
    }
    if out.is_empty() {
        Err(SubdivideError::NoSolution)
    } else {
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 100 × 50 rectangle, anticlockwise from the origin (grid-sized offset).
    fn rect() -> Vec<DVec2> {
        vec![
            DVec2::new(0.0, 0.0),
            DVec2::new(100.0, 0.0),
            DVec2::new(100.0, 50.0),
            DVec2::new(0.0, 50.0),
        ]
    }

    #[test]
    fn clip_and_area() {
        let r = rect();
        assert_eq!(signed_area(&r), 5000.0);
        let half = clip(&r, DVec2::ZERO, DVec2::X, 40.0);
        assert!((signed_area(&half) - 2000.0).abs() < 1e-9);
    }

    #[test]
    fn parallel_cut() {
        // 1500 off the south edge: a line 15 north of it, along the full width.
        let c = parallel(&rect(), 0, 1500.0).expect("cut");
        assert!((c.part_area - 1500.0).abs() < 1e-6);
        assert!((c.remainder_area - 3500.0).abs() < 1e-6);
        assert_eq!(c.segments.len(), 1);
        let (a, b) = c.segments[0];
        assert!((a.y - 15.0).abs() < 1e-6 && (b.y - 15.0).abs() < 1e-6);
        assert!((a.distance(b) - 100.0).abs() < 1e-6);
        // Clockwise parcels work too.
        let mut cw = rect();
        cw.reverse();
        let c = parallel(&cw, 2, 1500.0).expect("cut"); // cw[2]→cw[3] is the south edge
        assert!((c.segments[0].0.y - 15.0).abs() < 1e-6);
        assert_eq!(parallel(&rect(), 0, 6000.0), Err(SubdivideError::AreaOutOfRange));
    }

    #[test]
    fn non_convex_parallel() {
        // An L shape: the cut line crosses the parcel twice.
        let l = vec![
            DVec2::new(0.0, 0.0),
            DVec2::new(60.0, 0.0),
            DVec2::new(60.0, 20.0),
            DVec2::new(20.0, 20.0),
            DVec2::new(20.0, 60.0),
            DVec2::new(0.0, 60.0),
        ];
        // Off the west edge (from (0,60) to (0,0)): 10 wide strip = 600.
        let c = parallel(&l, 5, 600.0).expect("cut");
        assert!((c.part_area - 600.0).abs() < 1e-6);
        let (a, b) = c.segments[0];
        assert!((a.x - 10.0).abs() < 1e-6 && (b.x - 10.0).abs() < 1e-6);
        // 30 wide strip crosses both arms.
        let c = parallel(&l, 0, 30.0 * 20.0).expect("cut");
        assert!((c.part_area - 600.0).abs() < 1e-6);
    }

    #[test]
    fn swing_cut() {
        // Hinge at the south-west corner; a line to the east side cuts a triangle.
        // 1000 = ½·100·h → h = 20 on the east side, or 1000 off the other side.
        let sols = swing(&rect(), DVec2::ZERO, 1000.0).expect("cut");
        assert!(!sols.is_empty());
        for (_, c) in &sols {
            assert!((c.part_area - 1000.0).abs() < 1e-5, "{}", c.part_area);
            assert!((c.part_area + c.remainder_area - 5000.0).abs() < 1e-5);
        }
        let east = sols
            .iter()
            .find(|(_, c)| {
                c.segments
                    .iter()
                    .any(|(a, b)| (a.x - 100.0).abs() < 1e-6 || (b.x - 100.0).abs() < 1e-6)
            })
            .expect("a cut to the east side");
        let end = if east.1.segments[0].0.length() > 1e-6 {
            east.1.segments[0].0
        } else {
            east.1.segments[0].1
        };
        assert!((end - DVec2::new(100.0, 20.0)).length() < 1e-5, "{end:?}");
    }
}
