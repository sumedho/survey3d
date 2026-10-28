//! Georeferenced images (orthophotos) under the drawing. The project keeps a
//! reference to the image file and its placement, never the pixels; the
//! viewer draws it from a tile cache built from the file.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use survey_core::georef::{ControlPoint, FitKind, GeorefFit, ImageTransform, fit};

/// Where an image's placement came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// GeoTIFF tags or a world file.
    #[default]
    File,
    /// Fitted to control points.
    ControlPoints,
    /// Not placed yet: put roughly in view, waiting for control points.
    Rough,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageRecord {
    pub name: String,
    /// The image file (GeoTIFF, JPEG or PNG).
    pub path: PathBuf,
    /// Its size in pixels.
    pub width: u32,
    pub height: u32,
    /// Pixels to plan coordinates.
    pub transform: ImageTransform,
    pub placement: Placement,
    /// The GeoTIFF's EPSG code, when it gave one.
    pub epsg: Option<u32>,
    pub control_points: Vec<ControlPoint>,
    pub fit: FitKind,
    pub visible: bool,
    /// 0 (clear) to 1 (solid).
    pub opacity: f32,
    /// Height the image is drawn at in 3D views.
    pub elevation: f64,
    /// The placement before any control points (from the file, or rough):
    /// control points always fit from here, so removing them puts it back.
    pub original: Option<(ImageTransform, Placement)>,
}

impl Default for ImageRecord {
    fn default() -> Self {
        Self {
            name: String::new(),
            path: PathBuf::new(),
            width: 0,
            height: 0,
            transform: ImageTransform::north_up(glam::DVec2::ZERO, 1.0),
            placement: Placement::File,
            epsg: None,
            control_points: Vec::new(),
            fit: FitKind::Helmert,
            visible: true,
            opacity: 1.0,
            elevation: 0.0,
            original: None,
        }
    }
}

impl ImageRecord {
    /// Refits the transform to the enabled control points: a full fit with
    /// enough of them; else the original placement, shifted onto the one
    /// point when there is one.
    pub fn refit(&mut self) -> Result<Option<GeorefFit>, String> {
        let (base, base_placement) = *self.original.get_or_insert((self.transform, self.placement));
        let enabled: Vec<&ControlPoint> = self.control_points.iter().filter(|p| p.enabled).collect();
        if enabled.len() >= self.fit.min_points() {
            let f = fit(&self.control_points, self.fit)?;
            self.transform = f.transform;
            self.placement = Placement::ControlPoints;
            return Ok(Some(f));
        }
        self.transform = base;
        self.placement = base_placement;
        if let [p] = enabled[..] {
            // One point: move the image so it lands there, keeping its scale and rotation.
            let at = base.apply(glam::DVec2::from(p.pixel));
            let shift = glam::DVec2::from(p.world) - at;
            self.transform.c += shift.x;
            self.transform.f += shift.y;
        }
        Ok(None)
    }

    /// The plan corners: top-left, top-right, bottom-right, bottom-left.
    pub fn corners(&self) -> [glam::DVec2; 4] {
        self.transform
            .corners(f64::from(self.width), f64::from(self.height))
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec2;

    use super::*;

    #[test]
    fn removing_control_points_restores_the_original_placement() {
        let start = ImageTransform::north_up(DVec2::new(100.0, 200.0), 0.5);
        let mut r = ImageRecord {
            width: 100,
            height: 80,
            transform: start,
            placement: Placement::Rough,
            ..Default::default()
        };
        let point = |px: [f64; 2], w: [f64; 2]| ControlPoint {
            pixel: px,
            world: w,
            enabled: true,
        };
        r.control_points.push(point([0.0, 0.0], [110.0, 210.0]));
        r.refit().expect("one");
        assert!((r.transform.apply(DVec2::ZERO) - DVec2::new(110.0, 210.0)).length() < 1e-9);
        r.control_points.push(point([100.0, 80.0], [160.0, 160.0]));
        assert!(r.refit().expect("two").is_some());
        assert_eq!(r.placement, Placement::ControlPoints);
        // Back to one: the original shifted onto it, not the fit shifted.
        r.control_points.pop();
        r.refit().expect("one again");
        assert!((r.transform.pixel_size() - 0.5).abs() < 1e-12);
        assert!((r.transform.apply(DVec2::ZERO) - DVec2::new(110.0, 210.0)).length() < 1e-9);
        // None: exactly where it started.
        r.control_points.clear();
        r.refit().expect("none");
        assert_eq!((r.transform, r.placement), (start, Placement::Rough));
    }
}
