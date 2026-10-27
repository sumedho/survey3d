//! Construction alignments: a polyline with a starting chainage.

use serde::{Deserialize, Serialize};

use crate::ids::EntityId;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AlignmentRecord {
    pub name: String,
    /// The centreline polyline (straights and arcs).
    pub polyline: EntityId,
    /// Chainage at the polyline's first vertex.
    pub start_chainage: f64,
    /// Distance between chainage ticks drawn along it.
    pub tick_interval: f64,
    /// Draw chainage ticks and labels.
    pub show_chainages: bool,
    /// Clothoid spirals into and out of the centreline's arcs.
    pub transitions: Vec<survey_core::alignment::Transition>,
}

impl Default for AlignmentRecord {
    fn default() -> Self {
        Self {
            name: String::new(),
            polyline: EntityId(0),
            start_chainage: 0.0,
            tick_interval: 20.0,
            show_chainages: true,
            transitions: Vec::new(),
        }
    }
}
