use glam::DVec3;
use serde::{Deserialize, Serialize};
use survey_core::Aabb3;

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

/// An open or closed 3D polyline (a "string" in survey terms).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Polyline {
    pub name: String,
    pub vertices: Vec<DVec3>,
    pub closed: bool,
    pub code: String,
    pub layer: LayerId,
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
            Entity::Polyline(l) => Aabb3::from_points(l.vertices.iter().copied()),
        }
    }
}
