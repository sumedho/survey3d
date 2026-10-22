//! Plan sheets: page and scale, title block, what to show, a live preview of
//! the printed page and PDF/SVG export.

use egui::{Color32, Pos2, RichText, Stroke, pos2, vec2};
use egui_cad::chrome::switch_row;
use egui_cad::{CadTheme, icons};
use glam::DVec2;
use survey_core::style::LabelSet;
use survey_io::plot::{CAP_HEIGHT, Item, Plot, Rgb};
use survey_model::sheet::{GridMarks, Paper, STANDARD_SCALES, SheetRecord};
use survey_model::{Document, Keyed, RecordId};

use crate::plan::{build_plot, frame_size_world};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlotFormat {
    Pdf,
    Svg,
}

pub enum SheetsAction {
    Add(SheetRecord),
    Update(RecordId, SheetRecord),
    Delete(RecordId),
    Export(SheetRecord, PlotFormat),
}

/// What the viewport shows, for placing a sheet on it.
#[derive(Clone, Copy, Debug)]
pub struct ViewInfo {
    pub centre: DVec2,
    /// Grid bearing (degrees) pointing up the screen.
    pub up_bearing: f64,
    /// Width and height of the view in metres.
    pub extent: DVec2,
    /// The drawing's plan extent, if it has anything in it.
    pub bounds: Option<(DVec2, DVec2)>,
}

/// Today's date as `YYYY-MM-DD` (UTC).
pub fn today() -> String {
    let secs = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // Howard Hinnant's days-to-civil.
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The smallest standard scale at which a `size` (metres, along the sheet's
/// axes) fits in the sheet's frame with a little room.
fn scale_to_fit(sheet: &SheetRecord, size: DVec2) -> f64 {
    let mut s = sheet.clone();
    for scale in STANDARD_SCALES {
        s.scale = scale;
        let frame = frame_size_world(&s) * 0.9;
        if size.x <= frame.x && size.y <= frame.y {
            return scale;
        }
    }
    STANDARD_SCALES[STANDARD_SCALES.len() - 1]
}

/// The extent of an axis-aligned box seen along a sheet rotated by `rotation`.
fn rotated_size(min: DVec2, max: DVec2, rotation: f64) -> DVec2 {
    let (s, c) = rotation.to_radians().sin_cos();
    let d = max - min;
    DVec2::new(
        (d.x * c).abs() + (d.y * s).abs(),
        (d.x * s).abs() + (d.y * c).abs(),
    )
}

pub struct SheetsWindow {
    pub open: bool,
    /// The sheet shown (the first one if this one has gone).
    pub current: Option<RecordId>,
    /// The sheet being edited and the stored version it started from.
    draft: Option<(RecordId, SheetRecord, SheetRecord)>,
    /// Cached preview: its inputs and the plot.
    preview: Option<(u64, SheetRecord, LabelSet, Plot)>,
    /// Preview zoom (1 = whole page) and pan (page millimetres).
    zoom: f32,
    pan: egui::Vec2,
}

impl Default for SheetsWindow {
    fn default() -> Self {
        Self {
            open: false,
            current: None,
            draft: None,
            preview: None,
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
        }
    }
}

impl SheetsWindow {
    /// The sheet shown in the viewport while the window is open (with edits in progress).
    pub fn active(&self, doc: &Document) -> Option<SheetRecord> {
        if !self.open {
            return None;
        }
        let current = self.current_sheet(doc.sheets())?;
        match &self.draft {
            Some((id, draft, _)) if *id == current.id => Some(draft.clone()),
            _ => Some(current.record.clone()),
        }
    }

    fn current_sheet<'a>(&self, sheets: &'a [Keyed<SheetRecord>]) -> Option<&'a Keyed<SheetRecord>> {
        self.current
            .and_then(|id| sheets.iter().find(|s| s.id == id))
            .or(sheets.first())
    }

    /// A new sheet framing the current view.
    pub fn sheet_from_view(doc: &Document, view: &ViewInfo, number: usize) -> SheetRecord {
        let mut sheet = SheetRecord {
            name: format!("Sheet {number}"),
            centre: view.centre.to_array(),
            rotation: (view.up_bearing * 1e6).round() / 1e6,
            ..Default::default()
        };
        sheet.title.title = doc.meta().name.clone();
        sheet.title.date = today();
        sheet.title.sheet = format!("{number}");
        sheet.scale = scale_to_fit(&sheet, view.extent * 0.9);
        sheet
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        view: &ViewInfo,
        labels: LabelSet,
    ) -> Option<SheetsAction> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let sheets = doc.sheets();
        let stored = self.current_sheet(sheets);
        self.current = stored.map(|s| s.id);
        // Follow the stored sheet unless it is being edited; adopt outside changes (undo).
        match (&mut self.draft, stored) {
            (Some((id, draft, base)), Some(stored)) if *id == stored.id => {
                if stored.record != *base {
                    *draft = stored.record.clone();
                    *base = stored.record.clone();
                }
            }
            (_, Some(stored)) => self.draft = Some((stored.id, stored.record.clone(), stored.record.clone())),
            (_, None) => self.draft = None,
        }

        let mut action = None;
        let mut open = self.open;
        egui::Window::new("Plan sheets")
            .open(&mut open)
            .default_size(vec2(960.0, 640.0))
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    match stored {
                        None => {
                            ui.label(RichText::new("No sheets yet").color(theme.text_secondary));
                        }
                        Some(stored) => {
                            egui::ComboBox::from_id_salt("sheet_pick")
                                .selected_text(stored.name.clone())
                                .show_ui(ui, |ui| {
                                    for s in sheets {
                                        ui.selectable_value(&mut self.current, Some(s.id), s.name.clone());
                                    }
                                });
                        }
                    }
                    if ui
                        .button(format!("{} New from view", icons::PLUS))
                        .on_hover_text("A sheet centred on the view, at a scale that fits it")
                        .clicked()
                    {
                        let sheet = Self::sheet_from_view(doc, view, sheets.len() + 1);
                        action = Some(SheetsAction::Add(sheet));
                    }
                    if let Some((id, draft, _)) = &self.draft {
                        if ui.button("Duplicate").clicked() {
                            let mut copy = draft.clone();
                            copy.name = format!("{} copy", draft.name);
                            action = Some(SheetsAction::Add(copy));
                        }
                        if ui.button("Delete").clicked() {
                            action = Some(SheetsAction::Delete(*id));
                        }
                    }
                });
                ui.separator();
                let Some((id, draft, base)) = &mut self.draft else {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(
                                "A sheet is a printed page of the drawing at a true scale, with a \
                                 title block, grid, north arrow, scale bar and legend.",
                            )
                            .color(theme.text_secondary),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("Frame the area in the view, then press “New from view”.")
                                .color(theme.text_secondary),
                        );
                    });
                    return;
                };
                let id = *id;
                let height = ui.available_height().max(300.0);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        vec2(310.0, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("sheet_settings")
                                .show(ui, |ui| {
                                    settings(ui, draft, doc, view, &theme);
                                    ui.add_space(8.0);
                                    ui.horizontal(|ui| {
                                        if ui
                                            .button(format!("{} Export PDF…", icons::FILE_ARROW_UP))
                                            .clicked()
                                        {
                                            action =
                                                Some(SheetsAction::Export(draft.clone(), PlotFormat::Pdf));
                                        }
                                        if ui.button("Export SVG…").clicked() {
                                            action =
                                                Some(SheetsAction::Export(draft.clone(), PlotFormat::Svg));
                                        }
                                    });
                                });
                        },
                    );
                    ui.separator();
                    let key = (doc.revision(), draft.clone(), labels);
                    let stale = self
                        .preview
                        .as_ref()
                        .is_none_or(|(r, s, l, _)| (*r, s, *l) != (key.0, &key.1, key.2));
                    if stale {
                        let plot = build_plot(doc, draft, labels);
                        self.preview = Some((key.0, key.1, key.2, plot));
                    }
                    if let Some((_, _, _, plot)) = &self.preview {
                        preview(ui, plot, &mut self.zoom, &mut self.pan, &theme);
                    }
                });
                // Commit when the user lets go (not on every keystroke or drag step).
                let idle = !ctx.egui_wants_keyboard_input() && !ctx.input(|i| i.pointer.any_down());
                if action.is_none() && *draft != *base && idle {
                    action = Some(SheetsAction::Update(id, draft.clone()));
                    *base = draft.clone();
                }
            });
        self.open = open;
        action
    }
}

fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(84.0, 18.0), egui::Label::new(label));
        add(ui);
    });
}

fn heading(ui: &mut egui::Ui, text: &str, theme: &CadTheme) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).strong().color(theme.text));
    ui.add_space(2.0);
}

fn settings(ui: &mut egui::Ui, s: &mut SheetRecord, doc: &Document, view: &ViewInfo, theme: &CadTheme) {
    row(ui, "Name", |ui| {
        ui.text_edit_singleline(&mut s.name);
    });

    heading(ui, "Page", theme);
    row(ui, "Paper", |ui| {
        egui::ComboBox::from_id_salt("paper")
            .selected_text(s.paper.label())
            .show_ui(ui, |ui| {
                for p in Paper::ALL {
                    ui.selectable_value(&mut s.paper, p, p.label());
                }
            });
        ui.selectable_value(&mut s.landscape, true, "Landscape");
        ui.selectable_value(&mut s.landscape, false, "Portrait");
    });
    if s.paper == Paper::Custom {
        row(ui, "Size (mm)", |ui| {
            ui.add(egui::DragValue::new(&mut s.custom_size[0]).range(50.0..=5000.0));
            ui.label("×");
            ui.add(egui::DragValue::new(&mut s.custom_size[1]).range(50.0..=5000.0));
        });
    }
    row(ui, "Scale 1:", |ui| {
        ui.add(
            egui::DragValue::new(&mut s.scale)
                .range(1.0..=10_000_000.0)
                .speed(10.0),
        );
        egui::ComboBox::from_id_salt("scale")
            .selected_text("Standard")
            .width(90.0)
            .show_ui(ui, |ui| {
                for n in STANDARD_SCALES {
                    ui.selectable_value(&mut s.scale, n, format!("1:{n:.0}"));
                }
            });
    });

    heading(ui, "Position", theme);
    let (le, ln) = {
        let c = doc.conventions();
        (c.axis_labels.easting.clone(), c.axis_labels.northing.clone())
    };
    row(ui, "Centre", |ui| {
        ui.label(le);
        ui.add(egui::DragValue::new(&mut s.centre[0]).speed(1.0).max_decimals(3));
        ui.label(ln);
        ui.add(egui::DragValue::new(&mut s.centre[1]).speed(1.0).max_decimals(3));
    });
    row(ui, "Rotation", |ui| {
        ui.add(
            egui::DragValue::new(&mut s.rotation)
                .range(-360.0..=360.0)
                .suffix("°")
                .max_decimals(4),
        )
        .on_hover_text("Grid bearing pointing up the page (0 = north up)");
    });
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Centre on view")
            .on_hover_text("Move the sheet to the centre of the view")
            .clicked()
        {
            s.centre = view.centre.to_array();
        }
        if ui
            .button("Rotate as view")
            .on_hover_text("Use the view's rotation (plan view)")
            .clicked()
        {
            s.rotation = (view.up_bearing * 1e6).round() / 1e6;
        }
        if let Some((lo, hi)) = view.bounds
            && ui
                .button("Fit drawing")
                .on_hover_text("Centre on the drawing at the largest standard scale that fits")
                .clicked()
        {
            s.centre = ((lo + hi) * 0.5).to_array();
            s.scale = scale_to_fit(s, rotated_size(lo, hi, s.rotation));
        }
    });

    heading(ui, "Title block", theme);
    let t = &mut s.title;
    for (label, value) in [
        ("Title", &mut t.title),
        ("Subtitle", &mut t.subtitle),
        ("Surveyor", &mut t.surveyor),
        ("Reference", &mut t.reference),
        ("Sheet", &mut t.sheet),
    ] {
        row(ui, label, |ui| {
            ui.text_edit_singleline(value);
        });
    }
    row(ui, "Date", |ui| {
        ui.add(egui::TextEdit::singleline(&mut t.date).desired_width(110.0));
        if ui.small_button("Today").clicked() {
            t.date = today();
        }
    });
    row(ui, "Notes", |ui| {
        ui.add(egui::TextEdit::multiline(&mut t.notes).desired_rows(3));
    });

    heading(ui, "Show", theme);
    let c = &mut s.content;
    switch_row(ui, &mut c.linework, "Linework");
    switch_row(ui, &mut c.points, "Point symbols");
    switch_row(ui, &mut c.point_labels, "Point labels");
    switch_row(ui, &mut c.parcels, "Parcel bearings, distances and areas");
    switch_row(ui, &mut c.contours, "Contours");
    switch_row(ui, &mut c.chainages, "Alignment chainages");
    switch_row(ui, &mut c.north_arrow, "North arrow");
    switch_row(ui, &mut c.scale_bar, "Scale bar");
    switch_row(ui, &mut c.legend, "Legend");
    switch_row(ui, &mut c.monochrome, "Black and white");
    row(ui, "Grid", |ui| {
        egui::ComboBox::from_id_salt("grid")
            .selected_text(c.grid.label())
            .show_ui(ui, |ui| {
                for g in GridMarks::ALL {
                    ui.selectable_value(&mut c.grid, g, g.label());
                }
            });
    });
    if c.grid != GridMarks::None {
        row(ui, "Grid every", |ui| {
            ui.add(
                egui::DragValue::new(&mut s.grid_interval)
                    .range(0.0..=100_000.0)
                    .suffix(" m")
                    .custom_formatter(|v, _| if v == 0.0 { "auto".into() } else { format!("{v}") }),
            )
            .on_hover_text("0 chooses a spacing from the scale");
        });
    }

    heading(ui, "Sizes on paper", theme);
    row(ui, "Text", |ui| {
        ui.add(
            egui::DragValue::new(&mut s.text_size)
                .range(0.8..=10.0)
                .speed(0.05)
                .suffix(" mm"),
        );
    });
    row(ui, "Symbols", |ui| {
        ui.add(
            egui::DragValue::new(&mut s.symbol_scale)
                .range(0.05..=2.0)
                .speed(0.01)
                .suffix(" mm/pt"),
        )
        .on_hover_text("Paper size of one screen point of a code's symbol size");
    });
    row(ui, "Lines", |ui| {
        ui.add(
            egui::DragValue::new(&mut s.line_scale)
                .range(0.02..=1.0)
                .speed(0.01)
                .suffix(" mm/pt"),
        )
        .on_hover_text("Paper width of one screen point of a code's line weight");
    });
}

fn color(c: Rgb) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Paints the page; scroll to zoom, drag to pan, double-click to fit.
fn preview(ui: &mut egui::Ui, plot: &Plot, zoom: &mut f32, pan: &mut egui::Vec2, theme: &CadTheme) {
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme.bg);
    if response.double_clicked() {
        *zoom = 1.0;
        *pan = egui::Vec2::ZERO;
    }
    let fit = ((rect.width() - 24.0) / plot.width as f32).min((rect.height() - 24.0) / plot.height as f32);
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let old = *zoom;
            *zoom = (*zoom * (scroll * 0.003).exp()).clamp(1.0, 40.0);
            // Keep the page point under the cursor still.
            if let Some(p) = response.hover_pos() {
                let s0 = fit * old;
                let s1 = fit * *zoom;
                let centre = rect.center();
                let page = (p - centre - *pan * s0) / s0;
                *pan = (p - centre - page * s1) / s1;
            }
        }
    }
    let s = fit * *zoom;
    if response.dragged() {
        *pan += response.drag_delta() / s;
    }
    let size = vec2(plot.width as f32, plot.height as f32) * s;
    let page = egui::Rect::from_center_size(rect.center() + *pan * s, size);
    painter.rect_filled(page.translate(vec2(3.0, 3.0)), 0.0, Color32::from_black_alpha(60));
    painter.rect_filled(page, 0.0, Color32::WHITE);
    let to = |p: [f64; 2]| pos2(page.min.x + p[0] as f32 * s, page.max.y - p[1] as f32 * s);
    paint_items(&painter, &plot.items, &to, s);
    if *zoom == 1.0 {
        painter.text(
            rect.right_bottom() - vec2(8.0, 6.0),
            egui::Align2::RIGHT_BOTTOM,
            "Scroll to zoom · drag to pan · double-click to fit",
            egui::FontId::proportional(11.0),
            theme.text_secondary,
        );
    }
}

fn paint_items(painter: &egui::Painter, items: &[Item], to: &dyn Fn([f64; 2]) -> Pos2, s: f32) {
    let stroke = |p: &survey_io::plot::Pen| Stroke::new((p.width as f32 * s).max(0.6), color(p.color));
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
                let pts: Vec<Pos2> = points.iter().map(|p| to(*p)).collect();
                if let Some(f) = fill {
                    painter.add(egui::Shape::convex_polygon(pts.clone(), color(*f), Stroke::NONE));
                }
                if let Some(p) = pen {
                    let mut path = pts;
                    if *closed {
                        path.push(path[0]);
                    }
                    if p.dash.len() >= 2 {
                        let (on, off) = (p.dash[0] as f32 * s, p.dash[1] as f32 * s);
                        painter.extend(egui::Shape::dashed_line(
                            &path,
                            stroke(p),
                            on.max(0.5),
                            off.max(0.5),
                        ));
                    } else {
                        painter.add(egui::Shape::line(path, stroke(p)));
                    }
                }
            }
            Item::Circle {
                center,
                radius,
                pen,
                fill,
            } => {
                let r = *radius as f32 * s;
                if let Some(f) = fill {
                    painter.circle_filled(to(*center), r, color(*f));
                }
                if let Some(p) = pen {
                    painter.circle_stroke(to(*center), r, stroke(p));
                }
            }
            Item::Text(t) => {
                let px = t.size as f32 * s;
                if px < 1.5 {
                    continue;
                }
                let font = egui::FontId::proportional(px);
                let galley = painter.layout_no_wrap(t.text.clone(), font, color(t.color));
                // Put the galley's baseline on the text's origin.
                let baseline = galley
                    .rows
                    .first()
                    .and_then(|r| r.row.glyphs.first().map(|g| r.pos.y + g.pos.y))
                    .unwrap_or(px * CAP_HEIGHT as f32);
                let origin = to(t.origin());
                let angle = -(t.angle as f32);
                let (sin, cos) = angle.sin_cos();
                // Galley top-left = origin − baseline along the rotated "down".
                let pos = origin - vec2(-sin, cos) * baseline;
                painter.add(egui::epaint::TextShape::new(pos, galley, color(t.color)).with_angle(angle));
            }
            Item::Clip { rings, items } => {
                let Some(outer) = rings.first() else { continue };
                let pts: Vec<Pos2> = outer.iter().map(|p| to(*p)).collect();
                let clip = egui::Rect::from_points(&pts);
                let inner = painter.with_clip_rect(clip.intersect(painter.clip_rect()));
                paint_items(&inner, items, to, s);
                // Holes are covered by the (white) boxes drawn over them.
                for ring in &rings[1..] {
                    let pts: Vec<Pos2> = ring.iter().map(|p| to(*p)).collect();
                    painter.add(egui::Shape::convex_polygon(pts, Color32::WHITE, Stroke::NONE));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_and_dates() {
        let sheet = SheetRecord::default();
        // A3 landscape frame is 400 × 277 mm: 80 m × 55 m fits at 1:250 (90 % of 100 × 69 m).
        assert_eq!(scale_to_fit(&sheet, DVec2::new(80.0, 55.0)), 250.0);
        assert_eq!(scale_to_fit(&sheet, DVec2::new(100.0, 55.0)), 500.0);
        let r = rotated_size(DVec2::ZERO, DVec2::new(10.0, 2.0), 90.0);
        assert!((r.x - 2.0).abs() < 1e-9 && (r.y - 10.0).abs() < 1e-9);
        let d = today();
        assert_eq!(d.len(), 10);
        assert!(d.starts_with("20"));
    }
}
