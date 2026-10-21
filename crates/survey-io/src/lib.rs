//! Import and export of survey interchange formats.
//!
//! Each format implements [`PointImporter`] (and later richer importer traits for
//! linework, surfaces and raw observations). Importers return plain records in
//! canonical units; the application turns them into document edits.

pub mod csv_points;

pub use csv_points::{CsvField, CsvLayout, CsvPointImporter, HeaderMode, export_points_csv};

use thiserror::Error;

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
    #[error("no points found{0}")]
    NoPoints(String),
}

/// A format that yields survey points.
pub trait PointImporter {
    fn format_name(&self) -> &str;
    fn extensions(&self) -> &[&str];
    fn import(&self, bytes: &[u8]) -> Result<PointImport, IoError>;
}
