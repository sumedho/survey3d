//! Field-to-finish: turning coded survey points into linework.
//!
//! A [`FeatureLibrary`] lists the feature codes used in the field (with the layer
//! and colour their objects go on, and whether they form lines) and the
//! *control codes* that shape strings. A point's code is a feature code,
//! optionally followed directly by a string number (`KB1`, `KB2` for two kerb
//! lines picked up alternately), then any control codes separated by spaces.
//! The defaults follow Carlson's special codes (with the older `ST`/`END`/`CL`
//! kept), and each can be changed or given several spellings:
//!
//! | control (defaults) | meaning |
//! |---|---|
//! | `ST` `+7` | start a new string at this point |
//! | `END` `-7` | end the string at this point |
//! | `CLO` `CL` | close the string back to its first point |
//! | `PC` / `PT` | points from `PC` to `PT` form a circular arc; without a `PT`, the `PC` point and the next two |
//! | `GAP` | no line from this point to the next one on the string |
//! | `RECT` | close square: right angles at the first and this point meet in a new corner |
//! | `RECT`*d* | a rectangle *d* to the right (negative: left) of the last two points |
//! | `CIR`*r* | a circle of radius *r* centred on the point (`CIR` alone: the next point with the code is on the circle) |
//! | `OH`*d* / `OFL`*d* | a copy of the string offset *d* right / left (several give several copies) |
//! | `OV`*d* | the offset copy *d* higher (negative: lower) |
//! | `JPN`*name* | join to the point *name* (at a string's start, the string begins there) |
//!
//! Points are processed in the order given (normally survey order). Other
//! tokens are ignored, so field notes can follow the codes.

use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

use crate::arc::Arc;
use crate::geom::line_intersection_2d;
use crate::offset::offset_polyline;
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

/// The control codes, each one or more spellings separated by spaces or
/// commas (an empty one is off).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ControlCodes {
    pub start: String,
    pub end: String,
    pub close: String,
    pub arc_start: String,
    pub arc_end: String,
    pub gap: String,
    /// Close square, or a rectangle with an offset after the code.
    pub rectangle: String,
    /// A circle, with its radius after the code.
    pub circle: String,
    /// Offset copies: right (horizontal), left, and vertical, each with a distance.
    pub offset: String,
    pub offset_left: String,
    pub offset_vertical: String,
    /// Join to the point named after the code.
    pub join: String,
}

impl Default for ControlCodes {
    fn default() -> Self {
        Self {
            start: "ST +7".into(),
            end: "END -7".into(),
            close: "CLO CL".into(),
            arc_start: "PC".into(),
            arc_end: "PT".into(),
            gap: "GAP".into(),
            rectangle: "RECT".into(),
            circle: "CIR".into(),
            offset: "OH".into(),
            offset_left: "OFL".into(),
            offset_vertical: "OV".into(),
            join: "JPN".into(),
        }
    }
}

impl ControlCodes {
    /// The fields with their meanings, for settings screens and help.
    pub fn fields_mut(&mut self) -> [(&'static str, &mut String); 12] {
        [
            ("Start a string", &mut self.start),
            ("End a string", &mut self.end),
            ("Close", &mut self.close),
            ("Start an arc", &mut self.arc_start),
            ("End an arc", &mut self.arc_end),
            ("Gap to the next point", &mut self.gap),
            ("Rectangle (close square, or RECT10)", &mut self.rectangle),
            ("Circle (CIR2 = radius 2)", &mut self.circle),
            ("Offset right (OH0.5)", &mut self.offset),
            ("Offset left (OFL0.5)", &mut self.offset_left),
            ("Offset height (OV-0.15)", &mut self.offset_vertical),
            ("Join to point (JPN101)", &mut self.join),
        ]
    }
}

/// The spellings of a control code.
fn spellings(control: &str) -> impl Iterator<Item = &str> {
    control.split([' ', ',']).filter(|s| !s.is_empty())
}

/// Whether `token` is the control code.
fn is_control(token: &str, control: &str) -> bool {
    spellings(control).any(|c| token.eq_ignore_ascii_case(c))
}

/// What follows the control code in `token` (`"10"` in `RECT10`), if `token`
/// starts with it.
fn control_value<'a>(token: &'a str, control: &str) -> Option<&'a str> {
    spellings(control)
        .filter(|c| {
            token.len() >= c.len()
                && token.is_char_boundary(c.len())
                && token[..c.len()].eq_ignore_ascii_case(c)
        })
        .max_by_key(|c| c.len())
        .map(|c| &token[c.len()..])
}

/// A number after a control code (`.5`, `-0.8`), or none.
fn control_number(token: &str, control: &str) -> Option<Option<f64>> {
    let rest = control_value(token, control)?;
    if rest.is_empty() {
        return Some(None);
    }
    rest.parse::<f64>().ok().filter(|v| v.is_finite()).map(Some)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FeatureLibrary {
    pub codes: Vec<FeatureCode>,
    pub controls: ControlCodes,
}

/// A point's code split into its parts.
#[derive(Clone, Debug, PartialEq)]
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
    pub gap: bool,
    /// `RECT` (close square: `Some(None)`) or `RECT`*d*.
    pub rectangle: Option<Option<f64>>,
    /// `CIR` (radius from the next point: `Some(None)`) or `CIR`*r*.
    pub circle: Option<Option<f64>>,
    /// Horizontal offsets, right positive (`OH`, and `OFL` negated).
    pub offsets: Vec<f64>,
    pub offset_vertical: Option<f64>,
    /// The point named by `JPN`.
    pub join: Option<String>,
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
            gap: false,
            rectangle: None,
            circle: None,
            offsets: Vec::new(),
            offset_vertical: None,
            join: None,
        };
        let c = &self.controls;
        for t in tokens {
            if is_control(t, &c.start) {
                parsed.start = true;
            } else if is_control(t, &c.end) {
                parsed.end = true;
            } else if is_control(t, &c.close) {
                parsed.close = true;
            } else if is_control(t, &c.arc_start) {
                parsed.arc_start = true;
            } else if is_control(t, &c.arc_end) {
                parsed.arc_end = true;
            } else if is_control(t, &c.gap) {
                parsed.gap = true;
            } else if let Some(v) = control_number(t, &c.rectangle) {
                parsed.rectangle = Some(v);
            } else if let Some(v) = control_number(t, &c.circle) {
                parsed.circle = Some(v.filter(|r| *r > 0.0));
            } else if let Some(Some(v)) = control_number(t, &c.offset_left) {
                parsed.offsets.push(-v);
            } else if let Some(Some(v)) = control_number(t, &c.offset_vertical) {
                parsed.offset_vertical = Some(v);
            } else if let Some(Some(v)) = control_number(t, &c.offset) {
                parsed.offsets.push(v);
            } else if let Some(name) = control_value(t, &c.join).filter(|n| !n.is_empty()) {
                parsed.join = Some(name.to_string());
            }
        }
        Some(parsed)
    }

    /// Builds strings from `(position, code)` pairs in survey order.
    pub fn build_linework<'a>(
        &self,
        points: impl IntoIterator<Item = (DVec3, &'a str)>,
    ) -> Vec<GeneratedString> {
        self.build_linework_with(points, &|_| None)
    }

    /// As [`Self::build_linework`], with `find` giving the position of a point
    /// named by a join (`JPN`) code.
    pub fn build_linework_with<'a>(
        &self,
        points: impl IntoIterator<Item = (DVec3, &'a str)>,
        find: &dyn Fn(&str) -> Option<DVec3>,
    ) -> Vec<GeneratedString> {
        let parsed: Vec<(DVec3, ParsedCode)> = points
            .into_iter()
            .filter_map(|(p, raw)| Some((p, self.parse(raw)?)))
            .collect();
        let key_of = |c: &ParsedCode| format!("{}{}", self.codes[c.feature].code, c.string);
        let implicit_ends = self.implicit_arc_ends(&parsed, &key_of);
        let mut open: Vec<Chain> = Vec::new();
        let mut done = Vec::new();
        let mut sequence: std::collections::HashMap<String, usize> = Default::default();
        let mut next_sequence = |key: &str| {
            let n = sequence.entry(key.to_string()).or_default();
            *n += 1;
            *n
        };
        // Circle centres waiting for a point on the circle (`CIR` alone).
        let mut centres: std::collections::HashMap<String, DVec3> = Default::default();
        for (index, (p, code)) in parsed.iter().enumerate() {
            let p = *p;
            let key = key_of(code);
            let feature = code.feature;
            // Circles stand alone (point codes too), ending the string first.
            if code.circle.is_some() || centres.contains_key(&key) {
                if let Some(i) = open.iter().position(|c| c.key == key) {
                    done.extend(open.remove(i).finish());
                }
                let circle = match (centres.remove(&key), code.circle) {
                    (Some(centre), _) => Some((centre, centre.truncate().distance(p.truncate()))),
                    (None, Some(Some(r))) => Some((p, r)),
                    (None, _) => {
                        centres.insert(key.clone(), p);
                        None
                    }
                };
                if let Some((centre, r)) = circle.filter(|(_, r)| *r > 0.0) {
                    let n = next_sequence(&key);
                    done.push(GeneratedString {
                        feature,
                        name: format!("{key}.{n}"),
                        vertices: vec![centre + DVec3::new(r, 0.0, 0.0), centre - DVec3::new(r, 0.0, 0.0)],
                        bulges: vec![1.0, 1.0],
                        closed: true,
                    });
                }
                continue;
            }
            if !self.codes[feature].linework {
                continue;
            }
            if code.start
                && let Some(i) = open.iter().position(|c| c.key == key)
            {
                done.extend(open.remove(i).finish());
            }
            // A gap before this point: the string goes on as a new one.
            if let Some(i) = open.iter().position(|c| c.key == key && c.gap_next) {
                done.extend(open.remove(i).finish());
            }
            let i = match open.iter().position(|c| c.key == key) {
                Some(i) => i,
                None => {
                    let n = next_sequence(&key);
                    open.push(Chain::new(key.clone(), feature, n));
                    open.len() - 1
                }
            };
            let chain = &mut open[i];
            if let Some(to) = code.join.as_deref().and_then(find) {
                if chain.vertices.is_empty() {
                    chain.push(to);
                } else {
                    // Further on, a line of its own to the named point.
                    let n = next_sequence(&key);
                    done.push(GeneratedString {
                        feature,
                        name: format!("{key}.{n}"),
                        vertices: vec![p, to],
                        bulges: Vec::new(),
                        closed: false,
                    });
                }
            }
            let chain = &mut open[i];
            chain.push(p);
            if chain.offsets.is_empty() && !code.offsets.is_empty() {
                chain.offsets.clone_from(&code.offsets);
            }
            if let Some(v) = code.offset_vertical {
                chain.offset_vertical = v;
            }
            if code.arc_end || implicit_ends[index] {
                chain.close_arc();
            }
            if code.arc_start {
                chain.arc_from = Some(chain.vertices.len() - 1);
            }
            chain.gap_next = code.gap;
            let mut finished = code.close || code.end;
            if let Some(rect) = code.rectangle
                && chain.rectangle(rect)
            {
                finished = true;
            }
            if code.close {
                chain.closed = true;
            }
            if finished {
                done.extend(open.remove(i).finish());
            }
        }
        for chain in open {
            done.extend(chain.finish());
        }
        done
    }

    /// Points that end an arc whose `PC` has no `PT`: the second point after
    /// the `PC` on its string (Carlson's three-point arc).
    fn implicit_arc_ends(
        &self,
        parsed: &[(DVec3, ParsedCode)],
        key_of: &dyn Fn(&ParsedCode) -> String,
    ) -> Vec<bool> {
        let mut ends = vec![false; parsed.len()];
        for (i, (_, c)) in parsed.iter().enumerate() {
            if !c.arc_start {
                continue;
            }
            let key = key_of(c);
            let mut count = 0;
            let mut second = None;
            let mut has_end = false;
            for (j, (_, d)) in parsed.iter().enumerate().skip(i + 1) {
                if key_of(d) != key || d.circle.is_some() {
                    continue;
                }
                if d.start {
                    break;
                }
                count += 1;
                if count == 2 {
                    second = Some(j);
                }
                if d.arc_end {
                    has_end = true;
                    break;
                }
                if d.end || d.close || d.arc_start {
                    break;
                }
            }
            if !has_end && let Some(j) = second {
                ends[j] = true;
            }
        }
        ends
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
    /// No segment from the last point to the next.
    gap_next: bool,
    /// Offset copies (right positive), and how much higher they are.
    offsets: Vec<f64>,
    offset_vertical: f64,
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
            gap_next: false,
            offsets: Vec::new(),
            offset_vertical: 0.0,
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

    /// Completes a rectangle and closes the string: `Some(d)` builds it `d` to
    /// the right of the last two points, `None` closes square (right angles at
    /// the first and last points meet in a new corner). `false` when there are
    /// too few points.
    fn rectangle(&mut self, offset: Option<f64>) -> bool {
        let n = self.vertices.len();
        let corners = match offset {
            Some(d) if n >= 2 => {
                let (a, b) = (self.vertices[n - 2], self.vertices[n - 1]);
                let Some(dir) = (b - a).truncate().try_normalize() else {
                    return false;
                };
                let right = DVec2::new(dir.y, -dir.x) * d;
                vec![b + right.extend(0.0), a + right.extend(0.0)]
            }
            None if n >= 3 => {
                let (p0, p1) = (self.vertices[0].truncate(), self.vertices[1].truncate());
                let (q0, q1) = (self.vertices[n - 2].truncate(), self.vertices[n - 1].truncate());
                let Some((t, _)) = line_intersection_2d(p0, p0 + (p1 - p0).perp(), q1, q1 + (q1 - q0).perp())
                else {
                    return false;
                };
                let corner = p0 + (p1 - p0).perp() * t;
                vec![corner.extend(self.vertices[n - 1].z)]
            }
            _ => return false,
        };
        for c in corners {
            self.push(c);
        }
        self.closed = true;
        true
    }

    /// The string and its offset copies (named `CODE….n.k`).
    fn finish(mut self) -> Vec<GeneratedString> {
        self.close_arc();
        if self.vertices.len() < 2 {
            return Vec::new();
        }
        let has_arcs = self.bulges.iter().any(|b| *b != 0.0);
        let closed = self.closed && (self.vertices.len() >= 3 || has_arcs);
        let mut bulges = self.bulges;
        bulges.push(0.0);
        if !has_arcs {
            bulges.clear();
        }
        let name = format!("{}.{}", self.key, self.sequence);
        let mut out = Vec::new();
        for (k, d) in self.offsets.iter().enumerate() {
            if let Ok((vertices, b)) = offset_polyline(&self.vertices, &bulges, closed, *d) {
                let up = DVec3::new(0.0, 0.0, self.offset_vertical);
                out.push(GeneratedString {
                    feature: self.feature,
                    name: format!("{name}.{}", k + 1),
                    vertices: vertices.into_iter().map(|v| v + up).collect(),
                    bulges: if b.iter().any(|b| *b != 0.0) {
                        b
                    } else {
                        Vec::new()
                    },
                    closed,
                });
            }
        }
        out.insert(
            0,
            GeneratedString {
                feature: self.feature,
                name,
                vertices: self.vertices,
                bulges,
                closed,
            },
        );
        out
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
    fn carlson_start_end_and_close() {
        let lib = FeatureLibrary::example();
        let pts = [
            (p(0.0, 0.0), "KB +7"),
            (p(10.0, 0.0), "KB"),
            (p(20.0, 0.0), "KB -7"),
            (p(0.0, 20.0), "BLD"),
            (p(10.0, 20.0), "BLD"),
            (p(10.0, 30.0), "BLD CLO"),
        ];
        let lines = lib.build_linework(pts);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].vertices.len(), 3);
        assert!(lines[1].closed);
        // Spellings are configurable: one, several or none.
        let mut lib = lib;
        lib.controls.close = String::new();
        let lines = lib.build_linework(pts);
        assert!(!lines[1].closed, "closing turned off");
    }

    #[test]
    fn gaps_rectangles_and_circles() {
        let lib = FeatureLibrary::example();
        // A kerb broken across a driveway.
        let kerb = lib.build_linework([
            (p(0.0, 0.0), "KB"),
            (p(10.0, 0.0), "KB GAP"),
            (p(15.0, 0.0), "KB"),
            (p(25.0, 0.0), "KB"),
        ]);
        assert_eq!(kerb.len(), 2, "{kerb:?}");
        assert_eq!(kerb[0].vertices, vec![p(0.0, 0.0), p(10.0, 0.0)]);
        assert_eq!(kerb[1].vertices, vec![p(15.0, 0.0), p(25.0, 0.0)]);

        // RECT10: a 10 m deep pad to the right of the two points.
        let pad = lib.build_linework([(p(0.0, 0.0), "BLD"), (p(0.0, 20.0), "BLD RECT10")]);
        assert_eq!(pad.len(), 1);
        assert!(pad[0].closed);
        assert_eq!(
            pad[0].vertices,
            vec![p(0.0, 0.0), p(0.0, 20.0), p(10.0, 20.0), p(10.0, 0.0)]
        );
        // RECT alone squares off three points of a building.
        let shed = lib.build_linework([
            (p(0.0, 0.0), "BLD"),
            (p(8.0, 0.0), "BLD"),
            (p(8.0, 5.0), "BLD RECT"),
        ]);
        assert!(shed[0].closed && shed[0].vertices.len() == 4);
        assert!(
            (shed[0].vertices[3] - p(0.0, 5.0)).length() < 1e-9,
            "{:?}",
            shed[0].vertices
        );

        // CIR2 on a tree (a point code): a circle of radius 2; CIR alone takes
        // its radius from the next point with the code.
        let trees = lib.build_linework([
            (p(5.0, 5.0), "TR CIR2"),
            (p(50.0, 50.0), "TR CIR"),
            (p(53.0, 54.0), "TR"),
        ]);
        assert_eq!(trees.len(), 2);
        for (t, centre, r) in [(&trees[0], p(5.0, 5.0), 2.0), (&trees[1], p(50.0, 50.0), 5.0)] {
            assert!(t.closed && t.bulges == vec![1.0, 1.0]);
            let arc = Arc::from_bulge(t.vertices[0], t.vertices[1], 1.0).expect("arc");
            assert!((arc.radius - r).abs() < 1e-9 && (arc.center - centre.truncate()).length() < 1e-9);
        }
        assert_eq!(trees[0].name, "TR.1");
        assert!(is_generated_name(&trees[1].name, "TR"));
    }

    #[test]
    fn offsets_joins_and_three_point_arcs() {
        let lib = FeatureLibrary::example();
        // Back of kerb picked up, with the lip 0.5 right and 0.15 lower, and the
        // 1.2 m footpath edge to the left.
        let lines = lib.build_linework([
            (DVec3::new(0.0, 0.0, 10.0), "KB OH0.5 OFL1.2 OV-0.15"),
            (DVec3::new(20.0, 0.0, 10.0), "KB"),
        ]);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(lines[1].name, "KB.1.1");
        assert!(
            (lines[1].vertices[0] - DVec3::new(0.0, -0.5, 9.85)).length() < 1e-9,
            "{:?}",
            lines[1].vertices
        );
        assert!((lines[2].vertices[0] - DVec3::new(0.0, 1.2, 9.85)).length() < 1e-9);
        assert!(is_generated_name(&lines[1].name, "KB"));

        // JPN at a string's start begins it at the named point.
        let find = |name: &str| (name == "101").then(|| p(-5.0, 0.0));
        let lines = lib.build_linework_with([(p(0.0, 0.0), "FNC JPN101"), (p(10.0, 0.0), "FNC")], &find);
        assert_eq!(lines[0].vertices, vec![p(-5.0, 0.0), p(0.0, 0.0), p(10.0, 0.0)]);

        // A PC without a PT: an arc through it and the next two points, then
        // straight on.
        let r = 10.0;
        let h = std::f64::consts::FRAC_1_SQRT_2 * r;
        let lines = lib.build_linework([
            (p(0.0, 0.0), "EP PC"),
            (p(r - h, h), "EP"),
            (p(r, r), "EP"),
            (p(r, 2.0 * r), "EP"),
        ]);
        let l = &lines[0];
        assert_eq!(l.vertices.len(), 3, "{l:?}");
        assert!(l.bulges[0] != 0.0 && l.bulges[1] == 0.0);
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
