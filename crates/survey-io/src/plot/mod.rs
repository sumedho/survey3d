//! Paper-space drawings ("plots") and their PDF and SVG writers.
//!
//! A [`Plot`] is a page of vector items in millimetres with the origin at the
//! bottom-left corner and +y up. The plan builder produces one; the writers
//! turn it into a file and the app's preview paints the same items, so what is
//! previewed is what is printed.
//!
//! Text uses Helvetica (one of the PDF base-14 fonts, so nothing is embedded)
//! and is positioned with its metrics, which [`text_width`] exposes for layout.

mod font;
mod pdf;
mod svg;

pub use font::{CAP_HEIGHT, DESCENT, text_width};
pub use pdf::write_pdf;
pub use svg::write_svg;

pub type Rgb = [u8; 3];

pub const BLACK: Rgb = [0, 0, 0];
pub const WHITE: Rgb = [255, 255, 255];

/// How a path is stroked. Widths and dash lengths are millimetres.
#[derive(Clone, Debug, PartialEq)]
pub struct Pen {
    pub color: Rgb,
    pub width: f64,
    /// Alternating on/off lengths; empty for a solid line.
    pub dash: Vec<f64>,
}

impl Pen {
    pub fn solid(color: Rgb, width: f64) -> Self {
        Self {
            color,
            width,
            dash: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Centre,
    Right,
}

/// Which part of the text sits on the anchor point vertically.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VAlign {
    /// The baseline.
    #[default]
    Baseline,
    /// Halfway up the capitals.
    Middle,
    /// The top of the capitals.
    Top,
    /// The bottom of the descenders.
    Bottom,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub at: [f64; 2],
    pub text: String,
    /// Font size (em) in millimetres; capitals are [`CAP_HEIGHT`] of this.
    pub size: f64,
    /// Radians counter-clockwise from the page's +x.
    pub angle: f64,
    pub align: Align,
    pub valign: VAlign,
    pub color: Rgb,
    pub bold: bool,
    /// A white outline behind the letters, to keep text readable over linework.
    pub halo: bool,
}

impl Text {
    pub fn new(at: [f64; 2], text: impl Into<String>, size: f64) -> Self {
        Self {
            at,
            text: text.into(),
            size,
            angle: 0.0,
            align: Align::Left,
            valign: VAlign::Baseline,
            color: BLACK,
            bold: false,
            halo: false,
        }
    }

    pub fn align(mut self, align: Align, valign: VAlign) -> Self {
        self.align = align;
        self.valign = valign;
        self
    }

    pub fn angle(mut self, radians: f64) -> Self {
        self.angle = radians;
        self
    }

    pub fn color(mut self, color: Rgb) -> Self {
        self.color = color;
        self
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn halo(mut self) -> Self {
        self.halo = true;
        self
    }

    pub fn width(&self) -> f64 {
        text_width(&self.text, self.size, self.bold)
    }

    /// Start of the baseline after alignment, in page millimetres.
    pub fn origin(&self) -> [f64; 2] {
        let (s, c) = self.angle.sin_cos();
        let w = self.width();
        let dx = match self.align {
            Align::Left => 0.0,
            Align::Centre => -w / 2.0,
            Align::Right => -w,
        };
        let dy = match self.valign {
            VAlign::Baseline => 0.0,
            VAlign::Middle => -CAP_HEIGHT * self.size / 2.0,
            VAlign::Top => -CAP_HEIGHT * self.size,
            VAlign::Bottom => DESCENT * self.size,
        };
        [self.at[0] + dx * c - dy * s, self.at[1] + dx * s + dy * c]
    }

    /// The corners of the text's box (baseline-left first, counter-clockwise),
    /// from the descenders to the top of the capitals.
    pub fn corners(&self) -> [[f64; 2]; 4] {
        let [x, y] = self.origin();
        let (s, c) = self.angle.sin_cos();
        let w = self.width();
        let lo = -DESCENT * self.size;
        let hi = CAP_HEIGHT * self.size;
        let at = |u: f64, v: f64| [x + u * c - v * s, y + u * s + v * c];
        [at(0.0, lo), at(w, lo), at(w, hi), at(0.0, hi)]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Path {
        points: Vec<[f64; 2]>,
        closed: bool,
        pen: Option<Pen>,
        fill: Option<Rgb>,
    },
    Circle {
        center: [f64; 2],
        radius: f64,
        pen: Option<Pen>,
        fill: Option<Rgb>,
    },
    Text(Text),
    /// Items drawn only inside `rings`, combined by the even-odd rule (so a
    /// ring inside another cuts a hole).
    Clip {
        rings: Vec<Vec<[f64; 2]>>,
        items: Vec<Item>,
    },
    /// A raster (straight RGBA, rows from the top) filling the paper
    /// rectangle `min`–`max`.
    Image {
        min: [f64; 2],
        max: [f64; 2],
        width: u32,
        height: u32,
        rgba: std::sync::Arc<Vec<u8>>,
    },
}

impl Item {
    pub fn line(points: Vec<[f64; 2]>, pen: Pen) -> Self {
        Item::Path {
            points,
            closed: false,
            pen: Some(pen),
            fill: None,
        }
    }

    pub fn rect(min: [f64; 2], max: [f64; 2], pen: Option<Pen>, fill: Option<Rgb>) -> Self {
        Item::Path {
            points: rect_ring(min, max),
            closed: true,
            pen,
            fill,
        }
    }
}

/// The corners of an axis-aligned rectangle, counter-clockwise.
pub fn rect_ring(min: [f64; 2], max: [f64; 2]) -> Vec<[f64; 2]> {
    vec![min, [max[0], min[1]], max, [min[0], max[1]]]
}

/// One page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plot {
    /// Page size in millimetres.
    pub width: f64,
    pub height: f64,
    /// Document title (PDF metadata, SVG `<title>`).
    pub title: String,
    pub items: Vec<Item>,
}

impl Plot {
    pub fn new(width: f64, height: f64, title: impl Into<String>) -> Self {
        Self {
            width,
            height,
            title: title.into(),
            items: Vec::new(),
        }
    }

    pub fn push(&mut self, item: Item) {
        self.items.push(item);
    }

    /// Number of items, counting inside clip groups.
    pub fn len(&self) -> usize {
        fn count(items: &[Item]) -> usize {
            items
                .iter()
                .map(|i| match i {
                    Item::Clip { items, .. } => 1 + count(items),
                    _ => 1,
                })
                .sum()
        }
        count(&self.items)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// A minimal PNG (8-bit RGBA, one zlib stream) for embedding rasters.
pub(crate) fn png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for b in data {
            c ^= u32::from(*b);
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xedb8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
        }
        !c
    }
    let chunk = |out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &header);
    let row = width as usize * 4;
    let mut raw = Vec::with_capacity((row + 1) * height as usize);
    for r in rgba.chunks_exact(row) {
        raw.push(0); // no filter
        raw.extend_from_slice(r);
    }
    chunk(
        &mut out,
        b"IDAT",
        &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6),
    );
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Standard base64 (for data URIs).
pub(crate) fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            s.push(if i <= c.len() {
                char::from(T[(n >> shift & 63) as usize])
            } else {
                '='
            });
        }
    }
    s
}

/// A number for a file: at most three decimals (a micrometre), no trailing zeros.
fn num(v: f64) -> String {
    let s = format!("{:.3}", if v.abs() < 5e-4 { 0.0 } else { v });
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn sample() -> Plot {
        let mut plot = Plot::new(420.0, 297.0, "Test (plan)");
        plot.push(Item::rect(
            [10.0, 10.0],
            [410.0, 287.0],
            Some(Pen::solid(BLACK, 0.5)),
            None,
        ));
        plot.push(Item::Clip {
            rings: vec![
                rect_ring([10.0, 10.0], [410.0, 287.0]),
                rect_ring([230.0, 10.0], [410.0, 52.0]),
            ],
            items: vec![
                Item::line(
                    vec![[20.0, 20.0], [400.0, 280.0]],
                    Pen {
                        color: [200, 0, 0],
                        width: 0.35,
                        dash: vec![2.0, 1.0],
                    },
                ),
                Item::Circle {
                    center: [100.0, 100.0],
                    radius: 1.0,
                    pen: None,
                    fill: Some(BLACK),
                },
            ],
        });
        plot.push(Item::Text(
            Text::new([100.0, 100.0], "Lot 12 – 45°30′15″ (a\\b)", 2.5)
                .align(Align::Centre, VAlign::Middle)
                .angle(0.3)
                .halo(),
        ));
        plot
    }

    #[test]
    fn numbers_are_compact() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.25), "0.25");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(num(12.34567), "12.346");
    }

    #[test]
    fn text_alignment_moves_the_origin() {
        let t = Text::new([50.0, 50.0], "ABC", 10.0).align(Align::Right, VAlign::Top);
        let w = t.width();
        assert!((w - 10.0 * (0.667 + 0.667 + 0.722)).abs() < 1e-9);
        let [x, y] = t.origin();
        assert!((x - (50.0 - w)).abs() < 1e-9);
        assert!((y - (50.0 - CAP_HEIGHT * 10.0)).abs() < 1e-9);
        // Rotated a quarter turn, "right" runs up the page.
        let t = Text::new([0.0, 0.0], "ABC", 10.0)
            .align(Align::Right, VAlign::Baseline)
            .angle(std::f64::consts::FRAC_PI_2);
        let [x, y] = t.origin();
        assert!(x.abs() < 1e-9 && (y + w).abs() < 1e-9);
    }

    #[test]
    fn pdf_is_well_formed() {
        let bytes = write_pdf(&sample());
        // Binary bytes as '.', so string offsets are file offsets.
        let text: String = bytes
            .iter()
            .map(|&b| if b.is_ascii() { char::from(b) } else { '.' })
            .collect();
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.trim_end().ends_with("%%EOF"));
        // Every xref offset points at its object.
        let start: usize = text
            .rsplit("startxref")
            .next()
            .and_then(|t| t.split_whitespace().next())
            .and_then(|t| t.parse().ok())
            .expect("startxref");
        assert!(text[start..].starts_with("xref"));
        let entries: Vec<usize> = text[start..]
            .lines()
            .skip(3)
            .take_while(|l| l.ends_with(" n "))
            .map(|l| l[..10].parse().expect("offset"))
            .collect();
        assert_eq!(entries.len(), 7);
        for (i, off) in entries.iter().enumerate() {
            assert!(
                text[*off..].starts_with(&format!("{} 0 obj", i + 1)),
                "object {}",
                i + 1
            );
        }
        assert!(text.contains("/MediaBox [0 0 1190.551 841.89]"));
        assert!(text.contains("/Title (Test \\(plan\\))"));
    }

    #[test]
    fn svg_flips_y_and_escapes() {
        let svg = write_svg(&sample());
        assert!(svg.starts_with("<?xml"));
        assert!(svg.contains(r#"width="420mm" height="297mm" viewBox="0 0 420 297""#));
        // y is measured down from the top in SVG.
        assert!(svg.contains("M10 287"));
        assert!(svg.contains("clip-rule=\"evenodd\""));
        assert!(svg.contains("Lot 12 – 45°30′15″ (a\\b)"));
        assert!(svg.contains("stroke-dasharray=\"2 1\""));
        assert!(svg.ends_with("</svg>\n"));
    }
}
