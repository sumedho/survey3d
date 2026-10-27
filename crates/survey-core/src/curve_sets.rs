//! Curve combinations for the curve solver: a circular curve with equal
//! clothoid spirals each side, compound curves (two arcs turning the same
//! way) and reverse curves (two arcs turning opposite ways between parallel
//! tangents). Angles are radians, lengths metres.

use std::f64::consts::PI;

use crate::solvers::SolveError;
use crate::spiral::Spiral;

/// Roots of `f` on (lo, hi) by sampling and bisection.
fn roots(f: impl Fn(f64) -> f64, lo: f64, hi: f64) -> Vec<f64> {
    const N: usize = 2000;
    let at = |i: usize| lo + (hi - lo) * i as f64 / N as f64;
    let mut out: Vec<f64> = Vec::new();
    let mut prev = f(at(0));
    for i in 1..=N {
        let cur = f(at(i));
        if prev.is_finite() && cur.is_finite() && (prev == 0.0 || (prev < 0.0) != (cur < 0.0)) {
            let (mut a, mut b) = (at(i - 1), at(i));
            for _ in 0..200 {
                let m = 0.5 * (a + b);
                if (f(m) < 0.0) == (prev < 0.0) {
                    a = m;
                } else {
                    b = m;
                }
            }
            let r = 0.5 * (a + b);
            if out.last().is_none_or(|p| (r - p).abs() > 1e-9) {
                out.push(r);
            }
        }
        prev = cur;
    }
    out
}

fn positive(values: &[Option<f64>]) -> Result<(), SolveError> {
    if values.iter().flatten().any(|v| *v <= 0.0 || !v.is_finite()) {
        Err(SolveError::NotPositive)
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------- spiral curves

/// A circular curve with equal spirals each side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpiralCurve {
    pub radius: f64,
    /// Total deflection between the tangents.
    pub delta: f64,
    pub spiral: Spiral,
    /// Central angle of the circular arc (Δ − 2θs).
    pub delta_c: f64,
    pub arc_length: f64,
    /// TS to ST along the curve.
    pub total_length: f64,
    /// PI to the TS (and to the ST).
    pub tangent: f64,
    /// PI to the middle of the arc.
    pub external: f64,
}

/// What is known of a spiral curve: the radius and spiral length, and one
/// of the deflection, the tangent length (PI to TS) or the total length.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpiralCurveInput {
    pub radius: Option<f64>,
    pub spiral_length: Option<f64>,
    pub delta: Option<f64>,
    pub tangent: Option<f64>,
    pub total_length: Option<f64>,
}

impl SpiralCurve {
    pub fn new(radius: f64, delta: f64, spiral_length: f64) -> Result<Self, SolveError> {
        let s = Spiral::new(radius, spiral_length);
        let delta_c = delta - 2.0 * s.theta;
        if delta >= PI {
            return Err(SolveError::NoSolution);
        }
        if delta_c < -1e-12 {
            return Err(SolveError::SpiralsTooLong {
                spirals: (2.0 * s.theta).to_degrees(),
                delta: delta.to_degrees(),
            });
        }
        let delta_c = delta_c.max(0.0);
        let half = delta / 2.0;
        Ok(Self {
            radius,
            delta,
            spiral: s,
            delta_c,
            arc_length: radius * delta_c,
            total_length: radius * delta_c + 2.0 * spiral_length,
            tangent: (radius + s.p) * half.tan() + s.k,
            external: (radius + s.p) / half.cos() - radius,
        })
    }
}

pub fn solve_spiral_curve(input: SpiralCurveInput) -> Result<SpiralCurve, SolveError> {
    positive(&[
        input.radius,
        input.spiral_length,
        input.delta,
        input.tangent,
        input.total_length,
    ])?;
    let (Some(r), Some(ls)) = (input.radius, input.spiral_length) else {
        return Err(SolveError::WrongCount(3));
    };
    let third = [input.delta, input.tangent, input.total_length]
        .iter()
        .flatten()
        .count();
    if third != 1 {
        return Err(SolveError::WrongCount(3));
    }
    let theta = ls / (2.0 * r);
    let delta = match (input.delta, input.tangent, input.total_length) {
        (Some(d), ..) => d,
        // L = R(Δ − 2θs) + 2Ls.
        (_, _, Some(l)) => (l - 2.0 * ls) / r + 2.0 * theta,
        (_, Some(t), _) => {
            // The tangent grows with Δ: one root at most.
            let f = |d: f64| SpiralCurve::new(r, d, ls).map_or(f64::NAN, |c| c.tangent - t);
            *roots(f, 2.0 * theta, PI - 1e-9)
                .first()
                .ok_or(SolveError::NoSolution)?
        }
        _ => unreachable!("one given"),
    };
    SpiralCurve::new(r, delta, ls)
}

// ---------------------------------------------------------------- compound curves

/// Two arcs turning the same way: R₁ from the PC to the PCC, then R₂ to the PT.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompoundCurve {
    pub r1: f64,
    pub r2: f64,
    pub delta1: f64,
    pub delta2: f64,
    /// Total deflection Δ₁ + Δ₂.
    pub delta: f64,
    /// PC to the PI of the main tangents, and the PI to the PT.
    pub t1: f64,
    pub t2: f64,
    /// Each arc's own tangent (PC to V₁, V₂ to PT) and the common tangent V₁V₂.
    pub tangent1: f64,
    pub tangent2: f64,
    pub common_tangent: f64,
    pub length1: f64,
    pub length2: f64,
    /// PC to PT.
    pub long_chord: f64,
}

/// Both radii, and two of Δ₁, Δ₂, Δ, T₁ and T₂ (one at least an angle).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CompoundInput {
    pub r1: Option<f64>,
    pub r2: Option<f64>,
    pub delta1: Option<f64>,
    pub delta2: Option<f64>,
    pub delta: Option<f64>,
    pub t1: Option<f64>,
    pub t2: Option<f64>,
}

impl CompoundCurve {
    pub fn new(r1: f64, r2: f64, delta1: f64, delta2: f64) -> Option<Self> {
        let delta = delta1 + delta2;
        if !(delta1 > 0.0 && delta2 > 0.0 && delta < PI) {
            return None;
        }
        let (a, b) = (r1 * (delta1 / 2.0).tan(), r2 * (delta2 / 2.0).tan());
        let common = a + b;
        // PC at the origin heading east, turning left.
        let pcc = glam::DVec2::new(r1 * delta1.sin(), r1 * (1.0 - delta1.cos()));
        let pt = pcc
            + glam::DVec2::new(
                r2 * (delta.sin() - delta1.sin()),
                r2 * (delta1.cos() - delta.cos()),
            );
        Some(Self {
            r1,
            r2,
            delta1,
            delta2,
            delta,
            t1: a + common * delta2.sin() / delta.sin(),
            t2: b + common * delta1.sin() / delta.sin(),
            tangent1: a,
            tangent2: b,
            common_tangent: common,
            length1: r1 * delta1,
            length2: r2 * delta2,
            long_chord: pt.length(),
        })
    }
}

pub fn solve_compound(input: CompoundInput) -> Result<Vec<CompoundCurve>, SolveError> {
    positive(&[
        input.r1,
        input.r2,
        input.delta1,
        input.delta2,
        input.delta,
        input.t1,
        input.t2,
    ])?;
    let (Some(r1), Some(r2)) = (input.r1, input.r2) else {
        return Err(SolveError::WrongCount(4));
    };
    let given = [input.delta1, input.delta2, input.delta, input.t1, input.t2]
        .iter()
        .flatten()
        .count();
    if given != 2 {
        return Err(SolveError::WrongCount(4));
    }
    let make = |d1: f64, d2: f64| CompoundCurve::new(r1, r2, d1, d2);
    let tangent = |c: &CompoundCurve| match (input.t1, input.t2) {
        (Some(t), _) => c.t1 - t,
        (_, Some(t)) => c.t2 - t,
        _ => 0.0,
    };
    let out: Vec<CompoundCurve> = match (input.delta1, input.delta2, input.delta) {
        (Some(d1), Some(d2), _) => make(d1, d2).into_iter().collect(),
        (Some(d1), _, Some(d)) => make(d1, d - d1).into_iter().collect(),
        (_, Some(d2), Some(d)) => make(d - d2, d2).into_iter().collect(),
        // One angle and a tangent: search the other arc's angle.
        (Some(d1), None, None) => roots(|d2| make(d1, d2).map_or(f64::NAN, |c| tangent(&c)), 1e-9, PI - d1)
            .into_iter()
            .filter_map(|d2| make(d1, d2))
            .collect(),
        (None, Some(d2), None) => roots(|d1| make(d1, d2).map_or(f64::NAN, |c| tangent(&c)), 1e-9, PI - d2)
            .into_iter()
            .filter_map(|d1| make(d1, d2))
            .collect(),
        (None, None, Some(d)) => roots(|d1| make(d1, d - d1).map_or(f64::NAN, |c| tangent(&c)), 1e-9, d)
            .into_iter()
            .filter_map(|d1| make(d1, d - d1))
            .collect(),
        (None, None, None) => return Err(SolveError::NeedsAngle),
    };
    if out.is_empty() {
        Err(SolveError::NoSolution)
    } else {
        Ok(out)
    }
}

// ---------------------------------------------------------------- reverse curves

/// Two arcs turning opposite ways between parallel tangents, each turning Δ.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverseCurve {
    pub r1: f64,
    pub r2: f64,
    pub delta: f64,
    /// Distance between the parallel tangents.
    pub offset: f64,
    /// PC to PT measured along the tangents.
    pub along: f64,
    /// PC to PT.
    pub chord: f64,
    pub length1: f64,
    pub length2: f64,
}

/// The radii (equal, or each, or the first), and enough of Δ, the offset
/// between the tangents and the distance along them for two values with the
/// radii's sum.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReverseInput {
    /// Both radii when they are equal.
    pub radius: Option<f64>,
    pub r1: Option<f64>,
    pub r2: Option<f64>,
    pub delta: Option<f64>,
    pub offset: Option<f64>,
    pub along: Option<f64>,
}

pub fn solve_reverse(input: ReverseInput) -> Result<ReverseCurve, SolveError> {
    positive(&[
        input.radius,
        input.r1,
        input.r2,
        input.delta,
        input.offset,
        input.along,
    ])?;
    // The sum of the radii, when the radii give it.
    let sum = match (input.radius, input.r1, input.r2) {
        (Some(r), None, None) => Some(2.0 * r),
        (None, Some(a), Some(b)) => Some(a + b),
        (None, _, _) => None,
        _ => return Err(SolveError::WrongCount(2)),
    };
    let known =
        usize::from(sum.is_some()) + [input.delta, input.offset, input.along].iter().flatten().count();
    if known != 2 {
        return Err(SolveError::WrongCount(2));
    }
    // p = S(1 − cos Δ), x = S·sin Δ.
    let (s, delta) = match (sum, input.delta, input.offset, input.along) {
        (Some(s), Some(d), ..) => (s, d),
        (Some(s), None, Some(p), None) => {
            let c = 1.0 - p / s;
            if c.abs() > 1.0 {
                return Err(SolveError::NoSolution);
            }
            (s, c.acos())
        }
        (Some(s), None, None, Some(x)) => {
            if x > s {
                return Err(SolveError::NoSolution);
            }
            (s, (x / s).asin())
        }
        (None, Some(d), Some(p), None) => (p / (1.0 - d.cos()), d),
        (None, Some(d), None, Some(x)) => (x / d.sin(), d),
        (None, None, Some(p), Some(x)) => {
            let d = 2.0 * (p / x).atan();
            (x / d.sin(), d)
        }
        _ => return Err(SolveError::WrongCount(2)),
    };
    if !(delta > 0.0 && delta <= PI / 2.0 + 1e-12 && s > 0.0 && s.is_finite()) {
        return Err(SolveError::NoSolution);
    }
    let (r1, r2) = match (input.radius, input.r1, input.r2) {
        (Some(r), ..) => (r, r),
        (None, Some(a), Some(b)) => (a, b),
        (None, Some(a), None) if a < s => (a, s - a),
        (None, None, Some(b)) if b < s => (s - b, b),
        (None, None, None) => (s / 2.0, s / 2.0),
        _ => return Err(SolveError::NoSolution),
    };
    let (offset, along) = (s * (1.0 - delta.cos()), s * delta.sin());
    Ok(ReverseCurve {
        r1,
        r2,
        delta,
        offset,
        along,
        chord: offset.hypot(along),
        length1: r1 * delta,
        length2: r2 * delta,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deg(d: f64) -> f64 {
        d.to_radians()
    }

    #[test]
    fn spiral_curves_from_any_third_value() {
        let c = SpiralCurve::new(300.0, deg(40.0), 60.0).expect("curve");
        assert!((c.delta_c - (deg(40.0) - 0.2)).abs() < 1e-12);
        assert!((c.total_length - (300.0 * c.delta_c + 120.0)).abs() < 1e-9);
        let by = |input: SpiralCurveInput| solve_spiral_curve(input).expect("solve");
        let base = SpiralCurveInput {
            radius: Some(300.0),
            spiral_length: Some(60.0),
            ..Default::default()
        };
        let t = by(SpiralCurveInput {
            tangent: Some(c.tangent),
            ..base
        });
        assert!((t.delta - c.delta).abs() < 1e-9);
        let l = by(SpiralCurveInput {
            total_length: Some(c.total_length),
            ..base
        });
        assert!((l.delta - c.delta).abs() < 1e-12);
        // Spirals that turn more than the tangents do.
        assert!(SpiralCurve::new(100.0, deg(20.0), 60.0).is_err());
        assert!(
            solve_spiral_curve(SpiralCurveInput {
                radius: Some(300.0),
                ..Default::default()
            })
            .is_err()
        );
    }

    #[test]
    fn compound_curves() {
        let c = CompoundCurve::new(200.0, 100.0, deg(30.0), deg(20.0)).expect("curve");
        // The PT from the PI along the forward tangent matches the arcs' end.
        let pi = glam::DVec2::new(c.t1, 0.0);
        let pt_from_pi = pi + glam::DVec2::from_angle(c.delta) * c.t2;
        let pcc = glam::DVec2::new(200.0 * deg(30.0).sin(), 200.0 * (1.0 - deg(30.0).cos()));
        let pt = pcc
            + glam::DVec2::new(
                100.0 * (deg(50.0).sin() - deg(30.0).sin()),
                100.0 * (deg(30.0).cos() - deg(50.0).cos()),
            );
        assert!(pt_from_pi.distance(pt) < 1e-9, "{pt_from_pi} {pt}");
        assert!((c.long_chord - pt.length()).abs() < 1e-9);
        // The same curve from Δ and T₁, and from Δ₁ and T₂.
        let base = CompoundInput {
            r1: Some(200.0),
            r2: Some(100.0),
            ..Default::default()
        };
        let a = solve_compound(CompoundInput {
            delta: Some(deg(50.0)),
            t1: Some(c.t1),
            ..base
        })
        .expect("Δ and T₁");
        assert!(a.iter().any(|s| (s.delta1 - deg(30.0)).abs() < 1e-9), "{a:?}");
        let b = solve_compound(CompoundInput {
            delta1: Some(deg(30.0)),
            t2: Some(c.t2),
            ..base
        })
        .expect("Δ₁ and T₂");
        assert!(b.iter().any(|s| (s.delta2 - deg(20.0)).abs() < 1e-9), "{b:?}");
        assert_eq!(
            solve_compound(CompoundInput {
                t1: Some(10.0),
                t2: Some(10.0),
                ..base
            }),
            Err(SolveError::NeedsAngle)
        );
    }

    #[test]
    fn reverse_curves_between_parallel_tangents() {
        // Equal radii 100 m, tangents 10 m apart.
        let c = solve_reverse(ReverseInput {
            radius: Some(100.0),
            offset: Some(10.0),
            ..Default::default()
        })
        .expect("curve");
        assert!((c.delta - (1.0f64 - 10.0 / 200.0).acos()).abs() < 1e-12);
        assert!(
            (c.along - 200.0 * c.delta.sin()).abs() < 1e-9 && (c.chord - 10f64.hypot(c.along)).abs() < 1e-9
        );
        // Radii from the offset and the distance along.
        let d = solve_reverse(ReverseInput {
            offset: Some(10.0),
            along: Some(c.along),
            ..Default::default()
        })
        .expect("from p and x");
        assert!((d.r1 - 100.0).abs() < 1e-9 && (d.r2 - 100.0).abs() < 1e-9);
        // One radius given: the other makes up the sum.
        let e = solve_reverse(ReverseInput {
            r1: Some(150.0),
            offset: Some(10.0),
            along: Some(c.along),
            ..Default::default()
        })
        .expect("r1");
        assert!((e.r2 - 50.0).abs() < 1e-9);
        assert!(
            solve_reverse(ReverseInput {
                radius: Some(10.0),
                offset: Some(30.0),
                ..Default::default()
            })
            .is_err()
        );
    }
}
