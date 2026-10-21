use glam::DVec3;
use serde::{Deserialize, Serialize};
use survey_core::Aabb3;
use survey_core::arc::{Arc, bulge_segments};

use crate::ids::LayerId;

/// A named survey point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurveyPoint {
    pub name: String,
    /// `(easting, northing, height)`; `z` is 0 when `height_known` is false.
    pub position: DVec3,
    pub height_known: bool,
    pub code: String,
    pub description: String,
    pub layer: LayerId,
}

/// An open or closed 3D polyline (a "string" in survey terms) whose segments
/// may be circular arcs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Polyline {
    pub name: String,
    pub vertices: Vec<DVec3>,
    /// Bulge of the segment starting at each vertex (see [`survey_core::arc`]);
    /// missing entries are straight. Empty for an all-straight polyline.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bulges: Vec<f64>,
    pub closed: bool,
    pub code: String,
    pub layer: LayerId,
}

impl Polyline {
    pub fn bulge(&self, segment: usize) -> f64 {
        self.bulges.get(segment).copied().unwrap_or(0.0)
    }

    /// Segments as `(start, end, bulge)`.
    pub fn segments(&self) -> impl Iterator<Item = (DVec3, DVec3, f64)> + '_ {
        bulge_segments(&self.vertices, &self.bulges, self.closed)
    }

    /// Straight-line approximation (arcs tessellated to `tolerance`), for display and hit testing.
    pub fn tessellate(&self, tolerance: f64) -> Vec<Vec<DVec3>> {
        self.segments()
            .map(|(a, b, bulge)| match Arc::from_bulge(a, b, bulge) {
                Some(arc) => arc.tessellate(tolerance),
                None => vec![a, b],
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Entity {
    Point(SurveyPoint),
    Polyline(Polyline),
}

impl Entity {
    pub fn layer(&self) -> LayerId {
        match self {
            Entity::Point(p) => p.layer,
            Entity::Polyline(l) => l.layer,
        }
    }

    pub fn set_layer(&mut self, layer: LayerId) {
        match self {
            Entity::Point(p) => p.layer = layer,
            Entity::Polyline(l) => l.layer = layer,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Entity::Point(p) => &p.name,
            Entity::Polyline(l) => &l.name,
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Entity::Point(_) => "Point",
            Entity::Polyline(_) => "Polyline",
        }
    }

    pub fn bounds(&self) -> Aabb3 {
        match self {
            Entity::Point(p) => Aabb3::from_point(p.position),
            Entity::Polyline(l) => {
                let mut b = Aabb3::from_points(l.vertices.iter().copied());
                for (a, c, bulge) in l.segments() {
                    if let Some(arc) = Arc::from_bulge(a, c, bulge) {
                        // Extents of the arc: sample its quadrant-crossing tessellation.
                        let tol = (arc.radius * 1e-4).max(1e-6);
                        for p in arc.tessellate(tol) {
                            b.extend(p);
                        }
                    }
                }
                b
            }
        }
    }
}
