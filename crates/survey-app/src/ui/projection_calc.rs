//! Projection calculator: geographic ↔ grid with point scale factor,
//! grid convergence, height factor and combined scale factor.

use std::fmt::Write as _;

use egui::RichText;
use egui_cad::chrome::segmented;
use egui_cad::{CadTheme, fonts};
use glam::DVec3;
use survey_core::{Angle, Conventions};
use survey_model::{ChangeSet, Command, Document, Entity, EntityId, SurveyPoint};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    ToGrid,
    ToGeographic,
    Selected,
}

pub struct ProjectionWindow {
    pub open: bool,
    mode: Mode,
    lat: String,
    lon: String,
    easting: String,
    northing: String,
    height: String,
}

impl Default for ProjectionWindow {
    fn default() -> Self {
        Self {
            open: false,
            mode: Mode::Selected,
            lat: String::new(),
            lon: String::new(),
            easting: String::new(),
            northing: String::new(),
            height: "0".into(),
        }
    }
}

pub enum ProjectionAction {
    Create(Box<dyn Command>, String),
    Report(String, String),
}

/// One computed result.
struct Solved {
    lat: f64,
    lon: f64,
    easting: f64,
    northing: f64,
    height: f64,
    scale: f64,
    convergence: f64,
    height_factor: f64,
    name: Option<String>,
}

/// Latitude or longitude as signed d°m's.sssss" (finer than survey angles need).
fn geodetic(degrees: f64) -> String {
    let d = Angle::from_degrees(degrees.abs()).to_dms(5);
    format!(
        "{}{}°{:02}'{:08.5}\"",
        if degrees < 0.0 { "-" } else { "" },
        d.degrees,
        d.minutes,
        d.seconds
    )
}

fn signed_angle(conv: &Conventions, degrees: f64) -> String {
    let a = conv.format_angle(Angle::from_degrees(degrees.abs()));
    if degrees < 0.0 { format!("-{a}") } else { a }
}

impl ProjectionWindow {
    fn solve(&self, doc: &Document, selected: Option<EntityId>) -> Result<Solved, String> {
        let crs = &doc.meta().coord_system;
        let tm = crs
            .projection()
            .ok_or("Set a projection first (Project → Settings → Coordinates).")?;
        let conv = doc.conventions();
        let height = |text: &str| conv.parse_length(text).map_err(|e| format!("Height: {e}"));
        let (lat, lon, easting, northing, h, name) = match self.mode {
            Mode::ToGrid => {
                if self.lat.trim().is_empty() || self.lon.trim().is_empty() {
                    return Err("Enter a latitude and longitude (south and west negative).".into());
                }
                let lat = conv
                    .parse_angle(&self.lat)
                    .map_err(|e| format!("Latitude: {e}"))?
                    .degrees();
                let lon = conv
                    .parse_angle(&self.lon)
                    .map_err(|e| format!("Longitude: {e}"))?
                    .degrees();
                let g = tm.forward(lat, lon);
                (lat, lon, g.easting, g.northing, height(&self.height)?, None)
            }
            Mode::ToGeographic => {
                if self.easting.trim().is_empty() || self.northing.trim().is_empty() {
                    return Err("Enter an easting and northing.".into());
                }
                let e = conv
                    .parse_length(&self.easting)
                    .map_err(|e| format!("Easting: {e}"))?;
                let n = conv
                    .parse_length(&self.northing)
                    .map_err(|e| format!("Northing: {e}"))?;
                let g = tm.inverse(e, n);
                (g.latitude, g.longitude, e, n, height(&self.height)?, None)
            }
            Mode::Selected => {
                let (p, name) = match selected.and_then(|id| doc.entity(id)) {
                    Some(Entity::Point(p)) => (p.position, p.name.clone()),
                    _ => return Err("Select a point.".into()),
                };
                let g = tm.inverse(p.x, p.y);
                (g.latitude, g.longitude, p.x, p.y, p.z, Some(name))
            }
        };
        let g = tm.inverse(easting, northing);
        Ok(Solved {
            lat,
            lon,
            easting,
            northing,
            height: h,
            scale: g.scale,
            convergence: g.convergence,
            height_factor: tm.height_factor(lat, h + crs.geoid_separation()),
            name,
        })
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        selected: Option<EntityId>,
    ) -> Option<ProjectionAction> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let conv = doc.conventions().clone();
        let mut action = None;
        let mut open = self.open;
        egui::Window::new("Projection calculator")
            .open(&mut open)
            .default_width(440.0)
            .show(ctx, |ui| {
                ui.label(RichText::new(doc.meta().coord_system.display_name()).color(theme.text_secondary));
                segmented(
                    ui,
                    &mut self.mode,
                    &[
                        (Mode::Selected, "Selected point"),
                        (Mode::ToGrid, "Lat/long → grid"),
                        (Mode::ToGeographic, "Grid → lat/long"),
                    ],
                );
                ui.add_space(4.0);
                let field = |ui: &mut egui::Ui, label: &str, text: &mut String, hint: &str| {
                    ui.label(label);
                    ui.add_sized(
                        [180.0, 22.0],
                        egui::TextEdit::singleline(text)
                            .hint_text(hint)
                            .font(fonts::mono(12.0)),
                    );
                    ui.end_row();
                };
                egui::Grid::new("proj_inputs")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| match self.mode {
                        Mode::ToGrid => {
                            field(ui, "Latitude", &mut self.lat, "-37 57 03.7203 (south negative)");
                            field(ui, "Longitude", &mut self.lon, "144 25 29.5244 (west negative)");
                            field(ui, "Height", &mut self.height, "above the geoid");
                        }
                        Mode::ToGeographic => {
                            field(ui, "Easting", &mut self.easting, "");
                            field(ui, "Northing", &mut self.northing, "");
                            field(ui, "Height", &mut self.height, "above the geoid");
                        }
                        Mode::Selected => {
                            ui.label(
                                RichText::new("Uses the selected point's coordinates and height.").small(),
                            );
                            ui.end_row();
                        }
                    });
                ui.separator();
                match self.solve(doc, selected) {
                    Ok(r) => {
                        let text = describe(&conv, &r);
                        ui.label(RichText::new(&text).font(fonts::mono(12.0)));
                        ui.horizontal(|ui| {
                            if self.mode == Mode::ToGrid && ui.button("Create point").clicked() {
                                let name = doc.next_point_name();
                                let p = SurveyPoint {
                                    name: name.clone(),
                                    position: DVec3::new(r.easting, r.northing, r.height),
                                    height_known: true,
                                    code: String::new(),
                                    description: format!("From {} {}", geodetic(r.lat), geodetic(r.lon)),
                                    layer: doc.current_layer(),
                                };
                                action = Some(ProjectionAction::Create(
                                    Box::new(
                                        ChangeSet::new(format!("Point {name} from latitude/longitude"))
                                            .insert_entity(Entity::Point(p)),
                                    ),
                                    format!("Point {name} created"),
                                ));
                            }
                            if ui.button("Add to reports").clicked() {
                                action = Some(ProjectionAction::Report("Projection".into(), text.clone()));
                            }
                        });
                    }
                    Err(e) => {
                        ui.label(RichText::new(e).color(theme.text_secondary));
                    }
                }
            });
        self.open = open;
        action
    }
}

fn describe(conv: &Conventions, r: &Solved) -> String {
    let mut s = String::new();
    if let Some(n) = &r.name {
        let _ = writeln!(s, "Point            {n}");
    }
    let _ = writeln!(s, "Latitude         {}  ({:.9}°)", geodetic(r.lat), r.lat);
    let _ = writeln!(s, "Longitude        {}  ({:.9}°)", geodetic(r.lon), r.lon);
    let (ea, nb) = conv.horizontal_labels();
    let (a, b) = conv.horizontal_values(DVec3::new(r.easting, r.northing, 0.0));
    let _ = writeln!(s, "{ea:<16} {}", conv.format_length(a));
    let _ = writeln!(s, "{nb:<16} {}", conv.format_length(b));
    let _ = writeln!(s, "Height           {}", conv.format_length(r.height));
    let _ = writeln!(s, "Point scale      {:.9}", r.scale);
    let _ = writeln!(s, "Convergence      {}", signed_angle(conv, r.convergence));
    let _ = writeln!(s, "Height factor    {:.9}", r.height_factor);
    let _ = writeln!(s, "Combined factor  {:.9}", r.scale * r.height_factor);
    let _ = write!(s, "Grid distance = ground distance × combined factor");
    s
}
