//! Clothoid (Euler spiral) transitions: curvature grows linearly from 0 at
//! the tangent (TS) to 1/R where it meets the circular arc (SC), so
//! A² = R·Ls. Coordinates use the standard series in the spiral angle
//! θ = l²/(2·R·Ls), summed to full precision.
//!
//! [`fit`] places a spiral–arc–spiral between two tangent lines meeting at an
//! intersection point (PI): the arc is shifted inwards by the spirals' shift
//! p, and the tangent points move back from the PI by k.

use glam::DVec2;

/// Local coordinates of the point `l` along a spiral of `length` that
/// reaches `radius`: x along the start tangent, y towards the turn.
pub fn clothoid(l: f64, radius: f64, length: f64) -> DVec2 {
    if l <= 0.0 || length <= 0.0 {
        return DVec2::ZERO;
    }
    let theta = l * l / (2.0 * radius * length);
    let (mut x, mut y) = (0.0, 0.0);
    // Term n of each series: (−1)ⁿ θ²ⁿ / ((4n+1)(2n)!) and (−1)ⁿ θ²ⁿ⁺¹ / ((4n+3)(2n+1)!).
    let mut power = 1.0; // θ^(2n) / (2n)!
    for n in 0..30 {
        let n_f = n as f64;
        let sign = if n % 2 == 0 { 1.0 } else { -1.0 };
        let tx = sign * power / (4.0 * n_f + 1.0);
        let odd = power * theta / (2.0 * n_f + 1.0); // θ^(2n+1) / (2n+1)!
        let ty = sign * odd / (4.0 * n_f + 3.0);
        x += tx;
        y += ty;
        if tx.abs() < 1e-17 && ty.abs() < 1e-17 {
            break;
        }
        power = odd * theta / (2.0 * n_f + 2.0);
    }
    DVec2::new(l * x, l * y)
}

/// The elements of one spiral of length `length` into radius `radius`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spiral {
    pub length: f64,
    pub radius: f64,
    /// Spiral angle θs = Ls / 2R (radians).
    pub theta: f64,
    /// Coordinates of the SC from the TS along and square to the tangent.
    pub x: f64,
    pub y: f64,
    /// Shift of the circular arc inwards from the tangent.
    pub p: f64,
    /// Distance along the tangent from the TS to the shifted arc's tangent point.
    pub k: f64,
    /// Long and short tangents (TS and SC to the spiral's own intersection point).
    pub long_tangent: f64,
    pub short_tangent: f64,
    /// TS to SC.
    pub long_chord: f64,
    /// Deflection of the long chord from the tangent at the TS (radians).
    pub deflection: f64,
    /// Clothoid parameter A = √(R·Ls).
    pub a: f64,
}

impl Spiral {
    /// A spiral (a length of 0 gives none: every element 0).
    pub fn new(radius: f64, length: f64) -> Self {
        if length <= 0.0 {
            return Self {
                radius,
                ..Self::default()
            };
        }
        let theta = length / (2.0 * radius);
        let c = clothoid(length, radius, length);
        Self {
            length,
            radius,
            theta,
            x: c.x,
            y: c.y,
            p: c.y - radius * (1.0 - theta.cos()),
            k: c.x - radius * theta.sin(),
            long_tangent: c.x - c.y / theta.tan(),
            short_tangent: c.y / theta.sin(),
            long_chord: c.length(),
            deflection: c.y.atan2(c.x),
            a: (radius * length).sqrt(),
        }
    }
}

/// A spiral–arc–spiral between two tangents.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transitioned {
    pub pi: DVec2,
    pub ts: DVec2,
    pub sc: DVec2,
    pub cs: DVec2,
    pub st: DVec2,
    /// Centre of the (shifted) circular arc.
    pub centre: DVec2,
    pub radius: f64,
    /// Turns left (anticlockwise).
    pub left: bool,
    /// Deflection between the tangents (radians, positive).
    pub delta: f64,
    /// Central angle of the circular arc (Δ − θs₁ − θs₂).
    pub delta_c: f64,
    pub entry: Spiral,
    pub exit: Spiral,
    /// PI back to the TS, and PI on to the ST.
    pub t1: f64,
    pub t2: f64,
}

impl Transitioned {
    pub fn arc_length(&self) -> f64 {
        self.radius * self.delta_c
    }

    pub fn total_length(&self) -> f64 {
        self.entry.length + self.arc_length() + self.exit.length
    }
}

/// Left-hand normal.
fn perp(v: DVec2) -> DVec2 {
    DVec2::new(-v.y, v.x)
}

/// Fits a spiral–arc–spiral of `radius` with spirals `entry` and `exit` long
/// (either may be 0) between the tangent arriving at `pi` along `back` and
/// the one leaving it along `forward` (unit directions).
pub fn fit(
    pi: DVec2,
    back: DVec2,
    forward: DVec2,
    radius: f64,
    entry: f64,
    exit: f64,
) -> Result<Transitioned, String> {
    if radius.is_nan() || radius <= 0.0 || entry < 0.0 || exit < 0.0 {
        return Err("The radius must be positive and the spiral lengths 0 or more".into());
    }
    let turn = back.perp_dot(forward).atan2(back.dot(forward));
    let delta = turn.abs();
    if delta < 1e-9 {
        return Err("The tangents are in line: there is no curve to transition".into());
    }
    if delta > std::f64::consts::PI - 1e-6 {
        return Err("The tangents turn back on themselves".into());
    }
    let left = turn > 0.0;
    let side = if left { 1.0 } else { -1.0 };
    let (s1, s2) = (Spiral::new(radius, entry), Spiral::new(radius, exit));
    let delta_c = delta - s1.theta - s2.theta;
    if delta_c < -1e-12 {
        return Err(format!(
            "The spirals turn {:.4}° but the tangents only {:.4}°: shorten them or increase the radius",
            (s1.theta + s2.theta).to_degrees(),
            delta.to_degrees()
        ));
    }
    let (n1, n2) = (perp(back) * side, perp(forward) * side);
    // The centre is R + p from each tangent, on the inside.
    let (o1, o2) = (pi + n1 * (radius + s1.p), pi + n2 * (radius + s2.p));
    let cross = back.perp_dot(forward);
    let t = (o2 - o1).perp_dot(forward) / cross;
    let centre = o1 + back * t;
    let f1 = centre - n1 * (radius + s1.p);
    let f2 = centre - n2 * (radius + s2.p);
    let ts = f1 - back * s1.k;
    let st = f2 + forward * s2.k;
    let sc = ts + back * s1.x + perp(back) * side * s1.y;
    let cs = st - forward * s2.x + perp(forward) * side * s2.y;
    Ok(Transitioned {
        pi,
        ts,
        sc,
        cs,
        st,
        centre,
        radius,
        left,
        delta,
        delta_c: delta_c.max(0.0),
        entry: s1,
        exit: s2,
        t1: (pi - ts).dot(back),
        t2: (st - pi).dot(forward),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clothoid_series() {
        // Short spirals are nearly straight; θ small gives x ≈ l, y ≈ l·θ/3.
        let c = clothoid(10.0, 1000.0, 10.0);
        assert!(
            (c.x - 10.0).abs() < 1e-4 && (c.y - 10.0 * 0.005 / 3.0).abs() < 1e-7,
            "{c}"
        );
        // Against a numeric integral of cos and sin of θ(s) = s²/(2RL).
        let (r, l) = (50.0, 80.0);
        let n = 200_000;
        let (mut x, mut y) = (0.0, 0.0);
        for i in 0..n {
            let s = (i as f64 + 0.5) * l / n as f64;
            let th = s * s / (2.0 * r * l);
            x += th.cos() * l / n as f64;
            y += th.sin() * l / n as f64;
        }
        let c = clothoid(l, r, l);
        assert!((c.x - x).abs() < 1e-6 && (c.y - y).abs() < 1e-6, "{c} vs {x} {y}");
    }

    #[test]
    fn symmetric_spiral_curve_spiral() {
        // Tangents north then 40° right, R 300, Ls 60.
        let pi = DVec2::new(1000.0, 2000.0);
        let back = DVec2::Y;
        let forward = DVec2::from_angle(50f64.to_radians());
        let f = fit(pi, back, forward, 300.0, 60.0, 60.0).expect("fit");
        assert!(!f.left);
        let s = f.entry;
        assert!((s.theta - 0.1).abs() < 1e-12);
        // Textbook: Ts = (R + p)·tan(Δ/2) + k.
        let ts = (300.0 + s.p) * (20f64.to_radians()).tan() + s.k;
        assert!(
            (f.t1 - ts).abs() < 1e-9 && (f.t2 - ts).abs() < 1e-9,
            "{} {} {ts}",
            f.t1,
            f.t2
        );
        // p ≈ Ls²/24R, k ≈ Ls/2.
        assert!((s.p - 60.0 * 60.0 / 7200.0).abs() < 0.001 && (s.k - 30.0).abs() < 0.01);
        // The arc runs from SC to CS on the circle.
        assert!((f.sc.distance(f.centre) - 300.0).abs() < 1e-9);
        assert!((f.cs.distance(f.centre) - 300.0).abs() < 1e-9);
        assert!((f.delta_c - (40f64.to_radians() - 0.2)).abs() < 1e-12);
        assert!((f.total_length() - (300.0 * f.delta_c + 120.0)).abs() < 1e-9);
        // TS on the back tangent, ST on the forward one.
        assert!((f.ts - (pi - back * f.t1)).length() < 1e-9);
        assert!((f.st - (pi + forward * f.t2)).length() < 1e-9);
    }

    #[test]
    fn no_spirals_is_a_plain_curve_and_errors() {
        let pi = DVec2::ZERO;
        let f = fit(pi, DVec2::X, DVec2::Y, 20.0, 0.0, 0.0).expect("plain");
        // A left quarter circle: tangent length R.
        assert!(f.left && (f.t1 - 20.0).abs() < 1e-9 && (f.centre - DVec2::new(-20.0, 20.0)).length() < 1e-9);
        assert_eq!(f.ts, f.sc);
        // Unequal spirals still meet the circle.
        let f = fit(pi, DVec2::X, DVec2::Y, 100.0, 30.0, 50.0).expect("unequal");
        assert!(
            (f.sc.distance(f.centre) - 100.0).abs() < 1e-9 && (f.cs.distance(f.centre) - 100.0).abs() < 1e-9
        );
        assert!(f.t1 != f.t2);
        assert!(
            fit(pi, DVec2::X, DVec2::Y, 20.0, 40.0, 40.0).is_err(),
            "spirals longer than the turn"
        );
        assert!(fit(pi, DVec2::X, DVec2::X, 20.0, 5.0, 5.0).is_err(), "in line");
    }
}
