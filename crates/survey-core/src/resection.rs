//! Resection (free station): the position of an instrument from horizontal
//! directions and/or distances observed to known points.
//!
//! Directions are horizontal circle readings, so the circle's orientation is
//! solved along with the position. The solution is a weighted least-squares
//! fit (Gauss–Newton started from many points so the classic three-point case
//! needs no special formula); with redundant observations the residuals and
//! standard deviations are reported. Exactly determined cases with two
//! solutions (e.g. two distances only) return both.
//!
//! Everything is computed relative to the first known point, so grid
//! coordinates keep full precision.

use glam::DVec2;
use thiserror::Error;

use crate::angle::Angle;

/// One sighting from the instrument to a known point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sighting {
    pub target: DVec2,
    /// Horizontal circle reading to the target.
    pub direction: Option<Angle>,
    /// Horizontal distance to the target.
    pub distance: Option<f64>,
}

/// Standard deviations used to weight the observations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weights {
    pub direction: Angle,
    pub distance: f64,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            direction: Angle::from_degrees(5.0 / 3600.0),
            distance: 0.005,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Error)]
pub enum ResectionError {
    #[error("not enough observations: {0}")]
    NotEnough(&'static str),
    #[error("the known points must be distinct")]
    DuplicateTargets,
    #[error(
        "the geometry is too weak to fix the station (e.g. it lies on the circle through the known points)"
    )]
    WeakGeometry,
    #[error("no solution found")]
    NoSolution,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resection {
    pub position: DVec2,
    /// Add to a circle reading to get its bearing; `None` without directions.
    pub orientation: Option<Angle>,
    /// Observed minus computed, per sighting: (direction, distance).
    pub residuals: Vec<(Option<Angle>, Option<f64>)>,
    /// Observations beyond the minimum.
    pub redundancy: usize,
    /// A-posteriori standard deviations of easting and northing (needs redundancy).
    pub sigma: Option<DVec2>,
    /// Standard error of unit weight (needs redundancy).
    pub sigma0: Option<f64>,
}

fn wrap(a: f64) -> f64 {
    let t = std::f64::consts::TAU;
    let mut r = a % t;
    if r > std::f64::consts::PI {
        r -= t;
    } else if r < -std::f64::consts::PI {
        r += t;
    }
    r
}

struct Problem<'a> {
    sightings: &'a [Sighting],
    /// Targets relative to the first.
    targets: Vec<DVec2>,
    with_orientation: bool,
    w: Weights,
}

impl Problem<'_> {
    fn unknowns(&self) -> usize {
        2 + usize::from(self.with_orientation)
    }

    /// Weighted residuals (computed − observed) and Jacobian rows at `x = (E, N, ω)`.
    fn linearize(&self, x: [f64; 3]) -> Option<(Vec<f64>, Vec<[f64; 3]>)> {
        let p = DVec2::new(x[0], x[1]);
        let mut v = Vec::new();
        let mut j = Vec::new();
        for (s, t) in self.sightings.iter().zip(&self.targets) {
            let d = *t - p;
            let d2 = d.length_squared();
            if d2 < 1e-18 {
                return None;
            }
            let dist = d2.sqrt();
            if let Some(r) = s.direction {
                let sd = self.w.direction.radians();
                let bearing = d.x.atan2(d.y);
                v.push(wrap(bearing - x[2] - r.radians()) / sd);
                j.push([-d.y / d2 / sd, d.x / d2 / sd, -1.0 / sd]);
            }
            if let Some(obs) = s.distance {
                let sd = self.w.distance;
                v.push((dist - obs) / sd);
                j.push([-d.x / dist / sd, -d.y / dist / sd, 0.0]);
            }
        }
        Some((v, j))
    }

    fn cost(&self, x: [f64; 3]) -> Option<f64> {
        self.linearize(x).map(|(v, _)| v.iter().map(|r| r * r).sum())
    }

    /// Normal matrix and right-hand side for the active unknowns.
    fn normals(&self, v: &[f64], j: &[[f64; 3]]) -> ([[f64; 3]; 3], [f64; 3]) {
        let u = self.unknowns();
        let mut n = [[0.0; 3]; 3];
        let mut b = [0.0; 3];
        for (row, r) in j.iter().zip(v) {
            for a in 0..u {
                b[a] -= row[a] * r;
                for c in 0..u {
                    n[a][c] += row[a] * row[c];
                }
            }
        }
        (n, b)
    }

    fn initial_orientation(&self, p: DVec2) -> f64 {
        let (mut s, mut c) = (0.0, 0.0);
        for (sg, t) in self.sightings.iter().zip(&self.targets) {
            if let Some(r) = sg.direction {
                let d = *t - p;
                let a = d.x.atan2(d.y) - r.radians();
                s += a.sin();
                c += a.cos();
            }
        }
        s.atan2(c)
    }

    /// Gauss–Newton with step halving from `start`.
    fn solve_from(&self, start: DVec2, scale: f64) -> Option<[f64; 3]> {
        let mut x = [start.x, start.y, self.initial_orientation(start)];
        let mut cost = self.cost(x)?;
        let u = self.unknowns();
        for _ in 0..100 {
            let (v, j) = self.linearize(x)?;
            let (n, b) = self.normals(&v, &j);
            let dx = solve(&n, &b, u)?;
            let mut step = 1.0;
            let mut improved = false;
            for _ in 0..30 {
                let trial = [x[0] + dx[0] * step, x[1] + dx[1] * step, x[2] + dx[2] * step];
                if let Some(c) = self.cost(trial)
                    && c <= cost
                {
                    x = trial;
                    cost = c;
                    improved = true;
                    break;
                }
                step *= 0.5;
            }
            let moved = DVec2::new(dx[0], dx[1]).length() * step;
            if !improved || moved < 1e-11 * (1.0 + scale) {
                break;
            }
        }
        x[2] = wrap(x[2]);
        Some(x)
    }
}

/// Solves `n·x = b` for the first `u` unknowns (Gaussian elimination with pivoting).
fn solve(n: &[[f64; 3]; 3], b: &[f64; 3], u: usize) -> Option<[f64; 3]> {
    let mut a = [[0.0; 4]; 3];
    for r in 0..u {
        for c in 0..u {
            a[r][c] = n[r][c];
        }
        a[r][3] = b[r];
    }
    let scale = (0..u).map(|i| n[i][i].abs()).fold(0.0, f64::max).max(1e-300);
    for col in 0..u {
        let piv = (col..u).max_by(|x, y| a[*x][col].abs().total_cmp(&a[*y][col].abs()))?;
        if a[piv][col].abs() <= 1e-12 * scale {
            return None;
        }
        a.swap(col, piv);
        for r in 0..u {
            if r != col {
                let f = a[r][col] / a[col][col];
                let pivot = a[col];
                for (x, p) in a[r].iter_mut().zip(pivot).skip(col) {
                    *x -= f * p;
                }
            }
        }
    }
    let mut x = [0.0; 3];
    for i in 0..u {
        x[i] = a[i][3] / a[i][i];
    }
    Some(x)
}

/// Inverse of the active block of a symmetric normal matrix.
fn inverse(n: &[[f64; 3]; 3], u: usize) -> Option<[[f64; 3]; 3]> {
    let mut inv = [[0.0; 3]; 3];
    for c in 0..u {
        let mut e = [0.0; 3];
        e[c] = 1.0;
        let col = solve(n, &e, u)?;
        for r in 0..u {
            inv[r][c] = col[r];
        }
    }
    Some(inv)
}

/// Resects the station. Returns the best solution, or every exact solution when
/// the observations are exactly determined and ambiguous.
pub fn resect(sightings: &[Sighting], weights: Weights) -> Result<Vec<Resection>, ResectionError> {
    let used: Vec<Sighting> = sightings
        .iter()
        .copied()
        .filter(|s| s.direction.is_some() || s.distance.is_some())
        .collect();
    let n_dir = used.iter().filter(|s| s.direction.is_some()).count();
    let n_dist = used.iter().filter(|s| s.distance.is_some()).count();
    let with_orientation = n_dir > 0;
    let unknowns = 2 + usize::from(with_orientation);
    let m = n_dir + n_dist;
    if used.len() < 2 {
        return Err(ResectionError::NotEnough("sight at least two known points"));
    }
    if m < unknowns {
        return Err(ResectionError::NotEnough(
            "with directions, use at least three observations (e.g. three directions, or two distances and a direction)",
        ));
    }
    let origin = used[0].target;
    let targets: Vec<DVec2> = used.iter().map(|s| s.target - origin).collect();
    for i in 0..targets.len() {
        for k in i + 1..targets.len() {
            if targets[i].distance(targets[k]) < 1e-9 {
                return Err(ResectionError::DuplicateTargets);
            }
        }
    }
    let problem = Problem {
        sightings: &used,
        targets: targets.clone(),
        with_orientation,
        w: weights,
    };

    // Starting points: rings around the known points, plus distance–distance intersections.
    let centroid = targets.iter().copied().sum::<DVec2>() / targets.len() as f64;
    let span = targets
        .iter()
        .map(|t| t.distance(centroid))
        .fold(0.0, f64::max)
        .max(1.0);
    let mut starts = vec![centroid];
    for r in [0.3, 0.7, 1.5, 3.0] {
        for k in 0..16 {
            let a = k as f64 * std::f64::consts::TAU / 16.0;
            starts.push(centroid + DVec2::new(a.sin(), a.cos()) * span * r);
        }
    }
    let dists: Vec<(DVec2, f64)> = used
        .iter()
        .zip(&targets)
        .filter_map(|(s, t)| s.distance.map(|d| (*t, d)))
        .collect();
    for i in 0..dists.len() {
        for k in i + 1..dists.len() {
            starts.extend(crate::offset::circle_circle(
                dists[i].0, dists[i].1, dists[k].0, dists[k].1,
            ));
        }
    }

    let mut found: Vec<([f64; 3], f64)> = Vec::new();
    for s in starts {
        let Some(x) = problem.solve_from(s, span) else {
            continue;
        };
        let Some(c) = problem.cost(x) else {
            continue;
        };
        let p = DVec2::new(x[0], x[1]);
        if !found
            .iter()
            .any(|(y, _)| DVec2::new(y[0], y[1]).distance(p) < 1e-6 * (1.0 + span))
        {
            found.push((x, c));
        }
    }
    if found.is_empty() {
        return Err(ResectionError::NoSolution);
    }
    found.sort_by(|a, b| a.1.total_cmp(&b.1));
    let redundancy = m - unknowns;
    let best = found[0].1;
    let keep: Vec<[f64; 3]> = if redundancy == 0 {
        // Exact solutions only (normalised residuals ~ 0).
        found
            .iter()
            .filter(|(_, c)| *c <= best.max(1e-12) * 10.0 + 1e-12)
            .map(|(x, _)| *x)
            .collect()
    } else {
        vec![found[0].0]
    };

    let mut out = Vec::new();
    for x in keep {
        let (v, j) = problem.linearize(x).ok_or(ResectionError::NoSolution)?;
        let (n, _) = problem.normals(&v, &j);
        let inv = inverse(&n, unknowns).ok_or(ResectionError::WeakGeometry)?;
        let (sigma0, sigma) = if redundancy > 0 {
            let s0 = (v.iter().map(|r| r * r).sum::<f64>() / redundancy as f64).sqrt();
            (
                Some(s0),
                Some(DVec2::new(inv[0][0].sqrt(), inv[1][1].sqrt()) * s0),
            )
        } else {
            (None, None)
        };
        // Residuals in observation units (observed − computed).
        let mut it = v.iter();
        let residuals = used
            .iter()
            .map(|s| {
                let dir = s.direction.map(|_| {
                    Angle::from_radians(-it.next().copied().unwrap_or(0.0) * weights.direction.radians())
                });
                let dist = s
                    .distance
                    .map(|_| -it.next().copied().unwrap_or(0.0) * weights.distance);
                (dir, dist)
            })
            .collect();
        out.push(Resection {
            position: origin + DVec2::new(x[0], x[1]),
            orientation: with_orientation.then(|| Angle::from_radians(x[2]).normalized()),
            residuals,
            redundancy,
            sigma,
            sigma0,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::angle::Bearing;

    const O: DVec2 = DVec2::new(512_000.0, 6_912_000.0);

    fn sight(station: DVec2, target: DVec2, orientation_deg: f64, dir: bool, dist: bool) -> Sighting {
        let d = target - station;
        let bearing = Bearing::from_delta(d.x, d.y).expect("distinct").degrees();
        Sighting {
            target,
            direction: dir.then(|| Angle::from_degrees(bearing - orientation_deg).normalized()),
            distance: dist.then(|| d.length()),
        }
    }

    fn targets() -> [DVec2; 3] {
        [
            O + DVec2::new(0.0, 200.0),
            O + DVec2::new(180.0, 20.0),
            O + DVec2::new(-150.0, -60.0),
        ]
    }

    #[test]
    fn three_directions() {
        let station = O + DVec2::new(10.0, 30.0);
        let s: Vec<Sighting> = targets()
            .iter()
            .map(|t| sight(station, *t, 37.5, true, false))
            .collect();
        let r = resect(&s, Weights::default()).expect("resection");
        assert_eq!(r.len(), 1, "{r:?}");
        assert!(
            (r[0].position - station).length() < 1e-6,
            "{:?}",
            r[0].position - station
        );
        let o = r[0].orientation.expect("oriented").degrees();
        assert!((o - 37.5).abs() < 1e-7, "{o}");
        assert_eq!(r[0].redundancy, 0);
    }

    #[test]
    fn two_distances_are_ambiguous() {
        let station = O + DVec2::new(10.0, 30.0);
        let t = targets();
        let s = [
            sight(station, t[0], 0.0, false, true),
            sight(station, t[1], 0.0, false, true),
        ];
        let r = resect(&s, Weights::default()).expect("resection");
        assert_eq!(r.len(), 2);
        assert!(r.iter().any(|x| (x.position - station).length() < 1e-6));
        assert!(r[0].orientation.is_none());
    }

    #[test]
    fn redundant_with_residuals() {
        let station = O + DVec2::new(-20.0, 40.0);
        let mut s: Vec<Sighting> = targets()
            .iter()
            .map(|t| sight(station, *t, 200.0, true, true))
            .collect();
        // A 6 mm error on one distance.
        s[1].distance = s[1].distance.map(|d| d + 0.006);
        let r = resect(&s, Weights::default()).expect("resection");
        assert_eq!(r.len(), 1);
        let r = &r[0];
        assert_eq!(r.redundancy, 3);
        assert!(
            (r.position - station).length() < 0.006,
            "{:?}",
            r.position - station
        );
        assert!(r.sigma.is_some() && r.sigma0.is_some());
        // The erroneous distance carries a sizeable residual.
        let res = r.residuals[1].1.expect("distance");
        assert!(res > 0.002, "{res}");
    }

    #[test]
    fn not_enough() {
        let station = O + DVec2::new(10.0, 30.0);
        let t = targets();
        let s = [
            sight(station, t[0], 0.0, true, false),
            sight(station, t[1], 0.0, true, false),
        ];
        assert!(matches!(
            resect(&s, Weights::default()),
            Err(ResectionError::NotEnough(_))
        ));
    }
}
