use serde::{Deserialize, Serialize};

/// The coordinate reference of a project or dataset.
///
/// Phase 0 only records this; transformations between systems come later.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum CoordinateSystem {
    #[default]
    Unspecified,
    /// A map projection such as MGA2020 zone 56 or a UTM zone.
    Projected {
        name: String,
        epsg: Option<u32>,
        zone: Option<u32>,
    },
    /// An arbitrary site/local grid, optionally related to a projected system.
    LocalGrid { name: String, description: String },
}

impl CoordinateSystem {
    pub fn display_name(&self) -> String {
        match self {
            CoordinateSystem::Unspecified => "Unspecified".into(),
            CoordinateSystem::Projected { name, epsg, .. } => match epsg {
                Some(code) => format!("{name} (EPSG:{code})"),
                None => name.clone(),
            },
            CoordinateSystem::LocalGrid { name, .. } => format!("Local grid: {name}"),
        }
    }
}
