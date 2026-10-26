//! Reduction of raw field data to coordinates: each setup is oriented on its
//! backsight and its shots are radiated from the station, setup by setup, so
//! a point placed by one setup can be occupied by the next (an open traverse
//! carried forward). Shots to points that already have coordinates are checks.
//! Pointings to the same target from one setup (both faces, repeated shots)
//! are averaged. For a least squares solution of the control, use
//! [`crate::network`]; reduction then places the detail from it.

use std::collections::HashMap;

use glam::{DVec2, DVec3};

use crate::angle::{Angle, Bearing};
use crate::network::{FieldData, Known, Setup};

/// A point the reduction placed.
#[derive(Clone, Debug, PartialEq)]
pub struct ReducedPoint {
    pub name: String,
    pub position: DVec3,
    pub height_known: bool,
    pub code: String,
    /// Index of the setup that placed it.
    pub setup: usize,
    /// Pointings averaged into it.
    pub shots: usize,
}

/// A shot to a point that already had coordinates: how far the reduced
/// position falls from them (reduced − known).
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub setup: usize,
    pub target: String,
    pub delta: DVec2,
    pub dz: Option<f64>,
    /// This was the setup's backsight.
    pub backsight: bool,
}

/// How a setup's circle was oriented.
#[derive(Clone, Debug, PartialEq)]
pub enum Orientation {
    /// The file gave the bearing of the zero reading.
    Recorded,
    /// From a pointing to (or the recorded circle reading on) this point.
    Backsight(String),
    /// Nothing to orient on: the readings were taken as bearings.
    Assumed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SetupReduction {
    pub station: String,
    /// Bearing of the zero reading; `None` when the setup could not be reduced.
    pub orientation: Option<Bearing>,
    pub method: Option<Orientation>,
    pub shots: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reduction {
    pub points: Vec<ReducedPoint>,
    pub checks: Vec<Check>,
    pub setups: Vec<SetupReduction>,
    pub warnings: Vec<String>,
}

impl Reduction {
    /// The largest plan and height check differences, among non-backsight checks
    /// and backsights together.
    pub fn worst_check(&self) -> Option<(f64, Option<f64>)> {
        if self.checks.is_empty() {
            return None;
        }
        let h = self.checks.iter().map(|c| c.delta.length()).fold(0.0, f64::max);
        let v = self
            .checks
            .iter()
            .filter_map(|c| c.dz.map(f64::abs))
            .reduce(f64::max);
        Some((h, v))
    }
}

/// Pointings from one setup to one target: the target, each pointing's plan
/// position and height, and the code.
type Group = (String, Vec<(DVec2, Option<f64>)>, String);

/// Mean of directions (circular, so 359° and 1° average to 0°).
fn mean_angle(angles: &[Angle]) -> Angle {
    let (s, c) = angles.iter().fold((0.0, 0.0), |(s, c), a| {
        (s + a.radians().sin(), c + a.radians().cos())
    });
    Angle::from_degrees(s.atan2(c).to_degrees()).normalized()
}

/// Reduces `data` (with its office corrections) to coordinates. `known`
/// gives the coordinates of points that are not this file's own reductions:
/// control and points from other work. The file's recorded coordinates are
/// used for stations `known` does not have.
pub fn reduce(data: &FieldData, known: &dyn Fn(&str) -> Option<Known>) -> Reduction {
    let scale = data.scale_factor.unwrap_or(1.0);
    let mut out = Reduction::default();
    // Positions placed so far, with whether they have heights.
    let mut placed: HashMap<String, (DVec3, bool)> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let recorded = |name: &str| {
        data.points
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, p)| (*p, p.z != 0.0))
    };
    let lookup = |name: &str, placed: &HashMap<String, (DVec3, bool)>| {
        known(name)
            .map(|k| (k.position, k.height_known))
            .or_else(|| placed.get(name).copied())
            .or_else(|| recorded(name))
    };
    for (i, setup) in data.effective_setups().iter().enumerate() {
        let n = i + 1;
        let Some((station, station_height)) = lookup(&setup.station, &placed) else {
            out.warnings.push(format!(
                "Setup {n} at {}: the station has no coordinates, so its {} shots were not reduced",
                setup.station,
                setup.observations.len()
            ));
            out.setups.push(SetupReduction {
                station: setup.station.clone(),
                orientation: None,
                method: None,
                shots: 0,
            });
            continue;
        };
        let (orientation, method) = orient(setup, station, &|t| lookup(t, &placed));
        if method == Orientation::Assumed && setup.observations.iter().any(|o| o.hz.is_some()) {
            out.warnings.push(format!(
                "Setup {n} at {}: no backsight with coordinates, so the circle readings were taken as bearings",
                setup.station
            ));
        }
        let backsight = match &method {
            Orientation::Backsight(b) => Some(b.clone()),
            _ => None,
        };
        // Pointings grouped by target, in order of first pointing.
        let mut groups: Vec<Group> = Vec::new();
        for o in &setup.observations {
            let (Some(hz), (Some(hd), dz)) = (o.hz, o.reduce(setup.instrument_height)) else {
                continue;
            };
            if o.target == setup.station {
                continue;
            }
            let b = Bearing::new((orientation.angle() + hz).normalized());
            let p = station.truncate() + b.unit_vector() * hd * scale;
            let z = dz.filter(|_| station_height).map(|dz| station.z + dz);
            match groups.iter_mut().find(|(t, _, _)| *t == o.target) {
                Some((_, v, code)) => {
                    v.push((p, z));
                    if code.is_empty() {
                        code.clone_from(&o.code);
                    }
                }
                None => groups.push((o.target.clone(), vec![(p, z)], o.code.clone())),
            }
        }
        let shots = groups.iter().map(|(_, v, _)| v.len()).sum();
        for (target, v, code) in groups {
            let plan = v.iter().map(|(p, _)| *p).sum::<DVec2>() / v.len() as f64;
            let zs: Vec<f64> = v.iter().filter_map(|(_, z)| *z).collect();
            let z = (!zs.is_empty()).then(|| zs.iter().sum::<f64>() / zs.len() as f64);
            match lookup(&target, &placed) {
                Some((at, has_height)) => out.checks.push(Check {
                    setup: i,
                    delta: plan - at.truncate(),
                    dz: z.filter(|_| has_height).map(|z| z - at.z),
                    backsight: backsight.as_deref() == Some(target.as_str()),
                    target,
                }),
                None => {
                    placed.insert(target.clone(), (plan.extend(z.unwrap_or(0.0)), z.is_some()));
                    order.push(target.clone());
                    out.points.push(ReducedPoint {
                        name: target,
                        position: plan.extend(z.unwrap_or(0.0)),
                        height_known: z.is_some(),
                        code,
                        setup: i,
                        shots: v.len(),
                    });
                }
            }
        }
        out.setups.push(SetupReduction {
            station: setup.station.clone(),
            orientation: Some(orientation),
            method: Some(method),
            shots,
        });
    }
    out
}

/// The bearing of a setup's zero reading: recorded, from its backsight (a
/// pointing to it, else the recorded back circle), else from the first
/// pointing to any point with coordinates, else zero.
fn orient(
    setup: &Setup,
    station: DVec3,
    position: &dyn Fn(&str) -> Option<(DVec3, bool)>,
) -> (Bearing, Orientation) {
    if let Some(b) = setup.orientation {
        return (b, Orientation::Recorded);
    }
    let bearing_to = |name: &str| {
        let p = position(name)?.0;
        Bearing::from_delta(p.x - station.x, p.y - station.y)
    };
    let readings = |name: &str| -> Vec<Angle> {
        setup
            .observations
            .iter()
            .filter(|o| o.target == name)
            .filter_map(|o| o.hz)
            .collect()
    };
    if let Some(bs) = &setup.backsight
        && let Some(b) = bearing_to(bs)
    {
        let r = readings(bs);
        let reading = if r.is_empty() {
            setup.back_circle
        } else {
            Some(mean_angle(&r))
        };
        if let Some(reading) = reading {
            return (
                Bearing::new((b.angle() - reading).normalized()),
                Orientation::Backsight(bs.clone()),
            );
        }
    }
    for o in &setup.observations {
        if o.hz.is_none() || o.target == setup.station {
            continue;
        }
        if let Some(b) = bearing_to(&o.target) {
            let reading = mean_angle(&readings(&o.target));
            return (
                Bearing::new((b.angle() - reading).normalized()),
                Orientation::Backsight(o.target.clone()),
            );
        }
    }
    (Bearing::default(), Orientation::Assumed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::RawObservation;

    fn shot(target: &str, hz: f64, zenith: f64, sd: f64, ht: f64, code: &str) -> RawObservation {
        RawObservation {
            target: target.into(),
            hz: Some(Angle::from_degrees(hz)),
            zenith: Some(Angle::from_degrees(zenith)),
            distance: Some(sd),
            target_height: ht,
            dz: None,
            code: code.into(),
        }
    }

    fn known(name: &str) -> Option<Known> {
        let p = match name {
            "1" => DVec3::new(1000.0, 1000.0, 50.0),
            "2" => DVec3::new(1000.0, 1100.0, 51.0),
            _ => return None,
        };
        Some(Known {
            position: p,
            height_known: true,
            fixed: true,
        })
    }

    #[test]
    fn radiates_and_carries_a_traverse_forward() {
        // At 1, backsight 2 (due north) read at 10°: the zero points at 350°.
        let data = FieldData {
            name: "day1".into(),
            format: "test".into(),
            setups: vec![
                Setup {
                    station: "1".into(),
                    instrument_height: 1.5,
                    backsight: Some("2".into()),
                    observations: vec![
                        shot("2", 10.0, 90.0, 100.0, 1.5, ""),
                        // Due east, 50 m flat, both faces.
                        shot("3", 100.0, 90.0, 50.0, 1.5, "TR"),
                        shot("3", 280.0, 270.0, 50.0, 1.5, ""),
                        // 30 m north-east, 2 m up.
                        shot(
                            "4",
                            55.0,
                            90.0 - (2.0f64 / 30.0).atan().to_degrees(),
                            (900.0f64 + 4.0).sqrt(),
                            1.5,
                            "FH",
                        ),
                    ],
                    ..Default::default()
                },
                // Occupy 3, backsight 1 (due west) read at 0.
                Setup {
                    station: "3".into(),
                    instrument_height: 1.6,
                    backsight: Some("1".into()),
                    observations: vec![
                        shot("1", 0.0, 90.0, 50.0, 1.6, ""),
                        // Due south of 3, 20 m.
                        shot("5", 270.0, 90.0, 20.0, 1.6, "KB"),
                    ],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let r = reduce(&data, &known);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let p = |n: &str| r.points.iter().find(|p| p.name == n).expect(n);
        let three = p("3");
        assert!((three.position - DVec3::new(1050.0, 1000.0, 50.0)).length() < 1e-9);
        assert_eq!((three.shots, three.code.as_str()), (2, "TR"));
        let four = p("4");
        let ne = DVec2::new(1.0, 1.0).normalize() * 30.0;
        assert!((four.position.truncate() - (DVec2::new(1000.0, 1000.0) + ne)).length() < 1e-9);
        assert!((four.position.z - 52.0).abs() < 1e-9);
        let five = p("5");
        assert!((five.position - DVec3::new(1050.0, 980.0, 50.0)).length() < 1e-9);
        // Backsights 2 and 1 are checks; both close exactly.
        assert_eq!(r.checks.len(), 2);
        assert!(r.checks.iter().all(|c| c.backsight && c.delta.length() < 1e-9));
        assert_eq!(r.setups[0].method, Some(Orientation::Backsight("2".into())));
    }

    #[test]
    fn edits_and_missing_stations() {
        let mut data = FieldData {
            setups: vec![
                Setup {
                    station: "1".into(),
                    instrument_height: 1.5,
                    orientation: Some(Bearing::default()),
                    observations: vec![shot("10", 90.0, 90.0, 10.0, 1.5, "TR")],
                    ..Default::default()
                },
                Setup {
                    station: "99".into(),
                    observations: vec![shot("11", 0.0, 90.0, 10.0, 0.0, "")],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let r = reduce(&data, &known);
        assert_eq!(r.points.len(), 1);
        assert_eq!(r.warnings.len(), 1);
        assert!((r.points[0].position.z - 50.0).abs() < 1e-9);

        // The rod was 2.0, not 1.5: the point drops 0.5.
        data.apply_edit(
            crate::network::RawEdit {
                setup: 0,
                observation: Some(0),
                target_height: Some(2.0),
                code: Some("FH".into()),
                ..Default::default()
            },
            false,
        )
        .expect("edit");
        let r = reduce(&data, &known);
        assert!((r.points[0].position.z - 49.5).abs() < 1e-9);
        assert_eq!(r.points[0].code, "FH");
        // Setting it back to the recorded value removes that part of the edit.
        data.apply_edit(
            crate::network::RawEdit {
                setup: 0,
                observation: Some(0),
                target_height: Some(1.5),
                ..Default::default()
            },
            false,
        )
        .expect("edit");
        assert_eq!(data.edits.len(), 1);
        assert_eq!(data.edits[0].target_height, None);
        // Excluding the shot.
        data.apply_edit(
            crate::network::RawEdit {
                setup: 0,
                observation: Some(0),
                excluded: Some(true),
                ..Default::default()
            },
            false,
        )
        .expect("edit");
        assert!(reduce(&data, &known).points.is_empty());
        // Reverting.
        data.apply_edit(
            crate::network::RawEdit {
                setup: 0,
                observation: Some(0),
                ..Default::default()
            },
            true,
        )
        .expect("edit");
        assert!(data.edits.is_empty());
        // Fields in the wrong place.
        assert!(
            data.apply_edit(
                crate::network::RawEdit {
                    setup: 0,
                    observation: Some(0),
                    instrument_height: Some(1.0),
                    ..Default::default()
                },
                false
            )
            .is_err()
        );
        assert!(
            data.apply_edit(
                crate::network::RawEdit {
                    setup: 5,
                    ..Default::default()
                },
                false
            )
            .is_err()
        );
    }
}
