//! Curve and triangle solutions: complete a circular curve or a triangle from
//! any sufficient subset of its elements.

use std::f64::consts::{PI, TAU};

use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Error)]
pub enum SolveError {
    #[error("Enter exactly {0} values")]
    WrongCount(usize),
    #[error("A triangle needs at least one side")]
    NoSide,
    #[error("Values must be positive")]
    NotPositive,
    #[error("The angles are too large for a triangle")]
    AnglesTooLarge,
    #[error("No curve or triangle has these values")]
    NoSolution,
}

// ---------------------------------------------------------------- curves

/// Elements of a circular curve. `delta` is the central (deflection) angle in radians.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Curve {
    pub radius: f64,
    pub delta: f64,
    /// Arc length.
    pub length: f64,
    /// Long chord.
    pub chord: f64,
    /// Tangent length (PC or PT to the intersection point); infinite at 180°.
    pub tangent: f64,
    /// External distance (intersection point to the curve midpoint).
    pub external: f64,
    /// Middle ordinate (chord midpoint to the curve midpoint).
    pub mid_ordinate: f64,
}

/// Which curve elements are known (any two).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CurveInput {
    pub radius: Option<f64>,
    pub delta: Option<f64>,
    pub length: Option<f64>,
    pub chord: Option<f64>,
    pub tangent: Option<f64>,
    pub external: Option<f64>,
    pub mid_ordinate: Option<f64>,
}

impl Curve {
    pub fn from_radius_delta(radius: f64, delta: f64) -> Self {
        let h = delta / 2.0;
        Self {
            radius,
            delta,
            length: radius * delta,
            chord: 2.0 * radius * h.sin(),
            tangent: radius * h.tan(),
            external: radius * (1.0 / h.cos() - 1.0),
            mid_ordinate: radius * (1.0 - h.cos()),
        }
    }
}

/// The non-radius elements as functions of Δ for a unit radius.
const SHAPES: [fn(f64) -> f64; 5] = [
    |d| d,
    |d| 2.0 * (d / 2.0).sin(),
    |d| (d / 2.0).tan(),
    |d| 1.0 / (d / 2.0).cos() - 1.0,
    |d| 1.0 - (d / 2.0).cos(),
];

/// Every curve with the two given elements (two for some combinations).
pub fn solve_curve(input: CurveInput) -> Result<Vec<Curve>, SolveError> {
    let others = [
        input.length,
        input.chord,
        input.tangent,
        input.external,
        input.mid_ordinate,
    ];
    let known = usize::from(input.radius.is_some())
        + usize::from(input.delta.is_some())
        + others.iter().flatten().count();
    if known != 2 {
        return Err(SolveError::WrongCount(2));
    }
    let values = [input.radius, input.delta].into_iter().chain(others).flatten();
    if values.clone().any(|v| v <= 0.0 || !v.is_finite()) {
        return Err(SolveError::NotPositive);
    }
    // Tangent and external are only defined for Δ < 180°.
    let max_delta = |i: usize| if i == 2 || i == 3 { PI } else { TAU };
    let given: Vec<(usize, f64)> = others
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.map(|v| (i, v)))
        .collect();
    let deltas: Vec<(f64, f64)> = match (input.radius, input.delta) {
        (Some(r), Some(d)) => vec![(r, d)],
        (None, Some(d)) => {
            let (i, v) = given[0];
            if d >= max_delta(i) {
                return Err(SolveError::NoSolution);
            }
            vec![(v / SHAPES[i](d), d)]
        }
        (Some(r), None) => {
            let (i, v) = given[0];
            roots(|d| SHAPES[i](d) * r - v, max_delta(i))
                .into_iter()
                .map(|d| (r, d))
                .collect()
        }
        (None, None) => {
            let ((i, a), (j, b)) = (given[0], given[1]);
            roots(
                |d| SHAPES[i](d) * b - SHAPES[j](d) * a,
                max_delta(i).min(max_delta(j)),
            )
            .into_iter()
            .map(|d| (a / SHAPES[i](d), d))
            .collect()
        }
    };
    let out: Vec<Curve> = deltas
        .into_iter()
        .filter(|(r, d)| *r > 0.0 && r.is_finite() && *d > 0.0 && *d < TAU)
        .map(|(r, d)| Curve::from_radius_delta(r, d))
        .collect();
    if out.is_empty() {
        Err(SolveError::NoSolution)
    } else {
        Ok(out)
    }
}

/// Roots of `f` on (0, max) by sampling and bisection.
fn roots(f: impl Fn(f64) -> f64, max: f64) -> Vec<f64> {
    const N: usize = 4000;
    let eps = 1e-9;
    let mut out: Vec<f64> = Vec::new();
    let at = |i: usize| eps + (max - 2.0 * eps) * i as f64 / N as f64;
    let mut prev = f(at(0));
    for i in 1..=N {
        let x = at(i);
        let cur = f(x);
        if prev.is_finite() && cur.is_finite() && (prev == 0.0 || (prev < 0.0) != (cur < 0.0)) {
            let (mut lo, mut hi, flo) = (at(i - 1), x, prev);
            for _ in 0..200 {
                let mid = 0.5 * (lo + hi);
                if (f(mid) < 0.0) == (flo < 0.0) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let r = 0.5 * (lo + hi);
            if out.last().is_none_or(|p| (r - p).abs() > 1e-9) {
                out.push(r);
            }
        }
        prev = cur;
    }
    out
}

// ---------------------------------------------------------------- triangles

/// A solved triangle: side `a` is opposite angle `alpha`, and so on. Angles in radians.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Triangle {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub alpha: f64,
    pub beta: f64,
    pub gamma: f64,
}

impl Triangle {
    pub fn area(&self) -> f64 {
        0.5 * self.b * self.c * self.alpha.sin()
    }
}

/// Known triangle elements (any three, at least one a side).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TriangleInput {
    pub a: Option<f64>,
    pub b: Option<f64>,
    pub c: Option<f64>,
    pub alpha: Option<f64>,
    pub beta: Option<f64>,
    pub gamma: Option<f64>,
}

/// Every triangle with the given elements: none, one, or two (the ambiguous SSA case).
pub fn solve_triangle(input: TriangleInput) -> Result<Vec<Triangle>, SolveError> {
    let sides = [input.a, input.b, input.c];
    let angles = [input.alpha, input.beta, input.gamma];
    let n_sides = sides.iter().flatten().count();
    let n_angles = angles.iter().flatten().count();
    if n_sides + n_angles != 3 {
        return Err(SolveError::WrongCount(3));
    }
    if n_sides == 0 {
        return Err(SolveError::NoSide);
    }
    if sides
        .iter()
        .chain(&angles)
        .flatten()
        .any(|v| *v <= 0.0 || !v.is_finite())
    {
        return Err(SolveError::NotPositive);
    }
    if angles.iter().flatten().sum::<f64>() >= PI {
        return Err(SolveError::AnglesTooLarge);
    }
    // Rotate so the work is done on a fixed labelling, then rotate back.
    let mut out = Vec::new();
    for k in 0..3 {
        let s = |i: usize| sides[(i + k) % 3];
        let g = |i: usize| angles[(i + k) % 3];
        let solved: Vec<[f64; 6]> = match (s(0), s(1), s(2), g(0), g(1), g(2)) {
            // SSS
            (Some(a), Some(b), Some(c), None, None, None) if k == 0 => sss(a, b, c).into_iter().collect(),
            // SAS: sides b, c with the included angle α.
            (None, Some(b), Some(c), Some(al), None, None) => {
                let a = (b * b + c * c - 2.0 * b * c * al.cos()).max(0.0).sqrt();
                sss(a, b, c).into_iter().collect()
            }
            // SSA: sides a, b and the angle opposite one of them (0, 1 or 2 solutions).
            (Some(a), Some(b), None, Some(al), None, None) => ssa(a, b, al),
            (Some(a), Some(b), None, None, Some(be), None) => ssa(b, a, be)
                .into_iter()
                .map(|[b, a, c, be, al, ga]| [a, b, c, al, be, ga])
                .collect(),
            // One side and two angles (ASA or AAS): complete the angles, then sines.
            (Some(a), None, None, al, be, ga) if [al, be, ga].iter().flatten().count() == 2 => {
                let al = al.unwrap_or_else(|| PI - be.unwrap_or(0.0) - ga.unwrap_or(0.0));
                let be = be.unwrap_or(PI - al - ga.unwrap_or(0.0));
                let ga = PI - al - be;
                let k = a / al.sin();
                vec![[a, k * be.sin(), k * ga.sin(), al, be, ga]]
            }
            _ => continue,
        };
        // Undo the rotation: element i of the rotated labelling is element (i + k) % 3.
        for t in solved {
            let mut s3 = [0.0; 3];
            let mut g3 = [0.0; 3];
            for i in 0..3 {
                s3[(i + k) % 3] = t[i];
                g3[(i + k) % 3] = t[3 + i];
            }
            out.push(Triangle {
                a: s3[0],
                b: s3[1],
                c: s3[2],
                alpha: g3[0],
                beta: g3[1],
                gamma: g3[2],
            });
        }
        break;
    }
    if out.is_empty() {
        Err(SolveError::NoSolution)
    } else {
        Ok(out)
    }
}

/// Sides `a`, `b` and the angle `al` opposite `a`.
fn ssa(a: f64, b: f64, al: f64) -> Vec<[f64; 6]> {
    let sin_be = b * al.sin() / a;
    if sin_be > 1.0 + 1e-12 {
        return Vec::new();
    }
    let be1 = sin_be.min(1.0).asin();
    let mut v: Vec<[f64; 6]> = Vec::new();
    for be in [be1, PI - be1] {
        let ga = PI - al - be;
        if ga > 1e-12 && v.iter().all(|x| (x[4] - be).abs() > 1e-12) {
            v.push([a, b, a * ga.sin() / al.sin(), al, be, ga]);
        }
    }
    v
}

fn sss(a: f64, b: f64, c: f64) -> Option<[f64; 6]> {
    if a + b <= c || a + c <= b || b + c <= a {
        return None;
    }
    let al = ((b * b + c * c - a * a) / (2.0 * b * c)).clamp(-1.0, 1.0).acos();
    let be = ((a * a + c * c - b * b) / (2.0 * a * c)).clamp(-1.0, 1.0).acos();
    Some([a, b, c, al, be, PI - al - be])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deg(d: f64) -> f64 {
        d.to_radians()
    }

    #[test]
    fn curve_from_any_pair() {
        let truth = Curve::from_radius_delta(150.0, deg(40.0));
        let pairs: [CurveInput; 6] = [
            CurveInput {
                radius: Some(150.0),
                length: Some(truth.length),
                ..Default::default()
            },
            CurveInput {
                delta: Some(deg(40.0)),
                chord: Some(truth.chord),
                ..Default::default()
            },
            CurveInput {
                tangent: Some(truth.tangent),
                external: Some(truth.external),
                ..Default::default()
            },
            CurveInput {
                chord: Some(truth.chord),
                mid_ordinate: Some(truth.mid_ordinate),
                ..Default::default()
            },
            CurveInput {
                radius: Some(150.0),
                tangent: Some(truth.tangent),
                ..Default::default()
            },
            CurveInput {
                length: Some(truth.length),
                tangent: Some(truth.tangent),
                ..Default::default()
            },
        ];
        for p in pairs {
            let sol = solve_curve(p).expect("solvable");
            assert!(
                sol.iter()
                    .any(|c| (c.radius - 150.0).abs() < 1e-6 && (c.delta - deg(40.0)).abs() < 1e-9),
                "{p:?} → {sol:?}"
            );
        }
        // Radius and chord: the minor and the major arc.
        let sol = solve_curve(CurveInput {
            radius: Some(150.0),
            chord: Some(truth.chord),
            ..Default::default()
        })
        .expect("solvable");
        assert_eq!(sol.len(), 2);
        assert!(sol.iter().any(|c| (c.delta - deg(320.0)).abs() < 1e-9));
        assert_eq!(
            solve_curve(CurveInput {
                radius: Some(1.0),
                ..Default::default()
            }),
            Err(SolveError::WrongCount(2))
        );
        // A chord longer than the diameter has no curve.
        assert_eq!(
            solve_curve(CurveInput {
                radius: Some(10.0),
                chord: Some(25.0),
                ..Default::default()
            }),
            Err(SolveError::NoSolution)
        );
    }

    fn close(t: &Triangle, a: f64, b: f64, c: f64) -> bool {
        (t.a - a).abs() < 1e-9 && (t.b - b).abs() < 1e-9 && (t.c - c).abs() < 1e-9
    }

    #[test]
    fn triangle_cases() {
        // 3-4-5 right triangle: γ = 90°.
        let t = solve_triangle(TriangleInput {
            a: Some(3.0),
            b: Some(4.0),
            c: Some(5.0),
            ..Default::default()
        })
        .expect("sss")[0];
        assert!((t.gamma - deg(90.0)).abs() < 1e-12 && (t.area() - 6.0).abs() < 1e-9);
        // SAS in every rotation.
        let sas = solve_triangle(TriangleInput {
            a: Some(3.0),
            c: Some(5.0),
            beta: Some(t.beta),
            ..Default::default()
        })
        .expect("sas");
        assert!(close(&sas[0], 3.0, 4.0, 5.0));
        // ASA / AAS.
        let asa = solve_triangle(TriangleInput {
            c: Some(5.0),
            alpha: Some(t.alpha),
            beta: Some(t.beta),
            ..Default::default()
        })
        .expect("asa");
        assert!(close(&asa[0], 3.0, 4.0, 5.0), "{asa:?}");
        let aas = solve_triangle(TriangleInput {
            a: Some(3.0),
            alpha: Some(t.alpha),
            gamma: Some(t.gamma),
            ..Default::default()
        })
        .expect("aas");
        assert!(close(&aas[0], 3.0, 4.0, 5.0), "{aas:?}");
        // SSA ambiguous: a = 6, b = 8, α = 30° → two triangles.
        let ssa = solve_triangle(TriangleInput {
            a: Some(6.0),
            b: Some(8.0),
            alpha: Some(deg(30.0)),
            ..Default::default()
        })
        .expect("ssa");
        assert_eq!(ssa.len(), 2);
        for t in &ssa {
            assert!((t.alpha + t.beta + t.gamma - PI).abs() < 1e-12);
            assert!((t.a / t.alpha.sin() - t.c / t.gamma.sin()).abs() < 1e-9);
        }
        // The same with the angle opposite the other side.
        let swapped = solve_triangle(TriangleInput {
            a: Some(8.0),
            b: Some(6.0),
            beta: Some(deg(30.0)),
            ..Default::default()
        })
        .expect("ssa");
        assert_eq!(swapped.len(), 2);
        assert!(
            swapped
                .iter()
                .all(|t| (t.alpha + t.beta + t.gamma - PI).abs() < 1e-12 && (t.b - 6.0).abs() < 1e-12)
        );
        // SSA with b·sin α > a: none.
        assert_eq!(
            solve_triangle(TriangleInput {
                a: Some(3.0),
                b: Some(8.0),
                alpha: Some(deg(30.0)),
                ..Default::default()
            }),
            Err(SolveError::NoSolution)
        );
        assert_eq!(
            solve_triangle(TriangleInput {
                alpha: Some(1.0),
                beta: Some(1.0),
                gamma: Some(1.0),
                ..Default::default()
            }),
            Err(SolveError::NoSide)
        );
    }
}
