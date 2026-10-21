//! Angles and bearings.
//!
//! [`Angle`] is a plain signed angle in radians. [`Bearing`] is a whole-circle
//! direction measured clockwise from grid north, always normalised to `[0, 2π)`.

use std::f64::consts::{PI, TAU};
use std::ops::{Add, Mul, Neg, Sub};

use glam::DVec2;
use serde::{Deserialize, Serialize};

/// A signed angle stored in radians.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Angle(f64);

/// Degrees / minutes / seconds decomposition of an angle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dms {
    pub negative: bool,
    pub degrees: u32,
    pub minutes: u32,
    pub seconds: f64,
}

impl Angle {
    pub const ZERO: Angle = Angle(0.0);

    pub const fn from_radians(radians: f64) -> Self {
        Self(radians)
    }

    pub fn from_degrees(degrees: f64) -> Self {
        Self(degrees.to_radians())
    }

    /// Gons (grads): 400 per full circle.
    pub fn from_gons(gons: f64) -> Self {
        Self(gons * TAU / 400.0)
    }

    /// Builds an angle from degrees, minutes and seconds. The sign of the result
    /// is taken from `negative`; the components themselves must be non-negative.
    pub fn from_dms(negative: bool, degrees: f64, minutes: f64, seconds: f64) -> Self {
        let magnitude = degrees + minutes / 60.0 + seconds / 3600.0;
        Self::from_degrees(if negative { -magnitude } else { magnitude })
    }

    pub const fn radians(self) -> f64 {
        self.0
    }

    pub fn degrees(self) -> f64 {
        self.0.to_degrees()
    }

    pub fn gons(self) -> f64 {
        self.0 * 400.0 / TAU
    }

    /// Normalised to `[0, 2π)`.
    pub fn normalized(self) -> Self {
        let r = self.0.rem_euclid(TAU);
        // rem_euclid can return TAU itself for tiny negative inputs due to rounding.
        Self(if r >= TAU { 0.0 } else { r })
    }

    /// Normalised to `(-π, π]`.
    pub fn normalized_signed(self) -> Self {
        let r = self.normalized().0;
        Self(if r > PI { r - TAU } else { r })
    }

    /// Splits into degrees/minutes/seconds, rounding seconds to `second_decimals`
    /// places and carrying into minutes/degrees so that `59.9999"` never displays as `60"`.
    pub fn to_dms(self, second_decimals: u8) -> Dms {
        let negative = self.0 < 0.0;
        let scale = 10f64.powi(i32::from(second_decimals));
        // Work in integer units of the smallest displayed second fraction to make the carry exact.
        let total_units = (self.degrees().abs() * 3600.0 * scale).round();
        let units_per_minute = 60.0 * scale;
        let units_per_degree = 3600.0 * scale;
        let degrees = (total_units / units_per_degree).floor();
        let rem = total_units - degrees * units_per_degree;
        let minutes = (rem / units_per_minute).floor();
        let seconds = (rem - minutes * units_per_minute) / scale;
        Dms {
            negative: negative && total_units > 0.0,
            degrees: degrees as u32,
            minutes: minutes as u32,
            seconds,
        }
    }
}

impl Add for Angle {
    type Output = Angle;
    fn add(self, rhs: Angle) -> Angle {
        Angle(self.0 + rhs.0)
    }
}

impl Sub for Angle {
    type Output = Angle;
    fn sub(self, rhs: Angle) -> Angle {
        Angle(self.0 - rhs.0)
    }
}

impl Neg for Angle {
    type Output = Angle;
    fn neg(self) -> Angle {
        Angle(-self.0)
    }
}

impl Mul<f64> for Angle {
    type Output = Angle;
    fn mul(self, rhs: f64) -> Angle {
        Angle(self.0 * rhs)
    }
}

/// Quadrant of a quadrant-style bearing (e.g. `N 45° E`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quadrant {
    NE,
    SE,
    SW,
    NW,
}

impl Quadrant {
    pub fn prefix(self) -> char {
        match self {
            Quadrant::NE | Quadrant::NW => 'N',
            Quadrant::SE | Quadrant::SW => 'S',
        }
    }

    pub fn suffix(self) -> char {
        match self {
            Quadrant::NE | Quadrant::SE => 'E',
            Quadrant::SW | Quadrant::NW => 'W',
        }
    }
}

/// A whole-circle bearing measured clockwise from grid north, in `[0, 2π)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Bearing(Angle);

impl Bearing {
    pub fn new(angle: Angle) -> Self {
        Self(angle.normalized())
    }

    pub fn from_degrees(degrees: f64) -> Self {
        Self::new(Angle::from_degrees(degrees))
    }

    /// Bearing of the vector `(d_east, d_north)`; `None` for a zero-length vector.
    pub fn from_delta(d_east: f64, d_north: f64) -> Option<Self> {
        if d_east == 0.0 && d_north == 0.0 {
            None
        } else {
            Some(Self::new(Angle::from_radians(d_east.atan2(d_north))))
        }
    }

    pub fn from_quadrant(quadrant: Quadrant, angle: Angle) -> Self {
        let a = angle.radians();
        Self::new(Angle::from_radians(match quadrant {
            Quadrant::NE => a,
            Quadrant::SE => PI - a,
            Quadrant::SW => PI + a,
            Quadrant::NW => TAU - a,
        }))
    }

    pub fn angle(self) -> Angle {
        self.0
    }

    pub fn radians(self) -> f64 {
        self.0.radians()
    }

    pub fn degrees(self) -> f64 {
        self.0.degrees()
    }

    /// The opposite direction (back bearing).
    pub fn reverse(self) -> Self {
        Self::new(self.0 + Angle::from_radians(PI))
    }

    /// Unit vector `(east, north)` pointing along this bearing.
    pub fn unit_vector(self) -> DVec2 {
        let (s, c) = self.radians().sin_cos();
        DVec2::new(s, c)
    }

    pub fn to_quadrant(self) -> (Quadrant, Angle) {
        let a = self.radians();
        if a <= PI / 2.0 {
            (Quadrant::NE, Angle::from_radians(a))
        } else if a <= PI {
            (Quadrant::SE, Angle::from_radians(PI - a))
        } else if a <= 1.5 * PI {
            (Quadrant::SW, Angle::from_radians(a - PI))
        } else {
            (Quadrant::NW, Angle::from_radians(TAU - a))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-12;

    #[test]
    fn dms_round_trip() {
        let a = Angle::from_dms(false, 123.0, 45.0, 30.0);
        let d = a.to_dms(0);
        assert_eq!((d.negative, d.degrees, d.minutes), (false, 123, 45));
        assert!((d.seconds - 30.0).abs() < EPS);
    }

    #[test]
    fn dms_carry_on_rounding() {
        // 10°59'59.9999" rounds to 11°00'00" at 0 decimals.
        let a = Angle::from_dms(false, 10.0, 59.0, 59.9999);
        let d = a.to_dms(0);
        assert_eq!((d.degrees, d.minutes), (11, 0));
        assert!(d.seconds.abs() < EPS);
    }

    #[test]
    fn negative_dms() {
        let a = Angle::from_dms(true, 0.0, 30.0, 0.0);
        assert!((a.degrees() + 0.5).abs() < EPS);
        let d = a.to_dms(1);
        assert!(d.negative);
        assert_eq!((d.degrees, d.minutes), (0, 30));
    }

    #[test]
    fn normalization() {
        assert!((Angle::from_degrees(-90.0).normalized().degrees() - 270.0).abs() < 1e-9);
        assert!((Angle::from_degrees(450.0).normalized().degrees() - 90.0).abs() < 1e-9);
        assert!((Angle::from_degrees(270.0).normalized_signed().degrees() + 90.0).abs() < 1e-9);
    }

    #[test]
    fn bearing_from_delta() {
        let cases = [
            ((0.0, 1.0), 0.0),
            ((1.0, 0.0), 90.0),
            ((0.0, -1.0), 180.0),
            ((-1.0, 0.0), 270.0),
            ((1.0, 1.0), 45.0),
        ];
        for ((de, dn), expected) in cases {
            let b = Bearing::from_delta(de, dn).expect("non-zero");
            assert!((b.degrees() - expected).abs() < 1e-9, "{de},{dn}");
        }
        assert!(Bearing::from_delta(0.0, 0.0).is_none());
    }

    #[test]
    fn quadrant_round_trip() {
        for deg in [10.0, 100.0, 200.0, 300.0] {
            let b = Bearing::from_degrees(deg);
            let (q, a) = b.to_quadrant();
            let back = Bearing::from_quadrant(q, a);
            assert!((back.degrees() - deg).abs() < 1e-9);
        }
        let (q, a) = Bearing::from_degrees(135.0).to_quadrant();
        assert_eq!(q, Quadrant::SE);
        assert!((a.degrees() - 45.0).abs() < 1e-9);
    }

    #[test]
    fn reverse_bearing() {
        assert!((Bearing::from_degrees(30.0).reverse().degrees() - 210.0).abs() < 1e-9);
        assert!((Bearing::from_degrees(200.0).reverse().degrees() - 20.0).abs() < 1e-9);
    }
}
