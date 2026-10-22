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

    /// Keeps one point per `spacing` cube (the one nearest the cube's centre),
    /// in the original order, with the same style.
    pub fn thinned(&self, spacing: f64) -> PointCloud {
        let s = spacing.max(1e-6) as f32;
        let mut best: std::collections::HashMap<(i32, i32, i32), (u32, f32)> =
            std::collections::HashMap::new();
        for (i, p) in self.points.iter().enumerate() {
            let cell = p.map(|v| (v / s).floor());
            let d = (0..3).map(|k| (p[k] - (cell[k] + 0.5) * s).powi(2)).sum::<f32>();
            let key = (cell[0] as i32, cell[1] as i32, cell[2] as i32);
            let e = best.entry(key).or_insert((i as u32, f32::MAX));
            if d < e.1 {
                *e = (i as u32, d);
            }
        }
        let mut keep: Vec<u32> = best.into_values().map(|(i, _)| i).collect();
        keep.sort_unstable();
        PointCloud {
            name: self.name.clone(),
            origin: self.origin,
            points: keep.iter().map(|i| self.points[*i as usize]).collect(),
            colors: self
                .colors
                .as_ref()
                .map(|c| keep.iter().map(|i| c[*i as usize]).collect()),
            style: self.style,
        }
    }

    /// Thins with growing spacing until at most `max_points` remain; returns
    /// the cloud and the spacing used (`None` when no thinning was needed).
    pub fn thinned_to(&self, max_points: usize) -> (PointCloud, Option<f64>) {
        if self.len() <= max_points {
            return (self.clone(), None);
        }
        let b = self.bounds();
        let size = b.max - b.min;
        let area = (size.x * size.y).max(1e-6);
        let mut spacing = (area / max_points as f64).sqrt();
        loop {
            let t = self.thinned(spacing);
            if t.len() <= max_points {
                return (t, Some(spacing));
            }
            spacing *= 1.25;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinning_keeps_one_point_per_cube() {
        let pts: Vec<DVec3> = (0..1000)
            .map(|i| {
                DVec3::new(
                    500_000.0 + (i % 10) as f64 * 0.1,
                    7_000_000.0 + (i / 10 % 10) as f64 * 0.1,
                    (i / 100) as f64 * 0.1,
                )
            })
            .collect();
        let colors = Some((0..1000).map(|i| [(i % 256) as u8, 0, 0]).collect());
        let c = PointCloud::from_points("c", &pts, colors);
        let t = c.thinned(0.5);
        // 1 m cube of 10x10x10 points at 0.1 m: 2x2x2 cells of 0.5 m.
        assert_eq!(t.len(), 8);
        assert_eq!(t.colors.as_ref().map(Vec::len), Some(8));
        let (t, spacing) = c.thinned_to(100);
        assert!(t.len() <= 100 && spacing.is_some());
        assert_eq!(c.thinned_to(5000).1, None);
    }
}
