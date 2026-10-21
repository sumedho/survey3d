//! Field-to-finish: turning coded survey points into linework.
//!
//! A [`FeatureLibrary`] lists the feature codes used in the field (with the layer
//! and colour their objects go on, and whether they form lines) and the
//! *control codes* that shape strings. A point's code is a feature code,
//! optionally followed directly by a string number (`KB1`, `KB2` for two kerb
//! lines picked up alternately), then any control codes separated by spaces:
//!
//! | control (defaults) | meaning |
//! |---|---|
//! | `ST` | start a new string at this point |
//! | `END` | end the string at this point |
//! | `CL` | close the string back to its first point |
//! | `PC` / `PT` | points from `PC` to `PT` form a circular arc |
//!
//! Points are processed in the order given (normally survey order). Other
//! tokens are ignored, so field notes can follow the codes. The control codes
//! are configurable; no particular field-coding standard is assumed.

use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::arc::Arc;
use crate::style::{CodeStyle, Symbol};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeatureCode {
    /// The code, or a pattern with `*` (any run of characters) and `?` (one
    /// character), e.g. `MH*`, matched when no plain code fits.
    pub code: String,
    pub description: String,
    /// Layer for the points and lines of this feature.
    pub layer: String,
    /// sRGB colour for the layer when it has to be created.
    pub color: [u8; 3],
    /// Whether points with this code are joined into strings.
    pub linework: bool,
    /// How the feature's points and lines are drawn.
    #[serde(default)]
    pub style: CodeStyle,
}

impl FeatureCode {
    pub fn new(code: &str, description: &str, layer: &str, color: [u8; 3], linework: bool) -> Self {
        Self {
            code: code.into(),
            description: description.into(),
            layer: layer.into(),
            color,
            linework,
            style: CodeStyle::default(),
        }
    }

    /// Whether the code is a wildcard pattern.
    pub fn is_pattern(&self) -> bool {
        self.code.contains(['*', '?'])
    }
}

/// Case-insensitive glob match with `*` and `?`.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let t: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ControlCodes {
    pub start: String,
    pub end: String,
    pub close: String,
    pub arc_start: String,
    pub arc_end: String,
}

impl Default for ControlCodes {
    fn default() -> Self {
        Self {
            start: "ST".into(),
            end: "END".into(),
            close: "CL".into(),
            arc_start: "PC".into(),
            arc_end: "PT".into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FeatureLibrary {
    pub codes: Vec<FeatureCode>,
    pub controls: ControlCodes,
}

/// A point's code split into its parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedCode {
    /// Index into [`FeatureLibrary::codes`].
    pub feature: usize,
    /// The string number following the code (`"1"` in `KB1`), or empty.
    pub string: String,
    pub start: bool,
    pub end: bool,
    pub close: bool,
    pub arc_start: bool,
    pub arc_end: bool,
}

/// A line built from coded points.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratedString {
    /// Index into [`FeatureLibrary::codes`].
    pub feature: usize,
    /// `CODE[string].n`, e.g. `KB1.2` for the second `KB1` string.
    pub name: String,
    pub vertices: Vec<DVec3>,
    /// Bulge per segment (see [`crate::arc`]); empty when all straight.
    pub bulges: Vec<f64>,
    pub closed: bool,
}

/// Whether `name` is one this module generates for `code` (`CODE…​.n`), so
/// earlier linework can be replaced when processing again.
pub fn is_generated_name(name: &str, code: &str) -> bool {
    name.len() > code.len()
        && name[..code.len()].eq_ignore_ascii_case(code)
        && name.rsplit_once('.').is_some_and(|(head, n)| {
            head.len() >= code.len() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
        })
}

impl FeatureLibrary {
    /// A small, generic starting set (edit freely).
    pub fn example() -> Self {
        let code =
            |code: &str, description: &str, layer: &str, color: [u8; 3], linework: bool, symbol: Symbol| {
                let mut f = FeatureCode::new(code, description, layer, color, linework);
                f.style.symbol = symbol;
                f
            };
        let mut fence = code("FNC", "Fence", "FENCES", [251, 191, 36], true, Symbol::Post);
        fence.style.line_style = crate::style::LineStyle::DashDot;
        let mut top = code("TB", "Top of bank", "BANKS", [74, 222, 128], true, Symbol::Dot);
        top.style.line_style = crate::style::LineStyle::Dashed;
        Self {
            codes: vec![
                code(
                    "CP",
                    "Control point",
                    "CONTROL",
                    [244, 114, 182],
                    false,
                    Symbol::CircleCross,
                ),
                code(
                    "BLD",
                    "Building",
                    "BUILDINGS",
                    [167, 139, 250],
                    true,
                    Symbol::Square,
                ),
                fence,
                code("KB", "Kerb", "KERBS", [125, 211, 252], true, Symbol::Dot),
                code(
                    "EP",
                    "Edge of pavement",
                    "ROADS",
                    [148, 163, 184],
                    true,
                    Symbol::Dot,
                ),
                top,
                code("BB", "Bottom of bank", "BANKS", [45, 212, 191], true, Symbol::Dot),
                code("TR", "Tree", "VEGETATION", [34, 197, 94], false, Symbol::Tree),
                code(
                    "MH*",
                    "Manhole",
                    "SERVICES",
                    [248, 113, 113],
                    false,
                    Symbol::CircleX,
                ),
                code(
                    "NS",
                    "Natural surface",
                    "SURFACE",
                    [203, 213, 225],
                    false,
                    Symbol::Cross,
                ),
            ],
            controls: ControlCodes::default(),
        }
    }

    /// The feature whose code `token` starts with, followed only by digits
    /// (the longest such code wins, so `KBX` and `KB` can coexist).
    fn match_feature(&self, token: &str) -> Option<(usize, String)> {
        self.codes
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                !f.code.is_empty()
                    && !f.is_pattern()
                    && token.len() >= f.code.len()
                    && token[..f.code.len()].eq_ignore_ascii_case(&f.code)
                    && token[f.code.len()..].bytes().all(|b| b.is_ascii_digit())
            })
            .max_by_key(|(_, f)| f.code.len())
            .map(|(i, f)| (i, token[f.code.len()..].to_string()))
            .or_else(|| self.match_pattern(token).map(|i| (i, String::new())))
    }

    /// The most specific wildcard code matching `token` (most literal characters).
    fn match_pattern(&self, token: &str) -> Option<usize> {
        self.codes
            .iter()
            .enumerate()
            .filter(|(_, f)| f.is_pattern() && glob_match(&f.code, token))
            .max_by_key(|(_, f)| f.code.chars().filter(|c| !matches!(c, '*' | '?')).count())
            .map(|(i, _)| i)
    }

    /// The feature a point or line code belongs to (by its first token), for styling.
    pub fn feature_for(&self, raw: &str) -> Option<&FeatureCode> {
        let first = raw.split_whitespace().next()?;
        self.match_feature(first).map(|(i, _)| &self.codes[i])
    }

    pub fn find(&self, code: &str) -> Option<&FeatureCode> {
        self.codes.iter().find(|f| f.code.eq_ignore_ascii_case(code))
    }

    /// Parses a point code; `None` if its first token is not a library code.
    pub fn parse(&self, raw: &str) -> Option<ParsedCode> {
        let mut tokens = raw.split_whitespace();
        let first = tokens.next()?;
        let (feature, string) = self.match_feature(first)?;
        let mut parsed = ParsedCode {
            feature,
            string,
            start: false,
            end: false,
            close: false,
            arc_start: false,
            arc_end: false,
        };
        let c = &self.controls;
        let is = |t: &str, control: &str| !control.is_empty() && t.eq_ignore_ascii_case(control);
        for t in tokens {
            parsed.start |= is(t, &c.start);
            parsed.end |= is(t, &c.end);
            parsed.close |= is(t, &c.close);
            parsed.arc_start |= is(t, &c.arc_start);
            parsed.arc_end |= is(t, &c.arc_end);
        }
        Some(parsed)
    }

    /// Builds strings from `(position, code)` pairs in survey order.
    pub fn build_linework<'a>(
        &self,
        points: impl IntoIterator<Item = (DVec3, &'a str)>,
    ) -> Vec<GeneratedString> {
        let mut open: Vec<Chain> = Vec::new();
        let mut done = Vec::new();
        let mut sequence: std::collections::HashMap<String, usize> = Default::default();
        for (p, raw) in points {
            let Some(code) = self.parse(raw) else {
                continue;
            };
            if !self.codes[code.feature].linework {
                continue;
            }
            let key = format!("{}{}", self.codes[code.feature].code, code.string);
            let pos = open.iter().position(|c| c.key == key);
            if code.start
                && let Some(i) = pos
            {
                done.extend(open.remove(i).finish(code.feature));
            }
            let i = match open.iter().position(|c| c.key == key) {
                Some(i) => i,
                None => {
                    let n = sequence.entry(key.clone()).or_default();
                    *n += 1;
                    open.push(Chain::new(key.clone(), code.feature, *n));
                    open.len() - 1
                }
            };
            let chain = &mut open[i];
            chain.push(p);
            if code.arc_start {
                chain.arc_from = Some(chain.vertices.len() - 1);
            }
            if code.arc_end {
                chain.close_arc();
            }
            if code.close {
                chain.closed = true;
            }
            if code.close || code.end {
                done.extend(open.remove(i).finish(code.feature));
            }
        }
        for chain in open {
            let feature = chain.feature;
            done.extend(chain.finish(feature));
        }
        done
    }
}

struct Chain {
    key: String,
    feature: usize,
    sequence: usize,
    vertices: Vec<DVec3>,
    bulges: Vec<f64>,
    /// Vertex index where an unfinished arc starts.
    arc_from: Option<usize>,
    closed: bool,
}

impl Chain {
    fn new(key: String, feature: usize, sequence: usize) -> Self {
        Self {
            key,
            feature,
            sequence,
            vertices: Vec::new(),
            bulges: Vec::new(),
            arc_from: None,
            closed: false,
        }
    }

    fn push(&mut self, p: DVec3) {
        if !self.vertices.is_empty() {
            self.bulges.push(0.0);
        }
        self.vertices.push(p);
    }

    /// Replaces the points since the arc start with one arc segment through
    /// the middle one (two points stay a straight segment).
    fn close_arc(&mut self) {
        let Some(from) = self.arc_from.take() else {
            return;
        };
        let last = self.vertices.len() - 1;
        if last < from + 2 {
            return;
        }
        let (a, m, b) = (
            self.vertices[from],
            self.vertices[from + (last - from) / 2],
            self.vertices[last],
        );
        if let Some(bulge) = Arc::bulge_through(a, m, b) {
            self.vertices.truncate(from + 1);
            self.bulges.truncate(from);
            self.vertices.push(b);
            self.bulges.push(bulge);
        }
    }

    fn finish(mut self, feature: usize) -> Option<GeneratedString> {
        self.close_arc();
        if self.vertices.len() < 2 {
            return None;
        }
        let has_arcs = self.bulges.iter().any(|b| *b != 0.0);
        let closed = self.closed && (self.vertices.len() >= 3 || has_arcs);
        let mut bulges = self.bulges;
        bulges.push(0.0);
        if !has_arcs {
            bulges.clear();
        }
        Some(GeneratedString {
            feature,
            name: format!("{}.{}", self.key, self.sequence),
            vertices: self.vertices,
            bulges,
            closed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> DVec3 {
        DVec3::new(x, y, 0.0)
    }

    #[test]
    fn parses_codes_strings_and_controls() {
        let lib = FeatureLibrary::example();
        let c = lib.parse("kb2 st note").expect("kerb");
        assert_eq!(lib.codes[c.feature].code, "KB");
        assert_eq!(c.string, "2");
        assert!(c.start && !c.end);
        assert!(lib.parse("XYZ").is_none());
        assert!(
            lib.parse("KBX").is_none(),
            "letters after a code are not a string number"
        );
        assert!(is_generated_name("KB2.13", "KB") && !is_generated_name("KB", "KB"));
        assert!(!is_generated_name("Boundary.1", "KB"));
    }

    #[test]
    fn interleaved_strings_close_and_restart() {
        let lib = FeatureLibrary::example();
        let pts = [
            (p(0.0, 0.0), "KB1"),
            (p(0.0, 5.0), "KB2"),
            (p(10.0, 0.0), "KB1"),
            (p(10.0, 5.0), "KB2 END"),
            (p(20.0, 0.0), "KB1"),
            (p(50.0, 50.0), "TR"), // a point feature: no line
            (p(0.0, 20.0), "BLD"),
            (p(10.0, 20.0), "BLD"),
            (p(10.0, 30.0), "BLD CL"),
            (p(30.0, 0.0), "KB1 ST"), // starts a second KB1 string
            (p(40.0, 0.0), "KB1"),
        ];
        let lines = lib.build_linework(pts);
        let names: Vec<&str> = lines.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, vec!["KB2.1", "BLD.1", "KB1.1", "KB1.2"]);
        assert_eq!(lines[0].vertices.len(), 2);
        assert!(lines[1].closed && lines[1].vertices.len() == 3);
        assert_eq!(lines[2].vertices.len(), 3);
        assert_eq!(lines[3].vertices, vec![p(30.0, 0.0), p(40.0, 0.0)]);
    }

    #[test]
    fn arcs_between_pc_and_pt() {
        let lib = FeatureLibrary::example();
        let pts = [
            (p(-10.0, 0.0), "EP"),
            (p(0.0, 0.0), "EP PC"),
            (
                p(
                    10.0 - 10.0 * std::f64::consts::FRAC_1_SQRT_2,
                    10.0 * std::f64::consts::FRAC_1_SQRT_2,
                ),
                "EP",
            ),
            (p(10.0, 10.0), "EP PT"),
            (p(10.0, 20.0), "EP"),
        ];
        let lines = lib.build_linework(pts);
        assert_eq!(lines.len(), 1);
        let l = &lines[0];
        assert_eq!(l.vertices.len(), 4, "the arc's middle point is not a vertex");
        let arc = Arc::from_bulge(l.vertices[1], l.vertices[2], l.bulges[1]).expect("arc");
        assert!((arc.radius - 10.0).abs() < 1e-9);
        assert_eq!(l.bulges[0], 0.0);
    }

    #[test]
    fn wildcard_codes() {
        assert!(glob_match("MH*", "mh12a"));
        assert!(glob_match("T?EE", "TREE"));
        assert!(glob_match("*", ""));
        assert!(!glob_match("MH*", "SMH1"));
        assert!(glob_match("*PIT*", "SWPIT2"));
        let lib = FeatureLibrary::example();
        // Plain codes (with string numbers) win; patterns catch the rest.
        assert_eq!(lib.feature_for("KB2 ST").map(|f| f.code.as_str()), Some("KB"));
        assert_eq!(lib.feature_for("MH12 lid").map(|f| f.code.as_str()), Some("MH*"));
        assert!(lib.feature_for("XYZ").is_none());
        // The more specific pattern wins.
        let mut lib = lib;
        lib.codes.push(FeatureCode::new(
            "MHS*",
            "Sewer manhole",
            "SEWER",
            [1, 2, 3],
            false,
        ));
        assert_eq!(lib.feature_for("MHS4").map(|f| f.code.as_str()), Some("MHS*"));
        assert_eq!(lib.parse("MH7").map(|p| p.string), Some(String::new()));
    }
}
