//! Cadastral parcels: list, plan closure, reports and annotation switches.

use egui::RichText;
use egui_cad::chrome::IconButton;
use egui_cad::{CadTheme, fonts, icons};
use survey_core::Conventions;
use survey_core::accuracy::Tolerances;
use survey_core::arc::polyline_metrics;
use survey_core::closure::{Course, CourseArc, Figure, Solution, courses_from_polyline, figure, report};
use survey_model::{Document, Entity, EntityId, ParcelRecord, Polyline, RecordId};

/// A parcel's boundary polyline, if it still exists and is closed.
pub fn boundary<'a>(doc: &'a Document, p: &ParcelRecord) -> Option<&'a Polyline> {
    match doc.entity(p.boundary) {
        Some(Entity::Polyline(l)) if l.closed => Some(l),
        _ => None,
    }
}

/// The number after the highest "Lot n" so far, continuing the numbering.
pub fn next_lot_number(doc: &Document) -> u32 {
    doc.parcels()
        .iter()
        .filter_map(|p| {
            p.name
                .strip_prefix("Lot ")
                .and_then(|n| n.trim().parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0)
        + 1
}

/// The boundary's courses with bearings, distances and radii rounded to the
/// display precision (as a plan shows them), and the figure they make: its
/// misclosure is the plan closure.
pub fn plan_closure(l: &Polyline, conv: &Conventions) -> (Vec<Course>, Figure) {
    let round_len = |v: f64| conv.parse_length(&conv.format_length(v)).unwrap_or(v);
    let courses: Vec<Course> = courses_from_polyline(&l.vertices, &l.bulges, true)
        .into_iter()
        .map(|c| Course {
            bearing: c
                .bearing
                .map(|b| conv.parse_bearing(&conv.format_bearing(b)).unwrap_or(b)),
            distance: c.distance.map(round_len),
            arc: c.arc.map(|a| CourseArc {
                radius: round_len(a.radius),
                ..a
            }),
        })
        .collect();
    let fig = figure(&courses);
    (courses, fig)
}

/// Plain-text report for one parcel.
pub fn parcel_report(doc: &Document, p: &ParcelRecord, tolerances: &Tolerances) -> Option<String> {
    let l = boundary(doc, p)?;
    let conv = doc.conventions();
    let (courses, fig) = plan_closure(l, conv);
    let title = if p.plan.is_empty() {
        p.name.clone()
    } else {
        format!("{} {}", p.name, p.plan)
    };
    let mut text = report(
        &title,
        &Solution {
            courses,
            solved: Vec::new(),
            reversed: Vec::new(),
        },
        &fig,
        tolerances,
        conv,
    );
    if !p.description.is_empty() {
        text.push_str(&format!("Description         {}\n", p.description));
    }
    text.push_str("Bearings, distances and radii as shown on the plan (rounded to the display precision).\n");
    Some(text)
}

pub enum ParcelsAction {
    Update(RecordId, ParcelRecord),
    Delete(RecordId),
    Select(EntityId),
    Report(String, String),
    CreateFromSelection,
    ExportLandXml,
}

#[derive(Default)]
pub struct ParcelsWindow {
    pub open: bool,
    /// Text being edited: (parcel, name, plan).
    editing: Option<(RecordId, String, String)>,
}

impl ParcelsWindow {
    pub fn show(&mut self, ctx: &egui::Context, doc: &Document, can_create: bool) -> Option<ParcelsAction> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let conv = doc.conventions().clone();
        let tolerances = doc.meta().tolerances;
        let parcels = doc.parcels();
        let mut action = None;
        let mut open = self.open;
        egui::Window::new("Parcels")
            .open(&mut open)
            .default_width(640.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(can_create, egui::Button::new(format!("{} From selected polylines", icons::POLYGON)))
                        .on_disabled_hover_text("Select closed polylines (lot boundaries)")
                        .clicked()
                    {
                        action = Some(ParcelsAction::CreateFromSelection);
                    }
                    if ui.add_enabled(!parcels.is_empty(), egui::Button::new("Report all")).clicked() {
                        let text: Vec<String> = parcels
                            .iter()
                            .filter_map(|p| parcel_report(doc, p, &tolerances))
                            .collect();
                        action = Some(ParcelsAction::Report("Parcels".into(), text.join("\n")));
                    }
                    if ui
                        .add_enabled(!parcels.is_empty(), egui::Button::new("Export LandXML…"))
                        .clicked()
                    {
                        action = Some(ParcelsAction::ExportLandXml);
                    }
                });
                ui.add_space(6.0);
                if parcels.is_empty() {
                    ui.label(
                        RichText::new("No parcels yet. Draw or select closed polylines for the lot boundaries, then “From selected polylines”.")
                            .color(theme.text_secondary),
                    );
                    return;
                }
                let mut total = 0.0;
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    egui::Grid::new("parcels").num_columns(8).spacing([10.0, 4.0]).striped(true).show(ui, |ui| {
                        for h in ["Name", "Plan", "Area", "Perimeter", "Plan closure", "Labels", "", ""] {
                            ui.label(RichText::new(h).small().color(theme.text_secondary));
                        }
                        ui.end_row();
                        for p in parcels {
                            let i = p.id;
                            let (mut name, mut plan) = match &self.editing {
                                Some((k, n, pl)) if *k == i => (n.clone(), pl.clone()),
                                _ => (p.name.clone(), p.plan.clone()),
                            };
                            let rn = ui.add_sized([90.0, 22.0], egui::TextEdit::singleline(&mut name));
                            let rp = ui.add_sized([90.0, 22.0], egui::TextEdit::singleline(&mut plan));
                            if rn.changed() || rp.changed() {
                                self.editing = Some((i, name.clone(), plan.clone()));
                            }
                            if (rn.lost_focus() || rp.lost_focus())
                                && let Some((k, n, pl)) = self.editing.take()
                                && k == i
                                && (n != p.name || pl != p.plan)
                            {
                                action = Some(ParcelsAction::Update(i, ParcelRecord { name: n, plan: pl, ..p.record.clone() }));
                            }
                            match boundary(doc, p) {
                                Some(l) => {
                                    let (_, fig) = plan_closure(l, &conv);
                                    // The area of the boundary as drawn (not of the rounded courses).
                                    let (_, _, area) = polyline_metrics(&l.vertices, &l.bulges, true);
                                    let area = area.abs();
                                    total += area;
                                    ui.label(RichText::new(conv.format_area(area)).font(fonts::mono(12.0)));
                                    ui.label(RichText::new(conv.format_length_with_unit(fig.perimeter)).font(fonts::mono(12.0)));
                                    let closure = match fig.precision() {
                                        Some(n) => format!("{} (1:{n:.0})", conv.format_length_with_unit(fig.misclosure.length())),
                                        None => "closes".into(),
                                    };
                                    let check = tolerances.check_linear(fig.misclosure.length(), fig.perimeter);
                                    let color = match check {
                                        Some(c) if !c.pass => theme.error,
                                        _ => theme.text,
                                    };
                                    ui.label(RichText::new(closure).font(fonts::mono(12.0)).color(color));
                                }
                                None => {
                                    ui.label(RichText::new("boundary missing").color(theme.error));
                                    ui.label("");
                                    ui.label("");
                                }
                            }
                            let mut annotate = p.annotate;
                            if ui.checkbox(&mut annotate, "").on_hover_text("Bearings, distances and area in the drawing").changed() {
                                action = Some(ParcelsAction::Update(i, ParcelRecord { annotate, ..p.record.clone() }));
                            }
                            ui.horizontal(|ui| {
                                if IconButton::new(icons::CROSSHAIR, "Select boundary").size(20.0).show(ui).clicked() {
                                    action = Some(ParcelsAction::Select(p.boundary));
                                }
                                if IconButton::new(icons::FILE_TEXT, "Report").size(20.0).show(ui).clicked()
                                    && let Some(text) = parcel_report(doc, p, &tolerances)
                                {
                                    action = Some(ParcelsAction::Report(format!("Parcel {}", p.name), text));
                                }
                            });
                            if IconButton::new(icons::X, "Remove parcel (keeps the polyline)").size(20.0).show(ui).clicked() {
                                action = Some(ParcelsAction::Delete(i));
                            }
                            ui.end_row();
                        }
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{} parcels, total {}", parcels.len(), conv.format_area(total)))
                        .color(theme.text_secondary),
                );
            });
        self.open = open;
        action
    }
}
