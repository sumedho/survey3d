//! SVG writer for [`Plot`]s: one user unit is a millimetre, y flipped to SVG's
//! top-down axis.

use std::fmt::Write as _;

use super::{Item, Pen, Plot, Rgb, num};

const FONT: &str = "Helvetica, Arial, sans-serif";

fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn escape(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            _ => s.push(c),
        }
    }
    s
}

struct Writer {
    out: String,
    height: f64,
    next_clip: usize,
}

impl Writer {
    fn y(&self, y: f64) -> f64 {
        self.height - y
    }

    fn path_data(&self, points: &[[f64; 2]], closed: bool) -> String {
        let mut d = String::new();
        for (i, p) in points.iter().enumerate() {
            let _ = write!(
                d,
                "{}{} {}",
                if i == 0 { "M" } else { "L" },
                num(p[0]),
                num(self.y(p[1]))
            );
        }
        if closed {
            d.push('Z');
        }
        d
    }

    fn paint(pen: Option<&Pen>, fill: Option<Rgb>) -> String {
        let mut a = format!(" fill=\"{}\"", fill.map_or("none".into(), hex));
        if let Some(p) = pen {
            let _ = write!(
                a,
                " stroke=\"{}\" stroke-width=\"{}\"",
                hex(p.color),
                num(p.width)
            );
            if !p.dash.is_empty() {
                let dash: Vec<String> = p.dash.iter().map(|d| num(*d)).collect();
                let _ = write!(a, " stroke-dasharray=\"{}\"", dash.join(" "));
            }
        }
        a
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
                    let d = self.path_data(points, *closed);
                    let _ = writeln!(self.out, "<path d=\"{d}\"{}/>", Self::paint(pen.as_ref(), *fill));
                }
                Item::Circle {
                    center,
                    radius,
                    pen,
                    fill,
                } => {
                    let _ = writeln!(
                        self.out,
                        "<circle cx=\"{}\" cy=\"{}\" r=\"{}\"{}/>",
                        num(center[0]),
                        num(self.y(center[1])),
                        num(*radius),
                        Self::paint(pen.as_ref(), *fill)
                    );
                }
                Item::Text(t) => {
                    let [x, y] = t.origin();
                    let (x, y) = (num(x), num(self.y(y)));
                    let mut attrs = format!(
                        "x=\"{x}\" y=\"{y}\" font-size=\"{}\" fill=\"{}\"",
                        num(t.size),
                        hex(t.color)
                    );
                    if t.bold {
                        attrs.push_str(" font-weight=\"bold\"");
                    }
                    if t.angle != 0.0 {
                        let _ = write!(
                            attrs,
                            " transform=\"rotate({} {x} {y})\"",
                            num(-t.angle.to_degrees())
                        );
                    }
                    if t.halo {
                        let _ = write!(
                            attrs,
                            " stroke=\"#ffffff\" stroke-width=\"{}\" paint-order=\"stroke\"",
                            num(t.size * 0.28)
                        );
                    }
                    let _ = writeln!(self.out, "<text {attrs}>{}</text>", escape(&t.text));
                }
                Item::Image {
                    min,
                    max,
                    width,
                    height,
                    rgba,
                } => {
                    let data = super::base64(&super::png(rgba, *width, *height));
                    let _ = writeln!(
                        self.out,
                        "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" href=\"data:image/png;base64,{data}\"/>",
                        num(min[0]),
                        num(self.y(max[1])),
                        num(max[0] - min[0]),
                        num(max[1] - min[1]),
                    );
                }
                Item::Clip { rings, items } => {
                    self.next_clip += 1;
                    let id = format!("clip{}", self.next_clip);
                    let d: String = rings.iter().map(|r| self.path_data(r, true)).collect();
                    let _ = writeln!(
                        self.out,
                        "<clipPath id=\"{id}\"><path d=\"{d}\" clip-rule=\"evenodd\"/></clipPath>\n<g clip-path=\"url(#{id})\">"
                    );
                    self.items(items);
                    self.out.push_str("</g>\n");
                }
            }
        }
    }
}

/// The plot as an SVG document sized in millimetres.
pub fn write_svg(plot: &Plot) -> String {
    let (w, h) = (num(plot.width), num(plot.height));
    let mut writer = Writer {
        out: String::new(),
        height: plot.height,
        next_clip: 0,
    };
    let _ = writeln!(
        writer.out,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}mm\" height=\"{h}mm\" viewBox=\"0 0 {w} {h}\">\n\
         <title>{}</title>\n\
         <rect width=\"{w}\" height=\"{h}\" fill=\"#ffffff\"/>\n\
         <g stroke-linecap=\"round\" stroke-linejoin=\"round\" font-family=\"{FONT}\">",
        escape(&plot.title)
    );
    writer.items(&plot.items);
    writer.out.push_str("</g>\n</svg>\n");
    writer.out
}
