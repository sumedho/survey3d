//! Figure (lot) closure: misclosure, precision and area from courses, and
//! the missing values solved so the figure closes.

use glam::DVec3;
use survey_core::Bearing;
use survey_core::closure::{Course, CourseArc, Figure};
use survey_model::{ChangeSet, Document, Entity, Polyline};

/// A course as given: a blank bearing or distance is a missing value. An arc
/// has a radius, and its chord (`distance`) or arc length.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CourseInput {
    pub bearing: Option<Bearing>,
    /// Line length, or chord length for an arc.
    pub distance: Option<f64>,
    pub radius: Option<f64>,
    pub arc_length: Option<f64>,
    /// An arc turning right (clockwise).
    pub right: bool,
}

/// The courses, with arc chords worked out from arc lengths.
pub fn courses(inputs: &[CourseInput]) -> Result<Vec<Course>, String> {
    inputs
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let n = i + 1;
            let mut distance = c.distance;
            let arc = match c.radius {
                Some(radius) => {
                    if distance.is_none()
                        && let Some(l) = c.arc_length
                    {
                        distance = Some(
                            Course::chord_from_arc_length(radius, l)
                                .ok_or(format!("Course {n}: the arc length does not fit the radius"))?,
                        );
                    }
                    Some(CourseArc {
                        radius,
                        right: c.right,
                        large: c.arc_length.is_some_and(|l| l > std::f64::consts::PI * radius),
                    })
                }
                None => None,
            };
            Ok(Course {
                bearing: c.bearing,
                distance,
                arc,
            })
        })
        .collect()
}

/// A point name or typed coordinates.
pub fn start_point(doc: &Document, text: &str) -> Option<DVec3> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Some((_, p)) = doc.find_point_by_name(text) {
        return Some(p.position);
    }
    let c = doc.conventions().parse_coord(text).ok()?;
    Some(DVec3::new(c.easting, c.northing, c.height.unwrap_or(0.0)))
}

/// The figure as a polyline from `start`; closed (when it closes exactly
/// and `closed`) or open, ending where the courses end.
pub fn draw(doc: &Document, start: DVec3, fig: &Figure, name: &str, closed: bool) -> (ChangeSet, String) {
    let mut vertices: Vec<DVec3> = fig.vertices.iter().map(|v| start + v.extend(0.0)).collect();
    let closes = fig.precision().is_none();
    if closes {
        vertices.pop();
    }
    let name = if name.trim().is_empty() {
        "Closure"
    } else {
        name.trim()
    };
    let cs = ChangeSet::new(format!("Draw figure {name}")).insert_entity(Entity::Polyline(Polyline {
        name: name.to_string(),
        vertices,
        bulges: fig.bulges.clone(),
        closed: closes && closed,
        code: String::new(),
        layer: doc.current_layer(),
        heights_known: true,
    }));
    (cs, format!("Closure: drew {name}"))
}
