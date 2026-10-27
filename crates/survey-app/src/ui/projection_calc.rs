//! Projection calculator: geographic ↔ grid with point scale factor,
//! grid convergence, height factor and combined scale factor.

use egui::RichText;
use egui_cad::chrome::segmented;
use egui_cad::{CadTheme, fonts};
use glam::DVec3;
use survey_model::{ChangeSet, Command, Document, Entity, EntityId, SurveyPoint};

use crate::compute::projection::{Converted, HeightKind, from_geographic, from_grid, geodetic, report};

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
    /// What a typed latitude/longitude's height is above.
    height_kind: HeightKind,
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
            height_kind: HeightKind::Orthometric,
        }
    }
}

pub enum ProjectionAction {
    Create(Box<dyn Command>, String),
    Report(String, String),
}

impl ProjectionWindow {
    fn solve(&self, doc: &Document, selected: Option<EntityId>) -> Result<Converted, String> {
        let conv = doc.conventions();
        let height = |text: &str| conv.parse_length(text).map_err(|e| format!("Height: {e}"));
        match self.mode {
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
                from_geographic(doc, lat, lon, height(&self.height)?, self.height_kind, None)
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
                from_grid(doc, DVec3::new(e, n, height(&self.height)?), None)
            }
            Mode::Selected => match selected.and_then(|id| doc.entity(id)) {
                Some(Entity::Point(p)) => from_grid(doc, p.position, Some(p.name.clone())),
                _ => Err("Select a point.".into()),
            },
        }
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
                            field(ui, "Height", &mut self.height, "");
                            ui.label("");
                            segmented(
                                ui,
                                &mut self.height_kind,
                                &[
                                    (HeightKind::Orthometric, "Above the geoid (H)"),
                                    (HeightKind::Ellipsoidal, "Above the ellipsoid (h)"),
                                ],
                            );
                            ui.end_row();
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
                        let text = report(&conv, &r);
                        ui.label(RichText::new(&text).font(fonts::mono(12.0)));
                        ui.horizontal(|ui| {
                            if self.mode == Mode::ToGrid && ui.button("Create point").clicked() {
                                let name = doc.next_point_name();
                                let p = SurveyPoint {
                                    name: name.clone(),
                                    position: DVec3::new(r.easting, r.northing, r.height),
                                    height_known: true,
                                    code: String::new(),
                                    description: format!(
                                        "From {} {}",
                                        geodetic(r.latitude),
                                        geodetic(r.longitude)
                                    ),
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
