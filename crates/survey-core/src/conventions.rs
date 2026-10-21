//! Configurable regional conventions for parsing and displaying survey values.
//!
//! Nothing in the data model depends on these settings: they are a presentation
//! layer over canonical values (metres, radians, E/N/Z). Presets are provided for
//! common practice but every field can be changed independently.

use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::angle::{Angle, Bearing, Quadrant};
use crate::error::ParseError;

/// How angles are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AngleFormat {
    /// `123°45'30"`. Plain numbers typed by the user are read as packed DD.MMSS.
    DmsSymbols,
    /// `123.4530` (DD.MMSS). Plain numbers are read as packed DD.MMSS.
    DmsPacked,
    /// `123.7583°`. Plain numbers are read as decimal degrees.
    DecimalDegrees,
    /// `137.5093g` (400 per circle). Plain numbers are read as gons.
    Gons,
}

impl AngleFormat {
    pub const ALL: [AngleFormat; 4] = [
        AngleFormat::DmsSymbols,
        AngleFormat::DmsPacked,
        AngleFormat::DecimalDegrees,
        AngleFormat::Gons,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AngleFormat::DmsSymbols => "D°M'S\"",
            AngleFormat::DmsPacked => "DD.MMSS",
            AngleFormat::DecimalDegrees => "Decimal degrees",
            AngleFormat::Gons => "Gons",
        }
    }
}

/// How directions are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BearingStyle {
    /// 0–360 clockwise from north.
    WholeCircle,
    /// `N 45°30' E` style.
    Quadrant,
}

impl BearingStyle {
    pub const ALL: [BearingStyle; 2] = [BearingStyle::WholeCircle, BearingStyle::Quadrant];

    pub fn label(self) -> &'static str {
        match self {
            BearingStyle::WholeCircle => "Whole circle",
            BearingStyle::Quadrant => "Quadrant",
        }
    }
}

/// Order of horizontal coordinates in input and output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoordOrder {
    EastingNorthing,
    NorthingEasting,
}

impl CoordOrder {
    pub const ALL: [CoordOrder; 2] = [CoordOrder::EastingNorthing, CoordOrder::NorthingEasting];

    pub fn label(self) -> &'static str {
        match self {
            CoordOrder::EastingNorthing => "Easting, Northing",
            CoordOrder::NorthingEasting => "Northing, Easting",
        }
    }
}

/// Display unit for lengths. Values are always stored in metres.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinearUnit {
    Metre,
    InternationalFoot,
    UsSurveyFoot,
}

impl LinearUnit {
    pub const ALL: [LinearUnit; 3] = [
        LinearUnit::Metre,
        LinearUnit::InternationalFoot,
        LinearUnit::UsSurveyFoot,
    ];

    /// Metres per one display unit.
    pub fn metres_per_unit(self) -> f64 {
        match self {
            LinearUnit::Metre => 1.0,
            LinearUnit::InternationalFoot => 0.3048,
            LinearUnit::UsSurveyFoot => 1200.0 / 3937.0,
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            LinearUnit::Metre => "m",
            LinearUnit::InternationalFoot => "ft",
            LinearUnit::UsSurveyFoot => "sft",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LinearUnit::Metre => "Metres",
            LinearUnit::InternationalFoot => "International feet",
            LinearUnit::UsSurveyFoot => "US survey feet",
        }
    }
}

/// How chainages / stations are written.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ChainageStyle {
    /// `1234.567`
    Plain,
    /// `12+34.567` with `interval = 100`, or `1+234.567` with `interval = 1000`.
    Station { interval: u32 },
}

/// Short labels used for the three axes, e.g. `E`/`N`/`RL` or `E`/`N`/`Elev`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AxisLabels {
    pub easting: String,
    pub northing: String,
    pub height: String,
}

/// A complete set of parsing/formatting conventions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Conventions {
    pub name: String,
    pub angle_format: AngleFormat,
    /// Decimal places on the smallest angle unit (seconds for DMS formats, degrees/gons otherwise).
    pub angle_decimals: u8,
    pub bearing_style: BearingStyle,
    pub coord_order: CoordOrder,
    pub linear_unit: LinearUnit,
    pub linear_decimals: u8,
    pub chainage_style: ChainageStyle,
    pub axis_labels: AxisLabels,
}

impl Default for Conventions {
    fn default() -> Self {
        Self::metric_generic()
    }
}

/// Coordinates parsed from user input, in canonical order and metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParsedCoord {
    pub easting: f64,
    pub northing: f64,
    pub height: Option<f64>,
}

impl Conventions {
    /// Neutral metric preset: whole-circle DMS, E/N/Z.
    pub fn metric_generic() -> Self {
        Self {
            name: "Metric (generic)".into(),
            angle_format: AngleFormat::DmsSymbols,
            angle_decimals: 0,
            bearing_style: BearingStyle::WholeCircle,
            coord_order: CoordOrder::EastingNorthing,
            linear_unit: LinearUnit::Metre,
            linear_decimals: 3,
            chainage_style: ChainageStyle::Plain,
            axis_labels: AxisLabels {
                easting: "E".into(),
                northing: "N".into(),
                height: "Z".into(),
            },
        }
    }

    /// Australian practice: whole-circle DMS, E/N/RL, metres, plain chainage.
    pub fn australian() -> Self {
        Self {
            name: "Australian".into(),
            axis_labels: AxisLabels {
                easting: "E".into(),
                northing: "N".into(),
                height: "RL".into(),
            },
            ..Self::metric_generic()
        }
    }

    /// North American practice: quadrant bearings, N/E/Elev, US survey feet, 1+00 stations.
    pub fn north_american() -> Self {
        Self {
            name: "North American".into(),
            angle_format: AngleFormat::DmsSymbols,
            angle_decimals: 0,
            bearing_style: BearingStyle::Quadrant,
            coord_order: CoordOrder::NorthingEasting,
            linear_unit: LinearUnit::UsSurveyFoot,
            linear_decimals: 2,
            chainage_style: ChainageStyle::Station { interval: 100 },
            axis_labels: AxisLabels {
                easting: "E".into(),
                northing: "N".into(),
                height: "Elev".into(),
            },
        }
    }

    pub fn presets() -> Vec<Conventions> {
        vec![Self::metric_generic(), Self::australian(), Self::north_american()]
    }

    // ---------------------------------------------------------------- formatting

    pub fn format_angle(&self, angle: Angle) -> String {
        let decimals = usize::from(self.angle_decimals);
        match self.angle_format {
            AngleFormat::DmsSymbols => {
                let d = angle.to_dms(self.angle_decimals);
                let sign = if d.negative { "-" } else { "" };
                let sec_width = if decimals > 0 { decimals + 3 } else { 2 };
                format!(
                    "{sign}{}°{:02}'{:0sec_width$.decimals$}\"",
                    d.degrees, d.minutes, d.seconds
                )
            }
            AngleFormat::DmsPacked => {
                let d = angle.to_dms(self.angle_decimals);
                let sign = if d.negative { "-" } else { "" };
                // Seconds digits follow minutes directly: 123.4530 or 123.45305 (1 decimal).
                let sec_scaled = (d.seconds * 10f64.powi(i32::from(self.angle_decimals))).round();
                let sec_digits = 2 + decimals;
                format!(
                    "{sign}{}.{:02}{:0sec_digits$}",
                    d.degrees, d.minutes, sec_scaled as u64
                )
            }
            AngleFormat::DecimalDegrees => format!("{:.decimals$}°", angle.degrees()),
            AngleFormat::Gons => format!("{:.decimals$}g", angle.gons()),
        }
    }

    pub fn format_bearing(&self, bearing: Bearing) -> String {
        match self.bearing_style {
            BearingStyle::WholeCircle => self.format_angle(bearing.angle()),
            BearingStyle::Quadrant => {
                let (q, a) = bearing.to_quadrant();
                format!("{} {} {}", q.prefix(), self.format_angle(a), q.suffix())
            }
        }
    }

    /// Length in display units without a unit suffix.
    pub fn format_length(&self, metres: f64) -> String {
        let v = metres / self.linear_unit.metres_per_unit();
        format!("{:.*}", usize::from(self.linear_decimals), v)
    }

    /// Length in display units with the unit suffix, e.g. `12.345 m`.
    pub fn format_length_with_unit(&self, metres: f64) -> String {
        format!("{} {}", self.format_length(metres), self.linear_unit.suffix())
    }

    /// Area in square display units plus hectares (metres) or acres (feet).
    pub fn format_area(&self, square_metres: f64) -> String {
        let unit = self.linear_unit.metres_per_unit();
        let v = square_metres / (unit * unit);
        let decimals = usize::from(self.linear_decimals);
        let large = match self.linear_unit {
            LinearUnit::Metre => format!("{:.4} ha", v / 10_000.0),
            LinearUnit::InternationalFoot | LinearUnit::UsSurveyFoot => format!("{:.4} ac", v / 43_560.0),
        };
        format!("{v:.decimals$} {}² ({large})", self.linear_unit.suffix())
    }

    pub fn format_chainage(&self, metres: f64) -> String {
        let decimals = usize::from(self.linear_decimals);
        let v = metres / self.linear_unit.metres_per_unit();
        match self.chainage_style {
            ChainageStyle::Plain => format!("{v:.decimals$}"),
            ChainageStyle::Station { interval } => {
                let interval = f64::from(interval.max(1));
                let sign = if v < 0.0 { "-" } else { "" };
                let scale = 10f64.powi(i32::from(self.linear_decimals));
                let rounded = (v.abs() * scale).round() / scale;
                let stations = (rounded / interval).floor();
                let rem = rounded - stations * interval;
                let int_digits = interval.log10().ceil() as usize;
                let width = if decimals > 0 {
                    int_digits + 1 + decimals
                } else {
                    int_digits
                };
                format!("{sign}{stations}+{rem:0width$.decimals$}")
            }
        }
    }

    /// The two horizontal labels in display order.
    pub fn horizontal_labels(&self) -> (&str, &str) {
        match self.coord_order {
            CoordOrder::EastingNorthing => (&self.axis_labels.easting, &self.axis_labels.northing),
            CoordOrder::NorthingEasting => (&self.axis_labels.northing, &self.axis_labels.easting),
        }
    }

    /// `E 512345.678  N 6912345.678  RL 12.345` (order and labels per convention).
    pub fn format_coord(&self, pos: DVec3, height_known: bool) -> String {
        let (a, b) = self.horizontal_values(pos);
        let (la, lb) = self.horizontal_labels();
        let mut s = format!("{la} {}  {lb} {}", self.format_length(a), self.format_length(b));
        if height_known {
            s.push_str(&format!(
                "  {} {}",
                self.axis_labels.height,
                self.format_length(pos.z)
            ));
        }
        s
    }

    /// The horizontal components of `pos` in display order (metres).
    pub fn horizontal_values(&self, pos: DVec3) -> (f64, f64) {
        match self.coord_order {
            CoordOrder::EastingNorthing => (pos.x, pos.y),
            CoordOrder::NorthingEasting => (pos.y, pos.x),
        }
    }

    // ---------------------------------------------------------------- parsing

    /// Parses an angle. Inputs containing separators (`°`, `'`, `"`, space, `-`
    /// between numbers, `d`/`m`/`s`) are always read as degrees–minutes–seconds;
    /// plain numbers are read according to [`AngleFormat`].
    pub fn parse_angle(&self, text: &str) -> Result<Angle, ParseError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ParseError::Empty);
        }
        let (negative, body) = match text.strip_prefix('-') {
            Some(rest) => (true, rest.trim_start()),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        let has_separators = body.chars().any(|c| {
            matches!(
                c,
                '°' | '\'' | '"' | ' ' | '-' | 'd' | 'D' | 'm' | 'M' | 's' | 'S' | '′' | '″'
            )
        });
        let magnitude = if has_separators {
            parse_dms_separated(body)?
        } else {
            match self.angle_format {
                AngleFormat::DmsSymbols | AngleFormat::DmsPacked => parse_packed_dms(body)?,
                AngleFormat::DecimalDegrees => Angle::from_degrees(parse_number(body)?),
                AngleFormat::Gons => Angle::from_gons(parse_number(body.trim_end_matches(['g', 'G']))?),
            }
        };
        Ok(if negative { -magnitude } else { magnitude })
    }

    /// Parses a bearing: either a whole-circle angle or a quadrant bearing
    /// such as `N45.3000E`, `S 12°30' W` (accepted regardless of [`BearingStyle`]).
    pub fn parse_bearing(&self, text: &str) -> Result<Bearing, ParseError> {
        let t = text.trim();
        if t.is_empty() {
            return Err(ParseError::Empty);
        }
        let first = t.chars().next().map(|c| c.to_ascii_uppercase());
        let last = t.chars().last().map(|c| c.to_ascii_uppercase());
        if let (Some(p @ ('N' | 'S')), Some(s @ ('E' | 'W'))) = (first, last) {
            let inner = &t[1..t.len() - 1];
            let angle = self.parse_angle(inner)?;
            if angle.radians() < 0.0 || angle.degrees() > 90.0 {
                return Err(ParseError::InvalidQuadrant(t.to_string()));
            }
            let quadrant = match (p, s) {
                ('N', 'E') => Quadrant::NE,
                ('S', 'E') => Quadrant::SE,
                ('S', 'W') => Quadrant::SW,
                _ => Quadrant::NW,
            };
            return Ok(Bearing::from_quadrant(quadrant, angle));
        }
        Ok(Bearing::new(self.parse_angle(t)?))
    }

    /// Parses a length in display units (an explicit `m`/`ft`/`sft` suffix overrides) into metres.
    pub fn parse_length(&self, text: &str) -> Result<f64, ParseError> {
        let t = text.trim();
        if t.is_empty() {
            return Err(ParseError::Empty);
        }
        for unit in [
            LinearUnit::UsSurveyFoot,
            LinearUnit::InternationalFoot,
            LinearUnit::Metre,
        ] {
            if let Some(num) = t.strip_suffix(unit.suffix()) {
                return Ok(parse_number(num.trim())? * unit.metres_per_unit());
            }
        }
        Ok(parse_number(t)? * self.linear_unit.metres_per_unit())
    }

    /// Parses `a,b[,h]` (or whitespace-separated) in display order and units.
    pub fn parse_coord(&self, text: &str) -> Result<ParsedCoord, ParseError> {
        let parts = split_values(text);
        if !(2..=3).contains(&parts.len()) {
            return Err(ParseError::WrongValueCount {
                expected: "2 or 3",
                got: parts.len(),
            });
        }
        let a = self.parse_length(parts[0])?;
        let b = self.parse_length(parts[1])?;
        let height = parts.get(2).map(|h| self.parse_length(h)).transpose()?;
        let (easting, northing) = match self.coord_order {
            CoordOrder::EastingNorthing => (a, b),
            CoordOrder::NorthingEasting => (b, a),
        };
        Ok(ParsedCoord {
            easting,
            northing,
            height,
        })
    }
}

/// Splits on commas, or on whitespace when there are no commas.
pub(crate) fn split_values(text: &str) -> Vec<&str> {
    let t = text.trim();
    if t.is_empty() {
        return Vec::new();
    }
    if t.contains(',') {
        t.split(',').map(str::trim).collect()
    } else {
        t.split_whitespace().collect()
    }
}

pub(crate) fn parse_number(text: &str) -> Result<f64, ParseError> {
    let t = text.trim();
    if t.is_empty() {
        return Err(ParseError::Empty);
    }
    t.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| ParseError::NotANumber(t.to_string()))
}

/// Decodes packed `DDD.MMSSsss` using the digit string, avoiding float rounding:
/// `123.4` is 123°40', `123.45` is 123°45', `123.4530` is 123°45'30".
fn parse_packed_dms(text: &str) -> Result<Angle, ParseError> {
    let (deg_str, frac) = text.split_once('.').unwrap_or((text, ""));
    if !frac.chars().all(|c| c.is_ascii_digit()) {
        return Err(ParseError::NotANumber(text.to_string()));
    }
    let degrees = if deg_str.is_empty() {
        0.0
    } else {
        parse_number(deg_str)?
    };
    let mut digits = frac.to_string();
    while digits.len() < 4 {
        digits.push('0');
    }
    let minutes: f64 = parse_number(&digits[0..2])?;
    let seconds: f64 = parse_number(&format!("{}.{}", &digits[2..4], &digits[4..]))?;
    check_ms(minutes, seconds)?;
    Ok(Angle::from_dms(false, degrees, minutes, seconds))
}

/// Parses `123°45'30"`, `123 45 30`, `123-45-30`, `123d45m30s`.
fn parse_dms_separated(text: &str) -> Result<Angle, ParseError> {
    let parts: Vec<&str> = text
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() || parts.len() > 3 {
        return Err(ParseError::WrongValueCount {
            expected: "1 to 3 (degrees, minutes, seconds)",
            got: parts.len(),
        });
    }
    let degrees = parse_number(parts[0])?;
    let minutes = parts.get(1).map(|m| parse_number(m)).transpose()?.unwrap_or(0.0);
    let seconds = parts.get(2).map(|s| parse_number(s)).transpose()?.unwrap_or(0.0);
    check_ms(minutes, seconds)?;
    Ok(Angle::from_dms(false, degrees, minutes, seconds))
}

fn check_ms(minutes: f64, seconds: f64) -> Result<(), ParseError> {
    if minutes >= 60.0 {
        return Err(ParseError::MinutesOutOfRange(minutes.to_string()));
    }
    if seconds >= 60.0 {
        return Err(ParseError::SecondsOutOfRange(seconds.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deg(c: &Conventions, s: &str) -> f64 {
        c.parse_angle(s).expect(s).degrees()
    }

    #[test]
    fn parse_packed_dms_variants() {
        let c = Conventions::metric_generic();
        let expect = 123.0 + 45.0 / 60.0 + 30.0 / 3600.0;
        assert!((deg(&c, "123.4530") - expect).abs() < 1e-12);
        assert!((deg(&c, "123.4") - (123.0 + 40.0 / 60.0)).abs() < 1e-12);
        assert!((deg(&c, "123") - 123.0).abs() < 1e-12);
        assert!((deg(&c, "0.0030") - 30.0 / 3600.0).abs() < 1e-12);
        let with_dec = 10.0 + 5.0 / 60.0 + 7.25 / 3600.0;
        assert!((deg(&c, "10.050725") - with_dec).abs() < 1e-12);
        assert!(c.parse_angle("10.6000").is_err());
        assert!(c.parse_angle("10.0560").is_err());
    }

    #[test]
    fn parse_separated_dms() {
        let c = Conventions::metric_generic();
        let expect = 123.0 + 45.0 / 60.0 + 30.0 / 3600.0;
        for s in ["123°45'30\"", "123 45 30", "123-45-30", "123d45m30s"] {
            assert!((deg(&c, s) - expect).abs() < 1e-12, "{s}");
        }
        assert!((deg(&c, "-0 30") + 0.5).abs() < 1e-12);
    }

    #[test]
    fn plain_numbers_follow_format() {
        let mut c = Conventions::metric_generic();
        c.angle_format = AngleFormat::DecimalDegrees;
        assert!((deg(&c, "123.4530") - 123.4530).abs() < 1e-12);
        c.angle_format = AngleFormat::Gons;
        assert!((deg(&c, "100") - 90.0).abs() < 1e-12);
    }

    #[test]
    fn format_angles() {
        let mut c = Conventions::metric_generic();
        let a = Angle::from_dms(false, 123.0, 4.0, 5.5);
        assert_eq!(c.format_angle(a), "123°04'06\"");
        c.angle_decimals = 1;
        assert_eq!(c.format_angle(a), "123°04'05.5\"");
        c.angle_format = AngleFormat::DmsPacked;
        assert_eq!(c.format_angle(a), "123.04055");
        c.angle_decimals = 0;
        assert_eq!(c.format_angle(a), "123.0406");
        // Packed output re-parses to the same value.
        let parsed = c.parse_angle(&c.format_angle(Angle::from_dms(false, 7.0, 8.0, 9.0)));
        assert!((parsed.expect("parse").degrees() - (7.0 + 8.0 / 60.0 + 9.0 / 3600.0)).abs() < 1e-12);
    }

    #[test]
    fn quadrant_bearings() {
        let c = Conventions::north_american();
        let b = c.parse_bearing("S45.3000E").expect("parse");
        assert!((b.degrees() - (180.0 - 45.5)).abs() < 1e-9);
        assert_eq!(c.format_bearing(b), "S 45°30'00\" E");
        let wc = c.parse_bearing("225").expect("parse");
        assert_eq!(c.format_bearing(wc), "S 45°00'00\" W");
        assert!(c.parse_bearing("N95E").is_err());
    }

    #[test]
    fn lengths_and_units() {
        let mut c = Conventions::metric_generic();
        assert_eq!(c.format_length(12.3456), "12.346");
        c.linear_unit = LinearUnit::InternationalFoot;
        assert!((c.parse_length("10").expect("ok") - 3.048).abs() < 1e-12);
        assert!((c.parse_length("10 m").expect("ok") - 10.0).abs() < 1e-12);
        assert!((c.parse_length("10sft").expect("ok") - 10.0 * 1200.0 / 3937.0).abs() < 1e-12);
    }

    #[test]
    fn chainage_formats() {
        let mut c = Conventions::metric_generic();
        assert_eq!(c.format_chainage(1234.5), "1234.500");
        c.chainage_style = ChainageStyle::Station { interval: 1000 };
        assert_eq!(c.format_chainage(1234.5), "1+234.500");
        c.chainage_style = ChainageStyle::Station { interval: 100 };
        c.linear_decimals = 2;
        assert_eq!(c.format_chainage(1234.5), "12+34.50");
        assert_eq!(c.format_chainage(99.999), "1+00.00");
    }

    #[test]
    fn coordinates_respect_order() {
        let mut c = Conventions::australian();
        let p = c.parse_coord("500000, 7000000, 12.5").expect("ok");
        assert_eq!(
            (p.easting, p.northing, p.height),
            (500000.0, 7000000.0, Some(12.5))
        );
        c.coord_order = CoordOrder::NorthingEasting;
        let p = c.parse_coord("7000000 500000").expect("ok");
        assert_eq!((p.easting, p.northing, p.height), (500000.0, 7000000.0, None));
        let s = c.format_coord(DVec3::new(1.0, 2.0, 3.0), true);
        assert_eq!(s, "N 2.000  E 1.000  RL 3.000");
    }
}
