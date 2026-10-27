//! Resection (free station): fix an instrument station from directions
//! and/or distances observed to known points.

use std::fmt::Write as _;

use survey_core::resection::{Resection, Sighting, Weights, resect};
use survey_core::{Angle, Conventions};
use survey_model::{ChangeSet, Document, Entity, SurveyPoint};

/// An observation to a known point: its horizontal circle reading and/or
/// horizontal distance.
#[derive(Clone, Debug, PartialEq)]
pub struct Observed {
    pub point: String,
    pub direction: Option<Angle>,
    pub distance: Option<f64>,
}

/// The sightings, with their targets' names, from the known points.
pub fn sightings(doc: &Document, observed: &[Observed]) -> Result<Vec<(String, Sighting)>, String> {
    let mut out = Vec::new();
    for (i, o) in observed.iter().enumerate() {
        let n = i + 1;
        let name = o.point.trim();
        let (_, p) = doc
            .find_point_by_name(name)
            .ok_or_else(|| format!("Row {n}: no point named '{name}'"))?;
        if o.direction.is_none() && o.distance.is_none() {
            return Err(format!("Row {n}: enter a direction, a distance or both"));
        }
        out.push((
            name.to_string(),
            Sighting {
                target: p.position.truncate(),
                direction: o.direction,
                distance: o.distance,
            },
        ));
    }
    if out.is_empty() {
        return Err("Enter the known points sighted, with their directions and/or distances.".into());
    }
    Ok(out)
}

/// The solutions (two when the geometry allows both) for the sightings.
pub fn solve(sightings: &[(String, Sighting)], weights: Weights) -> Result<Vec<Resection>, String> {
    let only: Vec<Sighting> = sightings.iter().map(|(_, s)| *s).collect();
    resect(&only, weights).map_err(|e| e.to_string())
}

/// The station as a new point (plan only unless `height` is given).
pub fn station_command(
    doc: &Document,
    name: &str,
    r: &Resection,
    height: Option<f64>,
) -> Result<(ChangeSet, String), String> {
    if doc.find_point_by_name(name).is_some() {
        return Err(format!("A point named {name} already exists"));
    }
    let position = r.position.extend(height.unwrap_or(0.0));
    let point = SurveyPoint {
        name: name.to_string(),
        position,
        height_known: height.is_some(),
        code: "STN".into(),
        description: "Resection".into(),
        layer: doc.current_layer(),
    };
    let cs = ChangeSet::new(format!("Resection station {name}")).insert_entity(Entity::Point(point));
    let summary = format!(
        "Resection: station {name} at {}",
        doc.conventions().format_coord(position, height.is_some())
    );
    Ok((cs, summary))
}

pub fn report(conv: &Conventions, station: &str, sightings: &[(String, Sighting)], r: &Resection) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "RESECTION  station {station}");
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "{:<12} {:>16} {:>12} {:>10} {:>10}",
        "Known point", "Direction", "Distance", "v dir", "v dist"
    );
    for ((name, sg), (vd, vs)) in sightings.iter().zip(&r.residuals) {
        let _ = writeln!(
            s,
            "{:<12} {:>16} {:>12} {:>10} {:>10}",
            name,
            sg.direction.map(|a| conv.format_angle(a)).unwrap_or_default(),
            sg.distance.map(|d| conv.format_length(d)).unwrap_or_default(),
            vd.filter(|_| r.redundancy > 0)
                .map(|a| format!("{:+.1}\"", a.degrees() * 3600.0))
                .unwrap_or_default(),
            vs.filter(|_| r.redundancy > 0)
                .map(|d| format!("{:+.1}mm", d * 1000.0))
                .unwrap_or_default(),
        );
    }
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "Station         {}",
        conv.format_coord(r.position.extend(0.0), false)
    );
    if let Some(o) = r.orientation {
        let _ = writeln!(s, "Orientation     {}", conv.format_angle(o));
    }
    let _ = writeln!(s, "Redundancy      {}", r.redundancy);
    if let (Some(sd), Some(s0)) = (r.sigma, r.sigma0) {
        let (ea, nb) = conv.horizontal_labels();
        let _ = writeln!(
            s,
            "Std. deviation  {ea} {}  {nb} {}",
            conv.format_length_with_unit(sd.x),
            conv.format_length_with_unit(sd.y)
        );
        let _ = writeln!(s, "Sigma0          {s0:.2}");
    }
    s
}
