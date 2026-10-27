//! The survey document model.
//!
//! [`Document`] is the in-memory source of truth while editing. All edits are
//! expressed as [`Command`]s executed through [`History`], which provides
//! undo/redo, records audit entries and lets the document track which records
//! are dirty for incremental saving.

pub mod alignment;
pub mod cloud;
pub mod command;
pub mod coord_system;
pub mod document;
pub mod entity;
pub mod ground;
pub mod history;
pub mod ids;
pub mod layer;
pub mod parcel;
pub mod records;
pub mod selection;
pub mod sheet;
pub mod spatial;
pub mod surface;

pub use alignment::AlignmentRecord;
pub use cloud::{CloudColor, CloudRef, CloudStyle, PointCloud, Resident};
pub use command::{ChangeSet, Command};
pub use coord_system::{CoordinateSystem, DistanceKind, GroundFactor, GroundSettings};
pub use document::{DirtyState, Document, ProjectMeta};
pub use entity::{
    Dimension, DimensionKind, Entity, Polyline, SurveyPoint, TextAlign, TextNote, join_polylines,
};
pub use history::{AuditAction, AuditRecord, History, UNDO_MEMORY_LIMIT};
pub use ids::{EntityId, LayerId, PointCloudId, SurfaceId};
pub use layer::{Color, DEFAULT_LINE_WEIGHT, Layer};
pub use parcel::ParcelRecord;
pub use records::{AnyRecord, Keyed, Record, RecordId, RecordKind, Records};
pub use selection::Selection;
pub use sheet::SheetRecord;
pub use surface::{ContourStyle, Surface, SurfaceDefinition, SurfaceStyle};
