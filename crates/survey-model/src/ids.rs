use serde::{Deserialize, Serialize};

/// Identifier of an entity (point, polyline, …). Unique within a project.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntityId(pub i64);

/// Identifier of a layer. Unique within a project.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LayerId(pub i64);
