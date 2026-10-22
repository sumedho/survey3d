//! A one-page PDF 1.4 writer for [`Plot`]s.
//!
//! The content stream is in millimetres (one `cm` scales to points) and is
//! Flate-compressed. Fonts are the base-14 Helvetica pair with WinAnsi
//! encoding, so nothing has to be embedded.

use std::fmt::Write as _;

use super::font::win_ansi;
use super::{Item, Pen, Plot, Rgb, Text, num};

const PT_PER_MM: f64 = 72.0 / 25.4;
/// Bézier handle length for a quarter circle.
const KAPPA: f64 = 0.552_284_749_8;

/// Graphics state already set in the stream, so repeated items do not repeat it.
#[derive(Default)]
struct State {
    stroke: Option<Rgb>,
    fill: Option<Rgb>,
    width: Option<f64>,
    dash: Option<Vec<f64>>,
}

struct Writer {
    out: String,
    state: State,
}

fn rgb(c: Rgb) -> String {
    let f = |v: u8| num(f64::from(v) / 255.0);
    format!("{} {} {}", f(c[0]), f(c[1]), f(c[2]))
}

impl Writer {
    fn stroke_color(&mut self, c: Rgb) {
        if self.state.stroke != Some(c) {
            let _ = writeln!(self.out, "{} RG", rgb(c));
            self.state.stroke = Some(c);
        }
    }

    fn fill_color(&mut self, c: Rgb) {
        if self.state.fill != Some(c) {
            let _ = writeln!(self.out, "{} rg", rgb(c));
            self.state.fill = Some(c);
        }
    }

    fn pen(&mut self, pen: &Pen) {
        self.stroke_color(pen.color);
        if self.state.width != Some(pen.width) {
            let _ = writeln!(self.out, "{} w", num(pen.width));
            self.state.width = Some(pen.width);
        }
        if self.state.dash.as_ref() != Some(&pen.dash) {
            let dash: Vec<String> = pen.dash.iter().map(|d| num(*d)).collect();
            let _ = writeln!(self.out, "[{}] 0 d", dash.join(" "));
            self.state.dash = Some(pen.dash.clone());
        }
    }

    fn ring(&mut self, points: &[[f64; 2]], closed: bool) {
        for (i, p) in points.iter().enumerate() {
            let op = if i == 0 { "m" } else { "l" };
            let _ = writeln!(self.out, "{} {} {op}", num(p[0]), num(p[1]));
        }
        if closed {
            self.out.push_str("h\n");
        }
    }

    fn circle(&mut self, c: [f64; 2], r: f64) {
        let k = r * KAPPA;
        let [x, y] = c;
        let p = |a: f64, b: f64| format!("{} {}", num(a), num(b));
        let _ = writeln!(self.out, "{} m", p(x + r, y));
        let _ = writeln!(
            self.out,
            "{} {} {} c",
            p(x + r, y + k),
            p(x + k, y + r),
            p(x, y + r)
        );
        let _ = writeln!(
            self.out,
            "{} {} {} c",
            p(x - k, y + r),
            p(x - r, y + k),
            p(x - r, y)
        );
        let _ = writeln!(
            self.out,
            "{} {} {} c",
            p(x - r, y - k),
            p(x - k, y - r),
            p(x, y - r)
        );
        let _ = writeln!(
            self.out,
            "{} {} {} c",
            p(x + k, y - r),
            p(x + r, y - k),
            p(x + r, y)
        );
        self.out.push_str("h\n");
    }

    fn paint(&mut self, pen: Option<&Pen>, fill: Option<Rgb>) {
        if let Some(f) = fill {
            self.fill_color(f);
        }
        if let Some(p) = pen {
            self.pen(p);
        }
        let op = match (pen.is_some(), fill.is_some()) {
            (true, true) => "B",
            (true, false) => "S",
            (false, true) => "f",
            (false, false) => "n",
        };
        let _ = writeln!(self.out, "{op}");
    }

    fn text(&mut self, t: &Text) {
        let [x, y] = t.origin();
        let (s, c) = t.angle.sin_cos();
        let font = if t.bold { "F2" } else { "F1" };
        let string = pdf_string(&t.text);
        let matrix = format!(
            "{} {} {} {} {} {}",
            num(c),
            num(s),
            num(-s),
            num(c),
            num(x),
            num(y)
        );
        if t.halo {
            // White outline first, then the letters on top.
            let _ = writeln!(
                self.out,
                "q 1 1 1 RG {} w 1 j BT /{font} {} Tf 1 Tr {matrix} Tm {string} Tj ET Q",
                num(t.size * 0.28),
                num(t.size)
            );
        }
        self.fill_color(t.color);
        let _ = writeln!(
            self.out,
            "BT /{font} {} Tf {matrix} Tm {string} Tj ET",
            num(t.size)
        );
    }

    fn items(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Path {
                    points,
                    closed,
                    pen,
                    fill,
                } => {
                    if points.len() < 2 {
                        continue;
                    }
                    self.ring(points, *closed);
                    self.paint(pen.as_ref(), *fill);
                }
                Item::Circle {
                    center,
                    radius,
                    pen,
                    fill,
                } => {
                    self.circle(*center, *radius);
                    self.paint(pen.as_ref(), *fill);
                }
                Item::Text(t) => self.text(t),
                Item::Clip { rings, items } => {
                    self.out.push_str("q\n");
                    for r in rings {
                        self.ring(r, true);
                    }
                    self.out.push_str("W* n\n");
                    let outer = std::mem::take(&mut self.state);
                    self.items(items);
                    self.out.push_str("Q\n");
                    // `Q` restores what was set before the group.
                    self.state = outer;
                }
            }
        }
    }
}

/// A PDF literal string in WinAnsi encoding.
fn pdf_string(text: &str) -> String {
    let mut s = String::from("(");
    for c in text.chars() {
        match win_ansi(c) {
            b'(' => s.push_str("\\("),
            b')' => s.push_str("\\)"),
            b'\\' => s.push_str("\\\\"),
            b @ 32..=126 => s.push(char::from(b)),
            b => {
                let _ = write!(s, "\\{b:03o}");
            }
        }
    }
    s.push(')');
    s
}

/// The plot as a PDF file.
pub fn write_pdf(plot: &Plot) -> Vec<u8> {
    let mut w = Writer {
        out: String::new(),
        state: State::default(),
    };
    let _ = writeln!(w.out, "{} 0 0 {} 0 0 cm", num(PT_PER_MM), num(PT_PER_MM));
    w.out.push_str("1 J 1 j\n");
    w.items(&plot.items);
    let content = miniz_oxide::deflate::compress_to_vec_zlib(w.out.as_bytes(), 6);

    let (pw, ph) = (num(plot.width * PT_PER_MM), num(plot.height * PT_PER_MM));
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    let mut object = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    object(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    object(&mut out, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    object(
        &mut out,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw} {ph}] \
             /Resources << /Font << /F1 5 0 R /F2 6 0 R >> >> /Contents 4 0 R >>"
        )
        .as_bytes(),
    );
    let mut stream = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(&content);
    stream.extend_from_slice(b"\nendstream");
    object(&mut out, &stream);
    for name in ["Helvetica", "Helvetica-Bold"] {
        object(
            &mut out,
            format!("<< /Type /Font /Subtype /Type1 /BaseFont /{name} /Encoding /WinAnsiEncoding >>")
                .as_bytes(),
        );
    }
    let mut info = b"<< /Title ".to_vec();
    info.extend(pdf_string(&plot.title).chars().map(|c| c as u8));
    info.extend_from_slice(b" /Producer (survey3d) >>");
    object(&mut out, &info);

    let xref = out.len();
    let mut table = format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1);
    for o in &offsets {
        let _ = writeln!(table, "{o:010} 00000 n ");
    }
    let _ = write!(
        table,
        "trailer\n<< /Size {} /Root 1 0 R /Info {} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        offsets.len() + 1,
        offsets.len()
    );
    out.extend_from_slice(table.as_bytes());
    out
}
