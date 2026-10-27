//! Curve solutions for the command line, the calculator and the MCP server:
//! any two of a circular curve's elements give the rest; a curve with
//! spirals each side, compound curves and reverse curves likewise.

use std::fmt::Write as _;

use survey_core::curve_sets::{
    CompoundCurve, CompoundInput, ReverseCurve, ReverseInput, SpiralCurve, SpiralCurveInput, solve_compound,
    solve_reverse, solve_spiral_curve,
};
use survey_core::solvers::{Curve, CurveInput, solve_curve};
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

/// A curve problem typed on the command line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CurveRequest {
    Simple(CurveInput),
    Spiral(SpiralCurveInput),
    Compound(CompoundInput),
    Reverse(ReverseInput),
}

/// Help for the forms [`parse_request`] reads.
pub const REQUEST_HELP: &str = "curve: two of r= d= a= c= t= e= m=; with spirals: r= ls= and one of d= t= lt= (total length); \
     compound r1= r2= and two of d1= d2= d= t1= t2=; reverse (parallel tangents): r= (or r1= r2=) and d=, p= (offset) or x= (along), or p= x=";

/// Reads any curve problem: `r=… d=…` (simple), with `ls=` (spirals each
/// side), `compound r1=… r2=…` and `reverse r=… p=…`. The words are optional
/// where the keys say which it is (r1/r2/d1/d2/t1/t2 compound, p/x reverse).
pub fn parse_request(conv: &Conventions, text: &str) -> Result<CurveRequest, String> {
    let mut words = text.split([' ', ',']).filter(|p| !p.is_empty()).peekable();
    let kind = match words.peek().map(|w| w.to_ascii_lowercase()) {
        Some(w) if ["compound", "cc"].contains(&w.as_str()) => {
            words.next();
            Some("compound")
        }
        Some(w) if ["reverse", "rc"].contains(&w.as_str()) => {
            words.next();
            Some("reverse")
        }
        Some(w) if ["spiral", "sc"].contains(&w.as_str()) => {
            words.next();
            Some("spiral")
        }
        _ => None,
    };
    let mut pairs = Vec::new();
    for part in words {
        let Some((key, value)) = part.split_once('=') else {
            return Err(format!("'{part}' is not key=value: {REQUEST_HELP}"));
        };
        pairs.push((key.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    let has = |keys: &[&str]| pairs.iter().any(|(k, _)| keys.contains(&k.as_str()));
    let kind = kind.unwrap_or(if has(&["p", "x"]) {
        "reverse"
    } else if has(&["r1", "r2", "d1", "d2", "t1", "t2"]) {
        "compound"
    } else if has(&["ls", "lt"]) {
        "spiral"
    } else {
        "simple"
    });
    let length = |key: &str, v: &str| conv.parse_length(v).map_err(|e| format!("{key}=: {e}"));
    let angle = |key: &str, v: &str| {
        conv.parse_angle(v)
            .map(|a| a.radians().abs())
            .map_err(|e| format!("{key}=: {e}"))
    };
    let unknown = |key: &str| format!("'{key}' is not used for a {kind} curve: {REQUEST_HELP}");
    match kind {
        "simple" => parse_curve(conv, text).map(CurveRequest::Simple),
        "spiral" => {
            let mut i = SpiralCurveInput::default();
            for (k, v) in &pairs {
                match k.as_str() {
                    "r" => i.radius = Some(length(k, v)?),
                    "ls" | "s" => i.spiral_length = Some(length(k, v)?),
                    "d" => i.delta = Some(angle(k, v)?),
                    "t" => i.tangent = Some(length(k, v)?),
                    "lt" | "l" => i.total_length = Some(length(k, v)?),
                    _ => return Err(unknown(k)),
                }
            }
            Ok(CurveRequest::Spiral(i))
        }
        "compound" => {
            let mut i = CompoundInput::default();
            for (k, v) in &pairs {
                match k.as_str() {
                    "r1" => i.r1 = Some(length(k, v)?),
                    "r2" => i.r2 = Some(length(k, v)?),
                    "d1" => i.delta1 = Some(angle(k, v)?),
                    "d2" => i.delta2 = Some(angle(k, v)?),
                    "d" => i.delta = Some(angle(k, v)?),
                    "t1" => i.t1 = Some(length(k, v)?),
                    "t2" => i.t2 = Some(length(k, v)?),
                    _ => return Err(unknown(k)),
                }
            }
            Ok(CurveRequest::Compound(i))
        }
        _ => {
            let mut i = ReverseInput::default();
            for (k, v) in &pairs {
                match k.as_str() {
                    "r" => i.radius = Some(length(k, v)?),
                    "r1" => i.r1 = Some(length(k, v)?),
                    "r2" => i.r2 = Some(length(k, v)?),
                    "d" => i.delta = Some(angle(k, v)?),
                    "p" => i.offset = Some(length(k, v)?),
                    "x" => i.along = Some(length(k, v)?),
                    _ => return Err(unknown(k)),
                }
            }
            Ok(CurveRequest::Reverse(i))
        }
    }
}

/// The solutions of a curve problem as reports.
pub fn solve_request(conv: &Conventions, req: CurveRequest) -> Result<Vec<String>, String> {
    let err = |e: survey_core::solvers::SolveError| e.to_string();
    Ok(match req {
        CurveRequest::Simple(i) => solve_curve(i)
            .map_err(err)?
            .iter()
            .map(|c| curve_text(conv, c))
            .collect(),
        CurveRequest::Spiral(i) => vec![spiral_curve_text(conv, &solve_spiral_curve(i).map_err(err)?)],
        CurveRequest::Compound(i) => solve_compound(i)
            .map_err(err)?
            .iter()
            .map(|c| compound_text(conv, c))
            .collect(),
        CurveRequest::Reverse(i) => vec![reverse_text(conv, &solve_reverse(i).map_err(err)?)],
    })
}

fn angle_text(conv: &Conventions, radians: f64) -> String {
    conv.format_angle(Angle::from_radians(radians))
}

/// A spiral–curve–spiral's elements.
pub fn spiral_curve_text(conv: &Conventions, c: &SpiralCurve) -> String {
    let len = |v: f64| conv.format_length_with_unit(v);
    let s = &c.spiral;
    let mut t = String::new();
    let _ = writeln!(t, "Radius             {}", len(c.radius));
    let _ = writeln!(t, "Delta (total)      {}", angle_text(conv, c.delta));
    let _ = writeln!(
        t,
        "Spiral length Ls   {}  (A {})",
        len(s.length),
        conv.format_length(s.a)
    );
    let _ = writeln!(t, "Spiral angle θs    {}", angle_text(conv, s.theta));
    let _ = writeln!(t, "Arc Δc             {}", angle_text(conv, c.delta_c));
    let _ = writeln!(t, "Arc length         {}", len(c.arc_length));
    let _ = writeln!(t, "Total length       {}", len(c.total_length));
    let _ = writeln!(t, "Tangent Ts (PI–TS) {}", len(c.tangent));
    let _ = writeln!(t, "External Es        {}", len(c.external));
    let _ = writeln!(t, "Shift p            {}", len(s.p));
    let _ = writeln!(t, "k                  {}", len(s.k));
    let _ = writeln!(
        t,
        "SC from TS  x {}  y {}",
        conv.format_length(s.x),
        conv.format_length(s.y)
    );
    let _ = writeln!(
        t,
        "Long / short tangent {} / {}",
        conv.format_length(s.long_tangent),
        conv.format_length(s.short_tangent)
    );
    let _ = write!(
        t,
        "Long chord TS–SC   {}  deflection {}",
        len(s.long_chord),
        angle_text(conv, s.deflection)
    );
    t
}

pub fn compound_text(conv: &Conventions, c: &CompoundCurve) -> String {
    let len = |v: f64| conv.format_length_with_unit(v);
    let mut t = String::new();
    let _ = writeln!(
        t,
        "R1 {}  Δ1 {}  arc {}",
        len(c.r1),
        angle_text(conv, c.delta1),
        len(c.length1)
    );
    let _ = writeln!(
        t,
        "R2 {}  Δ2 {}  arc {}",
        len(c.r2),
        angle_text(conv, c.delta2),
        len(c.length2)
    );
    let _ = writeln!(t, "Delta (total)      {}", angle_text(conv, c.delta));
    let _ = writeln!(t, "T1 (PC–PI)         {}", len(c.t1));
    let _ = writeln!(t, "T2 (PI–PT)         {}", len(c.t2));
    let _ = writeln!(
        t,
        "Arc tangents       {} / {}",
        conv.format_length(c.tangent1),
        conv.format_length(c.tangent2)
    );
    let _ = writeln!(t, "Common tangent     {}", len(c.common_tangent));
    let _ = write!(t, "Long chord PC–PT   {}", len(c.long_chord));
    t
}

pub fn reverse_text(conv: &Conventions, c: &ReverseCurve) -> String {
    let len = |v: f64| conv.format_length_with_unit(v);
    let mut t = String::new();
    let _ = writeln!(t, "R1 {}  arc {}", len(c.r1), len(c.length1));
    let _ = writeln!(t, "R2 {}  arc {}", len(c.r2), len(c.length2));
    let _ = writeln!(t, "Delta (each)       {}", angle_text(conv, c.delta));
    let _ = writeln!(t, "Offset (tangents)  {}", len(c.offset));
    let _ = writeln!(t, "Along the tangents {}", len(c.along));
    let _ = write!(t, "Chord PC–PT        {}", len(c.chord));
    t
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

    #[test]
    fn spiral_compound_and_reverse_requests() {
        let conv = Conventions::default();
        let solve = |t: &str| solve_request(&conv, parse_request(&conv, t).expect(t)).expect(t);
        let text = solve("r=300 ls=60 d=40");
        assert!(text[0].contains("Spiral angle θs    5°43'46\""), "{}", text[0]);
        let text = solve("compound r1=200 r2=100 d1=30 d2=20");
        assert!(text[0].contains("Delta (total)      50°00'00\""), "{}", text[0]);
        // Keys alone say which: p= is a reverse curve.
        let text = solve("r=100 p=10");
        assert!(text[0].contains("Offset (tangents)  10.000"), "{}", text[0]);
        assert_eq!(
            parse_request(&conv, "reverse r=100 d=20").expect("reverse"),
            CurveRequest::Reverse(ReverseInput {
                radius: Some(100.0),
                delta: Some(20f64.to_radians()),
                ..Default::default()
            })
        );
        assert!(parse_request(&conv, "compound r1=200 ls=30").is_err());
        assert!(matches!(
            parse_request(&conv, "r=20 d=90"),
            Ok(CurveRequest::Simple(_))
        ));
    }
}
