//! Legend: the feature codes with their symbols, line styles and how many
//! points and lines use them.

use egui::{Color32, RichText};
use egui_cad::chrome::switch_row;
use egui_cad::symbols::{paint_line_sample, paint_symbol};
use egui_cad::{CadTheme, fonts};
use survey_model::{Document, Entity};

pub struct LegendWindow {
    pub open: bool,
    in_use_only: bool,
    /// Per feature: (points, lines), cached by document revision.
    counts: (u64, Vec<(usize, usize)>),
}

impl Default for LegendWindow {
    fn default() -> Self {
        Self {
            open: false,
            in_use_only: true,
            counts: (u64::MAX, Vec::new()),
        }
    }
}

impl LegendWindow {
    fn counts(&mut self, doc: &Document) -> &[(usize, usize)] {
        if self.counts.0 != doc.revision() {
            let library = &doc.meta().feature_codes;
            let mut counts = vec![(0, 0); library.codes.len()];
            for (_, e) in doc.entities() {
                let (code, line) = match e {
                    Entity::Point(p) => (p.code.as_str(), false),
                    Entity::Polyline(l) => (l.code.as_str(), true),
                };
                let Some(f) = library.feature_for(code) else {
                    continue;
                };
                if let Some(i) = library.codes.iter().position(|c| std::ptr::eq(c, f)) {
                    if line {
                        counts[i].1 += 1;
                    } else {
                        counts[i].0 += 1;
                    }
                }
            }
            self.counts = (doc.revision(), counts);
        }
        &self.counts.1
    }

    pub fn show(&mut self, ctx: &egui::Context, doc: &Document) {
        if !self.open {
            return;
        }
        let theme = CadTheme::current(ctx);
        let mut open = self.open;
        let mut in_use_only = self.in_use_only;
        let counts = self.counts(doc).to_vec();
        let library = &doc.meta().feature_codes;
        egui::Window::new("Legend")
            .open(&mut open)
            .default_width(380.0)
            .show(ctx, |ui| {
                switch_row(ui, &mut in_use_only, "Only codes used in this project");
                ui.add_space(4.0);
                let rows: Vec<usize> = (0..library.codes.len())
                    .filter(|i| !in_use_only || counts[*i] != (0, 0))
                    .collect();
                if rows.is_empty() {
                    ui.label(
                        RichText::new(if library.codes.is_empty() {
                            "No feature codes defined (Tools → Feature codes…)."
                        } else {
                            "No points or lines use the feature codes yet."
                        })
                        .color(theme.text_secondary),
                    );
                    return;
                }
                egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                    egui::Grid::new("legend")
                        .num_columns(5)
                        .spacing([10.0, 6.0])
                        .striped(true)
                        .show(ui, |ui| {
                            for i in rows {
                                let c = &library.codes[i];
                                let color = display_color(doc, c);
                                let (rect, _) =
                                    ui.allocate_exact_size(egui::vec2(22.0, 18.0), egui::Sense::hover());
                                paint_symbol(
                                    ui.painter(),
                                    rect.center(),
                                    (c.style.symbol_size * 0.6).clamp(4.0, 9.0),
                                    color,
                                    c.style.symbol,
                                );
                                let (rect, _) =
                                    ui.allocate_exact_size(egui::vec2(44.0, 18.0), egui::Sense::hover());
                                if c.linework {
                                    paint_line_sample(
                                        ui.painter(),
                                        rect,
                                        color,
                                        c.style.line_weight,
                                        c.style.line_style,
                                    );
                                }
                                ui.label(RichText::new(&c.code).font(fonts::mono(12.0)));
                                ui.label(&c.description);
                                let (pts, lines) = counts[i];
                                let plural = |n: usize, one: &str, many: &str| {
                                    format!("{n} {}", if n == 1 { one } else { many })
                                };
                                let mut n = plural(pts, "point", "points");
                                if lines > 0 {
                                    n.push_str(&format!(", {}", plural(lines, "line", "lines")));
                                }
                                ui.label(RichText::new(n).small().color(theme.text_secondary));
                                ui.end_row();
                            }
                        });
                });
            });
        self.in_use_only = in_use_only;
        self.open = open;
    }
}

/// Colour a feature is drawn in: its own, or its layer's when drawn by layer.
fn display_color(doc: &Document, c: &survey_core::field_to_finish::FeatureCode) -> Color32 {
    let [r, g, b] = c.color;
    let own = Color32::from_rgb(r, g, b);
    if !c.style.by_layer {
        return own;
    }
    match doc.layer_by_name(&c.layer) {
        Some(l) => crate::scene_build::color32(l.color),
        None => own,
    }
}
