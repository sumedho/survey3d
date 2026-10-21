//! Area subdivision of a parcel (a closed polyline): cut a given area off with
//! a line parallel to an edge, or with a line swung about a hinge point.

use egui::RichText;
use egui_cad::chrome::segmented;
use egui_cad::{CadTheme, fonts, icons};
use glam::{DVec2, DVec3};
use survey_core::arc::Arc;
use survey_core::subdivide::{Cut, parallel, swing};
use survey_core::{Bearing, Conventions};
use survey_model::{ChangeSet, Command, Document, Entity, EntityId, LayerId, Polyline};

/// Arc tessellation tolerance for parcels (metres).
const ARC_TOLERANCE: f64 = 0.0005;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    Parallel,
    Swing,
}

/// The parcel being divided, relative to `origin`.
struct Parcel {
    id: EntityId,
    name: String,
    layer: LayerId,
    origin: DVec3,
    polygon: Vec<DVec2>,
    /// Straight edges: (polygon edge index, original segment number).
    edges: Vec<(usize, usize)>,
    /// Polygon indices of the original corners.
    corners: Vec<usize>,
}

pub struct SubdivideWindow {
    pub open: bool,
    parcel: Option<Parcel>,
    method: Method,
    /// Index into `parcel.edges`.
    edge: usize,
    /// Hinge: a corner index, or a named point / coordinates when `None`.
    hinge_corner: Option<usize>,
    hinge_text: String,
    area: String,
    solution: usize,
}

impl Default for SubdivideWindow {
    fn default() -> Self {
        Self {
            open: false,
            parcel: None,
            method: Method::Parallel,
            edge: 0,
            hinge_corner: Some(0),
            hinge_text: String::new(),
            area: String::new(),
            solution: 0,
        }
    }
}

pub enum SubdivideAction {
    Draw(Box<dyn Command>, String),
    Report(String, String),
}

impl SubdivideWindow {
    /// Loads a closed polyline as the parcel; `false` if it is not closed.
    pub fn load(&mut self, id: EntityId, line: &Polyline) -> bool {
        if !line.closed || line.vertices.len() < 3 {
            return false;
        }
        let origin = line.vertices[0];
        let mut polygon = Vec::new();
        let mut edges = Vec::new();
        let mut corners = Vec::new();
        for (seg, (a, b, bulge)) in line.segments().enumerate() {
            corners.push(polygon.len());
            match Arc::from_bulge(a, b, bulge) {
                Some(arc) => {
                    let pts = arc.tessellate(ARC_TOLERANCE);
                    polygon.extend(pts[..pts.len() - 1].iter().map(|p| (*p - origin).truncate()));
                }
                None => {
                    edges.push((polygon.len(), seg));
                    polygon.push((a - origin).truncate());
                }
            }
        }
        self.parcel = Some(Parcel {
            id,
            name: if line.name.is_empty() {
                "Parcel".into()
            } else {
                line.name.clone()
            },
            layer: line.layer,
            origin,
            polygon,
            edges,
            corners,
        });
        self.edge = 0;
        self.hinge_corner = Some(0);
        self.solution = 0;
        self.open = true;
        true
    }

    fn hinge(&self, doc: &Document, parcel: &Parcel) -> Result<DVec2, String> {
        match self.hinge_corner {
            Some(i) => parcel
                .corners
                .get(i)
                .map(|k| parcel.polygon[*k])
                .ok_or_else(|| "Choose a corner".to_string()),
            None => {
                let t = self.hinge_text.trim();
                if t.is_empty() {
                    return Err("Enter the hinge point".into());
                }
                let p = match doc.find_point_by_name(t) {
                    Some((_, p)) => p.position,
                    None => {
                        let c = doc
                            .conventions()
                            .parse_coord(t)
                            .map_err(|e| format!("Hinge: not a point name or coordinates ({e})"))?;
                        DVec3::new(c.easting, c.northing, 0.0)
                    }
                };
                Ok((p - parcel.origin).truncate())
            }
        }
    }

    fn compute(&self, doc: &Document, parcel: &Parcel) -> Result<Vec<(Option<Bearing>, Cut)>, String> {
        let conv = doc.conventions();
        if self.area.trim().is_empty() {
            return Err("Enter the area to cut off.".into());
        }
        let unit = conv.linear_unit.metres_per_unit();
        let area = self
            .area
            .trim()
            .parse::<f64>()
            .map_err(|_| format!("Area: '{}' is not a number", self.area.trim()))?
            * unit
            * unit;
        match self.method {
            Method::Parallel => {
                let (edge, _) = *parcel
                    .edges
                    .get(self.edge)
                    .ok_or("The parcel has no straight edges")?;
                parallel(&parcel.polygon, edge, area)
                    .map(|c| vec![(None, c)])
                    .map_err(|e| e.to_string())
            }
            Method::Swing => {
                let hinge = self.hinge(doc, parcel)?;
                swing(&parcel.polygon, hinge, area)
                    .map(|v| v.into_iter().map(|(b, c)| (Some(b), c)).collect())
                    .map_err(|e| e.to_string())
            }
        }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        selected: Option<(EntityId, &Polyline)>,
    ) -> Option<SubdivideAction> {
        if !self.open {
            return None;
        }
        // Drop a parcel that was deleted or edited.
        if let Some(p) = &self.parcel
            && !matches!(doc.entity(p.id), Some(Entity::Polyline(l)) if l.vertices.first() == Some(&p.origin))
        {
            self.parcel = None;
        }
        let theme = CadTheme::current(ctx);
        let conv = doc.conventions().clone();
        let mut action = None;
        let mut open = self.open;
        let mut load = None;
        egui::Window::new("Subdivide area")
            .open(&mut open)
            .default_size([560.0, 460.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            selected.is_some_and(|(_, l)| l.closed),
                            egui::Button::new(format!("{} Use selected parcel", icons::POLYGON)),
                        )
                        .on_disabled_hover_text("Select a closed polyline")
                        .clicked()
                    {
                        load = selected.map(|(id, l)| (id, l.clone()));
                    }
                    if let Some(p) = &self.parcel {
                        ui.label(RichText::new(&p.name).strong());
                    }
                });
                ui.separator();
                let Some(parcel) = &self.parcel else {
                    ui.label(
                        RichText::new(
                            "Select a closed polyline (the parcel) and click “Use selected parcel”.",
                        )
                        .color(theme.text_secondary),
                    );
                    return;
                };
                let total = survey_core::subdivide::signed_area(&parcel.polygon).abs();
                ui.label(
                    RichText::new(format!("Parcel area {}", conv.format_area(total)))
                        .color(theme.text_secondary),
                );
                ui.add_space(4.0);
                egui::Grid::new("subdivide_form")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Method");
                        segmented(
                            ui,
                            &mut self.method,
                            &[
                                (Method::Parallel, "Parallel to an edge"),
                                (Method::Swing, "Swing about a point"),
                            ],
                        );
                        ui.end_row();
                        match self.method {
                            Method::Parallel => {
                                ui.label("Edge");
                                let label = |i: usize| edge_label(&conv, parcel, i);
                                egui::ComboBox::from_id_salt("subdivide_edge")
                                    .width(300.0)
                                    .selected_text(label(self.edge))
                                    .show_ui(ui, |ui| {
                                        for i in 0..parcel.edges.len() {
                                            ui.selectable_value(&mut self.edge, i, label(i));
                                        }
                                    });
                                ui.end_row();
                            }
                            Method::Swing => {
                                ui.label("Hinge");
                                ui.horizontal(|ui| {
                                    let text = match self.hinge_corner {
                                        Some(i) => format!("Corner {}", i + 1),
                                        None => "Point…".into(),
                                    };
                                    egui::ComboBox::from_id_salt("subdivide_hinge")
                                        .selected_text(text)
                                        .show_ui(ui, |ui| {
                                            for i in 0..parcel.corners.len() {
                                                ui.selectable_value(
                                                    &mut self.hinge_corner,
                                                    Some(i),
                                                    format!("Corner {}", i + 1),
                                                );
                                            }
                                            ui.selectable_value(&mut self.hinge_corner, None, "Point…");
                                        });
                                    if self.hinge_corner.is_none() {
                                        ui.add_sized(
                                            [180.0, 22.0],
                                            egui::TextEdit::singleline(&mut self.hinge_text)
                                                .hint_text("point name or coordinates")
                                                .font(fonts::mono(12.0)),
                                        );
                                    }
                                });
                                ui.end_row();
                            }
                        }
                        ui.label("Area to cut off");
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [120.0, 22.0],
                                egui::TextEdit::singleline(&mut self.area).font(fonts::mono(12.0)),
                            );
                            ui.label(format!("{}²", conv.linear_unit.suffix()));
                        });
                        ui.end_row();
                    });
                ui.add_space(6.0);
                ui.separator();
                match self.compute(doc, parcel) {
                    Ok(solutions) => {
                        if solutions.len() > 1 {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Solutions").color(theme.text_secondary));
                                self.solution = self.solution.min(solutions.len() - 1);
                                let opts: Vec<(usize, String)> =
                                    (0..solutions.len()).map(|i| (i, format!("{}", i + 1))).collect();
                                let opts: Vec<(usize, &str)> =
                                    opts.iter().map(|(i, s)| (*i, s.as_str())).collect();
                                segmented(ui, &mut self.solution, &opts);
                            });
                        }
                        let (bearing, cut) = &solutions[self.solution.min(solutions.len() - 1)];
                        let line = |ui: &mut egui::Ui, label: &str, value: String| {
                            ui.horizontal(|ui| {
                                ui.add_sized(
                                    [140.0, 16.0],
                                    egui::Label::new(RichText::new(label).color(theme.text_secondary)),
                                );
                                ui.label(RichText::new(value).font(fonts::mono(12.0)));
                            });
                        };
                        let (cut_brg, cut_len) = cut_line(cut);
                        if let Some(b) = bearing.or(cut_brg) {
                            line(ui, "Cut line bearing", conv.format_bearing(b));
                        }
                        line(ui, "Cut line length", conv.format_length_with_unit(cut_len));
                        line(ui, "Area cut off", conv.format_area(cut.part_area));
                        line(ui, "Remainder", conv.format_area(cut.remainder_area));
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui.button("Draw cut line").clicked() {
                                action = Some(self.draw(doc, parcel, cut, false));
                            }
                            if ui
                                .button("Draw both parcels")
                                .on_hover_text("Adds the two new parcels as closed polylines")
                                .clicked()
                            {
                                action = Some(self.draw(doc, parcel, cut, true));
                            }
                            if ui.button("Add to reports").clicked() {
                                action = Some(SubdivideAction::Report(
                                    format!("Subdivision {}", parcel.name),
                                    report(&conv, parcel, total, bearing.or(cut_brg), cut),
                                ));
                            }
                        });
                    }
                    Err(e) => {
                        ui.label(RichText::new(e).color(theme.text_secondary));
                    }
                }
            });
        if let Some((id, line)) = load {
            self.load(id, &line);
        }
        self.open = open;
        action
    }

    fn draw(&self, doc: &Document, parcel: &Parcel, cut: &Cut, parcels: bool) -> SubdivideAction {
        let to_world = |p: DVec2| parcel.origin + p.extend(0.0);
        let mut cs = ChangeSet::new(format!("Subdivide {}", parcel.name));
        for (a, b) in &cut.segments {
            cs.push_insert_entity(Entity::Polyline(Polyline {
                name: format!("{} cut", parcel.name),
                vertices: vec![to_world(*a), to_world(*b)],
                bulges: Vec::new(),
                closed: false,
                code: String::new(),
                layer: doc.current_layer(),
            }));
        }
        if parcels {
            for (suffix, ring) in [("A", &cut.part), ("B", &cut.remainder)] {
                cs.push_insert_entity(Entity::Polyline(Polyline {
                    name: format!("{} {suffix}", parcel.name),
                    vertices: ring.iter().map(|p| to_world(*p)).collect(),
                    bulges: Vec::new(),
                    closed: true,
                    code: String::new(),
                    layer: parcel.layer,
                }));
            }
        }
        let what = if parcels {
            "cut line and parcels"
        } else {
            "cut line"
        };
        SubdivideAction::Draw(
            Box::new(cs),
            format!("Subdivide {}: drew the {what}", parcel.name),
        )
    }
}

fn edge_label(conv: &Conventions, parcel: &Parcel, i: usize) -> String {
    let Some((k, seg)) = parcel.edges.get(i) else {
        return "—".into();
    };
    let a = parcel.polygon[*k];
    let b = parcel.polygon[(*k + 1) % parcel.polygon.len()];
    let d = b - a;
    let brg = Bearing::from_delta(d.x, d.y)
        .map(|b| conv.format_bearing(b))
        .unwrap_or_default();
    format!("Edge {}  {brg}  {}", seg + 1, conv.format_length(d.length()))
}

/// Bearing and total length of the cut line.
fn cut_line(cut: &Cut) -> (Option<Bearing>, f64) {
    let len = cut.segments.iter().map(|(a, b)| a.distance(*b)).sum();
    let brg = cut.segments.first().and_then(|(a, b)| {
        let d = *b - *a;
        Bearing::from_delta(d.x, d.y)
    });
    (brg, len)
}

fn report(conv: &Conventions, parcel: &Parcel, total: f64, bearing: Option<Bearing>, cut: &Cut) -> String {
    let mut s = format!("SUBDIVISION  {}\n\n", parcel.name);
    s.push_str(&format!("Parcel area     {}\n", conv.format_area(total)));
    s.push_str(&format!("Area cut off    {}\n", conv.format_area(cut.part_area)));
    s.push_str(&format!(
        "Remainder       {}\n\n",
        conv.format_area(cut.remainder_area)
    ));
    if let Some(b) = bearing {
        s.push_str(&format!("Cut bearing     {}\n", conv.format_bearing(b)));
    }
    for (i, (a, b)) in cut.segments.iter().enumerate() {
        let (pa, pb) = (parcel.origin + a.extend(0.0), parcel.origin + b.extend(0.0));
        s.push_str(&format!(
            "Cut {}  from {}\n       to   {}\n       length {}\n",
            i + 1,
            conv.format_coord(pa, false),
            conv.format_coord(pb, false),
            conv.format_length_with_unit(a.distance(*b))
        ));
    }
    s
}
