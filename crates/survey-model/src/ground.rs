//! Heights and distances that depend on the project's coordinate system:
//! the geoid separation at a point (from the loaded geoid model, else the
//! project's constant), combined scale factors, and grid or ground distances.

use glam::DVec3;

use crate::Document;
use crate::coord_system::{DistanceKind, GroundFactor};

/// The geoid separation at a point and where it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Separation {
    /// N (metres): ellipsoidal height = orthometric height + N.
    pub n: f64,
    /// The geoid model it was interpolated from; `None` for the project's constant.
    pub model: Option<String>,
}

impl Document {
    /// The geoid separation at a grid position: from the geoid model where
    /// it is loaded and covers the point, else the project's constant.
    pub fn separation_at(&self, easting: f64, northing: f64) -> Separation {
        let crs = &self.meta().coord_system;
        if let (Some(model), Some(tm)) = (self.geoid(), crs.projection()) {
            let g = tm.inverse(easting, northing);
            if let Some(n) = model.separation(g.latitude, g.longitude) {
                return Separation {
                    n,
                    model: Some(model.name.clone()),
                };
            }
        }
        Separation {
            n: crs.geoid_separation(),
            model: None,
        }
    }

    /// Height above the ellipsoid of a point whose height is above the geoid.
    pub fn ellipsoidal_height(&self, p: DVec3) -> f64 {
        p.z + self.separation_at(p.x, p.y).n
    }

    /// The combined (grid ÷ ground) scale factor at a point from the
    /// projection: point scale × height factor; `None` without a projection.
    pub fn combined_factor_at(&self, p: DVec3) -> Option<f64> {
        let tm = self.meta().coord_system.projection()?;
        Some(tm.combined_factor(p.x, p.y, self.ellipsoidal_height(p)))
    }

    /// The combined factor for a line from `a` to `b` under the project's
    /// ground setting: the fixed factor, or the projection's (by Simpson's
    /// rule along the line). 1 without a projection.
    pub fn line_factor(&self, a: DVec3, b: DVec3) -> f64 {
        match self.meta().ground.factor {
            GroundFactor::Fixed(k) if k.is_finite() && k > 0.0 => k,
            GroundFactor::Fixed(_) => 1.0,
            GroundFactor::Projection => {
                let at = |p: DVec3| self.combined_factor_at(p);
                match (at(a), at((a + b) * 0.5), at(b)) {
                    (Some(ka), Some(km), Some(kb)) => (ka + 4.0 * km + kb) / 6.0,
                    _ => 1.0,
                }
            }
        }
    }

    /// Which distances the project shows.
    pub fn distance_kind(&self) -> DistanceKind {
        self.meta().ground.distances
    }

    /// Whether distances are shown (and typed) at ground.
    pub fn ground_distances(&self) -> bool {
        self.distance_kind() == DistanceKind::Ground
    }

    /// A grid length measured along a line from `a` to `b` (a chord, arc or
    /// polyline), as the project shows it: unchanged on the grid, or ÷ the
    /// line's combined factor at ground.
    pub fn shown_length(&self, grid: f64, a: DVec3, b: DVec3) -> f64 {
        if self.ground_distances() {
            grid / self.line_factor(a, b)
        } else {
            grid
        }
    }

    /// The horizontal distance from `a` to `b` as the project shows it.
    pub fn shown_distance(&self, a: DVec3, b: DVec3) -> f64 {
        self.shown_length(a.truncate().distance(b.truncate()), a, b)
    }

    /// A grid area around `at` as the project shows it: ÷ the combined
    /// factor squared at ground.
    pub fn shown_area(&self, grid: f64, at: DVec3) -> f64 {
        if self.ground_distances() {
            grid / self.line_factor(at, at).powi(2)
        } else {
            grid
        }
    }

    /// The scale from a typed (shown) length to the grid for a line starting
    /// at `from` towards `to` (the far end, when known; else `from` alone).
    pub fn typed_to_grid(&self, from: DVec3, to: Option<DVec3>) -> f64 {
        if self.ground_distances() {
            self.line_factor(from, to.unwrap_or(from))
        } else {
            1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use survey_core::geoid::{GeoidGrid, GeoidModel};
    use survey_core::projection::{Ellipsoid, TransverseMercator};

    use super::*;
    use crate::CoordinateSystem;
    use crate::coord_system::GroundSettings;

    fn mga55(doc: &mut Document, ground: GroundSettings) {
        let mut meta = doc.meta().clone();
        meta.coord_system = CoordinateSystem::Projected {
            name: "MGA2020 zone 55".into(),
            epsg: Some(7855),
            zone: Some(55),
            projection: Some(TransverseMercator::utm(55, true, Ellipsoid::GRS80)),
            geoid_separation: 4.0,
            geoid: Some("flat.gtx".into()),
        };
        meta.ground = ground;
        doc.set_meta(meta);
    }

    #[test]
    fn separations_from_the_model_or_the_constant() {
        let mut doc = Document::new();
        mga55(&mut doc, GroundSettings::default());
        let p = DVec3::new(273_741.297, 5_796_489.777, 100.0);
        assert_eq!(doc.separation_at(p.x, p.y), Separation { n: 4.0, model: None });
        // A model round Flinders Peak with N = 5.5.
        let grid = GeoidGrid::new(-39.0, 143.0, 1.0, 1.0, 3, 3, vec![5.5; 9]).expect("value");
        doc.set_geoid(Some(Arc::new(
            GeoidModel::new("flat.gtx", vec![grid]).expect("value"),
        )));
        let s = doc.separation_at(p.x, p.y);
        assert!((s.n - 5.5).abs() < 1e-6 && s.model.as_deref() == Some("flat.gtx"));
        assert!((doc.ellipsoidal_height(p) - 105.5).abs() < 1e-6);
        // Outside the model: the constant.
        assert_eq!(doc.separation_at(500_000.0, 6_500_000.0).n, 4.0);
    }

    #[test]
    fn grid_and_ground_distances() {
        let mut doc = Document::new();
        let (a, b) = (
            DVec3::new(273_741.297, 5_796_489.777, 500.0),
            DVec3::new(274_741.297, 5_796_489.777, 500.0),
        );
        // Grid by default.
        assert_eq!(doc.shown_distance(a, b), 1000.0);
        mga55(
            &mut doc,
            GroundSettings {
                distances: DistanceKind::Ground,
                factor: GroundFactor::Projection,
            },
        );
        let k = doc.combined_factor_at((a + b) * 0.5).expect("value");
        // Well west of the central meridian (k > 1) but 504 m up (×0.99992).
        assert!((doc.line_factor(a, b) - k).abs() < 1e-8);
        assert!((doc.shown_distance(a, b) - 1000.0 / k).abs() < 1e-5);
        assert!((doc.typed_to_grid(a, Some(b)) - k).abs() < 1e-8);
        let mut meta = doc.meta().clone();
        meta.ground.factor = GroundFactor::Fixed(0.9996);
        doc.set_meta(meta);
        assert!((doc.shown_distance(a, b) - 1000.0 / 0.9996).abs() < 1e-9);
    }
}
