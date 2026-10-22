use serde::{Deserialize, Serialize};
use survey_core::style::LineStyle;

use crate::ids::LayerId;

/// Straight (non-premultiplied) sRGBA colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Color(pub [u8; 4]);

impl Color {
    pub const WHITE: Color = Color([255, 255, 255, 255]);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color([r, g, b, 255])
    }

    pub fn to_u32(self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    pub fn from_u32(v: u32) -> Self {
        Color(v.to_be_bytes())
    }
}

/// Line weight of objects with no feature-code style (screen points).
pub const DEFAULT_LINE_WEIGHT: f32 = 1.5;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub color: Color,
    pub visible: bool,
    pub locked: bool,
    /// Line style and weight (screen points) of lines on this layer whose code
    /// has no style of its own.
    #[serde(default)]
    pub line_style: LineStyle,
    #[serde(default = "default_line_weight")]
    pub line_weight: f32,
}

fn default_line_weight() -> f32 {
    DEFAULT_LINE_WEIGHT
}

impl Layer {
    /// Colours for new layers, in turn, so neighbouring layers differ.
    pub const PALETTE: [Color; 8] = [
        Color::rgb(0x7d, 0xd3, 0xfc),
        Color::rgb(0xfb, 0xbf, 0x24),
        Color::rgb(0x4a, 0xde, 0x80),
        Color::rgb(0xf4, 0x72, 0xb6),
        Color::rgb(0xa7, 0x8b, 0xfa),
        Color::rgb(0xf8, 0x71, 0x71),
        Color::rgb(0x2d, 0xd4, 0xbf),
        Color::rgb(0xfd, 0xba, 0x74),
    ];

    /// The palette colour for the `n`th layer.
    pub fn palette_color(n: usize) -> Color {
        Self::PALETTE[n % Self::PALETTE.len()]
    }

    pub fn new(id: LayerId, name: impl Into<String>, color: Color) -> Self {
        Self {
            id,
            name: name.into(),
            color,
            visible: true,
            locked: false,
            line_style: LineStyle::Solid,
            line_weight: DEFAULT_LINE_WEIGHT,
        }
    }
}
