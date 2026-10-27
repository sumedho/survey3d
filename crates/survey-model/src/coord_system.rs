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
        /// Used where no geoid model is set or the model does not cover a point.
        #[serde(default)]
        geoid_separation: f64,
        /// The geoid model's file name, found in the user's geoid folder.
        #[serde(default)]
        geoid: Option<String>,
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

    /// The geoid model's file name, when one is set.
    pub fn geoid_model(&self) -> Option<&str> {
        match self {
            CoordinateSystem::Projected { geoid, .. } => geoid.as_deref().filter(|g| !g.trim().is_empty()),
            _ => None,
        }
    }

    /// The EPSG code, when known.
    pub fn epsg(&self) -> Option<u32> {
        match self {
            CoordinateSystem::Projected { epsg, .. } => *epsg,
            _ => None,
        }
    }

    /// The projection as OGC WKT (version 1, as LAS, GeoPackage and `.prj`
    /// files take it); `None` without a projection.
    pub fn wkt(&self) -> Option<String> {
        let tm = self.projection()?;
        let e = tm.ellipsoid;
        let authority = self
            .epsg()
            .map(|c| format!(",AUTHORITY[\"EPSG\",\"{c}\"]"))
            .unwrap_or_default();
        Some(format!(
            "PROJCS[\"{name}\",GEOGCS[\"{0}\",DATUM[\"{0}\",SPHEROID[\"{0}\",{1},{2}]],PRIMEM[\"Greenwich\",0],\
             UNIT[\"degree\",0.0174532925199433]],PROJECTION[\"Transverse_Mercator\"],\
             PARAMETER[\"latitude_of_origin\",{3}],PARAMETER[\"central_meridian\",{4}],\
             PARAMETER[\"scale_factor\",{5}],PARAMETER[\"false_easting\",{6}],PARAMETER[\"false_northing\",{7}],\
             UNIT[\"metre\",1],AXIS[\"Easting\",EAST],AXIS[\"Northing\",NORTH]{authority}]",
            e.name,
            e.a,
            e.inv_f,
            tm.latitude_of_origin,
            tm.central_meridian,
            tm.scale,
            tm.false_easting,
            tm.false_northing,
            name = self.display_name().replace('"', "'"),
        ))
    }
}

/// Which distances are shown in reports and on plans, and typed in `bd`.
/// Coordinates are always on the grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DistanceKind {
    #[default]
    Grid,
    /// Grid distance ÷ the combined scale factor.
    Ground,
}

impl DistanceKind {
    pub fn label(self) -> &'static str {
        match self {
            DistanceKind::Grid => "grid",
            DistanceKind::Ground => "ground",
        }
    }
}

/// The combined (grid ÷ ground) scale factor for ground distances.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum GroundFactor {
    /// From the projection along each line: point scale factor × height
    /// factor at the points' heights above the ellipsoid.
    #[default]
    Projection,
    /// One factor for the whole project.
    Fixed(f64),
}

/// Grid or ground distances for a project.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroundSettings {
    pub distances: DistanceKind,
    pub factor: GroundFactor,
}
