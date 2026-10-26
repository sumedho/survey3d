//! Import and export of survey interchange formats.
//!
//! Each format implements [`PointImporter`] (and later richer importer traits for
//! linework, surfaces and raw observations). Importers return plain records in
//! canonical units; the application turns them into document edits.

pub mod code_sheet;
#[cfg(not(target_arch = "wasm32"))]
pub mod copc;
#[cfg(not(target_arch = "wasm32"))]
pub mod copc_write;
pub mod csv_points;
pub mod dxf;
pub mod landxml;
pub mod plot;
pub mod pointcloud;
pub mod rawdata;
pub mod stakeout;

pub use csv_points::{CsvField, CsvLayout, CsvPointImporter, HeaderMode, export_points_csv};

use thiserror::Error;

/// A finite number from text. Rust's parser also accepts `inf` and `NaN`,
/// which turn up in some exports and would poison bounds and indexes.
pub(crate) fn finite(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// A point read from an interchange file, in metres.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedPoint {
    pub name: String,
    pub easting: f64,
    pub northing: f64,
    pub height: Option<f64>,
    pub code: String,
    pub description: String,
}

/// Points plus non-fatal problems (e.g. unparsable lines that were skipped).
#[derive(Clone, Debug, Default)]
pub struct PointImport {
    pub points: Vec<ImportedPoint>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Error)]
pub enum IoError {
    #[error("file is not valid UTF-8 text")]
    NotText,
    #[error("layout must include easting and northing columns")]
    LayoutMissingCoordinates,
    #[error("nothing to import{0}")]
    NoPoints(String),
    #[error("{0}")]
    Invalid(String),
}

/// A format that yields survey points.
pub trait PointImporter {
    fn format_name(&self) -> &str;
    fn extensions(&self) -> &[&str];
    fn import(&self, bytes: &[u8]) -> Result<PointImport, IoError>;
}
