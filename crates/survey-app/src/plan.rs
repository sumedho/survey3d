//! Plan sheets as paper-space plots: the drawing at a true scale inside a
//! frame, with a coordinate grid, north arrow, scale bar, legend and title block.
//!
//! The preview in the sheets window and the PDF/SVG files are drawn from the
//! same [`Plot`], so the preview is what gets printed.

use std::collections::{BTreeMap, HashMap, HashSet};

use glam::DVec2;
use survey_core::style::{CodeStyle, LabelSet, Prim, Symbol, outline};
use survey_io::plot::{
    Align, BLACK, CAP_HEIGHT, Item, Pen, Plot, Rgb, Text, VAlign, WHITE, rect_ring, text_width,
};
use survey_model::sheet::{GridMarks, SheetRecord};
use survey_model::{Document, Entity, EntityId, Polyline};

use crate::scene_build::{LINE_WIDTH, POINT_SIZE};

const GREY: Rgb = [100, 100, 100];
const GRID_LINE: Rgb = [185, 185, 185];
const CONTOUR_MINOR: Rgb = [176, 128, 80];
const CONTOUR_MAJOR: Rgb = [140, 90, 45];
/// Arc tessellation tolerance on paper (millimetres).
const PAPER_TOLERANCE: f64 = 0.02;

/// An axis-aligned rectangle on the page (millimetres, y up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

impl Rect {
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self {
            min: [x0.min(x1), y0.min(y1)],
            max: [x0.max(x1), y0.max(y1)],
        }
    }

    pub fn width(&self) -> f64 {
        self.max[0] - self.min[0]
    }

    pub fn height(&self) -> f64 {
        self.max[1] - self.min[1]
    }

    pub fn centre(&self) -> [f64; 2] {
        [
            (self.min[0] + self.max[0]) / 2.0,
            (self.min[1] + self.max[1]) / 2.0,
        ]
    }

    fn contains(&self, p: [f64; 2]) -> bool {
        (self.min[0]..=self.max[0]).contains(&p[0]) && (self.min[1]..=self.max[1]).contains(&p[1])
    }

    fn contains_rect(&self, r: &Rect) -> bool {
        self.contains(r.min) && self.contains(r.max)
    }

    fn intersects(&self, r: &Rect) -> bool {
        self.min[0] < r.max[0] && r.min[0] < self.max[0] && self.min[1] < r.max[1] && r.min[1] < self.max[1]
    }

    fn inset(&self, d: f64) -> Rect {
        Rect::new(self.min[0] + d, self.min[1] + d, self.max[0] - d, self.max[1] - d)
    }

    fn ring(&self) -> Vec<[f64; 2]> {
        rect_ring(self.min, self.max)
    }

    fn bounding(points: &[[f64; 2]]) -> Rect {
        let mut r = Rect {
            min: [f64::MAX; 2],
            max: [f64::MIN; 2],
        };
        for p in points {
            r.min = [r.min[0].min(p[0]), r.min[1].min(p[1])];
            r.max = [r.max[0].max(p[0]), r.max[1].max(p[1])];
        }
        r
    }
}

/// Where things go on the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetLayout {
    pub page: [f64; 2],
    /// The drawing border.
    pub border: Rect,
    /// The map frame (the drawing is clipped to it, less the boxes inside it).
    pub frame: Rect,
    pub title: Rect,
    /// The title block is a strip under the frame (narrow pages) rather than
    /// a box in its bottom-right corner.
    pub strip: bool,
}

const TITLE_WIDTH: f64 = 180.0;

pub fn layout(sheet: &SheetRecord) -> SheetLayout {
    let page = sheet.page_size();
    let margin = if page[0].max(page[1]) > 800.0 { 15.0 } else { 10.0 };
    let border = Rect::new(margin, margin, page[0] - margin, page[1] - margin);
    if border.width() * 0.6 >= TITLE_WIDTH {
        let title = Rect::new(
            border.max[0] - TITLE_WIDTH,
            border.min[1],
            border.max[0],
            border.min[1] + 42.0,
        );
        SheetLayout {
            page,
            border,
            frame: border,
            title,
            strip: false,
        }
    } else {
        let title = Rect::new(border.min[0], border.min[1], border.max[0], border.min[1] + 36.0);
        let frame = Rect::new(border.min[0], title.max[1], border.max[0], border.max[1]);
        SheetLayout {
            page,
            border,
            frame,
            title,
            strip: true,
        }
    }
}

/// World plan coordinates ↔ page millimetres for a sheet.
#[derive(Clone, Copy, Debug)]
pub struct SheetTransform {
    centre: DVec2,
    paper: [f64; 2],
    /// Millimetres per metre.
    k: f64,
    sin: f64,
    cos: f64,
}

impl SheetTransform {
    pub fn new(sheet: &SheetRecord, frame: &Rect) -> Self {
        // Turning the world anticlockwise by the rotation brings that bearing up the page.
        let (sin, cos) = sheet.rotation.to_radians().sin_cos();
        Self {
            centre: DVec2::from(sheet.centre),
            paper: frame.centre(),
            k: sheet.mm_per_metre(),
            sin,
            cos,
        }
    }

    pub fn to_paper(&self, w: DVec2) -> [f64; 2] {
        let d = (w - self.centre) * self.k;
        [
            self.paper[0] + d.x * self.cos - d.y * self.sin,
            self.paper[1] + d.x * self.sin + d.y * self.cos,
        ]
    }

    pub fn to_world(&self, p: [f64; 2]) -> DVec2 {
        let (dx, dy) = (p[0] - self.paper[0], p[1] - self.paper[1]);
        self.centre + DVec2::new(dx * self.cos + dy * self.sin, -dx * self.sin + dy * self.cos) / self.k
    }

    /// A world direction on the page (unit length in, unit length out).
    fn dir(&self, d: DVec2) -> [f64; 2] {
        [d.x * self.cos - d.y * self.sin, d.x * self.sin + d.y * self.cos]
    }
}

/// The corners of a sheet's map frame in world coordinates (for drawing it in the viewport).
pub fn frame_corners(sheet: &SheetRecord) -> [DVec2; 4] {
    let lay = layout(sheet);
    let t = SheetTransform::new(sheet, &lay.frame);
    let f = lay.frame;
    [
        t.to_world(f.min),
        t.to_world([f.max[0], f.min[1]]),
        t.to_world(f.max),
        t.to_world([f.min[0], f.max[1]]),
    ]
}

/// The map frame's size in metres at the sheet's scale.
pub fn frame_size_world(sheet: &SheetRecord) -> DVec2 {
    let f = layout(sheet).frame;
    DVec2::new(f.width(), f.height()) / sheet.mm_per_metre()
}

/// A layer colour on white paper: white becomes black and very light colours
/// are darkened, as on the light canvas.
pub fn paper_color(c: [u8; 3], mono: bool) -> Rgb {
    if mono || (c[0] > 200 && c[1] > 200 && c[2] > 200) {
        return BLACK;
    }
    let luma = 0.2126 * f64::from(c[0]) + 0.7152 * f64::from(c[1]) + 0.0722 * f64::from(c[2]);
    if luma > 140.0 {
        let k = 110.0 / luma;
        c.map(|v| (f64::from(v) * k).round() as u8)
    } else {
        c
    }
}

/// 1, 2 or 5 × 10ⁿ: the largest not above `v`.
fn nice_floor(v: f64) -> f64 {
    if v <= 0.0 || !v.is_finite() {
        return 1.0;
    }
    let p = 10f64.powf(v.log10().floor());
    [5.0, 2.0, 1.0]
        .into_iter()
        .map(|m| m * p)
        .find(|s| *s <= v * (1.0 + 1e-9))
        .unwrap_or(p)
}

/// 1, 2, 2.5 or 5 × 10ⁿ: the smallest not below `v`.
fn nice_ceil(v: f64) -> f64 {
    if v <= 0.0 || !v.is_finite() {
        return 1.0;
    }
    let p = 10f64.powf(v.log10().floor());
    [1.0, 2.0, 2.5, 5.0, 10.0]
        .into_iter()
        .map(|m| m * p)
        .find(|s| *s >= v * (1.0 - 1e-9))
        .unwrap_or(10.0 * p)
}

/// The grid spacing a sheet uses (metres).
pub fn grid_interval(sheet: &SheetRecord) -> f64 {
    if sheet.grid_interval > 0.0 {
        sheet.grid_interval
    } else {
        nice_ceil(40.0 / sheet.mm_per_metre())
    }
}

/// `v` without trailing zeros.
fn trim_number(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Rotation that keeps text reading left to right.
fn upright(angle: f64) -> (f64, bool) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let a = (angle + PI).rem_euclid(2.0 * PI) - PI;
    if a > FRAC_PI_2 + 1e-9 {
        (a - PI, true)
    } else if a <= -FRAC_PI_2 + 1e-9 {
        (a + PI, true)
    } else {
        (a, false)
    }
}

/// Font size that fits `text` in `width`, from `size` down to `min`.
fn fit(text: &str, size: f64, width: f64, bold: bool) -> f64 {
    let w = text_width(text, size, bold);
    if w <= width {
        size
    } else {
        (size * width / w).max(size.min(1.2))
    }
}

/// Greedy word wrap.
fn wrap(text: &str, size: f64, width: f64) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && text_width(&candidate, size, false) > width {
                lines.push(std::mem::replace(&mut line, word.to_string()));
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    lines
}

/// Rectangles taken by text and symbols, so labels do not overlap.
struct Placer {
    rects: Vec<Rect>,
    cells: HashMap<(i32, i32), Vec<usize>>,
    /// Labels must stay inside.
    area: Rect,
}

const CELL: f64 = 8.0;

impl Placer {
    fn cells(r: &Rect) -> impl Iterator<Item = (i32, i32)> {
        let c = |v: f64| (v / CELL).floor() as i32;
        let (x0, x1, y0, y1) = (c(r.min[0]), c(r.max[0]), c(r.min[1]), c(r.max[1]));
        (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
    }

    fn is_free(&self, r: &Rect) -> bool {
        self.area.contains_rect(r)
            && Self::cells(r).all(|cell| {
                self.cells
                    .get(&cell)
                    .is_none_or(|ids| ids.iter().all(|i| !self.rects[*i].intersects(r)))
            })
    }

    fn add(&mut self, r: Rect) {
        let i = self.rects.len();
        self.rects.push(r);
        for cell in Self::cells(&r) {
            self.cells.entry(cell).or_default().push(i);
        }
    }

    /// Adds `r` if it is free.
    fn take(&mut self, r: Rect) -> bool {
        let free = self.is_free(&r);
        if free {
            self.add(r);
        }
        free
    }
}

/// A feature code's style as drawn: its index in the library, style and colour.
type Resolved = Option<(usize, CodeStyle, [u8; 3])>;

struct Builder<'a> {
    doc: &'a Document,
    sheet: &'a SheetRecord,
    t: SheetTransform,
    frame: Rect,
    mono: bool,
    styles: HashMap<&'a str, Resolved>,
    placer: Placer,
    /// World tessellation tolerance.
    tolerance: f64,
}

impl<'a> Builder<'a> {
    fn style(&mut self, code: &'a str) -> Resolved {
        if code.is_empty() {
            return None;
        }
        let doc = self.doc;
        *self.styles.entry(code).or_insert_with(|| {
            let library = &doc.meta().feature_codes;
            let f = library.feature_for(code)?;
            let index = library.codes.iter().position(|c| std::ptr::eq(c, f))?;
            let color = match doc.layer_by_name(&f.layer) {
                Some(layer) if f.style.by_layer => [layer.color.0[0], layer.color.0[1], layer.color.0[2]],
                _ => f.color,
            };
            Some((index, f.style, color))
        })
    }

    fn color(&self, c: [u8; 3]) -> Rgb {
        paper_color(c, self.mono)
    }

    fn entity_color(&mut self, layer: survey_model::LayerId, code: &'a str) -> Rgb {
        let c = match self.style(code) {
            Some((_, _, c)) => c,
            None => self
                .doc
                .layer(layer)
                .map_or([0, 0, 0], |l| [l.color.0[0], l.color.0[1], l.color.0[2]]),
        };
        self.color(c)
    }

    fn paper_line(&self, l: &Polyline) -> Vec<[f64; 2]> {
        let mut pts: Vec<[f64; 2]> = Vec::new();
        for part in l.tessellate(self.tolerance) {
            for (i, p) in part.iter().enumerate() {
                if i == 0 && !pts.is_empty() {
                    continue;
                }
                pts.push(self.t.to_paper(p.truncate()));
            }
        }
        pts
    }

    fn polyline_pen(&mut self, l: &'a Polyline) -> Pen {
        let color = self.entity_color(l.layer, &l.code);
        let (line_style, weight) = self
            .style(&l.code)
            .map_or((Default::default(), LINE_WIDTH), |(_, s, _)| {
                (s.line_style, s.line_weight)
            });
        let width = f64::from(weight) * self.sheet.line_scale;
        let unit = width.max(0.2);
        Pen {
            color,
            width,
            dash: line_style
                .pattern()
                .iter()
                .map(|v| f64::from(*v) * unit)
                .collect(),
        }
    }

    fn linework(&mut self, ids: &[EntityId], out: &mut Vec<Item>) {
        for id in ids {
            let Some(Entity::Polyline(l)) = self.doc.entity(*id) else {
                continue;
            };
            if !self.doc.is_layer_visible(l.layer) {
                continue;
            }
            let pen = self.polyline_pen(l);
            let points = self.paper_line(l);
            out.push(Item::Path {
                points,
                closed: false,
                pen: Some(pen),
                fill: None,
            });
        }
    }

    fn symbol(out: &mut Vec<Item>, at: [f64; 2], r: f64, symbol: Symbol, color: Rgb) {
        let pen = Pen::solid(color, (r * 0.17).max(0.1));
        let p = |c: [f32; 2]| [at[0] + f64::from(c[0]) * r, at[1] + f64::from(c[1]) * r];
        let paint = |fill: bool| {
            if fill {
                (None, Some(color))
            } else {
                (Some(pen.clone()), None)
            }
        };
        for prim in symbol.prims() {
            match *prim {
                Prim::Circle { c, r: cr, fill } => {
                    let (pen, fill) = paint(fill);
                    out.push(Item::Circle {
                        center: p(c),
                        radius: f64::from(cr) * r,
                        pen,
                        fill,
                    });
                }
                Prim::Square { c, h, fill } => {
                    let (pen, fill) = paint(fill);
                    out.push(Item::rect(
                        p([c[0] - h, c[1] - h]),
                        p([c[0] + h, c[1] + h]),
                        pen,
                        fill,
                    ));
                }
                Prim::Polygon { fill, .. } | Prim::Star { fill, .. } => {
                    let (pen, fill) = paint(fill);
                    out.push(Item::Path {
                        points: outline(prim).into_iter().map(p).collect(),
                        closed: true,
                        pen,
                        fill,
                    });
                }
                Prim::Segment { a, b } => out.push(Item::line(vec![p(a), p(b)], pen.clone())),
            }
        }
    }

    fn point_symbol(&mut self, code: &'a str, height_known: bool) -> (Symbol, f64) {
        let (symbol, size) = match self.style(code) {
            Some((_, s, _)) => (s.symbol, s.symbol_size),
            None if height_known => (Symbol::Dot, POINT_SIZE),
            None => (Symbol::Square, POINT_SIZE),
        };
        (symbol, f64::from(size) / 2.0 * self.sheet.symbol_scale)
    }

    /// Symbols, reserving their space for labels.
    fn points(&mut self, ids: &[EntityId], out: &mut Vec<Item>) {
        for id in ids {
            let Some(Entity::Point(p)) = self.doc.entity(*id) else {
                continue;
            };
            if !self.doc.is_layer_visible(p.layer) {
                continue;
            }
            let at = self.t.to_paper(p.position.truncate());
            if !self.frame.contains(at) {
                continue;
            }
            let (symbol, r) = self.point_symbol(&p.code, p.height_known);
            let color = self.entity_color(p.layer, &p.code);
            Self::symbol(out, at, r, symbol, color);
            self.placer
                .add(Rect::new(at[0] - r, at[1] - r, at[0] + r, at[1] + r));
        }
    }

    fn point_labels(&mut self, ids: &[EntityId], global: LabelSet, out: &mut Vec<Item>) {
        let conv = self.doc.conventions().clone();
        let size = self.sheet.text_size;
        let small = size * 0.85;
        for id in ids {
            let Some(Entity::Point(p)) = self.doc.entity(*id) else {
                continue;
            };
            if !self.doc.is_layer_visible(p.layer) {
                continue;
            }
            let at = self.t.to_paper(p.position.truncate());
            if !self.frame.contains(at) {
                continue;
            }
            let set = self
                .style(&p.code)
                .and_then(|(_, s, _)| s.labels)
                .unwrap_or(global);
            let color = self.entity_color(p.layer, &p.code);
            let mut lines: Vec<(String, f64, Rgb)> = Vec::new();
            if set.name && !p.name.is_empty() {
                lines.push((p.name.clone(), size, BLACK));
            }
            if set.code && !p.code.is_empty() {
                lines.push((p.code.clone(), small, color));
            }
            if set.height && p.height_known {
                lines.push((conv.format_length(p.position.z), small, GREY));
            }
            if set.description && !p.description.is_empty() {
                lines.push((p.description.clone(), small, GREY));
            }
            if lines.is_empty() {
                continue;
            }
            let width = lines
                .iter()
                .map(|(t, s, _)| text_width(t, *s, false))
                .fold(0.0, f64::max);
            let height: f64 =
                lines.iter().map(|(_, s, _)| s * 1.15).sum::<f64>() - 0.15 * lines[lines.len() - 1].1;
            let (_, r) = self.point_symbol(&p.code, p.height_known);
            let gap = r + 0.6;
            let d = r * 0.75 + 0.4;
            let candidates = [
                Rect::new(
                    at[0] + gap,
                    at[1] - height / 2.0,
                    at[0] + gap + width,
                    at[1] + height / 2.0,
                ),
                Rect::new(at[0] + d, at[1] + d, at[0] + d + width, at[1] + d + height),
                Rect::new(at[0] + d, at[1] - d - height, at[0] + d + width, at[1] - d),
                Rect::new(
                    at[0] - gap - width,
                    at[1] - height / 2.0,
                    at[0] - gap,
                    at[1] + height / 2.0,
                ),
                Rect::new(at[0] - d - width, at[1] + d, at[0] - d, at[1] + d + height),
                Rect::new(at[0] - d - width, at[1] - d - height, at[0] - d, at[1] - d),
            ];
            let Some(rect) = candidates.into_iter().find(|r| self.placer.take(*r)) else {
                continue;
            };
            let mut y = rect.max[1];
            for (text, s, color) in lines {
                out.push(Item::Text(
                    Text::new([rect.min[0], y], text, s)
                        .align(Align::Left, VAlign::Top)
                        .color(color)
                        .halo(),
                ));
                y -= s * 1.15;
            }
        }
    }

    /// Text along a direction on the paper, kept upright; placed only where
    /// there is room (when `check`).
    fn along_text(&mut self, text: Text, check: bool, out: &mut Vec<Item>) -> bool {
        let r = Rect::bounding(&text.corners());
        if check && !self.placer.take(r) {
            return false;
        }
        out.push(Item::Text(text));
        true
    }

    fn parcels(&mut self, out: &mut Vec<Item>) {
        let doc = self.doc;
        let conv = doc.conventions().clone();
        let size = self.sheet.text_size;
        let color = if self.mono { BLACK } else { [30, 30, 30] };
        let mut done: HashSet<[(i64, i64); 2]> = HashSet::new();
        let key = |p: glam::DVec3| ((p.x * 1000.0).round() as i64, (p.y * 1000.0).round() as i64);
        for parcel in doc.meta().parcels.iter().filter(|p| p.annotate) {
            let Some(l) = crate::ui::parcels::boundary(doc, parcel) else {
                continue;
            };
            if !doc.is_layer_visible(l.layer) {
                continue;
            }
            for (a, b, bulge) in l.segments() {
                let (ka, kb) = (key(a), key(b));
                if !done.insert(if ka < kb { [ka, kb] } else { [kb, ka] }) {
                    continue;
                }
                let (mid, tangent, top, bottom) = match survey_core::arc::Arc::from_bulge(a, b, bulge) {
                    Some(arc) => (
                        arc.midpoint().truncate(),
                        arc.tangent_at(0.5),
                        format!("R {}", conv.format_length(arc.radius)),
                        format!("A {}", conv.format_length(arc.length_2d())),
                    ),
                    None => {
                        let d = (b - a).truncate();
                        let Some(brg) = survey_core::Bearing::from_delta(d.x, d.y) else {
                            continue;
                        };
                        (
                            ((a + b) * 0.5).truncate(),
                            d,
                            conv.format_bearing(brg),
                            conv.format_length(d.length()),
                        )
                    }
                };
                let m = self.t.to_paper(mid);
                if !self.frame.contains(m) {
                    continue;
                }
                let length = (b - a).truncate().length() * self.t.k;
                let width = text_width(&top, size, false).max(text_width(&bottom, size, false));
                if length < width + 2.0 {
                    continue;
                }
                let d = self.t.dir(tangent.normalize_or_zero());
                let (angle, _) = upright(d[1].atan2(d[0]));
                let up = [-angle.sin(), angle.cos()];
                let off = 0.5;
                let above = Text::new([m[0] + up[0] * off, m[1] + up[1] * off], top, size)
                    .align(Align::Centre, VAlign::Bottom)
                    .angle(angle)
                    .color(color)
                    .halo();
                let below = Text::new([m[0] - up[0] * off, m[1] - up[1] * off], bottom, size)
                    .align(Align::Centre, VAlign::Top)
                    .angle(angle)
                    .color(color)
                    .halo();
                self.along_text(above, true, out);
                self.along_text(below, true, out);
            }
            // Name and area at the area-weighted centre.
            let ring: Vec<DVec2> = l
                .tessellate(self.tolerance)
                .into_iter()
                .flatten()
                .map(|p| p.truncate())
                .collect();
            let (mut area, mut c) = (0.0, DVec2::ZERO);
            for i in 0..ring.len() {
                let (p, q) = (ring[i], ring[(i + 1) % ring.len()]);
                let cross = p.perp_dot(q);
                area += cross;
                c += (p + q) * cross;
            }
            if area.abs() < 1e-9 {
                continue;
            }
            let centre = self.t.to_paper(c / (3.0 * area));
            if !self.frame.contains(centre) {
                continue;
            }
            let name = Text::new([centre[0], centre[1] + 0.4], parcel.name.clone(), size * 1.6)
                .align(Align::Centre, VAlign::Baseline)
                .bold()
                .color(color)
                .halo();
            let area = Text::new(
                [centre[0], centre[1] - 0.6],
                conv.format_area((area / 2.0).abs()),
                size,
            )
            .align(Align::Centre, VAlign::Top)
            .color(color)
            .halo();
            self.along_text(name, false, out);
            self.along_text(area, false, out);
        }
    }

    fn chainages(&mut self, out: &mut Vec<Item>) {
        let doc = self.doc;
        let conv = doc.conventions().clone();
        let size = self.sheet.text_size * 0.9;
        let color = if self.mono { BLACK } else { [20, 90, 170] };
        let pen = Pen::solid(color, 0.2);
        for a in doc.meta().alignments.iter().filter(|a| a.show_chainages) {
            let Some(l) = crate::ui::alignments::centreline(doc, a) else {
                continue;
            };
            if !doc.is_layer_visible(l.layer) {
                continue;
            }
            let (length, _, _) = survey_core::arc::polyline_metrics(&l.vertices, &l.bulges, l.closed);
            let step = a.tick_interval.max(0.01);
            // Label often enough that labels sit about 12 mm apart.
            let every = (12.0 / (step * self.t.k)).ceil().max(1.0) as i64;
            let first = (a.start_chainage / step).ceil() as i64;
            let last = ((a.start_chainage + length) / step).floor() as i64;
            if last - first > 20_000 {
                continue;
            }
            let mut ticks: Vec<(f64, bool)> = (first..=last)
                .map(|k| (k as f64 * step, k % every == 0))
                .collect();
            ticks.push((a.start_chainage, true));
            ticks.push((a.start_chainage + length, true));
            for (ch, label) in ticks {
                let along = ch - a.start_chainage;
                let (Some(p), Some(q)) = (
                    survey_core::cogo::point_at_station(&l.vertices, &l.bulges, l.closed, along, 0.0),
                    survey_core::cogo::point_at_station(&l.vertices, &l.bulges, l.closed, along, 1.0),
                ) else {
                    continue;
                };
                let sp = self.t.to_paper(p.truncate());
                if !self.frame.contains(sp) {
                    continue;
                }
                // Towards the right of the alignment.
                let right = self.t.dir((q - p).truncate().normalize_or_zero());
                let len = if label { 1.8 } else { 1.0 };
                out.push(Item::line(
                    vec![
                        [sp[0] - right[0] * len, sp[1] - right[1] * len],
                        [sp[0] + right[0] * len, sp[1] + right[1] * len],
                    ],
                    pen.clone(),
                ));
                if label {
                    let (angle, flipped) = upright(right[1].atan2(right[0]));
                    let at = [sp[0] + right[0] * (len + 0.6), sp[1] + right[1] * (len + 0.6)];
                    let align = if flipped { Align::Right } else { Align::Left };
                    let text = Text::new(at, conv.format_chainage(ch), size)
                        .align(align, VAlign::Middle)
                        .angle(angle)
                        .color(color)
                        .halo();
                    self.along_text(text, true, out);
                }
            }
        }
    }

    fn contours(&mut self, out: &mut Vec<Item>, labels: &mut Vec<Text>) {
        let doc = self.doc;
        let conv = doc.conventions().clone();
        let (minor, major) = if self.mono {
            (BLACK, BLACK)
        } else {
            (CONTOUR_MINOR, CONTOUR_MAJOR)
        };
        let size = self.sheet.text_size * 0.85;
        for (_, s) in doc.surfaces() {
            if !s.style.visible || !s.style.contours.show || s.tin.triangles.is_empty() {
                continue;
            }
            for c in crate::surface_view::surface_contours(s) {
                let mut pts: Vec<[f64; 2]> = c.points.iter().map(|p| self.t.to_paper(p.truncate())).collect();
                if c.closed && !pts.is_empty() {
                    pts.push(pts[0]);
                }
                if pts.len() < 2 {
                    continue;
                }
                let pen = if c.major {
                    Pen::solid(major, 0.25)
                } else {
                    Pen::solid(minor, 0.13)
                };
                if c.major && s.style.contours.labels {
                    let text = trim_number(conv.format_length(c.z).parse().unwrap_or(c.z));
                    // Every 120 mm along the line, starting 40 mm in.
                    let mut next = 40.0;
                    let mut run = 0.0;
                    for w in pts.windows(2) {
                        let (a, b) = (w[0], w[1]);
                        let seg = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
                        while seg > 0.0 && run + seg >= next {
                            let f = (next - run) / seg;
                            let at = [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
                            if self.frame.contains(at) {
                                let (angle, _) = upright((b[1] - a[1]).atan2(b[0] - a[0]));
                                labels.push(
                                    Text::new(at, text.clone(), size)
                                        .align(Align::Centre, VAlign::Middle)
                                        .angle(angle)
                                        .color(major)
                                        .halo(),
                                );
                            }
                            next += 120.0;
                        }
                        run += seg;
                    }
                }
                out.push(Item::line(pts, pen));
            }
        }
    }

    /// Grid lines crossing the frame: (value, is easting, clipped ends on the page).
    fn grid(&self) -> Vec<(f64, bool, [f64; 2], [f64; 2])> {
        let interval = grid_interval(self.sheet);
        let f = self.frame;
        let corners = [
            self.t.to_world(f.min),
            self.t.to_world([f.max[0], f.min[1]]),
            self.t.to_world(f.max),
            self.t.to_world([f.min[0], f.max[1]]),
        ];
        let lo = corners.iter().copied().reduce(DVec2::min).unwrap_or_default();
        let hi = corners.iter().copied().reduce(DVec2::max).unwrap_or_default();
        let mut lines = Vec::new();
        for easting in [true, false] {
            let (a0, a1, b0, b1) = if easting {
                (lo.x, hi.x, lo.y, hi.y)
            } else {
                (lo.y, hi.y, lo.x, hi.x)
            };
            let first = (a0 / interval).ceil() as i64;
            let last = (a1 / interval).floor() as i64;
            if last - first > 400 {
                continue;
            }
            for k in first..=last {
                let v = k as f64 * interval;
                let (p, q) = if easting {
                    (DVec2::new(v, b0), DVec2::new(v, b1))
                } else {
                    (DVec2::new(b0, v), DVec2::new(b1, v))
                };
                if let Some((p, q)) = clip_segment(self.t.to_paper(p), self.t.to_paper(q), &f) {
                    lines.push((v, easting, p, q));
                }
            }
        }
        lines
    }

    fn grid_lines(&self, out: &mut Vec<Item>) {
        for (_, _, p, q) in self.grid() {
            out.push(Item::line(vec![p, q], Pen::solid(GRID_LINE, 0.13)));
        }
    }

    /// Edge ticks, values and (optionally) crosses.
    fn grid_marks(&mut self, crosses: bool, out: &mut Vec<Item>) {
        let interval = grid_interval(self.sheet);
        let conv = self.doc.conventions().clone();
        let decimals = if interval >= 1.0 { 0 } else { 2 };
        let size = self.sheet.text_size * 0.9;
        let pen = Pen::solid(BLACK, 0.25);
        let lines = self.grid();
        let f = self.frame;
        let on_edge = |p: [f64; 2]| {
            (p[0] - f.min[0]).abs() < 1e-6
                || (p[0] - f.max[0]).abs() < 1e-6
                || (p[1] - f.min[1]).abs() < 1e-6
                || (p[1] - f.max[1]).abs() < 1e-6
        };
        for (v, easting, p, q) in &lines {
            let len = ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt();
            if len < 1e-6 {
                continue;
            }
            let label = if *easting {
                &conv.axis_labels.easting
            } else {
                &conv.axis_labels.northing
            };
            let text = format!("{label} {v:.decimals$}");
            for (end, other) in [(*p, *q), (*q, *p)] {
                if !on_edge(end) {
                    continue;
                }
                let inward = [(other[0] - end[0]) / len, (other[1] - end[1]) / len];
                let tick = 2.5_f64.min(len);
                out.push(Item::line(
                    vec![end, [end[0] + inward[0] * tick, end[1] + inward[1] * tick]],
                    pen.clone(),
                ));
                let (angle, flipped) = upright(inward[1].atan2(inward[0]));
                let at = [end[0] + inward[0] * 3.2, end[1] + inward[1] * 3.2];
                let align = if flipped { Align::Right } else { Align::Left };
                let t = Text::new(at, text.clone(), size)
                    .align(align, VAlign::Middle)
                    .angle(angle)
                    .halo();
                self.along_text(t, true, out);
            }
        }
        if !crosses {
            return;
        }
        let eastings: Vec<f64> = lines.iter().filter(|l| l.1).map(|l| l.0).collect();
        let northings: Vec<f64> = lines.iter().filter(|l| !l.1).map(|l| l.0).collect();
        if eastings.len() * northings.len() > 10_000 {
            return;
        }
        let inner = self.frame.inset(3.0);
        let e = self.t.dir(DVec2::X);
        let n = self.t.dir(DVec2::Y);
        let arm = 1.5;
        let pen = Pen::solid([60, 60, 60], 0.18);
        for x in &eastings {
            for y in &northings {
                let c = self.t.to_paper(DVec2::new(*x, *y));
                if !inner.contains(c) {
                    continue;
                }
                out.push(Item::line(
                    vec![
                        [c[0] - e[0] * arm, c[1] - e[1] * arm],
                        [c[0] + e[0] * arm, c[1] + e[1] * arm],
                    ],
                    pen.clone(),
                ));
                out.push(Item::line(
                    vec![
                        [c[0] - n[0] * arm, c[1] - n[1] * arm],
                        [c[0] + n[0] * arm, c[1] + n[1] * arm],
                    ],
                    pen.clone(),
                ));
            }
        }
    }
}

/// Liang–Barsky clipping of a segment to a rectangle.
fn clip_segment(p: [f64; 2], q: [f64; 2], r: &Rect) -> Option<([f64; 2], [f64; 2])> {
    let d = [q[0] - p[0], q[1] - p[1]];
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (pk, qk) in [
        (-d[0], p[0] - r.min[0]),
        (d[0], r.max[0] - p[0]),
        (-d[1], p[1] - r.min[1]),
        (d[1], r.max[1] - p[1]),
    ] {
        if pk.abs() < 1e-12 {
            if qk < 0.0 {
                return None;
            }
            continue;
        }
        let t = qk / pk;
        if pk < 0.0 {
            t0 = t0.max(t);
        } else {
            t1 = t1.min(t);
        }
        if t0 > t1 {
            return None;
        }
    }
    let at = |t: f64| [p[0] + d[0] * t, p[1] + d[1] * t];
    Some((at(t0), at(t1)))
}

/// A legend row: the feature's index, whether it has lines in the frame, and points.
type LegendRow = (usize, bool, bool);

fn legend_rows(doc: &Document, ids: &[EntityId]) -> Vec<LegendRow> {
    let library = &doc.meta().feature_codes;
    let mut used: BTreeMap<usize, (bool, bool)> = BTreeMap::new();
    for id in ids {
        let Some(e) = doc.entity(*id) else { continue };
        if !doc.is_layer_visible(e.layer()) {
            continue;
        }
        let (code, line) = match e {
            Entity::Point(p) => (p.code.as_str(), false),
            Entity::Polyline(l) => (l.code.as_str(), true),
        };
        if code.is_empty() {
            continue;
        }
        let Some(f) = library.feature_for(code) else {
            continue;
        };
        if let Some(i) = library.codes.iter().position(|c| std::ptr::eq(c, f)) {
            let entry = used.entry(i).or_default();
            if line {
                entry.0 = true;
            } else {
                entry.1 = true;
            }
        }
    }
    let mut rows: Vec<LegendRow> = used.into_iter().map(|(i, (l, p))| (i, l, p)).collect();
    rows.sort_by(|a, b| library.codes[a.0].code.cmp(&library.codes[b.0].code));
    rows
}

const LEGEND_WIDTH: f64 = 64.0;
const LEGEND_ROW: f64 = 4.6;

fn circle_ring(c: [f64; 2], r: f64) -> Vec<[f64; 2]> {
    (0..40)
        .map(|i| {
            let a = f64::from(i) / 40.0 * std::f64::consts::TAU;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        })
        .collect()
}

/// Builds the plot for a sheet. `labels` are the global point label settings
/// (feature codes may override them).
pub fn build_plot(doc: &Document, sheet: &SheetRecord, labels: LabelSet) -> Plot {
    let lay = layout(sheet);
    let t = SheetTransform::new(sheet, &lay.frame);
    let title = if sheet.title.title.trim().is_empty() {
        doc.meta().name.clone()
    } else {
        sheet.title.title.clone()
    };
    let mut plot = Plot::new(lay.page[0], lay.page[1], title.clone());
    let content = sheet.content;
    let mono = content.monochrome;

    // Entities near the frame.
    let f = lay.frame;
    let corners = [
        t.to_world(f.min),
        t.to_world([f.max[0], f.min[1]]),
        t.to_world(f.max),
        t.to_world([f.min[0], f.max[1]]),
    ];
    let lo = corners.iter().copied().reduce(DVec2::min).unwrap_or_default();
    let hi = corners.iter().copied().reduce(DVec2::max).unwrap_or_default();
    let mut ids = Vec::new();
    doc.query(
        &|b| b.min.x <= hi.x && b.max.x >= lo.x && b.min.y <= hi.y && b.max.y >= lo.y,
        &mut ids,
    );
    ids.sort();

    // Boxes inside the frame, which the map is cut around.
    let mut boxes: Vec<Rect> = Vec::new();
    let mut rings: Vec<Vec<[f64; 2]>> = vec![f.ring()];
    if !lay.strip {
        boxes.push(lay.title);
    }
    let rows = if content.legend {
        legend_rows(doc, &ids)
    } else {
        Vec::new()
    };
    let legend = (!rows.is_empty()).then(|| {
        let base = if lay.strip { f.min[1] } else { lay.title.max[1] };
        let room = ((f.height() - (base - f.min[1])) * 0.6 - 8.0).max(LEGEND_ROW);
        let shown = rows.len().min((room / LEGEND_ROW).floor() as usize).max(1);
        let more = usize::from(shown < rows.len());
        let height = 8.0 + (shown + more) as f64 * LEGEND_ROW + 1.5;
        (
            Rect::new(f.max[0] - LEGEND_WIDTH, base, f.max[0], base + height),
            shown,
        )
    });
    if let Some((r, _)) = legend {
        boxes.push(r);
    }
    let conv = doc.conventions().clone();
    let unit = conv.linear_unit.metres_per_unit();
    let bar_target = if lay.page[0] > 400.0 { 60.0 } else { 40.0 };
    let bar_len = nice_floor(bar_target / t.k / unit);
    let bar_mm = bar_len * unit * t.k;
    let scale_box = content
        .scale_bar
        .then(|| Rect::new(f.min[0], f.min[1], f.min[0] + bar_mm + 16.0, f.min[1] + 14.0));
    if let Some(r) = scale_box {
        boxes.push(r);
    }
    let north_centre = [f.max[0] - 12.0, f.max[1] - 12.0];
    for b in &boxes {
        rings.push(b.ring());
    }
    if content.north_arrow {
        rings.push(circle_ring(north_centre, 10.0));
    }

    let mut placer = Placer {
        rects: Vec::new(),
        cells: HashMap::new(),
        area: f.inset(0.5),
    };
    for b in &boxes {
        placer.add(*b);
    }
    if content.north_arrow {
        placer.add(Rect::new(
            north_centre[0] - 10.0,
            north_centre[1] - 10.0,
            north_centre[0] + 10.0,
            north_centre[1] + 10.0,
        ));
    }
    let mut b = Builder {
        doc,
        sheet,
        t,
        frame: f,
        mono,
        styles: HashMap::new(),
        placer,
        tolerance: PAPER_TOLERANCE / t.k,
    };

    let mut map = Vec::new();
    let mut contour_labels = Vec::new();
    if content.contours {
        b.contours(&mut map, &mut contour_labels);
    }
    if content.grid == GridMarks::Lines {
        b.grid_lines(&mut map);
    }
    if content.linework {
        b.linework(&ids, &mut map);
    }
    if content.points {
        b.points(&ids, &mut map);
    }
    if content.grid != GridMarks::None {
        b.grid_marks(content.grid == GridMarks::Crosses, &mut map);
    }
    if content.parcels {
        b.parcels(&mut map);
    }
    if content.chainages {
        b.chainages(&mut map);
    }
    if content.points && content.point_labels {
        b.point_labels(&ids, labels, &mut map);
    }
    for label in contour_labels {
        b.along_text(label, true, &mut map);
    }
    plot.push(Item::Clip { rings, items: map });

    // Border and frame.
    plot.push(Item::rect(
        lay.border.min,
        lay.border.max,
        Some(Pen::solid(BLACK, 0.5)),
        None,
    ));
    let box_pen = Pen::solid(BLACK, 0.25);
    if let Some(r) = scale_box {
        plot.push(Item::rect(r.min, r.max, Some(box_pen.clone()), Some(WHITE)));
        scale_bar(&mut plot, r, bar_len, bar_mm, sheet, &conv);
    }
    if let Some((r, shown)) = legend {
        plot.push(Item::rect(r.min, r.max, Some(box_pen.clone()), Some(WHITE)));
        legend_box(&mut plot, r, &rows, shown, doc, sheet);
    }
    if content.north_arrow {
        north_arrow(&mut plot, north_centre, t.dir(DVec2::Y));
    }
    title_block(&mut plot, lay.title, &title, doc, sheet);
    plot
}

fn scale_bar(
    plot: &mut Plot,
    r: Rect,
    len: f64,
    mm: f64,
    sheet: &SheetRecord,
    conv: &survey_core::Conventions,
) {
    plot.push(Item::Text(
        Text::new(
            [r.min[0] + 3.0, r.max[1] - 2.5],
            format!("SCALE {}", sheet.scale_label()),
            2.2,
        )
        .align(Align::Left, VAlign::Top)
        .bold(),
    ));
    let x0 = r.min[0] + 5.0;
    let y0 = r.min[1] + 5.2;
    let h = 1.6;
    let pen = Pen::solid(BLACK, 0.18);
    for i in 0..4 {
        let a = x0 + mm * f64::from(i) / 4.0;
        let b = x0 + mm * f64::from(i + 1) / 4.0;
        let fill = if i % 2 == 0 { BLACK } else { WHITE };
        plot.push(Item::rect([a, y0], [b, y0 + h], Some(pen.clone()), Some(fill)));
    }
    for (frac, text) in [
        (0.0, "0".to_string()),
        (0.5, trim_number(len / 2.0)),
        (1.0, format!("{} {}", trim_number(len), conv.linear_unit.suffix())),
    ] {
        let align = if frac == 1.0 { Align::Left } else { Align::Centre };
        let x = x0 + mm * frac - if frac == 1.0 { 1.0 } else { 0.0 };
        plot.push(Item::Text(
            Text::new([x, y0 - 0.8], text, 1.8).align(align, VAlign::Top),
        ));
    }
}

fn legend_box(
    plot: &mut Plot,
    r: Rect,
    rows: &[LegendRow],
    shown: usize,
    doc: &Document,
    sheet: &SheetRecord,
) {
    let library = &doc.meta().feature_codes;
    plot.push(Item::Text(
        Text::new([r.min[0] + 3.0, r.max[1] - 2.5], "LEGEND", 2.2)
            .align(Align::Left, VAlign::Top)
            .bold(),
    ));
    let mono = sheet.content.monochrome;
    let mut y = r.max[1] - 8.0 - LEGEND_ROW / 2.0;
    for &(i, lines, points) in rows.iter().take(shown) {
        let f = &library.codes[i];
        let color = match doc.layer_by_name(&f.layer) {
            Some(layer) if f.style.by_layer => [layer.color.0[0], layer.color.0[1], layer.color.0[2]],
            _ => f.color,
        };
        let color = paper_color(color, mono);
        let x = r.min[0] + 3.0;
        if lines {
            let width = f64::from(f.style.line_weight) * sheet.line_scale;
            let unit = width.max(0.2);
            let pen = Pen {
                color,
                width,
                dash: f
                    .style
                    .line_style
                    .pattern()
                    .iter()
                    .map(|v| f64::from(*v) * unit)
                    .collect(),
            };
            plot.push(Item::line(vec![[x, y], [x + 9.0, y]], pen));
        }
        if points || !lines {
            let radius = (f64::from(f.style.symbol_size) / 2.0 * sheet.symbol_scale).min(1.6);
            let at = if lines { [x + 4.5, y] } else { [x + 4.5, y] };
            let mut items = Vec::new();
            Builder::symbol(&mut items, at, radius, f.style.symbol, color);
            plot.items.extend(items);
        }
        let label = if f.description.trim().is_empty() {
            f.code.clone()
        } else {
            format!("{}  {}", f.code, f.description)
        };
        let room = r.max[0] - (x + 12.0) - 2.0;
        let size = fit(&label, 2.0, room, false);
        plot.push(Item::Text(
            Text::new([x + 12.0, y], label, size).align(Align::Left, VAlign::Middle),
        ));
        y -= LEGEND_ROW;
    }
    if shown < rows.len() {
        plot.push(Item::Text(
            Text::new(
                [r.min[0] + 15.0, y],
                format!("and {} more", rows.len() - shown),
                1.8,
            )
            .align(Align::Left, VAlign::Middle)
            .color(GREY),
        ));
    }
}

fn north_arrow(plot: &mut Plot, c: [f64; 2], n: [f64; 2]) {
    let p = |along: f64, across: f64| {
        [
            c[0] + n[0] * along + n[1] * across,
            c[1] + n[1] * along - n[0] * across,
        ]
    };
    let tip = p(5.5, 0.0);
    let notch = p(-3.0, 0.0);
    // Left half filled, right half outlined: the usual survey north point.
    plot.push(Item::Path {
        points: vec![tip, p(-6.0, -2.8), notch],
        closed: true,
        pen: Some(Pen::solid(BLACK, 0.2)),
        fill: Some(BLACK),
    });
    plot.push(Item::Path {
        points: vec![tip, notch, p(-6.0, 2.8)],
        closed: true,
        pen: Some(Pen::solid(BLACK, 0.2)),
        fill: Some(WHITE),
    });
    let angle = n[1].atan2(n[0]) - std::f64::consts::FRAC_PI_2;
    plot.push(Item::Text(
        Text::new(p(7.2, 0.0), "N", 3.0)
            .align(Align::Centre, VAlign::Bottom)
            .angle(angle)
            .bold(),
    ));
}

fn title_block(plot: &mut Plot, r: Rect, title: &str, doc: &Document, sheet: &SheetRecord) {
    let tb = &sheet.title;
    plot.push(Item::rect(
        r.min,
        r.max,
        Some(Pen::solid(BLACK, 0.35)),
        Some(WHITE),
    ));
    let split = r.min[0] + r.width() * 0.5;
    let line = Pen::solid(BLACK, 0.18);
    plot.push(Item::line(
        vec![[split, r.min[1]], [split, r.max[1]]],
        line.clone(),
    ));

    // Left: title, subtitle and notes.
    let left_w = split - r.min[0] - 6.0;
    let x = r.min[0] + 3.0;
    let mut y = r.max[1] - 3.0;
    let size = fit(title, 4.5, left_w, true);
    plot.push(Item::Text(
        Text::new([x, y], title, size)
            .align(Align::Left, VAlign::Top)
            .bold(),
    ));
    y -= size * CAP_HEIGHT + 2.5;
    if !tb.subtitle.trim().is_empty() {
        let size = fit(&tb.subtitle, 2.8, left_w, false);
        plot.push(Item::Text(
            Text::new([x, y], tb.subtitle.clone(), size).align(Align::Left, VAlign::Top),
        ));
        y -= size * CAP_HEIGHT + 2.5;
    }
    if !tb.notes.trim().is_empty() {
        for l in wrap(&tb.notes, 1.9, left_w) {
            if y - 1.9 < r.min[1] + 1.5 {
                break;
            }
            plot.push(Item::Text(
                Text::new([x, y], l, 1.9)
                    .align(Align::Left, VAlign::Top)
                    .color(GREY),
            ));
            y -= 1.9 * 1.25;
        }
    }

    // Right: a grid of captioned fields.
    let fields = [
        ("SURVEYOR", tb.surveyor.clone()),
        ("DATE", tb.date.clone()),
        ("REFERENCE", tb.reference.clone()),
        ("SHEET", tb.sheet.clone()),
        ("SCALE", sheet.scale_label()),
        ("COORDINATES", doc.meta().coord_system.display_name()),
    ];
    let cols = 2.0;
    let rows = 3.0;
    let cw = (r.max[0] - split) / cols;
    let ch = r.height() / rows;
    for i in 1..3 {
        let yy = r.min[1] + ch * f64::from(i);
        plot.push(Item::line(vec![[split, yy], [r.max[0], yy]], line.clone()));
    }
    let mid = split + cw;
    plot.push(Item::line(vec![[mid, r.min[1]], [mid, r.max[1]]], line));
    for (i, (caption, value)) in fields.into_iter().enumerate() {
        let col = (i % 2) as f64;
        let row = (i / 2) as f64;
        let x0 = split + col * cw;
        let y1 = r.max[1] - row * ch;
        plot.push(Item::Text(
            Text::new([x0 + 1.5, y1 - 1.5], caption, 1.5)
                .align(Align::Left, VAlign::Top)
                .color(GREY),
        ));
        let size = fit(&value, 2.5, cw - 3.0, false);
        plot.push(Item::Text(
            Text::new([x0 + 1.5, y1 - ch + 2.0], value, size).align(Align::Left, VAlign::Baseline),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use survey_core::field_to_finish::FeatureCode;
    use survey_model::{ChangeSet, History, SurveyPoint};

    #[test]
    fn transform_round_trips_and_rotates() {
        let mut sheet = SheetRecord {
            centre: [500_000.0, 7_000_000.0],
            scale: 500.0,
            ..Default::default()
        };
        let lay = layout(&sheet);
        let t = SheetTransform::new(&sheet, &lay.frame);
        assert_eq!(t.to_paper(DVec2::from(sheet.centre)), lay.frame.centre());
        // 10 m east is 20 mm right at 1:500.
        let p = t.to_paper(DVec2::new(500_010.0, 7_000_000.0));
        assert!((p[0] - lay.frame.centre()[0] - 20.0).abs() < 1e-9);
        // Bearing 90° up the page: east is up.
        sheet.rotation = 90.0;
        let t = SheetTransform::new(&sheet, &lay.frame);
        let p = t.to_paper(DVec2::new(500_010.0, 7_000_000.0));
        assert!((p[1] - lay.frame.centre()[1] - 20.0).abs() < 1e-9);
        assert!((p[0] - lay.frame.centre()[0]).abs() < 1e-9);
        let w = t.to_world([123.0, 45.0]);
        let back = t.to_paper(w);
        assert!((back[0] - 123.0).abs() < 1e-9 && (back[1] - 45.0).abs() < 1e-9);
    }

    #[test]
    fn layouts_and_nice_numbers() {
        let a3 = SheetRecord::default();
        let lay = layout(&a3);
        assert!(!lay.strip);
        assert_eq!(lay.frame, lay.border);
        let a4 = SheetRecord {
            paper: survey_model::sheet::Paper::A4,
            landscape: false,
            ..Default::default()
        };
        let lay = layout(&a4);
        assert!(lay.strip);
        assert!(lay.frame.min[1] >= lay.title.max[1]);
        assert_eq!(nice_floor(37.0), 20.0);
        assert_eq!(nice_floor(60.0), 50.0);
        assert_eq!(nice_ceil(17.0), 20.0);
        assert_eq!(nice_ceil(2.2), 2.5);
        assert_eq!(grid_interval(&a3), 20.0);
        assert_eq!(upright(std::f64::consts::PI).1, true);
        assert_eq!(paper_color([255, 255, 255], false), BLACK);
        assert_eq!(paper_color([200, 0, 0], false), [200, 0, 0]);
    }

    #[test]
    fn clipping() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        let (p, q) = clip_segment([-5.0, 5.0], [15.0, 5.0], &r).expect("crosses");
        assert_eq!((p, q), ([0.0, 5.0], [10.0, 5.0]));
        assert!(clip_segment([-5.0, 20.0], [15.0, 20.0], &r).is_none());
    }

    #[test]
    fn builds_a_plan() {
        let mut doc = Document::new();
        let mut history = History::new("t");
        let mut meta = doc.meta().clone();
        meta.feature_codes
            .codes
            .push(FeatureCode::new("FENCE", "Fence", "FENCES", [255, 255, 0], true));
        let layer = doc.layers()[0].id;
        let mut cmd = ChangeSet::new("seed").set_meta(meta);
        for (i, (x, y)) in [(0.0, 0.0), (30.0, 0.0), (30.0, 20.0)].into_iter().enumerate() {
            cmd = cmd.insert_entity(Entity::Point(SurveyPoint {
                name: format!("P{i}"),
                position: DVec3::new(500_000.0 + x, 7_000_000.0 + y, 10.0),
                height_known: true,
                code: "FENCE".into(),
                description: String::new(),
                layer,
            }));
        }
        cmd = cmd.insert_entity(Entity::Polyline(Polyline {
            name: String::new(),
            vertices: vec![
                DVec3::new(500_000.0, 7_000_000.0, 10.0),
                DVec3::new(500_030.0, 7_000_000.0, 10.0),
            ],
            bulges: Vec::new(),
            closed: false,
            code: "FENCE".into(),
            layer,
        }));
        history.execute(Box::new(cmd), &mut doc);
        let sheet = SheetRecord {
            centre: [500_015.0, 7_000_010.0],
            scale: 200.0,
            ..Default::default()
        };
        let labels = LabelSet {
            name: true,
            ..Default::default()
        };
        let plot = build_plot(&doc, &sheet, labels);
        assert_eq!([plot.width, plot.height], [420.0, 297.0]);
        let Item::Clip { items, rings } = &plot.items[0] else {
            panic!("map first");
        };
        // Frame, title block, legend, scale bar, north arrow.
        assert_eq!(rings.len(), 5);
        let texts: Vec<&str> = items
            .iter()
            .filter_map(|i| match i {
                Item::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect();
        for name in ["P0", "P1", "P2"] {
            assert!(texts.contains(&name), "label {name} in {texts:?}");
        }
        // The yellow fence is darkened for paper.
        let fence = items.iter().find_map(|i| match i {
            Item::Path {
                pen: Some(p), points, ..
            } if points.len() == 2 && p.color != BLACK && p.width > 0.2 => Some(p),
            _ => None,
        });
        assert!(fence.is_some_and(|p| p.color[2] == 0 && p.color[0] < 200));
        let all = survey_io::plot::write_svg(&plot);
        assert!(all.contains(">LEGEND<") && all.contains("FENCE  Fence"));
        assert!(all.contains("SCALE 1:200 at A3"));
    }
}
