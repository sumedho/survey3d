//! Sections through surfaces: a long section along a polyline, or cross
//! sections at an interval, plotted with a grid and a hover readout.

use std::fmt::Write as _;

use egui::{Color32, RichText, Stroke, pos2, vec2};
use egui_cad::chrome::segmented;
use egui_cad::{CadTheme, fonts};
use survey_core::Conventions;
use survey_core::tin::TinIndex;
use survey_model::{Document, Entity, EntityId, Polyline, SurfaceId};

use crate::compute::sections::{
    cross_line, height_at, indexes, length, long_line, report, sections, stations,
};

const PALETTE: [Color32; 6] = [
    Color32::from_rgb(251, 191, 36),
    Color32::from_rgb(125, 211, 252),
    Color32::from_rgb(74, 222, 128),
    Color32::from_rgb(244, 114, 182),
    Color32::from_rgb(167, 139, 250),
    Color32::from_rgb(248, 113, 113),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Long,
    Cross,
}

/// One surface's section, as drawn.
#[derive(Clone)]
struct Series {
    color: Color32,
    section: crate::compute::sections::Series,
}

pub struct SectionsWindow {
    pub open: bool,
    alignment: Option<EntityId>,
    mode: Mode,
    interval: f64,
    left: f64,
    right: f64,
    station: usize,
    /// Vertical exaggeration; 0 fits the plot.
    exaggeration: f64,
    indexes: (u64, Vec<(SurfaceId, TinIndex)>),
    /// Cached series keyed by what they depend on.
    cache: Option<(CacheKey, Vec<Series>)>,
}

#[derive(Clone, PartialEq)]
struct CacheKey {
    revision: u64,
    alignment: EntityId,
    mode: Mode,
    station: usize,
    interval: f64,
    left: f64,
    right: f64,
}

impl Default for SectionsWindow {
    fn default() -> Self {
        Self {
            open: false,
            alignment: None,
            mode: Mode::Long,
            interval: 20.0,
            left: 20.0,
            right: 20.0,
            station: 0,
            exaggeration: 0.0,
            indexes: (0, Vec::new()),
            cache: None,
        }
    }
}

pub enum SectionsAction {
    Report(String, String),
}

impl SectionsWindow {
    pub fn open_with(&mut self, alignment: Option<EntityId>) {
        self.open = true;
        if alignment.is_some() {
            self.alignment = alignment;
            self.station = 0;
        }
    }

    fn alignment<'a>(&self, doc: &'a Document) -> Option<&'a Polyline> {
        match doc.entity(self.alignment?) {
            Some(Entity::Polyline(l)) => Some(l),
            _ => None,
        }
    }

    fn stations(&self, length: f64) -> Vec<f64> {
        stations(length, self.interval)
    }

    fn series(&mut self, doc: &Document) -> &[Series] {
        let Some(id) = self.alignment else {
            return &[];
        };
        let key = CacheKey {
            revision: doc.revision(),
            alignment: id,
            mode: self.mode,
            station: self.station,
            interval: self.interval,
            left: self.left,
            right: self.right,
        };
        if self.cache.as_ref().is_some_and(|(k, _)| *k == key) {
            return self.cache.as_ref().map(|(_, s)| s.as_slice()).unwrap_or(&[]);
        }
        if self.indexes.0 != doc.surfaces_revision() {
            self.indexes = (doc.surfaces_revision(), indexes(doc));
        }
        let mut out = Vec::new();
        if let Some(l) = self.alignment(doc) {
            let (line, shift) = match self.mode {
                Mode::Long => (long_line(l), 0.0),
                Mode::Cross => {
                    let stations = self.stations(length(l));
                    let ch = stations[self.station.min(stations.len() - 1)];
                    (cross_line(l, ch, self.left, self.right), -self.left)
                }
            };
            out = sections(doc, &self.indexes.1, &line, shift)
                .into_iter()
                .enumerate()
                .map(|(k, section)| Series {
                    color: PALETTE[k % PALETTE.len()],
                    section,
                })
                .collect();
        }
        self.cache = Some((key, out));
        self.cache.as_ref().map(|(_, s)| s.as_slice()).unwrap_or(&[])
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        selected_line: Option<EntityId>,
    ) -> Option<SectionsAction> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let conv = doc.conventions().clone();
        let mut action = None;
        let mut open = self.open;
        egui::Window::new("Sections")
            .open(&mut open)
            .default_size([760.0, 440.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let name = self
                        .alignment(doc)
                        .map(|l| if l.name.is_empty() { "polyline".to_string() } else { l.name.clone() });
                    ui.label(match &name {
                        Some(n) => format!("Alignment: {n}"),
                        None => "Alignment: none".into(),
                    });
                    if ui
                        .add_enabled(selected_line.is_some(), egui::Button::new("Use selected polyline"))
                        .on_disabled_hover_text("Select a polyline to section along")
                        .clicked()
                    {
                        self.alignment = selected_line;
                        self.station = 0;
                    }
                    ui.separator();
                    segmented(ui, &mut self.mode, &[(Mode::Long, "Long section"), (Mode::Cross, "Cross sections")]);
                });
                let Some(l) = self.alignment(doc) else {
                    ui.label(
                        RichText::new("Select a polyline (road centreline, fence, pit crest…) and click “Use selected polyline”.")
                            .color(theme.text_secondary),
                    );
                    return;
                };
                let stations = self.stations(length(l));
                ui.horizontal(|ui| {
                    if self.mode == Mode::Cross {
                        ui.label("Every");
                        ui.add(egui::DragValue::new(&mut self.interval).range(0.1..=10_000.0).speed(0.5).suffix(" m"));
                        ui.label("left");
                        ui.add(egui::DragValue::new(&mut self.left).range(0.0..=10_000.0).speed(0.5).suffix(" m"));
                        ui.label("right");
                        ui.add(egui::DragValue::new(&mut self.right).range(0.0..=10_000.0).speed(0.5).suffix(" m"));
                        ui.separator();
                        self.station = self.station.min(stations.len().saturating_sub(1));
                        if ui.button("◀").clicked() {
                            self.station = self.station.saturating_sub(1);
                        }
                        ui.label(
                            RichText::new(format!("Ch {}", conv.format_chainage(stations[self.station])))
                                .font(fonts::mono(12.0)),
                        );
                        if ui.button("▶").clicked() && self.station + 1 < stations.len() {
                            self.station += 1;
                        }
                        ui.label(RichText::new(format!("{} / {}", self.station + 1, stations.len())).small());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(
                            egui::DragValue::new(&mut self.exaggeration)
                                .range(0.0..=100.0)
                                .speed(0.1)
                                .custom_formatter(|v, _| if v <= 0.0 { "fit".into() } else { format!("×{v:.1}") }),
                        )
                        .on_hover_text("Vertical exaggeration (0 fits the plot)");
                        ui.label("Vertical");
                    });
                });
                let mode = self.mode;
                let exaggeration = self.exaggeration;
                let series = self.series(doc).to_vec();
                let series = series.as_slice();
                if series.iter().all(|s| s.section.runs.is_empty()) {
                    ui.label(RichText::new("No visible surface under this line.").color(theme.text_secondary));
                    return;
                }
                plot(ui, &theme, &conv, series, mode, exaggeration);
                ui.horizontal(|ui| {
                    for s in series {
                        ui.label(RichText::new("━").color(s.color));
                        ui.label(&s.section.name);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Add to reports").clicked() {
                            let title = match mode {
                                Mode::Long => "Long section".to_string(),
                                Mode::Cross => format!("Cross section Ch {}", conv.format_chainage(stations[self.station.min(stations.len() - 1)])),
                            };
                            let only: Vec<_> = series.iter().map(|s| s.section.clone()).collect();
                            action = Some(SectionsAction::Report(title.clone(), report(&conv, &title, &only, mode == Mode::Cross)));
                        }
                    });
                });
            });
        self.open = open;
        action
    }
}

/// 1, 2 or 5 × 10ⁿ near `range / 6`.
fn nice_step(range: f64) -> f64 {
    let raw = (range / 6.0).max(1e-9);
    let p = 10f64.powf(raw.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * p)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * p)
}

fn plot(
    ui: &mut egui::Ui,
    theme: &CadTheme,
    conv: &Conventions,
    series: &[Series],
    mode: Mode,
    exaggeration: f64,
) {
    let size = vec2(ui.available_width(), (ui.available_height() - 30.0).max(220.0));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, theme.radius, theme.viewport_bg);
    let plot = rect.shrink2(vec2(56.0, 18.0)).translate(vec2(20.0, -6.0));

    let pts = series.iter().flat_map(|s| s.section.runs.iter().flatten());
    let (mut x0, mut x1, mut z0, mut z1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for (x, z) in pts {
        x0 = x0.min(*x);
        x1 = x1.max(*x);
        z0 = z0.min(*z);
        z1 = z1.max(*z);
    }
    if mode == Mode::Cross {
        x0 = x0.min(0.0);
        x1 = x1.max(0.0);
    }
    let xr = (x1 - x0).max(1e-6);
    let sx = plot.width() as f64 / xr;
    // Vertical scale: fitted, or a fixed exaggeration of the horizontal scale.
    let (sz, zc) = if exaggeration > 0.0 {
        (sx * exaggeration, 0.5 * (z0 + z1))
    } else {
        let zr = (z1 - z0).max(0.1) * 1.15;
        (plot.height() as f64 / zr, 0.5 * (z0 + z1))
    };
    let to_screen = |x: f64, z: f64| {
        pos2(
            plot.left() + ((x - x0) * sx) as f32,
            plot.center().y - ((z - zc) * sz) as f32,
        )
    };
    let zlo = zc - plot.height() as f64 / 2.0 / sz;
    let zhi = zc + plot.height() as f64 / 2.0 / sz;

    // Grid and labels.
    let grid = Stroke::new(1.0, theme.border);
    let font = fonts::mono(10.0);
    let step = nice_step(xr);
    let mut x = (x0 / step).ceil() * step;
    while x <= x1 + 1e-9 {
        let p = to_screen(x, zlo);
        painter.line_segment([pos2(p.x, plot.top()), pos2(p.x, plot.bottom())], grid);
        let xv = if x.abs() < step * 1e-6 { 0.0 } else { x };
        let label = match mode {
            Mode::Long => conv.format_chainage(xv),
            Mode::Cross => conv.format_length(xv),
        };
        painter.text(
            pos2(p.x, plot.bottom() + 3.0),
            egui::Align2::CENTER_TOP,
            label,
            font.clone(),
            theme.text_secondary,
        );
        x += step;
    }
    let zstep = nice_step(zhi - zlo);
    let mut z = (zlo / zstep).ceil() * zstep;
    while z <= zhi + 1e-9 {
        // Avoid "-0.000" from rounding.
        let z_label = if z.abs() < zstep * 1e-6 { 0.0 } else { z };
        let p = to_screen(x0, z);
        painter.line_segment([pos2(plot.left(), p.y), pos2(plot.right(), p.y)], grid);
        painter.text(
            pos2(plot.left() - 4.0, p.y),
            egui::Align2::RIGHT_CENTER,
            conv.format_length(z_label),
            font.clone(),
            theme.text_secondary,
        );
        z += zstep;
    }
    if mode == Mode::Cross {
        let c = to_screen(0.0, zlo).x;
        painter.line_segment(
            [pos2(c, plot.top()), pos2(c, plot.bottom())],
            Stroke::new(1.0, theme.accent.gamma_multiply(0.6)),
        );
    }
    let exag = sz / sx;
    painter.text(
        pos2(plot.right(), plot.top() - 2.0),
        egui::Align2::RIGHT_BOTTOM,
        format!("V ×{exag:.1}"),
        fonts::mono(10.0),
        theme.text_secondary,
    );

    let clip = painter.with_clip_rect(plot);
    for s in series {
        for run in &s.section.runs {
            let pts: Vec<egui::Pos2> = run.iter().map(|(x, z)| to_screen(*x, *z)).collect();
            clip.add(egui::Shape::line(pts, Stroke::new(1.8, s.color)));
        }
    }

    // Hover: distance and each surface's height.
    if let Some(m) = response.hover_pos().filter(|m| plot.contains(*m)) {
        let x = x0 + (m.x - plot.left()) as f64 / sx;
        clip.line_segment(
            [pos2(m.x, plot.top()), pos2(m.x, plot.bottom())],
            Stroke::new(1.0, theme.text_secondary),
        );
        let mut text = match mode {
            Mode::Long => format!("Ch {}", conv.format_chainage(x)),
            Mode::Cross => format!("Offset {}", conv.format_length(x)),
        };
        for s in series {
            if let Some(z) = s.section.runs.iter().find_map(|r| height_at(r, x)) {
                let _ = write!(text, "\n{}: {}", s.section.name, conv.format_length(z));
                clip.circle_filled(to_screen(x, z), 3.0, s.color);
            }
        }
        let galley = painter.layout_no_wrap(text, fonts::mono(11.0), theme.text);
        let pos = pos2(
            (m.x + 10.0).min(plot.right() - galley.size().x - 6.0),
            plot.top() + 6.0,
        );
        let bg = egui::Rect::from_min_size(pos, galley.size()).expand(4.0);
        painter.rect_filled(bg, theme.radius, theme.overlay);
        painter.galley(pos, galley, theme.text);
    }
}
