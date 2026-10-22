//! Survey networks from field observations: instrument setups (from raw
//! data files or saved traverses) are reduced to the plan directions,
//! distances and height differences that [`crate::adjustment`] solves, with
//! approximate coordinates propagated from the known stations and a priori
//! standard deviations from the instrument specification.

use std::collections::HashMap;
use std::fmt::Write as _;

use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::adjustment::{AdjustResult, NetStation, Observation};
use crate::angle::{Angle, Bearing};
use crate::conventions::Conventions;
use crate::traverse::{Direction, Reference, TraverseRecord};

/// One pointing from a setup.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawObservation {
    pub target: String,
    /// Horizontal circle reading (clockwise).
    pub hz: Option<Angle>,
    /// Zenith angle; with it, `distance` is a slope distance.
    pub zenith: Option<Angle>,
    /// Slope distance (with a zenith angle) or horizontal distance.
    pub distance: Option<f64>,
    pub target_height: f64,
    /// Height difference to the target when there is no zenith angle.
    #[serde(default)]
    pub dz: Option<f64>,
}

impl RawObservation {
    /// Horizontal distance and height difference (instrument to target point).
    pub fn reduce(&self, instrument_height: f64) -> (Option<f64>, Option<f64>) {
        match (self.zenith, self.distance) {
            (Some(z), Some(d)) => {
                let (s, c) = z.radians().sin_cos();
                (Some(d * s), Some(d * c + instrument_height - self.target_height))
            }
            (None, d) => (d, self.dz),
            (Some(_), None) => (None, None),
        }
    }
}

/// An instrument setup: every hz reading shares one orientation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Setup {
    pub station: String,
    pub instrument_height: f64,
    /// Bearing of the zero reading, when the circle was oriented to a known bearing.
    pub orientation: Option<Bearing>,
    pub observations: Vec<RawObservation>,
}

/// Field observations imported from one raw data file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FieldData {
    pub name: String,
    /// Source format, e.g. "Leica GSI" or "Trimble JobXML".
    pub format: String,
    pub setups: Vec<Setup>,
    /// Coordinates recorded in the file (control and computed points).
    #[serde(default)]
    pub points: Vec<(String, DVec3)>,
}

/// A priori precision of the observations.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkOptions {
    pub direction_sigma: Angle,
    /// Constant part of the distance standard deviation (metres).
    pub distance_sigma: f64,
    /// Distance-proportional part (parts per million).
    pub distance_ppm: f64,
    /// Constant part of height-difference standard deviation (metres).
    pub height_sigma: f64,
    /// Centring error of instrument and target (metres), added to directions.
    pub centring: f64,
    /// Ground-to-grid factor applied to horizontal distances.
    pub scale_factor: f64,
    pub heights: bool,
}

impl Default for NetworkOptions {
    fn default() -> Self {
        Self {
            direction_sigma: Angle::from_degrees(3.0 / 3600.0),
            distance_sigma: 0.002,
            distance_ppm: 2.0,
            height_sigma: 0.005,
            centring: 0.001,
            scale_factor: 1.0,
            heights: true,
        }
    }
}

/// A known station from the project.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Known {
    pub position: DVec3,
    pub height_known: bool,
    pub fixed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Network {
    pub stations: Vec<NetStation>,
    pub observations: Vec<Observation>,
    /// Short description per observation ("A → B distance").
    pub labels: Vec<String>,
    /// Stations that had no coordinates and were placed from the observations.
    pub new_stations: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Error)]
pub enum NetworkError {
    #[error("No setups to adjust")]
    Empty,
    #[error("No fixed control: fix at least one known station")]
    NoControl,
    #[error("Cannot place {0}: it needs an oriented setup with a direction and distance to it")]
    Unplaced(String),
}

/// Converts a saved traverse into setups: each leg is a setup at its start
/// with the backsight at reading zero, and the closing angle a setup at the end.
pub fn setups_from_traverse(t: &TraverseRecord) -> Vec<Setup> {
    let mut setups = Vec::new();
    let mut at = t.start.clone();
    let mut back: Option<Reference> = t.backsight.clone();
    let reference_obs = |r: &Reference| match r {
        Reference::Point(name) => (
            None,
            Some(RawObservation {
                target: name.clone(),
                hz: Some(Angle::from_degrees(0.0)),
                zenith: None,
                distance: None,
                target_height: 0.0,
                dz: None,
            }),
        ),
        Reference::Bearing(b) => (Some(*b), None),
    };
    for leg in &t.legs {
        let (hd, dz) = leg.reduce();
        let fore = |hz: Angle| RawObservation {
            target: leg.to.clone(),
            hz: Some(hz),
            zenith: None,
            distance: Some(hd),
            target_height: 0.0,
            dz,
        };
        let setup = match (leg.direction, &back) {
            (Direction::Bearing(b), _) => Setup {
                station: at.clone(),
                instrument_height: 0.0,
                orientation: Some(b),
                observations: vec![fore(Angle::from_degrees(0.0))],
            },
            (d, Some(r)) => {
                let (orientation, bs) = reference_obs(r);
                let mut observations: Vec<RawObservation> = bs.into_iter().collect();
                observations.push(fore(d.angle_right().unwrap_or_default().normalized()));
                Setup {
                    station: at.clone(),
                    instrument_height: 0.0,
                    orientation,
                    observations,
                }
            }
            // An angle with nothing to turn it from: the distance still counts.
            (_, None) => Setup {
                station: at.clone(),
                instrument_height: 0.0,
                orientation: None,
                observations: vec![RawObservation {
                    hz: None,
                    ..fore(Angle::default())
                }],
            },
        };
        setups.push(setup);
        back = Some(Reference::Point(std::mem::replace(&mut at, leg.to.clone())));
    }
    if let (Some(r), Some(angle), Some(Reference::Point(prev))) =
        (&t.closing_reference, t.closing_angle, &back)
    {
        let mut observations = vec![RawObservation {
            target: prev.clone(),
            hz: Some(Angle::from_degrees(0.0)),
            zenith: None,
            distance: None,
            target_height: 0.0,
            dz: None,
        }];
        let mut orientation = None;
        match r {
            Reference::Point(name) => observations.push(RawObservation {
                target: name.clone(),
                hz: Some(angle.normalized()),
                zenith: None,
                distance: None,
                target_height: 0.0,
                dz: None,
            }),
            // Known closing bearing: the zero reading points at bearing − angle.
            Reference::Bearing(b) => orientation = Some(Bearing::new((b.angle() - angle).normalized())),
        }
        setups.push(Setup {
            station: at,
            instrument_height: 0.0,
            orientation,
            observations,
        });
    }
    setups
}

/// Builds the network: stations are looked up with `known`; unknown ones are
/// placed by radiation from oriented setups.
pub fn build(
    setups: &[Setup],
    known: &dyn Fn(&str) -> Option<Known>,
    options: &NetworkOptions,
) -> Result<Network, NetworkError> {
    if setups.iter().all(|s| s.observations.is_empty()) {
        return Err(NetworkError::Empty);
    }
    // Station table in order of first appearance.
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut stations: Vec<NetStation> = Vec::new();
    let mut placed: Vec<bool> = Vec::new();
    let mut height_placed: Vec<bool> = Vec::new();
    let mut new_stations = Vec::new();
    let mut add =
        |name: &str, stations: &mut Vec<NetStation>, placed: &mut Vec<bool>, hp: &mut Vec<bool>| -> usize {
            if let Some(i) = index.get(name) {
                return *i;
            }
            let k = known(name);
            stations.push(NetStation {
                name: name.to_string(),
                position: k.map_or(DVec3::ZERO, |k| k.position),
                fixed_plan: k.is_some_and(|k| k.fixed),
                fixed_height: k.is_some_and(|k| k.fixed && k.height_known),
            });
            placed.push(k.is_some());
            hp.push(k.is_some_and(|k| k.height_known));
            index.insert(name.to_string(), stations.len() - 1);
            stations.len() - 1
        };
    let mut setup_at = Vec::new();
    for s in setups {
        let i = add(&s.station, &mut stations, &mut placed, &mut height_placed);
        let targets: Vec<usize> = s
            .observations
            .iter()
            .map(|o| add(&o.target, &mut stations, &mut placed, &mut height_placed))
            .collect();
        setup_at.push((i, targets));
    }
    if !stations.iter().any(|s| s.fixed_plan) {
        return Err(NetworkError::NoControl);
    }
    for (i, p) in placed.iter().enumerate() {
        if !p {
            new_stations.push(i);
        }
    }

    // Propagate approximate positions until nothing more can be placed.
    let mut orient: Vec<Option<f64>> = vec![None; setups.len()];
    loop {
        let mut progress = false;
        for (k, s) in setups.iter().enumerate() {
            let (at, targets) = &setup_at[k];
            if !placed[*at] {
                continue;
            }
            if orient[k].is_none() {
                orient[k] = s.orientation.map(|b| b.radians()).or_else(|| {
                    let (mut sn, mut cs, mut n) = (0.0, 0.0, 0);
                    for (o, t) in s.observations.iter().zip(targets) {
                        if let (Some(hz), true) = (o.hz, placed[*t]) {
                            let d = stations[*t].position.truncate() - stations[*at].position.truncate();
                            if d.length_squared() > 1e-12 {
                                let a = d.x.atan2(d.y) - hz.radians();
                                sn += a.sin();
                                cs += a.cos();
                                n += 1;
                            }
                        }
                    }
                    (n > 0).then(|| sn.atan2(cs))
                });
            }
            let Some(w) = orient[k] else { continue };
            for (o, t) in s.observations.iter().zip(targets) {
                let (hd, dz) = o.reduce(s.instrument_height);
                if !placed[*t]
                    && let (Some(hz), Some(hd)) = (o.hz, hd)
                {
                    let b = hz.radians() + w;
                    let from = stations[*at].position;
                    let p = from.truncate() + DVec2::new(b.sin(), b.cos()) * hd * options.scale_factor;
                    stations[*t].position = p.extend(from.z + dz.unwrap_or(0.0));
                    placed[*t] = true;
                    height_placed[*t] = height_placed[*at] && dz.is_some();
                    progress = true;
                }
            }
        }
        if !progress {
            break;
        }
    }
    if let Some(i) = placed.iter().position(|p| !p) {
        return Err(NetworkError::Unplaced(stations[i].name.clone()));
    }

    // Observations.
    let use_heights = options.heights && stations.iter().any(|s| s.fixed_height);
    let sd = |d: f64| (options.distance_sigma.powi(2) + (d * options.distance_ppm * 1e-6).powi(2)).sqrt();
    let mut observations = Vec::new();
    let mut labels = Vec::new();
    let mut set = 0;
    for (k, s) in setups.iter().enumerate() {
        let (at, targets) = &setup_at[k];
        let has_dirs = s.observations.iter().filter(|o| o.hz.is_some()).count();
        // A lone direction in an unoriented set carries no information.
        let use_set = s.orientation.is_none() && has_dirs >= 2;
        for (o, t) in s.observations.iter().zip(targets) {
            if t == at {
                continue;
            }
            let (hd, dz) = o.reduce(s.instrument_height);
            let len = (stations[*t].position - stations[*at].position)
                .truncate()
                .length()
                .max(1.0);
            let dir_sigma =
                (options.direction_sigma.radians().powi(2) + 2.0 * (options.centring / len).powi(2)).sqrt();
            let name = |what: &str| format!("{} → {} {what}", s.station, o.target);
            if let Some(hz) = o.hz {
                if let Some(b) = s.orientation {
                    observations.push(Observation::Bearing {
                        from: *at,
                        to: *t,
                        value: (b.angle() + hz).normalized(),
                        sigma: Angle::from_radians(dir_sigma),
                    });
                    labels.push(name("bearing"));
                } else if use_set {
                    observations.push(Observation::Direction {
                        set,
                        from: *at,
                        to: *t,
                        value: hz,
                        sigma: Angle::from_radians(dir_sigma),
                    });
                    labels.push(name("direction"));
                }
            }
            if let Some(hd) = hd {
                let grid = hd * options.scale_factor;
                observations.push(Observation::Distance {
                    from: *at,
                    to: *t,
                    value: grid,
                    sigma: sd(grid),
                });
                labels.push(name("distance"));
            }
            if use_heights && let (Some(dz), Some(hd)) = (dz, hd) {
                let zsig = if o.zenith.is_some() {
                    options.direction_sigma.radians() * hd
                } else {
                    0.0
                };
                observations.push(Observation::HeightDiff {
                    from: *at,
                    to: *t,
                    value: dz,
                    sigma: (options.height_sigma.powi(2) + zsig * zsig).sqrt(),
                });
                labels.push(name("height diff"));
            }
        }
        if use_set {
            set += 1;
        }
    }
    // Heights only for stations tied to a fixed height.
    if use_heights {
        for (i, s) in stations.iter_mut().enumerate() {
            if !height_placed[i] && !s.fixed_height {
                s.position.z = 0.0;
            }
        }
    }
    Ok(Network {
        stations,
        observations,
        labels,
        new_stations,
    })
}

/// A plain-text adjustment report.
pub fn report(net: &Network, r: &AdjustResult, options: &NetworkOptions, conv: &Conventions) -> String {
    let mut s = String::new();
    let sec = |rad: f64| rad.to_degrees() * 3600.0;
    let _ = writeln!(s, "LEAST-SQUARES NETWORK ADJUSTMENT");
    let _ = writeln!(
        s,
        "Stations {}  (fixed {})   Observations {}   Degrees of freedom {}   Iterations {}",
        net.stations.len(),
        net.stations.iter().filter(|s| s.fixed_plan).count(),
        net.observations.len(),
        r.dof,
        r.iterations
    );
    let _ = writeln!(
        s,
        "A priori: directions {:.1}\"  distances {:.1} mm + {:.1} ppm  heights {:.1} mm  centring {:.1} mm  scale {:.8}",
        sec(options.direction_sigma.radians()),
        options.distance_sigma * 1000.0,
        options.distance_ppm,
        options.height_sigma * 1000.0,
        options.centring * 1000.0,
        options.scale_factor
    );
    let _ = writeln!(s, "Standard error of unit weight σ₀ = {:.3}", r.sigma0);
    match r.chi_square {
        Some(c) => {
            let _ = writeln!(
                s,
                "χ² test (95%): vᵀPv = {:.2}, accept {:.2} … {:.2}  →  {}",
                c.value,
                c.lower,
                c.upper,
                if c.pass {
                    "PASS"
                } else if c.value > c.upper {
                    "FAIL (observations worse than their a priori precision, or a blunder)"
                } else {
                    "FAIL (a priori precision is pessimistic)"
                }
            );
        }
        None => {
            let _ = writeln!(s, "No redundancy: the network has no checks.");
        }
    }
    let (ea, nb) = conv.horizontal_labels();
    let _ = writeln!(s, "\nADJUSTED COORDINATES");
    let _ = writeln!(
        s,
        "{:<12} {:>14} {:>14} {:>10}  {:>6} {:>6} {:>6}  {:>7} {:>7} {:>10}",
        "Station", ea, nb, "Height", "σE mm", "σN mm", "σH mm", "a95 mm", "b95 mm", "Bearing a"
    );
    for (i, st) in net.stations.iter().enumerate() {
        let p = r.positions[i];
        let (a, b) = conv.horizontal_values(p);
        let _ = write!(
            s,
            "{:<12} {:>14} {:>14} {:>10}",
            st.name,
            conv.format_length(a),
            conv.format_length(b),
            conv.format_length(p.z)
        );
        match r.ellipses[i] {
            Some(e) => {
                let _ = writeln!(
                    s,
                    "  {:>6.1} {:>6.1} {:>6.1}  {:>7.1} {:>7.1} {:>10}",
                    r.sd[i].x * 1000.0,
                    r.sd[i].y * 1000.0,
                    r.sd[i].z * 1000.0,
                    e.semi_major * 1000.0,
                    e.semi_minor * 1000.0,
                    conv.format_bearing(Bearing::new(e.bearing))
                );
            }
            None => {
                let _ = writeln!(s, "  {}", if st.fixed_plan { "fixed" } else { "" });
            }
        }
    }
    if !r.orientations.is_empty() {
        let _ = writeln!(s, "\nSET ORIENTATIONS (bearing of zero reading)");
        for (k, w) in r.orientations.iter().enumerate() {
            let _ = writeln!(s, "  set {:<4} {}", k + 1, conv.format_bearing(Bearing::new(*w)));
        }
    }
    let _ = writeln!(s, "\nRESIDUALS (observed − adjusted; |w| > 3 flagged)");
    let _ = writeln!(s, "{:<34} {:>12} {:>8}", "Observation", "Residual", "w");
    for ((o, label), v) in net.observations.iter().zip(&net.labels).zip(&r.residuals) {
        let value = match o {
            Observation::Direction { .. } | Observation::Bearing { .. } => format!("{:.1}\"", sec(v.value)),
            _ => format!("{:.1} mm", v.value * 1000.0),
        };
        let w = v.standardised.map_or("—".to_string(), |w| format!("{w:.2}"));
        let flag = if v.standardised.is_some_and(|w| w.abs() > 3.0) {
            "  ◀ check"
        } else {
            ""
        };
        let _ = writeln!(s, "{label:<34} {value:>12} {w:>8}{flag}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adjustment::adjust;
    use crate::traverse::Leg;

    const O: DVec3 = DVec3::new(500_000.0, 7_000_000.0, 10.0);

    fn ang(deg: f64) -> Direction {
        Direction::Angle(Angle::from_degrees(deg))
    }

    /// The square loop from the traverse tests, closing back on A.
    fn square(first_leg_error: f64) -> TraverseRecord {
        TraverseRecord {
            name: "Loop".into(),
            start: "A".into(),
            backsight: Some(Reference::Bearing(Bearing::from_degrees(180.0))),
            legs: vec![
                Leg::new("B", ang(270.0), 100.0 + first_leg_error),
                Leg::new("C", ang(270.0), 100.0),
                Leg::new("D", ang(270.0), 100.0),
                Leg::new("A", ang(270.0), 100.0),
            ],
            close_on: Some("A".into()),
            closing_reference: Some(Reference::Point("B".into())),
            closing_angle: Some(Angle::from_degrees(270.0)),
            ..Default::default()
        }
    }

    fn known(name: &str) -> Option<Known> {
        (name == "A").then_some(Known {
            position: O,
            height_known: true,
            fixed: true,
        })
    }

    #[test]
    fn traverse_loop_adjusts() {
        let setups = setups_from_traverse(&square(0.0));
        assert_eq!(setups.len(), 5);
        assert_eq!(setups[0].orientation, Some(Bearing::from_degrees(180.0)));
        let net = build(&setups, &known, &NetworkOptions::default()).expect("network");
        assert_eq!(net.stations.len(), 4);
        assert_eq!(net.new_stations, vec![1, 2, 3]);
        let r = adjust(&net.stations, &net.observations).expect("adjust");
        let c = r.positions[2] - O;
        assert!(
            (c.truncate() - DVec2::new(100.0, -100.0)).length() < 1e-6,
            "{c:?}"
        );
        assert!(r.dof > 0);

        // A 30 mm error on one leg is spread and shows in the residuals.
        let net = build(
            &setups_from_traverse(&square(0.03)),
            &known,
            &NetworkOptions::default(),
        )
        .expect("network");
        let r = adjust(&net.stations, &net.observations).expect("adjust");
        let txt = report(
            &net,
            &r,
            &NetworkOptions::default(),
            &Conventions::metric_generic(),
        );
        assert!(
            txt.contains("ADJUSTED COORDINATES") && txt.contains("A → B distance"),
            "{txt}"
        );
        assert!(r.sigma0 > 1.0);
    }

    #[test]
    fn radiation_from_raw_setup() {
        let o = |t: &str, hz: f64, d: f64, zen: f64| RawObservation {
            target: t.into(),
            hz: Some(Angle::from_degrees(hz)),
            zenith: Some(Angle::from_degrees(zen)),
            distance: Some(d),
            target_height: 1.5,
            dz: None,
        };
        let setups = vec![Setup {
            station: "A".into(),
            instrument_height: 1.5,
            orientation: None,
            observations: vec![o("R", 0.0, 200.0, 90.0), o("P", 90.0, 50.0, 80.0)],
        }];
        let k = |n: &str| match n {
            "A" => known("A"),
            "R" => Some(Known {
                position: O + DVec3::new(0.0, 200.0, 0.0),
                height_known: true,
                fixed: true,
            }),
            _ => None,
        };
        let net = build(&setups, &k, &NetworkOptions::default()).expect("network");
        let p = net.stations[2].position - O;
        let hd = 50.0 * 80f64.to_radians().sin();
        assert!((p.x - hd).abs() < 1e-9 && p.y.abs() < 1e-9, "{p:?}");
        assert!((p.z - 50.0 * 80f64.to_radians().cos()).abs() < 1e-9);
        let r = adjust(&net.stations, &net.observations).expect("adjust");
        assert!((r.positions[2] - net.stations[2].position).length() < 1e-6);
        assert!(build(&setups, &|_| None, &NetworkOptions::default()) == Err(NetworkError::NoControl));
    }
}
