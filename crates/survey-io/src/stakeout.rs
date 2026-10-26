//! Stake-out files for controllers: points (and straight lines between them)
//! as Trimble JobXML, Carlson RW5 stored points or Leica GSI-16 coordinates.
//! Delimited text goes through [`crate::export_points_csv`].

use std::fmt::Write as _;

use crate::ImportedPoint;

/// A controller file format for stake-out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StakeFormat {
    /// Trimble JobXML (Trimble Access), points and lines.
    JobXml,
    /// Carlson RW5 stored points (SurvCE, SurvPC).
    Rw5,
    /// Leica GSI-16 coordinates (Captivate, FlexLine).
    Gsi,
    /// Delimited text, `P,N,E,Z,C` (most controllers import it).
    Csv,
}

impl StakeFormat {
    pub const ALL: [StakeFormat; 4] = [
        StakeFormat::JobXml,
        StakeFormat::Rw5,
        StakeFormat::Gsi,
        StakeFormat::Csv,
    ];

    pub fn key(self) -> &'static str {
        match self {
            StakeFormat::JobXml => "jobxml",
            StakeFormat::Rw5 => "rw5",
            StakeFormat::Gsi => "gsi",
            StakeFormat::Csv => "csv",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            StakeFormat::JobXml => "jxl",
            StakeFormat::Rw5 => "rw5",
            StakeFormat::Gsi => "gsi",
            StakeFormat::Csv => "csv",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            StakeFormat::JobXml => "Trimble JobXML (points and lines)",
            StakeFormat::Rw5 => "Carlson RW5 points",
            StakeFormat::Gsi => "Leica GSI-16 points",
            StakeFormat::Csv => "CSV (name, N, E, Z, code)",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|f| f.key() == s || f.extension() == s || (s == "gsi16" && *f == StakeFormat::Gsi))
    }
}

/// A straight line between two of the exported points, by name.
#[derive(Clone, Debug, PartialEq)]
pub struct StakeLine {
    pub name: String,
    pub code: String,
    pub from: String,
    pub to: String,
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Trimble JobXML: keyed-in points with grid coordinates, and two-point
/// lines between them (Trimble Access stakes both).
pub fn write_jobxml(job: &str, points: &[ImportedPoint], lines: &[StakeLine]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        out,
        r#"<JOBFile jobName="{}" product="survey3d" productVersion="{}" version="5.6">"#,
        escape(job),
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(out, "  <FieldBook>");
    let mut id = 0;
    let mut next = || {
        id += 1;
        format!("{id:08X}")
    };
    for p in points {
        let _ = writeln!(out, r#"    <PointRecord ID="{}">"#, next());
        let _ = writeln!(out, "      <Name>{}</Name>", escape(&p.name));
        let _ = writeln!(out, "      <Code>{}</Code>", escape(&p.code));
        if !p.description.is_empty() {
            let _ = writeln!(
                out,
                "      <Description1>{}</Description1>",
                escape(&p.description)
            );
        }
        let _ = writeln!(out, "      <Method>KeyedIn</Method>");
        let _ = writeln!(out, "      <Classification>Design</Classification>");
        let _ = writeln!(out, "      <Deleted>false</Deleted>");
        let _ = write!(
            out,
            "      <Grid><North>{:.4}</North><East>{:.4}</East>",
            p.northing, p.easting
        );
        if let Some(z) = p.height {
            let _ = write!(out, "<Elevation>{z:.4}</Elevation>");
        }
        let _ = writeln!(out, "</Grid>");
        let _ = writeln!(out, "    </PointRecord>");
    }
    for l in lines {
        let _ = writeln!(out, r#"    <LineRecord ID="{}">"#, next());
        let _ = writeln!(out, "      <Name>{}</Name>", escape(&l.name));
        let _ = writeln!(out, "      <Code>{}</Code>", escape(&l.code));
        let _ = writeln!(out, "      <Method>TwoPoints</Method>");
        let _ = writeln!(out, "      <StartPoint>{}</StartPoint>", escape(&l.from));
        let _ = writeln!(out, "      <EndPoint>{}</EndPoint>", escape(&l.to));
        let _ = writeln!(out, "      <StartStation>0</StartStation>");
        let _ = writeln!(out, "      <Deleted>false</Deleted>");
        let _ = writeln!(out, "    </LineRecord>");
    }
    let _ = writeln!(out, "  </FieldBook>");
    let _ = writeln!(out, "</JOBFile>");
    out
}

/// RW5 text fields cannot hold commas (they separate fields).
fn rw5_text(s: &str) -> String {
    s.replace(',', " ")
}

/// Carlson RW5 stored points (SP records) in metres, which SurvCE and SurvPC
/// read as a job's design points.
pub fn write_rw5_points(job: &str, points: &[ImportedPoint]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "JB,NM{},DT,TM", rw5_text(job));
    let _ = writeln!(out, "MO,AD0,UN1,SF1.00000000,EC0,EO0.0,AU0");
    for p in points {
        let _ = write!(
            out,
            "SP,PN{},N {:.4},E {:.4}",
            rw5_text(&p.name),
            p.northing,
            p.easting
        );
        if let Some(z) = p.height {
            let _ = write!(out, ",EL{z:.4}");
        }
        let note = match (p.code.is_empty(), p.description.is_empty()) {
            (false, false) => format!("{} {}", p.code, p.description),
            (false, true) => p.code.clone(),
            (true, _) => p.description.clone(),
        };
        let _ = writeln!(out, ",--{note}");
    }
    out
}

/// GSI text data: up to 16 characters, padded on the left with zeros.
/// Characters GSI cannot carry are dropped.
fn gsi_text(s: &str) -> (String, bool) {
    let clean: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    let changed = clean.len() != s.len() || clean.len() > 16;
    let tail = &clean[clean.len().saturating_sub(16)..];
    (format!("{tail:0>16}"), changed)
}

/// A GSI-16 length word in millimetres.
fn gsi_length(wi: u32, metres: f64) -> String {
    let mm = (metres * 1000.0).round();
    let sign = if mm < 0.0 { '-' } else { '+' };
    format!("{wi}..10{sign}{:016}", mm.abs() as u64)
}

/// Leica GSI-16 coordinates: point name (11), easting (81), northing (82),
/// height (83, when known) and code (71), in millimetres. Returns the text and
/// the names or codes that had to be shortened or cleaned.
pub fn write_gsi16(points: &[ImportedPoint]) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut changed = Vec::new();
    for (i, p) in points.iter().enumerate() {
        let (name, n_changed) = gsi_text(&p.name);
        let _ = write!(
            out,
            "*11{:04}+{name} {} {}",
            (i + 1) % 10_000,
            gsi_length(81, p.easting),
            gsi_length(82, p.northing)
        );
        if let Some(z) = p.height {
            let _ = write!(out, " {}", gsi_length(83, z));
        }
        let mut c_changed = false;
        if !p.code.is_empty() {
            let (code, c) = gsi_text(&p.code);
            c_changed = c;
            let _ = write!(out, " 71....+{code}");
        }
        out.push_str(" \r\n");
        if n_changed || c_changed {
            changed.push(p.name.clone());
        }
    }
    (out, changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rawdata::{read_gsi, read_jobxml, read_rw5};

    fn pts() -> Vec<ImportedPoint> {
        vec![
            ImportedPoint {
                name: "101".into(),
                easting: 500123.4567,
                northing: 7001234.5678,
                height: Some(12.345),
                code: "PEG".into(),
                description: "lot 3".into(),
            },
            ImportedPoint {
                name: "K2".into(),
                easting: -10.0,
                northing: 20.0,
                height: None,
                code: String::new(),
                description: String::new(),
            },
        ]
    }

    #[test]
    fn files_read_back() {
        let lines = [StakeLine {
            name: "FRONT".into(),
            code: "BDY".into(),
            from: "101".into(),
            to: "K2".into(),
        }];
        let xml = write_jobxml("job & co", &pts(), &lines);
        let d = read_jobxml("j", xml.as_bytes()).expect("jobxml");
        assert_eq!(d.points.len(), 2);
        assert!((d.points[0].1 - glam::DVec3::new(500123.4567, 7001234.5678, 12.345)).length() < 1e-9);
        assert!(xml.contains("<StartPoint>101</StartPoint>") && xml.contains("job &amp; co"));

        let rw5 = write_rw5_points("job", &pts());
        assert!(rw5.contains("SP,PN101,N 7001234.5678,E 500123.4567,EL12.3450,--PEG lot 3"));
        let d = read_rw5("r", rw5.as_bytes()).expect("rw5");
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
        assert!(
            d.points
                .iter()
                .any(|(n, p)| n == "K2" && (p.x + 10.0).abs() < 1e-9)
        );

        let (gsi, changed) = write_gsi16(&pts());
        assert!(changed.is_empty());
        assert!(gsi.starts_with("*110001+0000000000000101 81..10+0000000500123457"));
        let d = read_gsi("g", gsi.as_bytes()).expect("gsi");
        assert!((d.points[0].1 - glam::DVec3::new(500123.457, 7001234.568, 12.345)).length() < 1e-9);
        assert_eq!(d.points[1].0, "K2");
        assert!((d.points[1].1.x + 10.0).abs() < 1e-9);
        let (_, changed) = write_gsi16(&[ImportedPoint {
            name: "a very long point name".into(),
            ..pts()[1].clone()
        }]);
        assert_eq!(changed.len(), 1);
    }
}
