//! Circular arc segments in plan, expressed with DXF-style *bulges*.
//!
//! A polyline segment from `a` to `b` with bulge `β` is a circular arc whose
//! included angle `θ` satisfies `β = tan(θ/4)`. Positive bulges sweep
//! counter-clockwise (the arc bulges to the right of the travel direction),
//! negative bulges clockwise, and `β = 0` is a straight line. `|β| = 1` is a
//! semicircle. Heights vary linearly along the arc.

use std::f64::consts::TAU;

use glam::{DVec2, DVec3};

use crate::geom::line_intersection_2d;

/// Bulges smaller than this are treated as straight.
pub const STRAIGHT_BULGE: f64 = 1e-12;

/// An arc in plan, with linearly varying height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arc {
    pub center: DVec2,
    pub radius: f64,
    /// Polar angle (radians, from +X, anticlockwise) of the start point.
    pub start_angle: f64,
    /// Signed sweep: positive anticlockwise.
    pub sweep: f64,
    pub z_start: f64,
    pub z_end: f64,
}

impl Arc {
    /// The arc for segment `a → b` with `bulge`, or `None` for a straight or degenerate segment.
    pub fn from_bulge(a: DVec3, b: DVec3, bulge: f64) -> Option<Arc> {
        if bulge.abs() < STRAIGHT_BULGE {
            return None;
        }
        let (p, q) = (a.truncate(), b.truncate());
        let chord = q - p;
        let len = chord.length();
        if len == 0.0 {
            return None;
        }
        let left = chord.perp() / len;
        let center = (p + q) * 0.5 + left * ((1.0 - bulge * bulge) / (4.0 * bulge) * len);
        let radius = len * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
        let d = p - center;
        Some(Arc {
            center,
            radius,
            start_angle: d.y.atan2(d.x),
            sweep: 4.0 * bulge.atan(),
            z_start: a.z,
            z_end: b.z,
        })
    }

    /// Bulge of the arc from `a` through `m` to `b`; `None` if the points are collinear.
    pub fn bulge_through(a: DVec3, m: DVec3, b: DVec3) -> Option<f64> {
        let (p, mid_pt, q) = (a.truncate(), m.truncate(), b.truncate());
        let chord = q - p;
        let len = chord.length();
        let side = chord.perp_dot(mid_pt - p);
        if len == 0.0 || side.abs() <= 1e-12 * len * len {
            return None;
        }
        let center = circumcenter(p, mid_pt, q)?;
        let radius = center.distance(p);
        let mid = (p + q) * 0.5;
        // Unit normal from the chord towards the arc's middle (the side `m` is on).
        let toward = chord.perp().normalize() * side.signum();
        let sagitta = radius + (center - mid).dot(toward);
        // Arc on the left of travel is clockwise → negative bulge.
        Some(-side.signum() * sagitta / (len * 0.5))
    }

    pub fn end_angle(&self) -> f64 {
        self.start_angle + self.sweep
    }

    /// Point at parameter `t ∈ [0, 1]` along the arc.
    pub fn point_at(&self, t: f64) -> DVec3 {
        let ang = self.start_angle + self.sweep * t;
        (self.center + DVec2::from_angle(ang) * self.radius)
            .extend(self.z_start + (self.z_end - self.z_start) * t)
    }

    pub fn midpoint(&self) -> DVec3 {
        self.point_at(0.5)
    }

    /// Unit direction of travel (plan) at parameter `t`.
    pub fn tangent_at(&self, t: f64) -> DVec2 {
        DVec2::from_angle(self.start_angle + self.sweep * t).perp() * self.sweep.signum()
    }

    /// Bulge of the part of this circle swept from `from` to `to` (plan points on the
    /// circle) in this arc's direction; `0.0` when they coincide.
    pub fn bulge_between(&self, from: DVec2, to: DVec2) -> f64 {
        let (u, v) = (from - self.center, to - self.center);
        let (a0, a1) = (u.y.atan2(u.x), v.y.atan2(v.x));
        let sweep = if self.sweep >= 0.0 {
            (a1 - a0).rem_euclid(TAU)
        } else {
            -(a0 - a1).rem_euclid(TAU)
        };
        (sweep / 4.0).tan()
    }

    /// Plan length.
    pub fn length_2d(&self) -> f64 {
        self.radius * self.sweep.abs()
    }

    /// Length along the (helical) arc.
    pub fn length_3d(&self) -> f64 {
        self.length_2d().hypot(self.z_end - self.z_start)
    }

    /// Signed area between the chord and the arc (positive for anticlockwise sweeps).
    pub fn segment_area(&self) -> f64 {
        let t = self.sweep.abs();
        self.sweep.signum() * 0.5 * self.radius * self.radius * (t - t.sin())
    }

    pub fn center3(&self) -> DVec3 {
        self.center.extend((self.z_start + self.z_end) * 0.5)
    }

    /// Parameter `t ∈ [0, 1]` of the polar angle `ang`, if it lies on the arc.
    pub fn param_of_angle(&self, ang: f64) -> Option<f64> {
        let rel = if self.sweep >= 0.0 {
            (ang - self.start_angle).rem_euclid(TAU)
        } else {
            (self.start_angle - ang).rem_euclid(TAU)
        };
        let t = rel / self.sweep.abs();
        (t <= 1.0 + 1e-12).then(|| t.min(1.0))
    }

    /// Closest point on the arc (in plan) to `p`, with its height.
    pub fn closest_point(&self, p: DVec2) -> DVec3 {
        let d = p - self.center;
        if d.length_squared() > 0.0
            && let Some(t) = self.param_of_angle(d.y.atan2(d.x))
        {
            return self.point_at(t);
        }
        let (a, b) = (self.point_at(0.0), self.point_at(1.0));
        if a.truncate().distance(p) <= b.truncate().distance(p) {
            a
        } else {
            b
        }
    }

    /// Points where the line through `a–b` meets the arc (plan), with the line parameter.
    pub fn intersect_line(&self, a: DVec2, b: DVec2) -> Vec<(f64, DVec3)> {
        let d = b - a;
        let f = a - self.center;
        let (qa, qb, qc) = (d.dot(d), 2.0 * f.dot(d), f.dot(f) - self.radius * self.radius);
        let disc = qb * qb - 4.0 * qa * qc;
        if qa == 0.0 || disc < 0.0 {
            return Vec::new();
        }
        let s = disc.sqrt();
        let mut out = Vec::new();
        for u in [(-qb - s) / (2.0 * qa), (-qb + s) / (2.0 * qa)] {
            let p = a + d * u;
            let v = p - self.center;
            if let Some(t) = self.param_of_angle(v.y.atan2(v.x)) {
                let z = self.z_start + (self.z_end - self.z_start) * t;
                out.push((u, p.extend(z)));
            }
            if s == 0.0 {
                break;
            }
        }
        out
    }

    /// Intersections with another arc (plan); heights from `self`.
    pub fn intersect_arc(&self, other: &Arc) -> Vec<DVec3> {
        let d = other.center - self.center;
        let dist = d.length();
        let (r0, r1) = (self.radius, other.radius);
        if dist == 0.0 || dist > r0 + r1 || dist < (r0 - r1).abs() {
            return Vec::new();
        }
        let a = (r0 * r0 - r1 * r1 + dist * dist) / (2.0 * dist);
        let h = (r0 * r0 - a * a).max(0.0).sqrt();
        let base = self.center + d * (a / dist);
        let off = d.perp() * (h / dist);
        let mut out = Vec::new();
        for p in [base + off, base - off] {
            let (v0, v1) = (p - self.center, p - other.center);
            if let (Some(t), Some(_)) = (
                self.param_of_angle(v0.y.atan2(v0.x)),
                other.param_of_angle(v1.y.atan2(v1.x)),
            ) {
                out.push(p.extend(self.z_start + (self.z_end - self.z_start) * t));
            }
            if h == 0.0 {
                break;
            }
        }
        out
    }

    /// Polyline approximation with chord error at most `tolerance` (plan units).
    pub fn tessellate(&self, tolerance: f64) -> Vec<DVec3> {
        let tol = tolerance.clamp(1e-9, self.radius);
        let max_step = 2.0 * (1.0 - tol / self.radius).clamp(-1.0, 1.0).acos();
        let n = ((self.sweep.abs() / max_step.max(1e-6)).ceil() as usize).clamp(2, 512);
        (0..=n).map(|i| self.point_at(i as f64 / n as f64)).collect()
    }
}

/// Bulge of the arc that leaves `start` in direction `tangent` and ends at `end`;
/// `None` when the end lies straight behind the start (no such arc).
pub fn tangent_bulge(start: DVec2, tangent: DVec2, end: DVec2) -> Option<f64> {
    let chord = end - start;
    if chord.length_squared() == 0.0 || tangent.length_squared() == 0.0 {
        return None;
    }
    // The included angle is twice the angle between tangent and chord.
    let phi = tangent.perp_dot(chord).atan2(tangent.dot(chord));
    (phi.abs() < std::f64::consts::PI - 1e-9).then(|| (phi / 2.0).tan())
}

/// Direction of travel (plan, not normalised) leaving the end of segment `a → b`.
pub fn end_direction(a: DVec3, b: DVec3, bulge: f64) -> DVec2 {
    match Arc::from_bulge(a, b, bulge) {
        Some(arc) => arc.tangent_at(1.0),
        None => (b - a).truncate(),
    }
}

/// Reverses a polyline's direction: vertex order and segment bulges.
pub fn reverse_bulges(vertices: &mut [DVec3], bulges: &mut Vec<f64>, closed: bool) {
    let n = vertices.len();
    vertices.reverse();
    if bulges.iter().all(|b| b.abs() < STRAIGHT_BULGE) {
        bulges.clear();
        return;
    }
    let old: Vec<f64> = (0..n).map(|i| bulges.get(i).copied().unwrap_or(0.0)).collect();
    // New segment i runs along old segment n-2-i backwards (wrapping for the closing segment).
    *bulges = (0..n)
        .map(|i| {
            let j = (n as isize - 2 - i as isize).rem_euclid(n as isize) as usize;
            if !closed && i == n - 1 { 0.0 } else { -old[j] }
        })
        .collect();
}

fn circumcenter(a: DVec2, b: DVec2, c: DVec2) -> Option<DVec2> {
    // Intersection of the perpendicular bisectors of ab and bc.
    let (m1, m2) = ((a + b) * 0.5, (b + c) * 0.5);
    let (d1, d2) = ((b - a).perp(), (c - b).perp());
    let (t, _) = line_intersection_2d(m1, m1 + d1, m2, m2 + d2)?;
    Some(m1 + d1 * t)
}

/// Iterates a polyline as `(start, end, bulge)` segments.
pub fn bulge_segments<'a>(
    vertices: &'a [DVec3],
    bulges: &'a [f64],
    closed: bool,
) -> impl Iterator<Item = (DVec3, DVec3, f64)> + 'a {
    let n = vertices.len();
    let count = if closed && n > 2 || closed && n == 2 && bulges.iter().any(|b| b.abs() > STRAIGHT_BULGE) {
        n
    } else {
        n.saturating_sub(1)
    };
    (0..count).map(move |i| {
        (
            vertices[i],
            vertices[(i + 1) % n],
            bulges.get(i).copied().unwrap_or(0.0),
        )
    })
}

/// Plan length, 3D length and signed plan area (positive anticlockwise; only
/// meaningful when `closed`) of a polyline with bulges.
pub fn polyline_metrics(vertices: &[DVec3], bulges: &[f64], closed: bool) -> (f64, f64, f64) {
    let Some(first) = vertices.first() else {
        return (0.0, 0.0, 0.0);
    };
    // Shoelace relative to the first vertex keeps precision at large coordinates.
    let o = first.truncate();
    let (mut len2, mut len3, mut area) = (0.0, 0.0, 0.0);
    for (a, b, bulge) in bulge_segments(vertices, bulges, closed) {
        area += 0.5 * (a.truncate() - o).perp_dot(b.truncate() - o);
        match Arc::from_bulge(a, b, bulge) {
            Some(arc) => {
                len2 += arc.length_2d();
                len3 += arc.length_3d();
                area += arc.segment_area();
            }
            None => {
                len2 += a.truncate().distance(b.truncate());
                len3 += a.distance(b);
            }
        }
    }
    (len2, len3, area)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn v(x: f64, y: f64) -> DVec3 {
        DVec3::new(x, y, 0.0)
    }

    #[test]
    fn semicircle_from_bulge() {
        // From (0,0) to (2,0) anticlockwise: centre (1,0), dips below the chord.
        let arc = Arc::from_bulge(v(0.0, 0.0), v(2.0, 0.0), 1.0).expect("arc");
        assert!((arc.center - DVec2::new(1.0, 0.0)).length() < 1e-12);
        assert!((arc.radius - 1.0).abs() < 1e-12);
        assert!((arc.sweep - PI).abs() < 1e-12);
        assert!((arc.midpoint() - v(1.0, -1.0)).length() < 1e-12);
        assert!((arc.point_at(1.0) - v(2.0, 0.0)).length() < 1e-12);
        assert!((arc.length_2d() - PI).abs() < 1e-12);
    }

    #[test]
    fn three_point_bulge_round_trips() {
        let o = DVec3::new(500_000.0, 7_000_000.0, 0.0);
        for (m, expect_ccw) in [((5.0, -2.0), true), ((5.0, 3.0), false), ((5.0, -8.0), true)] {
            let (a, b) = (o, o + v(10.0, 0.0));
            let mid = o + v(m.0, m.1);
            let bulge = Arc::bulge_through(a, mid, b).expect("not collinear");
            assert_eq!(bulge > 0.0, expect_ccw);
            let arc = Arc::from_bulge(a, b, bulge).expect("arc");
            // The arc passes through the middle point.
            assert!((arc.center.distance(mid.truncate()) - arc.radius).abs() < 1e-7);
        }
        assert!(Arc::bulge_through(v(0.0, 0.0), v(1.0, 0.0), v(2.0, 0.0)).is_none());
    }

    #[test]
    fn segment_area_of_semicircle() {
        let arc = Arc::from_bulge(v(0.0, 0.0), v(2.0, 0.0), 1.0).expect("arc");
        assert!((arc.segment_area() - PI / 2.0).abs() < 1e-12);
    }

    #[test]
    fn line_and_arc_intersections() {
        let arc = Arc::from_bulge(v(-1.0, 0.0), v(1.0, 0.0), -1.0).expect("upper semicircle");
        let hits = arc.intersect_line(DVec2::new(-2.0, 0.5), DVec2::new(2.0, 0.5));
        assert_eq!(hits.len(), 2);
        let other = Arc::from_bulge(v(0.0, -1.0), v(0.0, 1.0), 1.0).expect("right semicircle");
        let pts = arc.intersect_arc(&other);
        // Unit circles centred at the origin coincide → treated as no proper intersection.
        assert!(pts.is_empty() || pts.iter().all(|p| (p.truncate().length() - 1.0).abs() < 1e-9));
        // Left half of a unit circle centred at (1, 0).
        let shifted = Arc {
            center: DVec2::new(1.0, 0.0),
            start_angle: PI / 2.0,
            ..other
        };
        let pts = arc.intersect_arc(&shifted);
        assert_eq!(pts.len(), 1, "{pts:?}");
        assert!((pts[0].truncate() - DVec2::new(0.5, 0.75f64.sqrt())).length() < 1e-9);
    }

    #[test]
    fn circle_metrics() {
        // A full circle as two semicircles: r = 5.
        let (l2, l3, area) = polyline_metrics(&[v(0.0, 0.0), v(10.0, 0.0)], &[1.0, 1.0], true);
        assert!((l2 - 10.0 * PI).abs() < 1e-9 && (l3 - l2).abs() < 1e-12);
        assert!((area - 25.0 * PI).abs() < 1e-9, "{area}");
        // Square with one outward-bulging semicircular side.
        let sq = [v(0.0, 0.0), v(2.0, 0.0), v(2.0, 2.0), v(0.0, 2.0)];
        let (_, _, a) = polyline_metrics(&sq, &[1.0, 0.0, 0.0, 0.0], true);
        assert!((a - (4.0 + PI / 2.0)).abs() < 1e-9, "{a}");
    }

    #[test]
    fn tangent_arcs() {
        // Heading east from the origin, ending at (10, 10): a quarter circle turning left.
        let bulge = tangent_bulge(DVec2::ZERO, DVec2::X, DVec2::new(10.0, 10.0)).expect("arc");
        let arc = Arc::from_bulge(v(0.0, 0.0), v(10.0, 10.0), bulge).expect("arc");
        assert!((arc.sweep - PI / 2.0).abs() < 1e-12);
        assert!((arc.tangent_at(0.0) - DVec2::X).length() < 1e-12);
        assert!((arc.tangent_at(1.0) - DVec2::Y).length() < 1e-12);
        assert!((end_direction(v(0.0, 0.0), v(10.0, 10.0), bulge).normalize() - DVec2::Y).length() < 1e-12);
        // Straight ahead is a zero bulge; straight behind has no arc.
        assert!(
            tangent_bulge(DVec2::ZERO, DVec2::X, DVec2::new(5.0, 0.0))
                .expect("line")
                .abs()
                < 1e-15
        );
        assert!(tangent_bulge(DVec2::ZERO, DVec2::X, DVec2::new(-5.0, 0.0)).is_none());
    }

    #[test]
    fn reversing_keeps_geometry() {
        let mut verts = vec![v(0.0, 0.0), v(2.0, 0.0), v(2.0, 2.0)];
        let mut bulges = vec![1.0, 0.0, 0.0];
        let (_, _, area) = polyline_metrics(&verts, &bulges, true);
        reverse_bulges(&mut verts, &mut bulges, true);
        let (_, _, reversed) = polyline_metrics(&verts, &bulges, true);
        assert!((area + reversed).abs() < 1e-12, "{area} {reversed}");
        let mut open_v = vec![v(0.0, 0.0), v(2.0, 0.0), v(4.0, 0.0)];
        let mut open_b = vec![0.5, -0.25, 0.0];
        reverse_bulges(&mut open_v, &mut open_b, false);
        assert_eq!(open_b, vec![0.25, -0.5, 0.0]);
    }

    #[test]
    fn tessellation_respects_tolerance() {
        let arc = Arc::from_bulge(v(0.0, 0.0), v(100.0, 0.0), 0.5).expect("arc");
        let pts = arc.tessellate(0.001);
        for w in pts.windows(2) {
            let mid = (w[0] + w[1]) * 0.5;
            let err = arc.radius - mid.truncate().distance(arc.center);
            assert!(err <= 0.001 + 1e-9);
        }
        assert!((pts[0] - v(0.0, 0.0)).length() < 1e-9);
        assert!((pts.last().copied().unwrap_or_default() - v(100.0, 0.0)).length() < 1e-9);
    }
}
