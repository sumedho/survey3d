//! Least-squares adjustment of survey networks.
//!
//! The plan network is solved by weighted Gauss–Newton iteration with
//! unknown easting/northing for every free station and an orientation for
//! every set of directions. Heights are solved separately from height
//! differences (a linear problem). The results include residuals,
//! standardised residuals (for blunder detection), the standard error of
//! unit weight σ₀ with the χ² global test, and 95 % error ellipses.

use glam::{DVec2, DVec3};
use thiserror::Error;

use crate::angle::Angle;

/// A station of the network.
#[derive(Clone, Debug, PartialEq)]
pub struct NetStation {
    pub name: String,
    /// Approximate (or, when fixed, known) coordinates.
    pub position: DVec3,
    pub fixed_plan: bool,
    /// Fixed height (only meaningful with height observations).
    pub fixed_height: bool,
}

/// One observation between two stations (indices into the stations).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Observation {
    /// A horizontal circle reading in direction set `set` (each set has its own orientation).
    Direction {
        set: usize,
        from: usize,
        to: usize,
        value: Angle,
        sigma: Angle,
    },
    /// A horizontal (grid) distance.
    Distance {
        from: usize,
        to: usize,
        value: f64,
        sigma: f64,
    },
    /// A grid bearing.
    Bearing {
        from: usize,
        to: usize,
        value: Angle,
        sigma: Angle,
    },
    /// Height of `to` minus height of `from`.
    HeightDiff {
        from: usize,
        to: usize,
        value: f64,
        sigma: f64,
    },
}

impl Observation {
    pub fn stations(&self) -> (usize, usize) {
        match *self {
            Observation::Direction { from, to, .. }
            | Observation::Distance { from, to, .. }
            | Observation::Bearing { from, to, .. }
            | Observation::HeightDiff { from, to, .. } => (from, to),
        }
    }

    fn is_plan(&self) -> bool {
        !matches!(self, Observation::HeightDiff { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Error)]
pub enum AdjustError {
    #[error("The network has no observations")]
    NoObservations,
    #[error("Observation {0} refers to a station that does not exist")]
    BadStation(usize),
    #[error("The network cannot be solved: fix more control or add observations (datum defect)")]
    Singular,
    #[error("The adjustment did not converge (check approximate coordinates and observations)")]
    NoConvergence,
}

/// A 95 % error ellipse.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ellipse {
    pub semi_major: f64,
    pub semi_minor: f64,
    /// Bearing of the major axis.
    pub bearing: Angle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Residual {
    /// Observed minus adjusted, in the observation's units (radians or metres).
    pub value: f64,
    /// Residual over its standard deviation; |w| > 3 suggests a blunder.
    pub standardised: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdjustResult {
    pub positions: Vec<DVec3>,
    /// Standard deviations of easting, northing and height (0 for fixed).
    pub sd: Vec<DVec3>,
    pub ellipses: Vec<Option<Ellipse>>,
    /// Orientation of each direction set: bearing = reading + orientation.
    pub orientations: Vec<Angle>,
    pub residuals: Vec<Residual>,
    /// Degrees of freedom (plan and height together).
    pub dof: usize,
    /// Standard error of unit weight (1 when the weights are right).
    pub sigma0: f64,
    /// χ² global test of vᵀPv at 95 %: `Some(pass)` when there is redundancy.
    pub chi_square: Option<ChiSquare>,
    pub iterations: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChiSquare {
    pub value: f64,
    pub lower: f64,
    pub upper: f64,
    pub pass: bool,
}

fn wrap(a: f64) -> f64 {
    let t = std::f64::consts::TAU;
    let r = a.rem_euclid(t);
    if r > std::f64::consts::PI { r - t } else { r }
}

/// Wilson–Hilferty approximation of the χ² quantile for `k` degrees of freedom.
pub fn chi2_quantile(k: f64, z: f64) -> f64 {
    let h = 2.0 / (9.0 * k);
    k * (1.0 - h + z * h.sqrt()).powi(3)
}

/// Cholesky solve of a symmetric positive-definite system; `None` if singular.
fn cholesky(n: &[f64], size: usize) -> Option<Vec<f64>> {
    let mut l = vec![0.0; size * size];
    let scale = (0..size)
        .map(|i| n[i * size + i].abs())
        .fold(0.0, f64::max)
        .max(1e-300);
    for i in 0..size {
        for j in 0..=i {
            let mut s = n[i * size + j];
            for k in 0..j {
                s -= l[i * size + k] * l[j * size + k];
            }
            if i == j {
                if s <= 1e-12 * scale {
                    return None;
                }
                l[i * size + i] = s.sqrt();
            } else {
                l[i * size + j] = s / l[j * size + j];
            }
        }
    }
    Some(l)
}

fn chol_solve(l: &[f64], size: usize, b: &[f64]) -> Vec<f64> {
    let mut y = vec![0.0; size];
    for i in 0..size {
        let mut s = b[i];
        for k in 0..i {
            s -= l[i * size + k] * y[k];
        }
        y[i] = s / l[i * size + i];
    }
    let mut x = vec![0.0; size];
    for i in (0..size).rev() {
        let mut s = y[i];
        for k in i + 1..size {
            s -= l[k * size + i] * x[k];
        }
        x[i] = s / l[i * size + i];
    }
    x
}

/// Inverse of the normal matrix from its Cholesky factor (dense).
fn chol_inverse(l: &[f64], size: usize) -> Vec<f64> {
    let mut inv = vec![0.0; size * size];
    let mut e = vec![0.0; size];
    for c in 0..size {
        e.iter_mut().for_each(|v| *v = 0.0);
        e[c] = 1.0;
        let col = chol_solve(l, size, &e);
        for r in 0..size {
            inv[r * size + c] = col[r];
        }
    }
    inv
}

/// A sparse row of the design matrix: (unknown index, coefficient).
type Row = Vec<(usize, f64)>;

/// Adjusts the network.
pub fn adjust(stations: &[NetStation], observations: &[Observation]) -> Result<AdjustResult, AdjustError> {
    if observations.is_empty() {
        return Err(AdjustError::NoObservations);
    }
    for (i, o) in observations.iter().enumerate() {
        let (a, b) = o.stations();
        if a >= stations.len() || b >= stations.len() || a == b {
            return Err(AdjustError::BadStation(i + 1));
        }
    }
    // Work relative to the first station for precision.
    let origin = stations
        .first()
        .map(|s| s.position.truncate())
        .unwrap_or_default();
    let mut pos: Vec<DVec2> = stations.iter().map(|s| s.position.truncate() - origin).collect();

    // Unknown numbering: free stations' (E, N), then set orientations.
    let mut index: Vec<Option<usize>> = Vec::with_capacity(stations.len());
    let mut u = 0;
    for s in stations {
        index.push((!s.fixed_plan).then(|| {
            u += 2;
            u - 2
        }));
    }
    let sets: usize = observations
        .iter()
        .filter_map(|o| match o {
            Observation::Direction { set, .. } => Some(*set + 1),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let set_index = |s: usize| u + s;
    let unknowns = u + sets;
    let plan_obs: Vec<&Observation> = observations.iter().filter(|o| o.is_plan()).collect();

    // Initial orientations: mean of (bearing − reading) per set.
    let mut omega = vec![0.0; sets];
    {
        let mut acc = vec![(0.0, 0.0); sets];
        for o in &plan_obs {
            if let Observation::Direction {
                set, from, to, value, ..
            } = **o
            {
                let d = pos[to] - pos[from];
                let a = d.x.atan2(d.y) - value.radians();
                acc[set].0 += a.sin();
                acc[set].1 += a.cos();
            }
        }
        for (k, (s, c)) in acc.into_iter().enumerate() {
            omega[k] = s.atan2(c);
        }
    }

    // Linearise: (misclosure obs − computed, row, weight).
    let linearise = |pos: &[DVec2], omega: &[f64]| -> Vec<(f64, Row, f64)> {
        plan_obs
            .iter()
            .map(|o| {
                let (from, to) = o.stations();
                let d = pos[to] - pos[from];
                let d2 = d.length_squared().max(1e-18);
                let dist = d2.sqrt();
                let mut row: Row = Vec::new();
                let mut put = |st: usize, de: f64, dn: f64| {
                    if let Some(k) = index[st] {
                        row.push((k, de));
                        row.push((k + 1, dn));
                    }
                };
                let bearing = d.x.atan2(d.y);
                match **o {
                    Observation::Direction {
                        set, value, sigma, ..
                    } => {
                        put(to, d.y / d2, -d.x / d2);
                        put(from, -d.y / d2, d.x / d2);
                        row.push((set_index(set), -1.0));
                        let w = 1.0 / sigma.radians().powi(2);
                        (wrap(value.radians() - (bearing - omega[set])), row, w)
                    }
                    Observation::Bearing { value, sigma, .. } => {
                        put(to, d.y / d2, -d.x / d2);
                        put(from, -d.y / d2, d.x / d2);
                        (
                            wrap(value.radians() - bearing),
                            row,
                            1.0 / sigma.radians().powi(2),
                        )
                    }
                    Observation::Distance { value, sigma, .. } => {
                        put(to, d.x / dist, d.y / dist);
                        put(from, -d.x / dist, -d.y / dist);
                        (value - dist, row, 1.0 / sigma.powi(2))
                    }
                    Observation::HeightDiff { .. } => unreachable!("plan observations only"),
                }
            })
            .collect()
    };

    let mut iterations = 0;
    let mut chol: Option<Vec<f64>> = None;
    if unknowns > 0 && !plan_obs.is_empty() {
        let mut converged = false;
        for it in 0..20 {
            iterations = it + 1;
            let rows = linearise(&pos, &omega);
            let mut n = vec![0.0; unknowns * unknowns];
            let mut b = vec![0.0; unknowns];
            for (l, row, w) in &rows {
                for &(i, ai) in row {
                    b[i] += ai * w * l;
                    for &(j, aj) in row {
                        n[i * unknowns + j] += ai * w * aj;
                    }
                }
            }
            let l = cholesky(&n, unknowns).ok_or(AdjustError::Singular)?;
            let dx = chol_solve(&l, unknowns, &b);
            for (st, k) in index.iter().enumerate() {
                if let Some(k) = k {
                    pos[st] += DVec2::new(dx[*k], dx[*k + 1]);
                }
            }
            for s in 0..sets {
                omega[s] += dx[set_index(s)];
            }
            chol = Some(l);
            let step = index
                .iter()
                .flatten()
                .map(|k| DVec2::new(dx[*k], dx[*k + 1]).length())
                .fold(0.0, f64::max);
            if step < 1e-7 {
                converged = true;
                break;
            }
        }
        if !converged {
            return Err(AdjustError::NoConvergence);
        }
    }

    // Plan residuals and statistics.
    let rows = if plan_obs.is_empty() {
        Vec::new()
    } else {
        linearise(&pos, &omega)
    };
    let qxx = chol.as_ref().map(|l| chol_inverse(l, unknowns));
    let mut vtpv = 0.0;
    let mut plan_res: Vec<(f64, Option<f64>)> = Vec::new();
    for (l, row, w) in &rows {
        // l is now obs − computed at the solution: the residual (observed − adjusted).
        vtpv += w * l * l;
        let q_ll = 1.0 / w;
        let q_hat = qxx.as_ref().map_or(0.0, |q| {
            let mut s = 0.0;
            for &(i, ai) in row {
                for &(j, aj) in row {
                    s += ai * q[i * unknowns + j] * aj;
                }
            }
            s
        });
        let q_vv = q_ll - q_hat;
        plan_res.push((*l, (q_vv > 1e-12 * q_ll).then(|| l / q_vv.sqrt())));
    }

    // Heights: a linear adjustment of the height differences.
    let height_obs: Vec<&Observation> = observations.iter().filter(|o| !o.is_plan()).collect();
    let mut heights: Vec<f64> = stations.iter().map(|s| s.position.z).collect();
    let mut h_index: Vec<Option<usize>> = Vec::new();
    let mut hu = 0;
    let involved: Vec<bool> = (0..stations.len())
        .map(|i| {
            height_obs.iter().any(|o| {
                let (a, b) = o.stations();
                a == i || b == i
            })
        })
        .collect();
    for (i, s) in stations.iter().enumerate() {
        h_index.push((involved[i] && !s.fixed_height).then(|| {
            hu += 1;
            hu - 1
        }));
    }
    let mut h_res: Vec<(f64, Option<f64>)> = Vec::new();
    let mut h_q: Option<Vec<f64>> = None;
    if !height_obs.is_empty() {
        let mut n = vec![0.0; hu * hu];
        let mut b = vec![0.0; hu];
        let lin = |heights: &[f64]| -> Vec<(f64, Row, f64)> {
            height_obs
                .iter()
                .map(|o| match **o {
                    Observation::HeightDiff {
                        from,
                        to,
                        value,
                        sigma,
                    } => {
                        let mut row: Row = Vec::new();
                        if let Some(k) = h_index[to] {
                            row.push((k, 1.0));
                        }
                        if let Some(k) = h_index[from] {
                            row.push((k, -1.0));
                        }
                        (value - (heights[to] - heights[from]), row, 1.0 / sigma.powi(2))
                    }
                    _ => unreachable!("height observations only"),
                })
                .collect()
        };
        if hu > 0 {
            for (l, row, w) in lin(&heights) {
                for &(i, ai) in &row {
                    b[i] += ai * w * l;
                    for &(j, aj) in &row {
                        n[i * hu + j] += ai * w * aj;
                    }
                }
            }
            let l = cholesky(&n, hu).ok_or(AdjustError::Singular)?;
            let dx = chol_solve(&l, hu, &b);
            for (i, k) in h_index.iter().enumerate() {
                if let Some(k) = k {
                    heights[i] += dx[*k];
                }
            }
            h_q = Some(chol_inverse(&l, hu));
        }
        for (l, row, w) in lin(&heights) {
            vtpv += w * l * l;
            let q_ll = 1.0 / w;
            let q_hat = h_q.as_ref().map_or(0.0, |q| {
                let mut s = 0.0;
                for &(i, ai) in &row {
                    for &(j, aj) in &row {
                        s += ai * q[i * hu + j] * aj;
                    }
                }
                s
            });
            let q_vv = q_ll - q_hat;
            h_res.push((l, (q_vv > 1e-12 * q_ll).then(|| l / q_vv.sqrt())));
        }
    }

    let n_obs = plan_obs.len() + height_obs.len();
    let dof = n_obs.saturating_sub(unknowns + hu);
    let sigma0 = if dof > 0 { (vtpv / dof as f64).sqrt() } else { 1.0 };
    let chi_square = (dof > 0).then(|| {
        let k = dof as f64;
        let (lower, upper) = (chi2_quantile(k, -1.959_964), chi2_quantile(k, 1.959_964));
        ChiSquare {
            value: vtpv,
            lower,
            upper,
            pass: vtpv >= lower && vtpv <= upper,
        }
    });
    // Standardised residuals use σ₀ = 1 (a priori); scale with σ₀ for a posteriori.
    let mut plan_iter = plan_res.into_iter();
    let mut h_iter = h_res.into_iter();
    let residuals = observations
        .iter()
        .map(|o| {
            let (v, w) = if o.is_plan() {
                plan_iter.next().unwrap_or((0.0, None))
            } else {
                h_iter.next().unwrap_or((0.0, None))
            };
            Residual {
                value: v,
                standardised: w,
            }
        })
        .collect();

    // Standard deviations and 95 % ellipses (a posteriori variance).
    let k95 = 2.447_746_830_680_816; // √χ²(2, 0.95)
    let s0sq = sigma0 * sigma0;
    let mut sd = Vec::with_capacity(stations.len());
    let mut ellipses = Vec::with_capacity(stations.len());
    for i in 0..stations.len() {
        let (se, sn, ellipse) = match (index[i], &qxx) {
            (Some(k), Some(q)) => {
                let (qee, qnn, qen) = (
                    q[k * unknowns + k],
                    q[(k + 1) * unknowns + k + 1],
                    q[k * unknowns + k + 1],
                );
                let (cee, cnn, cen) = (qee * s0sq, qnn * s0sq, qen * s0sq);
                let mean = 0.5 * (cee + cnn);
                let diff = (0.25 * (cee - cnn).powi(2) + cen * cen).sqrt();
                let (l1, l2) = (mean + diff, (mean - diff).max(0.0));
                // Major axis direction (bearing from north).
                let theta = 0.5 * (2.0 * cen).atan2(cnn - cee);
                (
                    cee.sqrt(),
                    cnn.sqrt(),
                    Some(Ellipse {
                        semi_major: k95 * l1.sqrt(),
                        semi_minor: k95 * l2.sqrt(),
                        bearing: Angle::from_radians(theta).normalized(),
                    }),
                )
            }
            _ => (0.0, 0.0, None),
        };
        let sh = match (h_index[i], &h_q) {
            (Some(k), Some(q)) => (q[k * hu + k] * s0sq).sqrt(),
            _ => 0.0,
        };
        sd.push(DVec3::new(se, sn, sh));
        ellipses.push(ellipse);
    }

    Ok(AdjustResult {
        positions: pos
            .iter()
            .zip(&heights)
            .map(|(p, z)| (*p + origin).extend(*z))
            .collect(),
        sd,
        ellipses,
        orientations: omega
            .into_iter()
            .map(|w| Angle::from_radians(w).normalized())
            .collect(),
        residuals,
        dof,
        sigma0,
        chi_square,
        iterations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::angle::Bearing;

    const O: DVec3 = DVec3::new(512_000.0, 6_912_000.0, 50.0);

    fn st(name: &str, x: f64, y: f64, fixed: bool) -> NetStation {
        NetStation {
            name: name.into(),
            position: O + DVec3::new(x, y, 0.0),
            fixed_plan: fixed,
            fixed_height: fixed,
        }
    }

    fn bearing(a: DVec3, b: DVec3) -> f64 {
        let d = b - a;
        Bearing::from_delta(d.x, d.y).expect("distinct").radians()
    }

    /// Observations computed from `truth`, with direction sets at each station.
    fn observe(truth: &[DVec3], set_orient: f64) -> Vec<Observation> {
        let sig_d = Angle::from_degrees(3.0 / 3600.0);
        let mut obs = Vec::new();
        let mut set = 0;
        for i in 0..truth.len() {
            for j in 0..truth.len() {
                if i == j {
                    continue;
                }
                obs.push(Observation::Direction {
                    set,
                    from: i,
                    to: j,
                    value: Angle::from_radians(wrap(bearing(truth[i], truth[j]) - set_orient)),
                    sigma: sig_d,
                });
                if i < j {
                    obs.push(Observation::Distance {
                        from: i,
                        to: j,
                        value: (truth[j] - truth[i]).truncate().length(),
                        sigma: 0.003,
                    });
                }
            }
            set += 1;
        }
        obs
    }

    #[test]
    fn recovers_a_free_point_exactly() {
        let truth = [
            O + DVec3::new(0.0, 0.0, 0.0),
            O + DVec3::new(200.0, 0.0, 0.0),
            O + DVec3::new(90.0, 150.0, 0.0),
        ];
        // Two fixed, one free with a poor approximation.
        let stations = vec![
            st("A", 0.0, 0.0, true),
            st("B", 200.0, 0.0, true),
            st("C", 80.0, 140.0, false),
        ];
        let r = adjust(&stations, &observe(&truth, 0.7)).expect("adjust");
        assert!(
            (r.positions[2] - truth[2]).truncate().length() < 1e-6,
            "{:?}",
            r.positions[2] - truth[2]
        );
        assert!(r.residuals.iter().all(|v| v.value.abs() < 1e-6));
        assert!(r.dof > 0 && r.iterations > 1);
        for o in &r.orientations {
            assert!(
                (o.radians() - 0.7).abs() < 1e-8 || (o.radians() - 0.7 - std::f64::consts::TAU).abs() < 1e-8
            );
        }
        assert!(r.ellipses[2].is_some() && r.ellipses[0].is_none());
    }

    #[test]
    fn statistics_and_blunders() {
        let truth = [
            O,
            O + DVec3::new(300.0, 0.0, 0.0),
            O + DVec3::new(300.0, 250.0, 0.0),
            O + DVec3::new(0.0, 250.0, 0.0),
            O + DVec3::new(140.0, 120.0, 0.0),
        ];
        let stations = vec![
            st("1", 0.0, 0.0, true),
            st("2", 300.0, 0.0, true),
            st("3", 301.0, 249.0, false),
            st("4", -1.0, 251.0, false),
            st("5", 141.0, 119.0, false),
        ];
        let mut obs = observe(&truth, 0.0);
        // A 5 cm blunder on one distance.
        let k = obs
            .iter()
            .position(|o| matches!(o, Observation::Distance { from: 2, to: 4, .. }))
            .expect("obs");
        if let Observation::Distance { value, .. } = &mut obs[k] {
            *value += 0.05;
        }
        let r = adjust(&stations, &obs).expect("adjust");
        let chi = r.chi_square.expect("redundant");
        assert!(!chi.pass, "a blunder fails the global test: {chi:?}");
        // The blunder has the largest standardised residual.
        let worst = r
            .residuals
            .iter()
            .enumerate()
            .max_by(|a, b| {
                a.1.standardised
                    .unwrap_or(0.0)
                    .abs()
                    .total_cmp(&b.1.standardised.unwrap_or(0.0).abs())
            })
            .map(|(i, _)| i);
        assert_eq!(worst, Some(k));
        assert!(r.sigma0 > 1.0);
    }

    #[test]
    fn heights_and_datum_defect() {
        let stations = vec![
            NetStation {
                name: "BM".into(),
                position: O + DVec3::new(0.0, 0.0, 0.0),
                fixed_plan: true,
                fixed_height: true,
            },
            NetStation {
                name: "P".into(),
                position: O + DVec3::new(100.0, 0.0, 0.0),
                fixed_plan: true,
                fixed_height: false,
            },
        ];
        // Two levelling runs disagreeing by 2 mm: the mean wins.
        let obs = [
            Observation::HeightDiff {
                from: 0,
                to: 1,
                value: 1.234,
                sigma: 0.001,
            },
            Observation::HeightDiff {
                from: 0,
                to: 1,
                value: 1.236,
                sigma: 0.001,
            },
        ];
        let r = adjust(&stations, &obs).expect("adjust");
        assert!((r.positions[1].z - (O.z + 1.235)).abs() < 1e-9);
        assert_eq!(r.dof, 1);
        assert!((r.residuals[0].value + 0.001).abs() < 1e-9);
        // Nothing fixed in plan: singular.
        let free = vec![st("A", 0.0, 0.0, false), st("B", 100.0, 0.0, false)];
        let d = [Observation::Distance {
            from: 0,
            to: 1,
            value: 100.0,
            sigma: 0.003,
        }];
        assert_eq!(adjust(&free, &d), Err(AdjustError::Singular));
        assert!((chi2_quantile(10.0, 1.959_964) - 20.483).abs() < 0.1);
    }
}
