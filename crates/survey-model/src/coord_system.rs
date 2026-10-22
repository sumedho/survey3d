use serde::{Deserialize, Serialize};
use survey_core::projection::TransverseMercator;

/// The coordinate reference of a project or dataset.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum CoordinateSystem {
    #[default]
    Unspecified,
    /// A map projection such as MGA2020 zone 56 or a UTM zone.
    Projected {
        name: String,
        epsg: Option<u32>,
        zone: Option<u32>,
        /// The projection itself, for geographic conversion and scale factors.
        #[serde(default)]
        projection: Option<TransverseMercator>,
        /// Geoid–ellipsoid separation N (metres): ellipsoidal height = height + N.
        #[serde(default)]
        geoid_separation: f64,
    },
    /// An arbitrary site/local grid, optionally related to a projected system.
    LocalGrid { name: String, description: String },
}

impl CoordinateSystem {
    pub fn display_name(&self) -> String {
        match self {
            CoordinateSystem::Unspecified => "Unspecified".into(),
            CoordinateSystem::Projected {
                name,
                epsg,
                projection,
                ..
            } => {
                let name = if name.trim().is_empty() {
                    match projection {
                        Some(tm) if tm.scale == 0.9996 && tm.false_easting == 500_000.0 => format!(
                            "UTM zone {}{} ({})",
                            TransverseMercator::utm_zone(tm.central_meridian),
                            if tm.false_northing > 0.0 { "S" } else { "N" },
                            tm.ellipsoid.name
                        ),
                        Some(tm) => format!(
                            "Transverse Mercator, CM {}° ({})",
                            tm.central_meridian, tm.ellipsoid.name
                        ),
                        None => "Projected grid".into(),
                    }
                } else {
                    name.clone()
                };
                match epsg {
                    Some(code) => format!("{name} (EPSG:{code})"),
                    None => name,
                }
            }
            CoordinateSystem::LocalGrid { name, .. } => format!("Local grid: {name}"),
        }
    }

    /// The map projection, when one is defined.
    pub fn projection(&self) -> Option<TransverseMercator> {
        match self {
            CoordinateSystem::Projected { projection, .. } => *projection,
            _ => None,
        }
    }

    /// Geoid separation for projected systems (0 otherwise).
    pub fn geoid_separation(&self) -> f64 {
        match self {
            CoordinateSystem::Projected { geoid_separation, .. } => *geoid_separation,
            _ => 0.0,
        }
    }

    /// Combined (grid / ground) scale factor at a point with a height above the
    /// geoid; `None` without a projection.
    pub fn combined_factor(&self, easting: f64, northing: f64, height: f64) -> Option<f64> {
        let tm = self.projection()?;
        Some(tm.combined_factor(easting, northing, height + self.geoid_separation()))
    }
}
