//! Point clouds: many points stored compactly relative to an origin.
//!
//! A cloud is either *inline* (its points live in the project file, for small
//! XYZ/LAS imports) or *out-of-core* (`source` is set: the points live in a
//! COPC file on disk, and `points` holds only a bounded working set re-read
//! from that file — the project stores the [`CloudRef`], never the points).

use std::path::PathBuf;
use std::sync::Arc;

use glam::{DVec3, Vec3};
use serde::{Deserialize, Serialize, Serializer};
use survey_core::Aabb3;

use crate::layer::Color;

/// A reference to an out-of-core point cloud (a COPC file on disk). The project
/// file stores this instead of the points; the resident working set in
/// [`PointCloud::points`] is re-read from `path` when the project opens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CloudRef {
    /// Path the cloud was loaded from. Resolved on open, with a fallback to the
    /// data directory by file name when it has moved.
    pub path: PathBuf,
    /// Points in the whole file (not the resident working set).
    pub total: u64,
    /// Bounds of the whole file.
    pub full_bounds: Aabb3,
    /// Whether the file carries colour.
    pub has_color: bool,
}

/// Default point size (pt) of an out-of-core cloud. There it is the smallest
/// size of points that are otherwise sized to their spacing; below about 7 the
/// densest areas (points clamped up to it) look visibly different from areas
/// at the data's own spacing, a line that sweeps across the view as you zoom.
pub const OUT_OF_CORE_POINT_SIZE: f32 = 7.0;

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
            // As mine3d's 6 px discs on a 2x display: neighbours drawn about
            // 1.5 px apart (see the COPC streamer) then cover the view.
            point_size: 3.0,
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
    /// Shared, so the renderer's background scene build takes a reference
    /// rather than a copy of millions of points.
    pub points: Arc<Vec<[f32; 3]>>,
    pub colors: Option<Arc<Vec<[u8; 3]>>>,
    pub style: CloudStyle,
    /// Set when the cloud is backed by a COPC file on disk; then `points` is a
    /// bounded, re-readable working set rather than the whole cloud.
    pub source: Option<CloudRef>,
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

/// Points in a cloud's storage form (`f32` offsets from an origin), so a
/// streamed working set can be prepared off the UI thread and swapped in.
pub struct Resident {
    pub origin: DVec3,
    pub points: Arc<Vec<[f32; 3]>>,
    pub colors: Option<Arc<Vec<[u8; 3]>>>,
}

impl Resident {
    /// `points` are absolute world coordinates.
    pub fn new(points: &[DVec3], colors: Option<Vec<[u8; 3]>>) -> Self {
        let origin = points.first().map_or(DVec3::ZERO, |p| p.floor());
        Self {
            origin,
            points: Arc::new(
                points
                    .iter()
                    .map(|p| (*p - origin).as_vec3().to_array())
                    .collect(),
            ),
            colors: colors.filter(|c| c.len() == points.len()).map(Arc::new),
        }
    }
}

impl PointCloud {
    pub fn from_points(name: impl Into<String>, points: &[DVec3], colors: Option<Vec<[u8; 3]>>) -> Self {
        let r = Resident::new(points, colors);
        Self {
            name: name.into(),
            origin: r.origin,
            points: r.points,
            colors: r.colors,
            style: CloudStyle::default(),
            source: None,
        }
    }

    /// An out-of-core cloud: a [`CloudRef`] plus a resident working set (the
    /// points read so far). `resident` are absolute world coordinates.
    pub fn from_source(
        name: impl Into<String>,
        source: CloudRef,
        resident: &[DVec3],
        colors: Option<Vec<[u8; 3]>>,
    ) -> Self {
        let mut cloud = Self::from_points(name, resident, colors);
        cloud.source = Some(source);
        cloud.style.point_size = OUT_OF_CORE_POINT_SIZE;
        cloud
    }

    /// Replaces the resident working set in place (streaming or reloading from
    /// disk). Keeps the cloud's identity, style and source.
    pub fn set_resident(&mut self, resident: Resident) {
        self.origin = resident.origin;
        self.points = resident.points;
        self.colors = resident.colors.filter(|c| c.len() == self.points.len());
    }

    /// Whether the cloud's points live in a file on disk.
    pub fn is_out_of_core(&self) -> bool {
        self.source.is_some()
    }

    /// Points in the whole cloud: the file's total for an out-of-core cloud,
    /// else the resident count.
    pub fn full_count(&self) -> u64 {
        self.source.as_ref().map_or(self.points.len() as u64, |s| s.total)
    }

    /// Bounds of the whole cloud: the file's bounds for an out-of-core cloud,
    /// else the resident bounds.
    pub fn full_bounds(&self) -> Aabb3 {
        match &self.source {
            Some(s) => s.full_bounds,
            None => self.bounds(),
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
            points: Arc::new(keep.iter().map(|i| self.points[*i as usize]).collect()),
            colors: self
                .colors
                .as_ref()
                .map(|c| Arc::new(keep.iter().map(|i| c[*i as usize]).collect())),
            style: self.style,
            // Thinning materializes points, so the result is a plain inline cloud.
            source: None,
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
        assert_eq!(t.colors.as_ref().map(|c| c.len()), Some(8));
        let (t, spacing) = c.thinned_to(100);
        assert!(t.len() <= 100 && spacing.is_some());
        assert_eq!(c.thinned_to(5000).1, None);
    }

    #[test]
    fn out_of_core_cloud_reports_whole_file_and_reloads_resident() {
        let source = CloudRef {
            path: "/data/scan.copc.laz".into(),
            total: 200_000_000,
            full_bounds: Aabb3::new(
                DVec3::new(500_000.0, 7_000_000.0, 0.0),
                DVec3::new(501_000.0, 7_001_000.0, 80.0),
            ),
            has_color: true,
        };
        // A small resident working set standing in for a coarse preview.
        let resident = [
            DVec3::new(500_100.0, 7_000_200.0, 12.0),
            DVec3::new(500_300.5, 7_000_450.25, 15.5),
        ];
        let mut c = PointCloud::from_source("big", source.clone(), &resident, None);
        assert!(c.is_out_of_core());
        assert_eq!(c.len(), 2); // resident points
        assert_eq!(c.full_count(), 200_000_000); // whole file
        assert_eq!(c.full_bounds(), source.full_bounds);
        assert!((c.point(1) - resident[1]).length() < 1e-3);

        // Streaming a new working set keeps identity and source.
        let next = [DVec3::new(500_500.0, 7_000_500.0, 20.0)];
        c.set_resident(Resident::new(&next, None));
        assert_eq!(c.len(), 1);
        assert_eq!(c.source.as_ref(), Some(&source));
        assert!((c.point(0) - next[0]).length() < 1e-3);
    }
}
