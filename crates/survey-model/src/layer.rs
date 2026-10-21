use serde::{Deserialize, Serialize};

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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub color: Color,
    pub visible: bool,
    pub locked: bool,
}

impl Layer {
    pub fn new(id: LayerId, name: impl Into<String>, color: Color) -> Self {
        Self {
            id,
            name: name.into(),
            color,
            visible: true,
            locked: false,
        }
    }
}
