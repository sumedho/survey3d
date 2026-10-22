//! Cadastral parcels: a named lot whose boundary is a closed polyline.

use serde::{Deserialize, Serialize};

use crate::ids::EntityId;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ParcelRecord {
    /// Lot name or number, e.g. "Lot 12".
    pub name: String,
    /// Plan or deposited plan reference.
    pub plan: String,
    pub description: String,
    /// The closed polyline forming the boundary.
    pub boundary: EntityId,
    /// Draw bearing/distance and area annotation in the viewport.
    pub annotate: bool,
}

impl Default for ParcelRecord {
    fn default() -> Self {
        Self {
            name: String::new(),
            plan: String::new(),
            description: String::new(),
            boundary: EntityId(0),
            annotate: true,
        }
    }
}
