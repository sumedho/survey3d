//! Point clouds: many points stored compactly relative to an origin.

use glam::{DVec3, Vec3};
use serde::{Deserialize, Serialize, Serializer};
use survey_core::Aabb3;

use crate::layer::Color;

/// How cloud points are coloured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudColor {
    /// The file's colours (falls back to height when there are none).
    #[default]
    Rgb,
    Height,
    Single,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudStyle {
    pub visible: bool,
    /// Point size in screen points.
    pub point_size: f32,
    pub color: CloudColor,
    pub single: Color,
}

impl Default for CloudStyle {
    fn default() -> Self {
        Self {
            visible: true,
            point_size: 2.0,
            color: CloudColor::Rgb,
            single: Color::rgb(200, 200, 200),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointCloud {
    pub name: String,
    /// Points are stored as `f32` offsets from here (sub-millimetre within kilometres).
    pub origin: DVec3,
    pub points: Vec<[f32; 3]>,
    pub colors: Option<Vec<[u8; 3]>>,
    pub style: CloudStyle,
}

/// The audit trail records a cloud's name and size, not its points.
impl Serialize for PointCloud {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Summary<'a> {
            name: &'a str,
            points: usize,
            colors: bool,
        }
        Summary {
            name: &self.name,
            points: self.points.len(),
            colors: self.colors.is_some(),
        }
        .serialize(s)
    }
}

impl PointCloud {
    pub fn from_points(name: impl Into<String>, points: &[DVec3], colors: Option<Vec<[u8; 3]>>) -> Self {
        let origin = points.first().map_or(DVec3::ZERO, |p| p.floor());
        Self {
            name: name.into(),
            origin,
            points: points
                .iter()
                .map(|p| (*p - origin).as_vec3().to_array())
                .collect(),
            colors: colors.filter(|c| c.len() == points.len()),
            style: CloudStyle::default(),
        }
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn point(&self, i: usize) -> DVec3 {
        self.origin + Vec3::from_array(self.points[i]).as_dvec3()
    }

    pub fn iter(&self) -> impl Iterator<Item = DVec3> + '_ {
        self.points
            .iter()
            .map(|p| self.origin + Vec3::from_array(*p).as_dvec3())
    }

    pub fn bounds(&self) -> Aabb3 {
        Aabb3::from_points(self.iter())
    }
}
