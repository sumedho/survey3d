//! Circular curve solutions for the command line, the calculator and the MCP
//! server: any two of a curve's elements give the rest.

use std::fmt::Write as _;

use survey_core::solvers::{Curve, CurveInput};
use survey_core::{Angle, Conventions};

/// Element keys as typed (`r=20 a=15.71`): radius, delta, arc length, chord,
/// tangent, external and mid-ordinate.
pub const KEYS: [(&str, &str); 7] = [
    ("r", "radius"),
    ("d", "delta"),
    ("a", "arc length"),
    ("c", "chord"),
    ("t", "tangent"),
    ("e", "external"),
    ("m", "mid-ordinate"),
];

/// Reads `key=value` elements (lengths in display units, delta as an angle).
pub fn parse_curve(conv: &Conventions, text: &str) -> Result<CurveInput, String> {
    let mut input = CurveInput::default();
    for part in text.split([' ', ',']).filter(|p| !p.is_empty()) {
        let Some((key, value)) = part.split_once('=') else {
            return Err(format!("'{part}' is not key=value: use {}", key_help()));
        };
        let key = key.trim().to_ascii_lowercase();
        let length = || conv.parse_length(value).map_err(|e| format!("{key}=: {e}"));
        let slot = match key.as_str() {
            "r" => &mut input.radius,
            "a" | "l" => &mut input.length,
            "c" => &mut input.chord,
            "t" => &mut input.tangent,
            "e" => &mut input.external,
            "m" => &mut input.mid_ordinate,
            "d" => {
                let a = conv.parse_angle(value).map_err(|e| format!("d=: {e}"))?;
                input.delta = Some(a.radians().abs());
                continue;
            }
            _ => return Err(format!("'{key}' is not a curve element: use {}", key_help())),
        };
        *slot = Some(length()?);
    }
    Ok(input)
}

fn key_help() -> String {
    KEYS.iter()
        .map(|(k, name)| format!("{k}= {name}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The curve's elements, one per line.
pub fn curve_text(conv: &Conventions, c: &Curve) -> String {
    let len = |v: f64| conv.format_length_with_unit(v);
    let mut s = String::new();
    let _ = writeln!(s, "Radius        {}", len(c.radius));
    let _ = writeln!(
        s,
        "Delta         {}",
        conv.format_angle(Angle::from_radians(c.delta))
    );
    let _ = writeln!(s, "Arc length    {}", len(c.length));
    let _ = writeln!(s, "Chord         {}", len(c.chord));
    if c.delta < std::f64::consts::PI {
        let _ = writeln!(s, "Tangent       {}", len(c.tangent));
        let _ = writeln!(s, "External      {}", len(c.external));
    }
    let _ = writeln!(s, "Mid-ordinate  {}", len(c.mid_ordinate));
    let _ = write!(s, "Segment area  {}", conv.format_area(segment_area(c)));
    s
}

/// Area between the chord and the arc.
pub fn segment_area(c: &Curve) -> f64 {
    0.5 * c.radius * c.radius * (c.delta - c.delta.sin())
}

#[cfg(test)]
mod tests {
    use super::*;
    use survey_core::solvers::solve_curve;

    #[test]
    fn typed_elements_solve() {
        let conv = Conventions::default();
        let input = parse_curve(&conv, "r=20 d=90").expect("parse");
        let c = solve_curve(input).expect("solve")[0];
        assert!((c.length - 10.0 * std::f64::consts::PI).abs() < 1e-9);
        let text = curve_text(&conv, &c);
        assert!(
            text.contains("Arc length    31.416") && text.contains("Tangent       20.000"),
            "{text}"
        );
        assert!(parse_curve(&conv, "r=20 q=3").is_err());
        assert!(parse_curve(&conv, "20").is_err());
    }
}
