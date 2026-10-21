//! The survey document model.
//!
//! [`Document`] is the in-memory source of truth while editing. All edits are
//! expressed as [`Command`]s executed through [`History`], which provides
//! undo/redo, records audit entries and lets the document track which records
//! are dirty for incremental saving.

pub mod command;
pub mod coord_system;
pub mod document;
pub mod entity;
pub mod history;
pub mod ids;
pub mod layer;
pub mod selection;
pub mod spatial;

pub use command::{ChangeSet, Command};
pub use coord_system::CoordinateSystem;
pub use document::{DirtyState, Document, ProjectMeta};
pub use entity::{Entity, Polyline, SurveyPoint};
pub use history::{AuditAction, AuditRecord, History};
pub use ids::{EntityId, LayerId};
pub use layer::{Color, Layer};
pub use selection::Selection;
