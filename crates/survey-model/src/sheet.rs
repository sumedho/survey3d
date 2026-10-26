//! Plan sheets: a page of a given paper size showing part of the drawing at a
//! true scale, with a title block, grid, north arrow, scale bar and legend.

use serde::{Deserialize, Serialize};

/// Paper sizes (ISO A series and US sizes) plus a custom size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Paper {
    A4,
    #[default]
    A3,
    A2,
    A1,
    A0,
    Letter,
    Legal,
    Tabloid,
    AnsiC,
    AnsiD,
    AnsiE,
    Custom,
}

impl Paper {
    pub const ALL: [Paper; 12] = [
        Paper::A4,
        Paper::A3,
        Paper::A2,
        Paper::A1,
        Paper::A0,
        Paper::Letter,
        Paper::Legal,
        Paper::Tabloid,
        Paper::AnsiC,
        Paper::AnsiD,
        Paper::AnsiE,
        Paper::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Paper::A4 => "A4",
            Paper::A3 => "A3",
            Paper::A2 => "A2",
            Paper::A1 => "A1",
            Paper::A0 => "A0",
            Paper::Letter => "Letter",
            Paper::Legal => "Legal",
            Paper::Tabloid => "Tabloid (ANSI B)",
            Paper::AnsiC => "ANSI C",
            Paper::AnsiD => "ANSI D",
            Paper::AnsiE => "ANSI E",
            Paper::Custom => "Custom",
        }
    }

    /// Portrait size (width, height) in millimetres; `None` for custom.
    pub fn size_mm(self) -> Option<[f64; 2]> {
        let inch = |w: f64, h: f64| [w * 25.4, h * 25.4];
        Some(match self {
            Paper::A4 => [210.0, 297.0],
            Paper::A3 => [297.0, 420.0],
            Paper::A2 => [420.0, 594.0],
            Paper::A1 => [594.0, 841.0],
            Paper::A0 => [841.0, 1189.0],
            Paper::Letter => inch(8.5, 11.0),
            Paper::Legal => inch(8.5, 14.0),
            Paper::Tabloid => inch(11.0, 17.0),
            Paper::AnsiC => inch(17.0, 22.0),
            Paper::AnsiD => inch(22.0, 34.0),
            Paper::AnsiE => inch(34.0, 44.0),
            Paper::Custom => return None,
        })
    }
}

/// How the coordinate grid is marked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GridMarks {
    None,
    /// Ticks and values around the frame only.
    Ticks,
    /// Ticks around the frame and crosses at the intersections.
    #[default]
    Crosses,
    /// Full grid lines.
    Lines,
}

impl GridMarks {
    pub const ALL: [GridMarks; 4] = [
        GridMarks::None,
        GridMarks::Ticks,
        GridMarks::Crosses,
        GridMarks::Lines,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GridMarks::None => "None",
            GridMarks::Ticks => "Edge ticks",
            GridMarks::Crosses => "Ticks and crosses",
            GridMarks::Lines => "Grid lines",
        }
    }
}

/// Where the legend goes in the map frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegendPosition {
    /// The corner with the least of the drawing under it.
    #[default]
    Auto,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl LegendPosition {
    pub const ALL: [LegendPosition; 5] = [
        LegendPosition::Auto,
        LegendPosition::TopLeft,
        LegendPosition::TopRight,
        LegendPosition::BottomLeft,
        LegendPosition::BottomRight,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LegendPosition::Auto => "Clearest corner",
            LegendPosition::TopLeft => "Top left",
            LegendPosition::TopRight => "Top right",
            LegendPosition::BottomLeft => "Bottom left",
            LegendPosition::BottomRight => "Bottom right",
        }
    }
}

/// Which parcel boundaries go in line and curve tables (labelled L1, C1… on
/// the drawing) instead of carrying their bearing and distance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryTables {
    /// Every boundary is labelled along its line.
    #[default]
    None,
    /// Boundaries too short for their label.
    Short,
    /// Every boundary.
    All,
}

impl BoundaryTables {
    pub const ALL: [BoundaryTables; 3] = [BoundaryTables::None, BoundaryTables::Short, BoundaryTables::All];

    pub fn label(self) -> &'static str {
        match self {
            BoundaryTables::None => "None (label every line)",
            BoundaryTables::Short => "Short lines and curves",
            BoundaryTables::All => "All lines and curves",
        }
    }
}

/// What a sheet shows.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SheetContent {
    pub linework: bool,
    pub points: bool,
    pub point_labels: bool,
    /// Bearings, distances and areas of parcels marked for annotation.
    pub parcels: bool,
    /// Parcel boundaries listed in line and curve tables.
    pub tables: BoundaryTables,
    pub contours: bool,
    pub chainages: bool,
    pub grid: GridMarks,
    pub north_arrow: bool,
    pub scale_bar: bool,
    pub legend: bool,
    pub legend_position: LegendPosition,
    /// Everything in black (for photocopies and faxes of old).
    pub monochrome: bool,
}

impl Default for SheetContent {
    fn default() -> Self {
        Self {
            linework: true,
            points: true,
            point_labels: true,
            parcels: true,
            tables: BoundaryTables::None,
            contours: true,
            chainages: true,
            grid: GridMarks::default(),
            north_arrow: true,
            scale_bar: true,
            legend: true,
            legend_position: LegendPosition::Auto,
            monochrome: false,
        }
    }
}

/// Title block text.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TitleBlock {
    pub title: String,
    /// Client, locality or project.
    pub subtitle: String,
    pub surveyor: String,
    pub date: String,
    /// Job or plan reference.
    pub reference: String,
    pub sheet: String,
    pub notes: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SheetRecord {
    pub name: String,
    pub paper: Paper,
    /// Portrait size used when `paper` is custom (millimetres).
    pub custom_size: [f64; 2],
    pub landscape: bool,
    /// Scale denominator (500 for 1:500).
    pub scale: f64,
    /// World coordinates at the centre of the map frame.
    pub centre: [f64; 2],
    /// Grid bearing (degrees) that points up the sheet; 0 is north up.
    pub rotation: f64,
    pub title: TitleBlock,
    pub content: SheetContent,
    /// Grid spacing in metres; 0 picks one from the scale.
    pub grid_interval: f64,
    /// Label font size on paper (millimetres).
    pub text_size: f64,
    /// Paper millimetres per screen point for symbols (so a 7 pt symbol is 7 × this).
    pub symbol_scale: f64,
    /// Paper millimetres per screen point for line weights.
    pub line_scale: f64,
}

impl Default for SheetRecord {
    fn default() -> Self {
        Self {
            name: "Sheet 1".into(),
            paper: Paper::default(),
            custom_size: [297.0, 420.0],
            landscape: true,
            scale: 500.0,
            centre: [0.0, 0.0],
            rotation: 0.0,
            title: TitleBlock::default(),
            content: SheetContent::default(),
            grid_interval: 0.0,
            text_size: 2.0,
            symbol_scale: 0.2,
            line_scale: 0.18,
        }
    }
}

/// Common plan scales (denominators).
pub const STANDARD_SCALES: [f64; 16] = [
    50.0, 100.0, 200.0, 250.0, 500.0, 1000.0, 1250.0, 2000.0, 2500.0, 5000.0, 10_000.0, 20_000.0, 25_000.0,
    50_000.0, 100_000.0, 250_000.0,
];

impl SheetRecord {
    /// Page size (width, height) in millimetres, oriented.
    pub fn page_size(&self) -> [f64; 2] {
        let [w, h] = self.paper.size_mm().unwrap_or(self.custom_size);
        let (short, long) = (w.min(h).max(50.0), w.max(h).max(50.0));
        if self.landscape {
            [long, short]
        } else {
            [short, long]
        }
    }

    /// Paper millimetres per world metre.
    pub fn mm_per_metre(&self) -> f64 {
        1000.0 / self.scale.max(1.0)
    }

    /// `1:500 at A3`.
    pub fn scale_label(&self) -> String {
        let n = self.scale;
        let n = if n.fract() == 0.0 {
            format!("{n:.0}")
        } else {
            format!("{n}")
        };
        if self.paper == Paper::Custom {
            format!("1:{n}")
        } else {
            format!("1:{n} at {}", self.paper.label())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_sizes_and_scale() {
        let mut s = SheetRecord::default();
        assert_eq!(s.page_size(), [420.0, 297.0]);
        s.landscape = false;
        s.paper = Paper::A1;
        assert_eq!(s.page_size(), [594.0, 841.0]);
        s.paper = Paper::Custom;
        s.custom_size = [600.0, 300.0];
        assert_eq!(s.page_size(), [300.0, 600.0]);
        s.scale = 250.0;
        assert_eq!(s.mm_per_metre(), 4.0);
        assert_eq!(s.scale_label(), "1:250");
        let json = serde_json::to_string(&SheetRecord::default()).expect("json");
        let back: SheetRecord = serde_json::from_str(&json).expect("parse");
        assert_eq!(back, SheetRecord::default());
        // Older or partial records fill in defaults.
        let partial: SheetRecord = serde_json::from_str(r#"{"name":"P","paper":"a1"}"#).expect("partial");
        assert_eq!(partial.paper, Paper::A1);
        assert_eq!(partial.scale, 500.0);
    }
}
