//! Set-out (stake-out) computations: from an instrument station and a
//! backsight, the horizontal angle, bearing and distance to each point, with
//! the height difference and, given instrument and target heights, the slope
//! distance and zenith angle to dial up.

use std::fmt::Write as _;

use survey_core::cogo::inverse;
use survey_core::{Angle, Bearing, Conventions};
use survey_model::{Document, Entity, EntityId, SurveyPoint};

/// One point to set out.
#[derive(Clone, Debug)]
pub struct Row {
    pub name: String,
    /// Clockwise from the backsight.
    pub angle: Option<Angle>,
    pub bearing: Option<Bearing>,
    /// Horizontal distance as the project shows it (grid or ground).
    pub distance: f64,
    pub grid_distance: f64,
    /// Ground height difference (point − station), when both have heights.
    pub dz: Option<f64>,
    /// From the instrument (station + hi) to the target (point + ht).
    pub slope: Option<f64>,
    pub zenith: Option<Angle>,
    pub description: String,
}

/// The set-out from one station.
#[derive(Clone, Debug)]
pub struct Setout {
    pub station: String,
    /// The backsight as typed and its bearing.
    pub backsight: Option<(String, Bearing)>,
    /// Horizontal distance to a backsight point, for checking the setup.
    pub backsight_distance: Option<f64>,
    pub hi: f64,
    pub ht: f64,
    /// Ground distances (else grid).
    pub ground: bool,
    pub rows: Vec<Row>,
}

/// Computes the set-out from `station` (a point name) with `backsight` (a
/// point name or bearing; blank for bearings only) to the points among
/// `targets`, with instrument height `hi` and target height `ht`.
pub fn setout(
    doc: &Document,
    station: &str,
    backsight: &str,
    targets: &[EntityId],
    hi: f64,
    ht: f64,
) -> Result<Setout, String> {
    let conv = doc.conventions();
    let station = doc
        .find_point_by_name(station.trim())
        .map(|(_, p)| p)
        .ok_or_else(|| "Enter the instrument station (a point name).".to_string())?;
    let bs = backsight.trim();
    let (backsight, backsight_distance) = if bs.is_empty() {
        (None, None)
    } else if let Some((_, p)) = doc.find_point_by_name(bs) {
        let d = p.position - station.position;
        let b = Bearing::from_delta(d.x, d.y).ok_or("The backsight is at the station")?;
        (
            Some((bs.to_string(), b)),
            Some(doc.shown_distance(station.position, p.position)),
        )
    } else {
        let b = conv
            .parse_bearing(bs)
            .map_err(|e| format!("Backsight: not a point or bearing ({e})"))?;
        (Some((bs.to_string(), b)), None)
    };
    let points: Vec<&SurveyPoint> = targets
        .iter()
        .filter_map(|id| match doc.entity(*id) {
            Some(Entity::Point(p)) if p.name != station.name => Some(p),
            _ => None,
        })
        .collect();
    if points.is_empty() {
        return Err("Select the points to set out.".into());
    }
    let rows = points
        .into_iter()
        .map(|p| {
            let inv = inverse(station.position, p.position);
            let heights = station.height_known && p.height_known;
            let dv = inv.delta.z + ht - hi;
            let shown = doc.shown_distance(station.position, p.position);
            Row {
                name: p.name.clone(),
                angle: match (inv.bearing, &backsight) {
                    (Some(b), Some((_, bs))) => Some((b.angle() - bs.angle()).normalized()),
                    _ => None,
                },
                bearing: inv.bearing,
                distance: shown,
                grid_distance: inv.horizontal,
                dz: heights.then_some(inv.delta.z),
                slope: heights.then(|| shown.hypot(dv)),
                zenith: heights.then(|| Angle::from_radians(shown.atan2(dv))),
                description: p.description.clone(),
            }
        })
        .collect();
    Ok(Setout {
        station: station.name.clone(),
        backsight,
        backsight_distance,
        hi,
        ht,
        ground: doc.ground_distances(),
        rows,
    })
}

impl Setout {
    pub fn header(&self, conv: &Conventions) -> String {
        let mut s = match &self.backsight {
            Some((text, b)) => format!(
                "Station {}  backsight {text} ({})",
                self.station,
                conv.format_bearing(*b)
            ),
            None => format!("Station {}  (no backsight: bearings only)", self.station),
        };
        if let Some(d) = self.backsight_distance {
            s.push_str(&format!("  check distance {}", conv.format_length(d)));
        }
        if self.hi != 0.0 || self.ht != 0.0 {
            s.push_str(&format!(
                "  HI {}  HT {}",
                conv.format_length(self.hi),
                conv.format_length(self.ht)
            ));
        }
        s.push_str(if self.ground {
            "  (ground distances)"
        } else {
            "  (grid distances)"
        });
        s
    }

    pub fn report(&self, conv: &Conventions) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "STAKE-OUT");
        let _ = writeln!(s, "{}", self.header(conv));
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "{:<10} {:>14} {:>16} {:>12} {:>9} {:>12} {:>14}  Description",
            "Point", "Angle", "Bearing", "Distance", "dZ", "Slope", "Zenith"
        );
        for r in &self.rows {
            let _ = writeln!(
                s,
                "{:<10} {:>14} {:>16} {:>12} {:>9} {:>12} {:>14}  {}",
                r.name,
                r.angle.map(|a| conv.format_angle(a)).unwrap_or_default(),
                r.bearing.map(|b| conv.format_bearing(b)).unwrap_or_default(),
                conv.format_length(r.distance),
                r.dz.map(|d| conv.format_length(d)).unwrap_or_default(),
                r.slope.map(|d| conv.format_length(d)).unwrap_or_default(),
                r.zenith.map(|z| conv.format_angle(z)).unwrap_or_default(),
                r.description
            );
        }
        s
    }
}
