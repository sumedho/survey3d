//! Display styles for coded survey data: point symbols and line styles.
//!
//! Symbols are defined once as a few vector primitives in a unit square
//! (radius 1, +y up); the renderer evaluates them as signed distance fields on
//! the GPU (so any number of points stays fast and edges stay crisp at any
//! size) and the UI draws the same primitives for previews and legends.

use serde::{Deserialize, Serialize};

/// One building block of a symbol, in unit coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Prim {
    Circle {
        c: [f32; 2],
        r: f32,
        fill: bool,
    },
    /// Axis-aligned square of half-size `h`.
    Square {
        c: [f32; 2],
        h: f32,
        fill: bool,
    },
    /// Regular polygon with `n` corners on a circle of radius `r`; the first
    /// corner is `rot` radians clockwise from straight up.
    Polygon {
        c: [f32; 2],
        r: f32,
        n: u32,
        rot: f32,
        fill: bool,
    },
    /// Star with `n` points between radii `r` and `r · inner`.
    Star {
        c: [f32; 2],
        r: f32,
        n: u32,
        inner: f32,
        fill: bool,
    },
    /// Stroked line segment.
    Segment {
        a: [f32; 2],
        b: [f32; 2],
    },
}

const O: [f32; 2] = [0.0, 0.0];
const PI: f32 = std::f32::consts::PI;

const fn circle(r: f32, fill: bool) -> Prim {
    Prim::Circle { c: O, r, fill }
}
const fn square(h: f32, fill: bool) -> Prim {
    Prim::Square { c: O, h, fill }
}
const fn poly(n: u32, r: f32, rot: f32, fill: bool) -> Prim {
    Prim::Polygon {
        c: O,
        r,
        n,
        rot,
        fill,
    }
}
const fn seg(a: [f32; 2], b: [f32; 2]) -> Prim {
    Prim::Segment { a, b }
}
const PLUS: [Prim; 2] = [seg([-1.0, 0.0], [1.0, 0.0]), seg([0.0, -1.0], [0.0, 1.0])];
const D: f32 = std::f32::consts::FRAC_1_SQRT_2;
const X: [Prim; 2] = [seg([-D, -D], [D, D]), seg([-D, D], [D, -D])];

macro_rules! symbols {
    ($( $variant:ident => $key:literal, $label:literal, [$($prim:expr),* $(,)?]; )*) => {
        /// A point symbol. Stored by its key (`"circle_cross"`), so the list can grow.
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum Symbol {
            #[default]
            $($variant,)*
        }

        impl Symbol {
            pub const ALL: &'static [Symbol] = &[$(Symbol::$variant,)*];

            /// Stable key used in files (`circle_cross`).
            pub fn key(self) -> &'static str {
                match self { $(Symbol::$variant => $key,)* }
            }

            /// Display name, with typical uses.
            pub fn label(self) -> &'static str {
                match self { $(Symbol::$variant => $label,)* }
            }

            pub fn prims(self) -> &'static [Prim] {
                match self { $(Symbol::$variant => { const P: &[Prim] = &[$($prim),*]; P })* }
            }
        }
    };
}

symbols! {
    Dot => "dot", "Dot", [circle(1.0, true)];
    Square => "square", "Square", [square(0.85, true)];
    Plus => "plus", "Plus", [PLUS[0], PLUS[1]];
    Cross => "cross", "Cross (×)", [X[0], X[1]];
    Circle => "circle", "Circle", [circle(0.9, false)];
    SquareOpen => "square_open", "Square outline", [square(0.8, false)];
    Triangle => "triangle", "Triangle", [poly(3, 1.0, 0.0, false)];
    TriangleFilled => "triangle_filled", "Triangle, filled", [poly(3, 1.0, 0.0, true)];
    Diamond => "diamond", "Diamond", [poly(4, 1.0, 0.0, false)];
    DiamondFilled => "diamond_filled", "Diamond, filled", [poly(4, 1.0, 0.0, true)];
    Hexagon => "hexagon", "Hexagon", [poly(6, 0.95, PI / 6.0, false)];
    Star => "star", "Star", [Prim::Star { c: O, r: 1.0, n: 5, inner: 0.45, fill: true }];
    CircleCross => "circle_cross", "Circle with cross (control mark)", [circle(0.75, false), PLUS[0], PLUS[1]];
    CircleX => "circle_x", "Circle with × (manhole)", [circle(0.9, false), seg([-0.62, -0.62], [0.62, 0.62]), seg([-0.62, 0.62], [0.62, -0.62])];
    CircleDot => "circle_dot", "Circle with dot (peg, mark)", [circle(0.9, false), circle(0.3, true)];
    DoubleCircle => "double_circle", "Double circle (well, bore)", [circle(0.95, false), circle(0.5, false)];
    SquareCross => "square_cross", "Square with × (pit, drain)", [square(0.8, false), seg([-0.8, -0.8], [0.8, 0.8]), seg([-0.8, 0.8], [0.8, -0.8])];
    SquareDot => "square_dot", "Square with dot", [square(0.8, false), circle(0.28, true)];
    TriangleDot => "triangle_dot", "Triangle with dot (trig station)", [poly(3, 1.0, 0.0, false), circle(0.22, true)];
    Benchmark => "benchmark", "Benchmark", [circle(0.95, false), poly(3, 0.5, 0.0, true)];
    Target => "target", "Target", [circle(0.95, false), circle(0.38, true)];
    Tree => "tree", "Tree", [Prim::Star { c: O, r: 1.0, n: 8, inner: 0.78, fill: false }, circle(0.18, true)];
    Shrub => "shrub", "Shrub", [Prim::Star { c: O, r: 0.85, n: 6, inner: 0.6, fill: false }];
    PowerPole => "power_pole", "Power pole", [circle(0.55, true), seg([-1.0, 0.0], [1.0, 0.0])];
    LightPole => "light_pole", "Light pole", [circle(0.45, false), seg([0.48, 0.48], [0.9, 0.9]), seg([-0.48, 0.48], [-0.9, 0.9]), seg([0.48, -0.48], [0.9, -0.9]), seg([-0.48, -0.48], [-0.9, -0.9])];
    Hydrant => "hydrant", "Hydrant", [circle(0.55, true), seg([-1.0, 0.0], [-0.5, 0.0]), seg([0.5, 0.0], [1.0, 0.0]), seg([0.0, 0.5], [0.0, 1.0])];
    Valve => "valve", "Valve", [Prim::Polygon { c: [-0.42, 0.0], r: 0.55, n: 3, rot: PI / 2.0, fill: true }, Prim::Polygon { c: [0.42, 0.0], r: 0.55, n: 3, rot: -PI / 2.0, fill: true }];
    Sign => "sign", "Sign", [Prim::Square { c: [0.0, 0.45], h: 0.5, fill: true }, seg([0.0, 0.0], [0.0, -1.0])];
    Post => "post", "Post (fence, bollard)", [square(0.45, true)];
    Bollard => "bollard", "Bollard", [square(0.85, false), circle(0.4, true)];
    Grate => "grate", "Grate (inlet)", [square(0.85, false), square(0.4, true)];
}

impl Symbol {
    /// Index used by the renderer.
    pub fn id(self) -> u32 {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0) as u32
    }

    pub fn from_key(key: &str) -> Option<Self> {
        let k = key.trim();
        Self::ALL
            .iter()
            .copied()
            .find(|s| s.key().eq_ignore_ascii_case(k))
    }
}

/// How a line is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    DashDot,
}

impl LineStyle {
    pub const ALL: [LineStyle; 4] = [
        LineStyle::Solid,
        LineStyle::Dashed,
        LineStyle::Dotted,
        LineStyle::DashDot,
    ];

    pub fn key(self) -> &'static str {
        match self {
            LineStyle::Solid => "solid",
            LineStyle::Dashed => "dashed",
            LineStyle::Dotted => "dotted",
            LineStyle::DashDot => "dash_dot",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LineStyle::Solid => "Solid",
            LineStyle::Dashed => "Dashed",
            LineStyle::Dotted => "Dotted",
            LineStyle::DashDot => "Dash-dot",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        let k = key.trim();
        Self::ALL.into_iter().find(|s| s.key().eq_ignore_ascii_case(k))
    }

    /// On/off lengths in multiples of the line width (empty for solid). The
    /// renderer and UI previews share these.
    pub fn pattern(self) -> &'static [f32] {
        match self {
            LineStyle::Solid => &[],
            LineStyle::Dashed => &[6.0, 4.0],
            LineStyle::Dotted => &[1.0, 2.5],
            LineStyle::DashDot => &[6.0, 2.5, 1.0, 2.5],
        }
    }

    pub fn id(self) -> u32 {
        match self {
            LineStyle::Solid => 0,
            LineStyle::Dashed => 1,
            LineStyle::Dotted => 2,
            LineStyle::DashDot => 3,
        }
    }
}

/// Which point labels a feature shows (overrides the global label settings).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LabelSet {
    pub name: bool,
    pub code: bool,
    pub height: bool,
    pub description: bool,
}

/// How a feature code's points and lines are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CodeStyle {
    pub symbol: Symbol,
    /// Symbol size in screen points.
    pub symbol_size: f32,
    /// Draw in the layer's colour (otherwise the feature's own colour).
    pub by_layer: bool,
    pub line_style: LineStyle,
    /// Line weight in screen points.
    pub line_weight: f32,
    /// Labels for this feature's points; `None` follows the global settings.
    pub labels: Option<LabelSet>,
}

impl Default for CodeStyle {
    fn default() -> Self {
        Self {
            symbol: Symbol::Dot,
            symbol_size: 7.0,
            by_layer: true,
            line_style: LineStyle::Solid,
            line_weight: 1.5,
            labels: None,
        }
    }
}

/// Primitive kinds as encoded for the GPU.
pub mod gpu {
    use super::{Prim, Symbol};

    pub const CIRCLE: f32 = 0.0;
    pub const SQUARE: f32 = 1.0;
    pub const POLYGON: f32 = 2.0;
    pub const STAR: f32 = 3.0;
    pub const SEGMENT: f32 = 4.0;

    /// Each primitive as two vec4s: `(kind, fill, x, y)` and parameters
    /// (`r` / `h`, `n`, `rot` / `inner`, —) or, for segments, `(bx, by, 0, 0)`.
    pub fn encode(p: &Prim) -> [[f32; 4]; 2] {
        let f = |b: bool| if b { 1.0 } else { 0.0 };
        match *p {
            Prim::Circle { c, r, fill } => [[CIRCLE, f(fill), c[0], c[1]], [r, 0.0, 0.0, 0.0]],
            Prim::Square { c, h, fill } => [[SQUARE, f(fill), c[0], c[1]], [h, 0.0, 0.0, 0.0]],
            Prim::Polygon { c, r, n, rot, fill } => [[POLYGON, f(fill), c[0], c[1]], [r, n as f32, rot, 0.0]],
            Prim::Star { c, r, n, inner, fill } => [[STAR, f(fill), c[0], c[1]], [r, n as f32, inner, 0.0]],
            Prim::Segment { a, b } => [[SEGMENT, 0.0, a[0], a[1]], [b[0], b[1], 0.0, 0.0]],
        }
    }

    /// The symbol table: per symbol `(first primitive, count)`, and the
    /// encoded primitives.
    pub fn table() -> (Vec<[u32; 4]>, Vec<[f32; 4]>) {
        let mut ranges = Vec::new();
        let mut prims = Vec::new();
        for s in Symbol::ALL {
            let start = prims.len() as u32 / 2;
            for p in s.prims() {
                prims.extend(encode(p));
            }
            ranges.push([start, s.prims().len() as u32, 0, 0]);
        }
        (ranges, prims)
    }
}

/// Corners of a polygon or star primitive, in unit coordinates (clockwise from the top).
pub fn outline(p: &Prim) -> Vec<[f32; 2]> {
    let at = |c: [f32; 2], r: f32, a: f32| [c[0] + r * a.sin(), c[1] + r * a.cos()];
    match *p {
        Prim::Polygon { c, r, n, rot, .. } => (0..n)
            .map(|i| at(c, r, rot + i as f32 * std::f32::consts::TAU / n as f32))
            .collect(),
        Prim::Star { c, r, n, inner, .. } => (0..2 * n)
            .map(|i| {
                let radius = if i % 2 == 0 { r } else { r * inner };
                at(c, radius, i as f32 * PI / n as f32)
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_and_are_unique() {
        let mut keys = std::collections::HashSet::new();
        for s in Symbol::ALL {
            assert!(keys.insert(s.key()), "duplicate key {}", s.key());
            assert_eq!(Symbol::from_key(s.key()), Some(*s));
            assert!(!s.prims().is_empty());
            assert_eq!(Symbol::ALL[s.id() as usize], *s);
        }
        assert!(Symbol::ALL.len() >= 30);
        for l in LineStyle::ALL {
            assert_eq!(LineStyle::from_key(l.key()), Some(l));
        }
        let json = serde_json::to_string(&Symbol::CircleCross).expect("json");
        assert_eq!(json, "\"circle_cross\"");
    }

    #[test]
    fn gpu_table_fits_and_stays_in_the_unit_square() {
        let (ranges, prims) = gpu::table();
        assert_eq!(ranges.len(), Symbol::ALL.len());
        // The shader's uniform arrays hold 64 symbols and 128 primitives.
        assert!(
            ranges.len() <= 64 && prims.len() / 2 <= 128,
            "{} prims",
            prims.len() / 2
        );
        for s in Symbol::ALL {
            for p in s.prims() {
                let pts: Vec<[f32; 2]> = match *p {
                    Prim::Circle { c, r, .. } => vec![[c[0] + r, c[1]], [c[0], c[1] + r]],
                    Prim::Square { c, h, .. } => vec![[c[0] + h, c[1] + h], [c[0] - h, c[1] - h]],
                    Prim::Segment { a, b } => vec![a, b],
                    _ => outline(p),
                };
                for q in pts {
                    assert!(q[0].abs() <= 1.0001 && q[1].abs() <= 1.0001, "{s:?} {q:?}");
                }
            }
        }
    }
}
