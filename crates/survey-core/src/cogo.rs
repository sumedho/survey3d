//! Coordinate geometry (COGO) computations.

use glam::{DVec2, DVec3};

use crate::angle::Bearing;

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
}
