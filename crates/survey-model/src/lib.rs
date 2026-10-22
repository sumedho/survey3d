//! The survey document model.
//!
//! [`Document`] is the in-memory source of truth while editing. All edits are
//! expressed as [`Command`]s executed through [`History`], which provides
//! undo/redo, records audit entries and lets the document track which records
//! are dirty for incremental saving.

pub mod alignment;
pub mod command;
pub mod coord_system;
pub mod document;
pub mod entity;
pub mod history;
pub mod ids;
pub mod layer;
pub mod parcel;
pub mod selection;
pub mod spatial;
pub mod surface;

pub use alignment::AlignmentRecord;
pub use command::{ChangeSet, Command};
pub use coord_system::CoordinateSystem;
pub use document::{DirtyState, Document, ProjectMeta};
pub use entity::{Entity, Polyline, SurveyPoint, join_polylines};
pub use history::{AuditAction, AuditRecord, History};
pub use ids::{EntityId, LayerId, SurfaceId};
pub use layer::{Color, Layer};
pub use parcel::ParcelRecord;
pub use selection::Selection;
pub use surface::{ContourStyle, Surface, SurfaceDefinition, SurfaceStyle};
