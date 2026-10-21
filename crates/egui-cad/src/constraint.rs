//! Direction constraints (ortho / polar tracking).

use glam::DVec3;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrthoSettings {
    /// Toggle (F8).
    pub enabled: bool,
    /// Allowed direction increment in degrees: 90 = ortho, 45, 15, …
    pub increment_deg: f64,
}

impl Default for OrthoSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            increment_deg: 90.0,
        }
    }
}

/// Constrains `p` so that the plan direction `base → p` is a multiple of
/// `increment` (radians) measured from `reference` (a bearing, radians).
/// The distance along the constrained direction is the projection of `p`; the
/// height of `p` is kept.
pub fn constrain_direction(base: DVec3, p: DVec3, increment: f64, reference: f64) -> DVec3 {
    let d = (p - base).truncate();
    if d.length_squared() == 0.0 || increment <= 0.0 {
        return p;
    }
    // Bearing: clockwise from north (+Y).
    let bearing = d.x.atan2(d.y);
    let snapped = reference + ((bearing - reference) / increment).round() * increment;
    let dir = glam::DVec2::new(snapped.sin(), snapped.cos());
    let along = d.dot(dir);
    (base.truncate() + dir * along).extend(p.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ortho_and_polar() {
        let base = DVec3::new(500_000.0, 7_000_000.0, 10.0);
        let p = base + DVec3::new(10.0, 1.0, 3.0);
        let c = constrain_direction(base, p, 90f64.to_radians(), 0.0);
        assert!((c - (base + DVec3::new(10.0, 0.0, 3.0))).length() < 1e-9);

        let p = base + DVec3::new(10.0, 9.0, 0.0);
        let c = constrain_direction(base, p, 45f64.to_radians(), 0.0);
        let d = c - base;
        assert!((d.x - d.y).abs() < 1e-9, "snapped to 45°");

        // A rotated reference (e.g. a rotated plan view) shifts the allowed directions.
        let p = base + DVec3::new(0.0, 10.0, 0.0);
        let c = constrain_direction(base, p, 90f64.to_radians(), 30f64.to_radians());
        let d = (c - base).truncate();
        let bearing = d.x.atan2(d.y).to_degrees();
        assert!((bearing - 30.0).abs() < 1e-9 || (bearing + 60.0).abs() < 1e-9);
    }
}
