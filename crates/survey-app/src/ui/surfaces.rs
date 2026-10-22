//! Surfaces: the side-panel list (visibility, style, rebuild) and the window
//! that defines a surface from layers, codes, breaklines and a boundary.

use egui::RichText;
use egui_cad::chrome::{IconButton, switch_row};
use egui_cad::{CadTheme, fonts, icons};
use survey_core::tin::{Tin, TinError, build};
use survey_model::{Document, Entity, EntityId, Surface, SurfaceDefinition, SurfaceId, SurfaceStyle};

pub enum SurfaceAction {
    New,
    Edit(SurfaceId),
    Rebuild(SurfaceId),
    Delete(SurfaceId),
    Style(SurfaceId, SurfaceStyle),
}

/// Builds the surface a definition describes from the current project data.
pub fn build_from(doc: &Document, def: &SurfaceDefinition) -> Result<Tin, TinError> {
    build(&def.gather(doc))
}

/// The Surfaces section body: one row per surface, and the active one's style.
pub fn surfaces_panel(
    ui: &mut egui::Ui,
    doc: &Document,
    active: &mut Option<SurfaceId>,
) -> Option<SurfaceAction> {
    let theme = CadTheme::current(ui.ctx());
    let mut action = None;
    if doc.surfaces().next().is_none() {
        ui.label(
            RichText::new(
                "No surfaces yet. Surface → Create surface… builds one from points and breaklines.",
            )
            .small()
            .color(theme.text_secondary),
        );
        return None;
    }
    if active.is_some_and(|id| doc.surface(id).is_none()) {
        *active = None;
    }
    for (id, s) in doc.surfaces() {
        let selected = *active == Some(id);
        ui.horizontal(|ui| {
            let eye = if s.style.visible {
                icons::EYE
            } else {
                icons::EYE_SLASH
            };
            if IconButton::new(eye, if s.style.visible { "Hide" } else { "Show" })
                .size(20.0)
                .show(ui)
                .clicked()
            {
                let mut style = s.style;
                style.visible = !style.visible;
                action = Some(SurfaceAction::Style(id, style));
            }
            let label = ui.selectable_label(selected, RichText::new(&s.name).strong());
            if label.clicked() {
                *active = if selected { None } else { Some(id) };
            }
            ui.label(
                RichText::new(format!("{} tri", s.tin.triangles.len()))
                    .small()
                    .color(theme.text_secondary),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if IconButton::new(icons::TRASH, "Delete surface")
                    .size(20.0)
                    .show(ui)
                    .clicked()
                {
                    action = Some(SurfaceAction::Delete(id));
                }
                if s.definition.is_some() {
                    if IconButton::new(icons::GEAR, "Edit definition")
                        .size(20.0)
                        .show(ui)
                        .clicked()
                    {
                        action = Some(SurfaceAction::Edit(id));
                    }
                    if IconButton::new(icons::ARROW_CLOCKWISE, "Rebuild")
                        .description("Re-triangulate from the current points and breaklines")
                        .size(20.0)
                        .show(ui)
                        .clicked()
                    {
                        action = Some(SurfaceAction::Rebuild(id));
                    }
                }
            });
        });
    }
    if let Some(id) = *active
        && let Some(s) = doc.surface(id)
    {
        ui.add_space(4.0);
        let mut style = s.style;
        egui::Frame::new()
            .fill(theme.bg)
            .stroke(egui::Stroke::new(1.0, theme.border))
            .corner_radius(theme.radius)
            .inner_margin(6.0)
            .show(ui, |ui| {
                switch_row(ui, &mut style.shaded, "Shaded");
                ui.add_enabled_ui(style.shaded, |ui| {
                    switch_row(ui, &mut style.elevation_colors, "Colour by height");
                });
                switch_row(ui, &mut style.triangles, "Triangle edges");
                ui.horizontal(|ui| {
                    ui.label("Colour");
                    let mut rgb = [style.color.0[0], style.color.0[1], style.color.0[2]];
                    if ui.color_edit_button_srgb(&mut rgb).changed() {
                        style.color.0 = [rgb[0], rgb[1], rgb[2], 255];
                    }
                });
                ui.separator();
                switch_row(ui, &mut style.contours.show, "Contours");
                ui.add_enabled_ui(style.contours.show, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Interval");
                        ui.add(
                            egui::DragValue::new(&mut style.contours.interval)
                                .range(0.01..=1000.0)
                                .speed(0.05)
                                .suffix(" m"),
                        );
                        ui.label("major every");
                        ui.add(egui::DragValue::new(&mut style.contours.major_every).range(1..=100));
                    });
                    switch_row(ui, &mut style.contours.labels, "Label major contours");
                });
                let t = &s.tin;
                let b = t.bounds();
                let conv = doc.conventions();
                ui.label(
                    RichText::new(format!(
                        "{} points, {} triangles · heights {} to {} · plan area {}",
                        t.vertices.len(),
                        t.triangles.len(),
                        conv.format_length(b.min.z),
                        conv.format_length(b.max.z),
                        conv.format_area(t.plan_area())
                    ))
                    .small()
                    .color(theme.text_secondary),
                );
            });
        if style != s.style {
            action = Some(SurfaceAction::Style(id, style));
        }
    }
    action
}

// ---------------------------------------------------------------- definition window

/// Create or edit a surface definition.
#[derive(Default)]
pub struct SurfaceWindow {
    pub open: bool,
    /// Editing an existing surface; `None` creates a new one.
    editing: Option<SurfaceId>,
    name: String,
    all_point_layers: bool,
    point_layers: Vec<String>,
    point_codes: String,
    breakline_layers: Vec<String>,
    breakline_codes: String,
    boundary: Option<EntityId>,
    holes: Vec<EntityId>,
    max_edge: Option<f64>,
    error: Option<String>,
}

pub enum SurfaceWindowAction {
    Create(Surface),
    Update(SurfaceId, Surface),
}

fn split_codes(text: &str) -> Vec<String> {
    text.split([',', ' '])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

impl SurfaceWindow {
    pub fn create(&mut self, doc: &Document) {
        let n = doc.surfaces().count() + 1;
        *self = Self {
            open: true,
            name: if n == 1 {
                "Existing".into()
            } else {
                format!("Surface {n}")
            },
            all_point_layers: true,
            ..Default::default()
        };
    }

    pub fn edit(&mut self, doc: &Document, id: SurfaceId) {
        let Some(s) = doc.surface(id) else {
            return;
        };
        let def = s.definition.clone().unwrap_or_default();
        *self = Self {
            open: true,
            editing: Some(id),
            name: s.name.clone(),
            all_point_layers: def.point_layers.is_empty(),
            point_layers: def.point_layers,
            point_codes: def.point_codes.join(", "),
            breakline_layers: def.breakline_layers,
            breakline_codes: def.breakline_codes.join(", "),
            boundary: def.boundary,
            holes: def.holes,
            max_edge: def.max_edge,
            error: None,
        };
    }

    fn definition(&self) -> SurfaceDefinition {
        SurfaceDefinition {
            point_layers: if self.all_point_layers {
                Vec::new()
            } else {
                self.point_layers.clone()
            },
            point_codes: split_codes(&self.point_codes),
            breakline_layers: self.breakline_layers.clone(),
            breakline_codes: split_codes(&self.breakline_codes),
            boundary: self.boundary,
            holes: self.holes.clone(),
            max_edge: self.max_edge,
        }
    }

    /// `selected` are the selected closed polylines (for boundary and holes).
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        selected: &[EntityId],
    ) -> Option<SurfaceWindowAction> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let mut action = None;
        let mut open = self.open;
        let title = if self.editing.is_some() {
            "Edit surface"
        } else {
            "Create surface"
        };
        egui::Window::new(title)
            .open(&mut open)
            .default_width(460.0)
            .show(ctx, |ui| {
                egui::Grid::new("surface_name")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Name");
                        ui.add_sized([220.0, 22.0], egui::TextEdit::singleline(&mut self.name));
                        ui.end_row();
                    });
                ui.add_space(4.0);
                ui.label(
                    RichText::new("POINTS (with heights)")
                        .font(fonts::medium(11.0))
                        .color(theme.text_secondary),
                );
                switch_row(ui, &mut self.all_point_layers, "From every layer");
                if !self.all_point_layers {
                    layer_checklist(ui, doc, &mut self.point_layers, "surface_point_layers");
                }
                ui.horizontal(|ui| {
                    ui.label("Codes");
                    ui.add_sized(
                        [240.0, 22.0],
                        egui::TextEdit::singleline(&mut self.point_codes)
                            .hint_text("any code (or e.g. NS, TB*, BB*)")
                            .font(fonts::mono(12.0)),
                    );
                });
                ui.add_space(6.0);
                ui.label(
                    RichText::new("BREAKLINES")
                        .font(fonts::medium(11.0))
                        .color(theme.text_secondary),
                );
                ui.label(
                    RichText::new(
                        "Polylines on these layers or with these codes (their vertex heights are used).",
                    )
                    .small()
                    .color(theme.text_secondary),
                );
                layer_checklist(ui, doc, &mut self.breakline_layers, "surface_break_layers");
                ui.horizontal(|ui| {
                    ui.label("Codes");
                    ui.add_sized(
                        [240.0, 22.0],
                        egui::TextEdit::singleline(&mut self.breakline_codes)
                            .hint_text("e.g. KB*, TB, BB")
                            .font(fonts::mono(12.0)),
                    );
                });
                ui.add_space(6.0);
                ui.label(
                    RichText::new("LIMITS")
                        .font(fonts::medium(11.0))
                        .color(theme.text_secondary),
                );
                ui.horizontal(|ui| {
                    ui.label(match self.boundary {
                        Some(id) => format!("Boundary: {}", entity_name(doc, id)),
                        None => "Boundary: none".into(),
                    });
                    let one = selected.len() == 1;
                    if ui
                        .add_enabled(one, egui::Button::new("Use selected"))
                        .on_disabled_hover_text("Select one closed polyline")
                        .clicked()
                    {
                        self.boundary = selected.first().copied();
                    }
                    if self.boundary.is_some() && ui.button("Clear").clicked() {
                        self.boundary = None;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(format!("Holes: {}", self.holes.len()));
                    if ui
                        .add_enabled(!selected.is_empty(), egui::Button::new("Add selected"))
                        .on_disabled_hover_text("Select closed polylines")
                        .clicked()
                    {
                        for id in selected {
                            if !self.holes.contains(id) && Some(*id) != self.boundary {
                                self.holes.push(*id);
                            }
                        }
                    }
                    if !self.holes.is_empty() && ui.button("Clear").clicked() {
                        self.holes.clear();
                    }
                });
                ui.horizontal(|ui| {
                    let mut on = self.max_edge.is_some();
                    ui.checkbox(&mut on, "Maximum triangle edge");
                    let mut v = self.max_edge.unwrap_or(50.0);
                    ui.add_enabled(
                        on,
                        egui::DragValue::new(&mut v)
                            .range(0.1..=100_000.0)
                            .speed(0.5)
                            .suffix(" m"),
                    );
                    self.max_edge = on.then_some(v);
                });
                ui.add_space(6.0);
                ui.separator();
                let def = self.definition();
                let input = def.gather(doc);
                ui.label(
                    RichText::new(format!(
                        "{} points with heights, {} breaklines{}",
                        input.points.len(),
                        input.breaklines.len(),
                        if input.boundary.is_some() {
                            ", boundary"
                        } else {
                            ""
                        }
                    ))
                    .color(theme.text_secondary),
                );
                ui.horizontal(|ui| {
                    let verb = if self.editing.is_some() {
                        "Rebuild"
                    } else {
                        "Create"
                    };
                    if ui
                        .button(format!("{} {verb} surface", icons::MAGIC_WAND))
                        .clicked()
                    {
                        match build(&input) {
                            Ok(tin) => {
                                let name = if self.name.trim().is_empty() {
                                    "Surface".to_string()
                                } else {
                                    self.name.trim().to_string()
                                };
                                let style = self
                                    .editing
                                    .and_then(|id| doc.surface(id))
                                    .map_or_else(SurfaceStyle::default, |s| s.style);
                                let surface = Surface {
                                    name,
                                    definition: Some(def.clone()),
                                    tin,
                                    style,
                                };
                                action = Some(match self.editing {
                                    Some(id) => SurfaceWindowAction::Update(id, surface),
                                    None => SurfaceWindowAction::Create(surface),
                                });
                            }
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                });
                if let Some(e) = &self.error {
                    ui.label(RichText::new(e).color(theme.error));
                }
            });
        if action.is_some() {
            open = false;
        }
        if !open {
            self.error = None;
        }
        self.open = open;
        action
    }
}

fn entity_name(doc: &Document, id: EntityId) -> String {
    match doc.entity(id) {
        Some(Entity::Polyline(l)) if !l.name.is_empty() => l.name.clone(),
        Some(_) => format!("polyline #{}", id.0),
        None => "(deleted)".into(),
    }
}

fn layer_checklist(ui: &mut egui::Ui, doc: &Document, chosen: &mut Vec<String>, id: &str) {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(110.0)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for l in doc.layers() {
                    let mut on = chosen.iter().any(|n| n.eq_ignore_ascii_case(&l.name));
                    if ui.checkbox(&mut on, &l.name).changed() {
                        if on {
                            chosen.push(l.name.clone());
                        } else {
                            chosen.retain(|n| !n.eq_ignore_ascii_case(&l.name));
                        }
                    }
                }
            });
        });
}
