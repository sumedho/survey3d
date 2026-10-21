//! Basic `f64` geometry: bounding boxes and point/segment/ray relationships.

use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

/// Axis-aligned bounding box. An empty box has `min > max`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Aabb3 {
    pub min: DVec3,
    pub max: DVec3,
}

impl Default for Aabb3 {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Aabb3 {
    pub const EMPTY: Aabb3 = Aabb3 {
        min: DVec3::splat(f64::INFINITY),
        max: DVec3::splat(f64::NEG_INFINITY),
    };

    pub fn new(min: DVec3, max: DVec3) -> Self {
        Self { min, max }
    }

    pub fn from_point(p: DVec3) -> Self {
        Self { min: p, max: p }
    }

    pub fn from_points(points: impl IntoIterator<Item = DVec3>) -> Self {
        let mut b = Self::EMPTY;
        for p in points {
            b.extend(p);
        }
        b
    }

    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y || self.min.z > self.max.z
    }

    pub fn extend(&mut self, p: DVec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    pub fn union(&self, other: &Aabb3) -> Aabb3 {
        Aabb3 {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    pub fn expanded(&self, margin: f64) -> Aabb3 {
        Aabb3 {
            min: self.min - DVec3::splat(margin),
            max: self.max + DVec3::splat(margin),
        }
    }

    pub fn intersects(&self, other: &Aabb3) -> bool {
        self.min.cmple(other.max).all() && other.min.cmple(self.max).all()
    }

    pub fn contains(&self, p: DVec3) -> bool {
        self.min.cmple(p).all() && p.cmple(self.max).all()
    }

    pub fn center(&self) -> DVec3 {
        (self.min + self.max) * 0.5
    }

    pub fn size(&self) -> DVec3 {
        self.max - self.min
    }

    pub fn corners(&self) -> [DVec3; 8] {
        let (a, b) = (self.min, self.max);
        [
            DVec3::new(a.x, a.y, a.z),
            DVec3::new(b.x, a.y, a.z),
            DVec3::new(a.x, b.y, a.z),
            DVec3::new(b.x, b.y, a.z),
            DVec3::new(a.x, a.y, b.z),
            DVec3::new(b.x, a.y, b.z),
            DVec3::new(a.x, b.y, b.z),
            DVec3::new(b.x, b.y, b.z),
        ]
    }
}

/// Closest point to `p` on segment `a–b`. Returns `(t, point)` with `t ∈ [0, 1]`.
pub fn closest_point_on_segment(p: DVec3, a: DVec3, b: DVec3) -> (f64, DVec3) {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 == 0.0 {
        return (0.0, a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (t, a + ab * t)
}

/// Intersection of 2D segments `a–b` and `c–d`. Returns parameters `(t, u)` along
/// each segment (both in `[0, 1]` when the segments actually cross), or `None` if parallel.
pub fn line_intersection_2d(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> Option<(f64, f64)> {
    let r = b - a;
    let s = d - c;
    let denom = r.perp_dot(s);
    let scale = r.length() * s.length();
    if scale == 0.0 || denom.abs() <= scale * 1e-12 {
        return None;
    }
    let ac = c - a;
    Some((ac.perp_dot(s) / denom, ac.perp_dot(r) / denom))
}

/// Closest approach between the ray `origin + t·dir` (`t ≥ 0`) and segment `a–b`.
/// Returns `(t_ray, s_segment, distance)`.
pub fn closest_ray_segment(origin: DVec3, dir: DVec3, a: DVec3, b: DVec3) -> (f64, f64, f64) {
    let d1 = dir;
    let d2 = b - a;
    let r = origin - a;
    let a11 = d1.dot(d1);
    let a22 = d2.dot(d2);
    let a12 = d1.dot(d2);
    let b1 = d1.dot(r);
    let b2 = d2.dot(r);
    let denom = a11 * a22 - a12 * a12;

    let mut s = if a22 == 0.0 {
        0.0
    } else if denom.abs() > 1e-15 * a11 * a22 {
        ((a11 * b2 - a12 * b1) / denom).clamp(0.0, 1.0)
    } else {
        0.0
    };
    // Re-solve the ray parameter for the clamped segment parameter, then re-clamp.
    let mut t = ((a12 * s - b1) / a11).max(0.0);
    if a22 > 0.0 {
        s = ((t * a12 + b2) / a22).clamp(0.0, 1.0);
        t = ((a12 * s - b1) / a11).max(0.0);
    }
    let p_ray = origin + d1 * t;
    let p_seg = a + d2 * s;
    (t, s, p_ray.distance(p_seg))
}

/// Total 3D length of a polyline.
pub fn polyline_length(vertices: &[DVec3], closed: bool) -> f64 {
    segments(vertices, closed).map(|(a, b)| a.distance(b)).sum()
}

/// Total plan (XY) length of a polyline.
pub fn polyline_length_2d(vertices: &[DVec3], closed: bool) -> f64 {
    segments(vertices, closed)
        .map(|(a, b)| a.truncate().distance(b.truncate()))
        .sum()
}

/// Signed plan area by the shoelace formula (positive = anticlockwise).
/// Coordinates are taken relative to the first vertex to preserve precision at large offsets.
pub fn polygon_area_signed(vertices: &[DVec3]) -> f64 {
    let Some(first) = vertices.first() else {
        return 0.0;
    };
    let o = first.truncate();
    let n = vertices.len();
    let mut sum = 0.0;
    for i in 0..n {
        let p = vertices[i].truncate() - o;
        let q = vertices[(i + 1) % n].truncate() - o;
        sum += p.perp_dot(q);
    }
    sum * 0.5
}

/// Iterates the segments of a polyline.
pub fn segments(vertices: &[DVec3], closed: bool) -> impl Iterator<Item = (DVec3, DVec3)> + '_ {
    let n = vertices.len();
    let count = if closed && n > 2 { n } else { n.saturating_sub(1) };
    (0..count).map(move |i| (vertices[i], vertices[(i + 1) % n]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aabb_basics() {
        let b = Aabb3::from_points([DVec3::ZERO, DVec3::new(2.0, 3.0, 4.0)]);
        assert!(!b.is_empty());
        assert!(Aabb3::EMPTY.is_empty());
        assert_eq!(b.center(), DVec3::new(1.0, 1.5, 2.0));
        assert!(b.contains(DVec3::ONE));
        assert!(!b.intersects(&Aabb3::from_point(DVec3::splat(5.0))));
    }

    #[test]
    fn segment_intersection() {
        let (t, u) = line_intersection_2d(
            DVec2::new(0.0, 0.0),
            DVec2::new(2.0, 2.0),
            DVec2::new(0.0, 2.0),
            DVec2::new(2.0, 0.0),
        )
        .expect("crossing");
        assert!((t - 0.5).abs() < 1e-12 && (u - 0.5).abs() < 1e-12);
        assert!(line_intersection_2d(DVec2::ZERO, DVec2::X, DVec2::Y, DVec2::new(1.0, 1.0)).is_none());
    }

    #[test]
    fn ray_segment() {
        // Ray straight down at x=1 over a segment along x.
        let (t, s, d) = closest_ray_segment(
            DVec3::new(1.0, 0.0, 10.0),
            DVec3::NEG_Z,
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(4.0, 0.0, 0.0),
        );
        assert!((t - 10.0).abs() < 1e-9 && (s - 0.25).abs() < 1e-12 && d < 1e-9);
    }

    #[test]
    fn area_and_length_at_large_offsets() {
        let o = DVec3::new(500_000.0, 7_000_000.0, 0.0);
        let square: Vec<DVec3> = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
            .iter()
            .map(|&(x, y)| o + DVec3::new(x, y, 0.0))
            .collect();
        assert!((polygon_area_signed(&square) - 100.0).abs() < 1e-9);
        assert!((polyline_length_2d(&square, true) - 40.0).abs() < 1e-9);
        assert!((polyline_length(&square, false) - 30.0).abs() < 1e-9);
    }
}
