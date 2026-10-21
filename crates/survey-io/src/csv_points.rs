//! Delimited text point files (PNEZD, PENZD, ENZ, …) with configurable columns.

use serde::{Deserialize, Serialize};
use survey_core::conventions::LinearUnit;

use crate::{ImportedPoint, IoError, PointImport, PointImporter};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CsvField {
    Name,
    Easting,
    Northing,
    Height,
    Code,
    Description,
    Skip,
}

impl CsvField {
    pub const ALL: [CsvField; 7] = [
        CsvField::Name,
        CsvField::Easting,
        CsvField::Northing,
        CsvField::Height,
        CsvField::Code,
        CsvField::Description,
        CsvField::Skip,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CsvField::Name => "Name",
            CsvField::Easting => "Easting",
            CsvField::Northing => "Northing",
            CsvField::Height => "Height",
            CsvField::Code => "Code",
            CsvField::Description => "Description",
            CsvField::Skip => "(skip)",
        }
    }

    fn letter(self) -> char {
        match self {
            CsvField::Name => 'P',
            CsvField::Easting => 'E',
            CsvField::Northing => 'N',
            CsvField::Height => 'Z',
            CsvField::Code => 'C',
            CsvField::Description => 'D',
            CsvField::Skip => '-',
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeaderMode {
    /// Treat the first line as a header if its coordinate columns are not numeric.
    Auto,
    Yes,
    No,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CsvLayout {
    pub fields: Vec<CsvField>,
    /// `None` detects comma, semicolon, tab or whitespace from the first line.
    pub delimiter: Option<char>,
    pub header: HeaderMode,
    pub unit: LinearUnit,
}

impl CsvLayout {
    pub fn from_letters(letters: &str) -> Self {
        let fields = letters
            .chars()
            .filter_map(|c| match c.to_ascii_uppercase() {
                'P' => Some(CsvField::Name),
                'E' => Some(CsvField::Easting),
                'N' => Some(CsvField::Northing),
                'Z' => Some(CsvField::Height),
                'C' => Some(CsvField::Code),
                'D' => Some(CsvField::Description),
                '-' => Some(CsvField::Skip),
                _ => None,
            })
            .collect();
        Self {
            fields,
            delimiter: None,
            header: HeaderMode::Auto,
            unit: LinearUnit::Metre,
        }
    }

    /// Common layouts, named by column letters (P=name, C=code, D=description).
    pub fn presets() -> Vec<(&'static str, CsvLayout)> {
        ["PENZC", "PNEZC", "PENZD", "PNEZD", "PENZ", "PNEZ", "ENZ", "NEZ"]
            .into_iter()
            .map(|s| (s, Self::from_letters(s)))
            .collect()
    }

    pub fn letters(&self) -> String {
        self.fields.iter().map(|f| f.letter()).collect()
    }

    fn has_coordinates(&self) -> bool {
        self.fields.contains(&CsvField::Easting) && self.fields.contains(&CsvField::Northing)
    }
}

/// Splits one line using the given delimiter (`None` = whitespace), honouring double quotes.
fn split_line(line: &str, delimiter: Option<char>) -> Vec<String> {
    let Some(delim) = delimiter else {
        return line.split_whitespace().map(str::to_string).collect();
    };
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .delimiter(delim as u8)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(line.as_bytes());
    reader
        .records()
        .next()
        .and_then(Result::ok)
        .map(|r| r.iter().map(str::to_string).collect())
        .unwrap_or_default()
}

fn detect_delimiter(line: &str) -> Option<char> {
    [',', '\t', ';']
        .into_iter()
        .map(|d| (d, line.matches(d).count()))
        .filter(|(_, n)| *n > 0)
        .max_by_key(|(_, n)| *n)
        .map(|(d, _)| d)
}

#[derive(Clone, Debug)]
pub struct CsvPointImporter {
    pub layout: CsvLayout,
}

impl CsvPointImporter {
    pub fn new(layout: CsvLayout) -> Self {
        Self { layout }
    }

    pub fn import_text(&self, text: &str) -> Result<PointImport, IoError> {
        let layout = &self.layout;
        if !layout.has_coordinates() {
            return Err(IoError::LayoutMissingCoordinates);
        }
        let scale = layout.unit.metres_per_unit();
        let mut lines = text
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .peekable();
        let Some((_, first)) = lines.peek() else {
            return Err(IoError::NoPoints(" (file is empty)".into()));
        };
        let delimiter = layout.delimiter.or_else(|| detect_delimiter(first));

        let mut result = PointImport::default();
        let mut first_line = true;
        for (line_no, line) in lines {
            let values = split_line(line, delimiter);
            match parse_record(&layout.fields, &values, scale) {
                Ok(p) => result.points.push(p),
                Err(msg) => {
                    let is_header = first_line && layout.header != HeaderMode::No;
                    if !is_header {
                        result.warnings.push(format!("line {}: {msg}", line_no + 1));
                    }
                }
            }
            if first_line && layout.header == HeaderMode::Yes && !result.points.is_empty() {
                // Explicit header that happened to parse: drop it.
                result.points.pop();
            }
            first_line = false;
        }
        if result.points.is_empty() {
            let hint = result
                .warnings
                .first()
                .map(|w| format!(" ({w})"))
                .unwrap_or_default();
            return Err(IoError::NoPoints(hint));
        }
        Ok(result)
    }
}

fn parse_record(fields: &[CsvField], values: &[String], scale: f64) -> Result<ImportedPoint, String> {
    let mut p = ImportedPoint {
        name: String::new(),
        easting: f64::NAN,
        northing: f64::NAN,
        height: None,
        code: String::new(),
        description: String::new(),
    };
    let num = |v: &str, what: &str| -> Result<f64, String> {
        v.parse::<f64>()
            .ok()
            .filter(|x| x.is_finite())
            .map(|x| x * scale)
            .ok_or_else(|| format!("{what} '{v}' is not a number"))
    };
    for (field, value) in fields.iter().zip(values) {
        let value = value.as_str();
        match field {
            CsvField::Name => p.name = value.to_string(),
            CsvField::Easting => p.easting = num(value, "easting")?,
            CsvField::Northing => p.northing = num(value, "northing")?,
            CsvField::Height if !value.is_empty() => p.height = Some(num(value, "height")?),
            CsvField::Height => {}
            CsvField::Code => p.code = value.to_string(),
            CsvField::Description => p.description = value.to_string(),
            CsvField::Skip => {}
        }
    }
    if p.easting.is_nan() || p.northing.is_nan() {
        return Err(format!(
            "expected {} columns, found {}",
            fields.len(),
            values.len()
        ));
    }
    Ok(p)
}

impl PointImporter for CsvPointImporter {
    fn format_name(&self) -> &str {
        "Delimited text points"
    }

    fn extensions(&self) -> &[&str] {
        &["csv", "txt", "pnt", "xyz"]
    }

    fn import(&self, bytes: &[u8]) -> Result<PointImport, IoError> {
        let text = std::str::from_utf8(bytes).map_err(|_| IoError::NotText)?;
        self.import_text(text.strip_prefix('\u{feff}').unwrap_or(text))
    }
}

/// Writes points as comma-separated text using `layout` (delimiter defaults to comma).
pub fn export_points_csv<'a>(
    points: impl IntoIterator<Item = &'a ImportedPoint>,
    layout: &CsvLayout,
    decimals: usize,
) -> String {
    let delim = layout.delimiter.unwrap_or(',');
    let scale = layout.unit.metres_per_unit();
    let quote = |s: &str| {
        if s.contains(delim) || s.contains('"') {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    };
    let mut out = String::new();
    for p in points {
        let cols: Vec<String> = layout
            .fields
            .iter()
            .map(|f| match f {
                CsvField::Name => quote(&p.name),
                CsvField::Easting => format!("{:.decimals$}", p.easting / scale),
                CsvField::Northing => format!("{:.decimals$}", p.northing / scale),
                CsvField::Height => p
                    .height
                    .map(|h| format!("{:.decimals$}", h / scale))
                    .unwrap_or_default(),
                CsvField::Code => quote(&p.code),
                CsvField::Description => quote(&p.description),
                CsvField::Skip => String::new(),
            })
            .collect();
        out.push_str(&cols.join(&delim.to_string()));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_penzc_with_header() {
        let text = "Name,E,N,Z,Code\n1,500000.123,7000000.456,12.5,CP\n2,500010,7000010,,FENCE\nbad line\n";
        let imp = CsvPointImporter::new(CsvLayout::from_letters("PENZC"))
            .import_text(text)
            .expect("ok");
        assert_eq!(imp.points.len(), 2);
        assert_eq!(imp.points[0].easting, 500000.123);
        assert_eq!(imp.points[0].height, Some(12.5));
        assert_eq!(imp.points[1].height, None);
        assert_eq!(imp.points[1].code, "FENCE");
        assert_eq!(imp.warnings.len(), 1, "{:?}", imp.warnings);
    }

    #[test]
    fn whitespace_and_order() {
        let text = "1  7000000.0   500000.0  10.0\n2\t7000001\t500001\t11";
        let imp = CsvPointImporter::new(CsvLayout::from_letters("PNEZ"))
            .import_text(text)
            .expect("ok");
        // Second line is tab-delimited but the delimiter is detected from line 1 (whitespace).
        assert_eq!(imp.points.len(), 2);
        assert_eq!(imp.points[0].northing, 7000000.0);
        assert_eq!(imp.points[1].easting, 500001.0);
    }

    #[test]
    fn units_and_round_trip() {
        let mut layout = CsvLayout::from_letters("PENZD");
        layout.unit = LinearUnit::InternationalFoot;
        let imp = CsvPointImporter::new(layout.clone())
            .import_text("A,100,200,10,\"pin, found\"")
            .expect("ok");
        assert!((imp.points[0].easting - 30.48).abs() < 1e-9);
        assert_eq!(imp.points[0].description, "pin, found");
        let out = export_points_csv(&imp.points, &layout, 3);
        assert_eq!(out, "A,100.000,200.000,10.000,\"pin, found\"\n");
    }

    #[test]
    fn errors() {
        let imp = CsvPointImporter::new(CsvLayout::from_letters("PZ"));
        assert!(matches!(
            imp.import_text("1,2"),
            Err(IoError::LayoutMissingCoordinates)
        ));
        let imp = CsvPointImporter::new(CsvLayout::from_letters("PEN"));
        assert!(matches!(imp.import_text("a,b,c"), Err(IoError::NoPoints(_))));
    }
}
