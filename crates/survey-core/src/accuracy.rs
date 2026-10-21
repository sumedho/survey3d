//! User-defined accuracy limits for misclosures.
//!
//! Limits differ between jurisdictions and survey classes, so none are set by
//! default; each limit is optional and only checked when present.

use serde::{Deserialize, Serialize};

use crate::angle::Angle;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tolerances {
    /// Allowed angular misclosure in seconds of arc times √n (n = number of angles).
    pub angular_seconds: Option<f64>,
    /// Minimum precision `N` of `1 : N` (length over linear misclosure).
    pub min_precision: Option<f64>,
    /// Linear misclosures up to this many metres always pass ("1 : N or 20 mm").
    pub linear_floor: Option<f64>,
    /// Allowed vertical misclosure in millimetres times √(length in km).
    pub vertical_mm_per_root_km: Option<f64>,
}

/// The outcome of one check.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Check {
    /// Allowed magnitude (radians for angles, metres for lengths).
    pub allowed: f64,
    pub pass: bool,
}

impl Tolerances {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Angular misclosure over `angles` observed angles.
    pub fn check_angular(&self, misclosure: Angle, angles: usize) -> Option<Check> {
        let k = self.angular_seconds?;
        let allowed = Angle::from_degrees(k * (angles.max(1) as f64).sqrt() / 3600.0).radians();
        Some(Check {
            allowed,
            pass: misclosure.radians().abs() <= allowed + 1e-12,
        })
    }

    /// Linear misclosure over a traverse or figure of total `length`.
    pub fn check_linear(&self, misclosure: f64, length: f64) -> Option<Check> {
        let by_ratio = self.min_precision.filter(|n| *n > 0.0).map(|n| length / n);
        let allowed = match (by_ratio, self.linear_floor) {
            (None, None) => return None,
            (Some(r), None) => r,
            (None, Some(f)) => f,
            (Some(r), Some(f)) => r.max(f),
        };
        Some(Check {
            allowed,
            pass: misclosure.abs() <= allowed + 1e-12,
        })
    }

    /// Vertical misclosure over a run of horizontal `length` (metres).
    pub fn check_vertical(&self, misclosure: f64, length: f64) -> Option<Check> {
        let k = self.vertical_mm_per_root_km?;
        let allowed = k * 1e-3 * (length / 1000.0).max(0.0).sqrt();
        Some(Check {
            allowed,
            pass: misclosure.abs() <= allowed + 1e-12,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_limits_by_default() {
        let t = Tolerances::default();
        assert!(t.is_empty());
        assert!(t.check_angular(Angle::from_degrees(1.0), 4).is_none());
        assert!(t.check_linear(1.0, 100.0).is_none());
        assert!(t.check_vertical(1.0, 100.0).is_none());
    }

    #[test]
    fn checks() {
        let t = Tolerances {
            angular_seconds: Some(10.0),
            min_precision: Some(10_000.0),
            linear_floor: Some(0.02),
            vertical_mm_per_root_km: Some(12.0),
        };
        // 4 angles: 20" allowed.
        let c = t
            .check_angular(Angle::from_degrees(19.0 / 3600.0), 4)
            .expect("set");
        assert!(c.pass && (Angle::from_radians(c.allowed).degrees() * 3600.0 - 20.0).abs() < 1e-9);
        assert!(
            !t.check_angular(Angle::from_degrees(-21.0 / 3600.0), 4)
                .expect("set")
                .pass
        );
        // 100 m: the 20 mm floor wins over 10 mm from the ratio.
        let c = t.check_linear(0.015, 100.0).expect("set");
        assert!(c.pass && (c.allowed - 0.02).abs() < 1e-12);
        // 1 km: the ratio allows 100 mm.
        assert!(t.check_linear(0.09, 1000.0).expect("set").pass);
        assert!(!t.check_linear(0.11, 1000.0).expect("set").pass);
        // 4 km: 24 mm.
        let c = t.check_vertical(-0.02, 4000.0).expect("set");
        assert!(c.pass && (c.allowed - 0.024).abs() < 1e-12);
    }
}
