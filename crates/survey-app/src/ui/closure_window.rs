//! Figure (lot) closure: misclosure, precision and area, and solving missing values.
//!
//! Courses are typed (or loaded from a selected polyline); a blank bearing or
//! distance is a missing value. With none missing the closure is checked; with
//! exactly two missing they are solved so the figure closes.

use egui::RichText;
use egui_cad::chrome::{IconButton, segmented, switch_row};
use egui_cad::{CadTheme, fonts, icons};
use glam::DVec3;
use survey_core::accuracy::Tolerances;
use survey_core::closure::{
    Course, CourseArc, Figure, Solution, Unknown, courses_from_polyline, figure, report, solve,
};
use survey_core::{Bearing, Conventions};
use survey_model::{ChangeSet, Command, Document, Entity, Polyline};

#[derive(Clone, Default)]
struct Row {
    arc: bool,
    bearing: String,
    /// Line length, or chord length for arcs.
    distance: String,
    radius: String,
    arc_length: String,
    right: bool,
}

pub struct ClosureWindow {
    pub open: bool,
    title: String,
    /// Point name or coordinates of the first corner; only needed to draw the figure.
    start: String,
    rows: Vec<Row>,
    /// Which solution is shown when there are two.
    solution: usize,
    closed_polyline: bool,
}

impl Default for ClosureWindow {
    fn default() -> Self {
        Self {
            open: false,
            title: String::new(),
            start: String::new(),
            rows: vec![Row::default(); 3],
            solution: 0,
            closed_polyline: true,
        }
    }
}

pub enum ClosureAction {
    /// Draw the figure: the command and a one-line summary.
    Draw(Box<dyn Command>, String),
    /// Add the report (title, text) to the Reports window.
    Report(String, String),
}

impl ClosureWindow {
    /// Fills the form from a polyline (its first vertex becomes the start).
    pub fn load_polyline(&mut self, doc: &Document, line: &Polyline) {
        let conv = doc.conventions();
        let courses = courses_from_polyline(&line.vertices, &line.bulges, line.closed);
        self.rows = courses
            .iter()
            .map(|c| {
                let fmt = |v: Option<f64>| v.map(|d| conv.format_length(d)).unwrap_or_default();
                Row {
                    arc: c.arc.is_some(),
                    bearing: c.bearing.map(|b| conv.format_bearing(b)).unwrap_or_default(),
                    distance: fmt(c.distance),
                    radius: fmt(c.arc.map(|a| a.radius)),
                    arc_length: if c.arc.is_some() {
                        fmt(c.length())
                    } else {
                        String::new()
                    },
                    right: c.arc.is_some_and(|a| a.right),
                }
            })
            .collect();
        if self.rows.is_empty() {
            self.rows.push(Row::default());
        }
        let first = line.vertices.first().copied().unwrap_or_default();
        self.start = doc
            .entities()
            .find_map(|(_, e)| match e {
                Entity::Point(p) if p.position.truncate().distance(first.truncate()) < 1e-6 => {
                    Some(p.name.clone())
                }
                _ => None,
            })
            .unwrap_or_else(|| {
                let (a, b) = conv.horizontal_values(first);
                format!("{}, {}", conv.format_length(a), conv.format_length(b))
            });
        self.title = if line.name.is_empty() {
            "Polyline".into()
        } else {
            line.name.clone()
        };
        self.solution = 0;
        self.open = true;
    }

    fn courses(&self, conv: &Conventions) -> Result<Vec<Course>, String> {
        let blank = |s: &str| s.trim().is_empty();
        let mut out = Vec::with_capacity(self.rows.len());
        for (i, r) in self.rows.iter().enumerate() {
            let n = i + 1;
            let length = |label: &str, text: &str| -> Result<Option<f64>, String> {
                if blank(text) {
                    return Ok(None);
                }
                conv.parse_length(text)
                    .map(Some)
                    .map_err(|e| format!("Course {n} {label}: {e}"))
            };
            let bearing = if blank(&r.bearing) {
                None
            } else {
                Some(
                    conv.parse_bearing(&r.bearing)
                        .map_err(|e| format!("Course {n} bearing: {e}"))?,
                )
            };
            let mut distance = length("distance", &r.distance)?;
            let arc = if r.arc {
                let radius =
                    length("radius", &r.radius)?.ok_or(format!("Course {n}: enter the arc's radius"))?;
                let arc_length = length("arc length", &r.arc_length)?;
                if distance.is_none()
                    && let Some(l) = arc_length
                {
                    distance = Some(
                        Course::chord_from_arc_length(radius, l)
                            .ok_or(format!("Course {n}: the arc length does not fit the radius"))?,
                    );
                }
                Some(CourseArc {
                    radius,
                    right: r.right,
                    large: arc_length.is_some_and(|l| l > std::f64::consts::PI * radius),
                })
            } else {
                None
            };
            out.push(Course {
                bearing,
                distance,
                arc,
            });
        }
        Ok(out)
    }

    /// Start of the figure: a point name or typed coordinates.
    fn start(&self, doc: &Document) -> Option<DVec3> {
        let text = self.start.trim();
        if text.is_empty() {
            return None;
        }
        if let Some((_, p)) = doc.find_point_by_name(text) {
            return Some(p.position);
        }
        let c = doc.conventions().parse_coord(text).ok()?;
        Some(DVec3::new(c.easting, c.northing, c.height.unwrap_or(0.0)))
    }

    fn draw(&self, doc: &Document, start: DVec3, fig: &Figure) -> (ChangeSet, String) {
        let mut vertices: Vec<DVec3> = fig.vertices.iter().map(|v| start + v.extend(0.0)).collect();
        let closes = fig.precision().is_none();
        if closes {
            vertices.pop();
        }
        let name = if self.title.trim().is_empty() {
            "Closure".to_string()
        } else {
            self.title.trim().to_string()
        };
        let cs = ChangeSet::new(format!("Draw figure {name}")).insert_entity(Entity::Polyline(Polyline {
            name: name.clone(),
            vertices,
            bulges: fig.bulges.clone(),
            closed: closes && self.closed_polyline,
            code: String::new(),
            layer: doc.current_layer(),
        }));
        (cs, format!("Closure: drew {name}"))
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        selected: Option<&Polyline>,
    ) -> Option<ClosureAction> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let conv = doc.conventions().clone();
        let tolerances = doc.meta().tolerances;
        let mut action = None;
        let mut open = self.open;
        let mut load = None;
        egui::Window::new("Closure")
            .open(&mut open)
            .default_size([780.0, 560.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            selected.is_some(),
                            egui::Button::new(format!("{} From selected polyline", icons::LINE_SEGMENTS)),
                        )
                        .on_disabled_hover_text("Select a polyline first")
                        .clicked()
                    {
                        load = selected.cloned();
                    }
                    if ui.button("Clear").clicked() {
                        *self = Self {
                            open: true,
                            ..Self::default()
                        };
                    }
                });
                ui.add_space(6.0);
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    self.form(ui, &theme);
                    ui.add_space(8.0);
                    ui.separator();
                    match self
                        .courses(&conv)
                        .and_then(|c| solve(&c).map_err(|e| e.to_string()))
                    {
                        Ok(solutions) => {
                            self.results(ui, &theme, &conv, &tolerances, doc, &solutions, &mut action);
                        }
                        Err(e) => {
                            ui.label(RichText::new(e).color(theme.text_secondary));
                        }
                    }
                });
            });
        if let Some(line) = load {
            self.load_polyline(doc, &line);
        }
        self.open = open;
        action
    }

    #[allow(clippy::too_many_arguments)]
    fn results(
        &mut self,
        ui: &mut egui::Ui,
        theme: &CadTheme,
        conv: &Conventions,
        tolerances: &Tolerances,
        doc: &Document,
        solutions: &[Solution],
        action: &mut Option<ClosureAction>,
    ) {
        if solutions.len() > 1 {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Two solutions").color(theme.text_secondary));
                self.solution = self.solution.min(solutions.len() - 1);
                segmented(ui, &mut self.solution, &[(0, "Solution 1"), (1, "Solution 2")]);
            });
            ui.add_space(4.0);
        }
        let sol = &solutions[self.solution.min(solutions.len() - 1)];
        let fig = figure(&sol.courses);
        let line = |ui: &mut egui::Ui, label: &str, value: String, color: egui::Color32| {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [150.0, 16.0],
                    egui::Label::new(RichText::new(label).color(theme.text_secondary)),
                );
                ui.label(RichText::new(value).font(fonts::mono(12.0)).color(color));
            });
        };
        for u in &sol.solved {
            let c = sol.courses[u.course()];
            let (label, value) = match u {
                Unknown::Bearing(i) => (
                    format!("Course {} bearing", i + 1),
                    c.bearing.map(|b| conv.format_bearing(b)).unwrap_or_default(),
                ),
                Unknown::Distance(i) => (
                    format!(
                        "Course {} {}",
                        i + 1,
                        if c.arc.is_some() { "chord" } else { "distance" }
                    ),
                    c.distance
                        .map(|d| conv.format_length_with_unit(d))
                        .unwrap_or_default(),
                ),
            };
            line(ui, &label, value, theme.accent);
            if let (Unknown::Distance(_), Some(l)) = (u, c.arc.and(c.length())) {
                line(
                    ui,
                    "      arc length",
                    conv.format_length_with_unit(l),
                    theme.accent,
                );
            }
        }
        for i in &sol.reversed {
            line(
                ui,
                &format!("Course {}", i + 1),
                "solved against its given bearing (reversed)".into(),
                theme.warning,
            );
        }
        if !sol.solved.is_empty() {
            ui.add_space(4.0);
        }
        let m = fig.misclosure;
        match (fig.precision(), Bearing::from_delta(m.x, m.y)) {
            (Some(n), Some(b)) => {
                line(
                    ui,
                    "Misclosure",
                    conv.format_length_with_unit(m.length()),
                    theme.text,
                );
                line(ui, "Misclosure bearing", conv.format_bearing(b), theme.text);
                line(ui, "Precision", format!("1 : {n:.0}"), theme.text);
            }
            _ => line(ui, "Misclosure", "closes exactly".into(), theme.success),
        }
        if let Some(c) = tolerances.check_linear(m.length(), fig.perimeter) {
            let (text, color) = if c.pass {
                ("PASS", theme.success)
            } else {
                ("FAIL", theme.error)
            };
            line(
                ui,
                "Allowed",
                format!("{}  {text}", conv.format_length_with_unit(c.allowed)),
                color,
            );
        }
        line(
            ui,
            "Perimeter",
            conv.format_length_with_unit(fig.perimeter),
            theme.text,
        );
        line(ui, "Area", conv.format_area(fig.area), theme.text);
        ui.add_space(6.0);
        let start = self.start(doc);
        ui.horizontal(|ui| {
            if !sol.solved.is_empty() && ui.button("Fill in solved values").clicked() {
                self.fill(conv, sol);
            }
            let draw = ui
                .add_enabled(start.is_some(), egui::Button::new("Draw figure"))
                .on_disabled_hover_text("Enter a start point to draw the figure");
            if draw.clicked()
                && let Some(start) = start
            {
                let (cs, summary) = self.draw(doc, start, &fig);
                *action = Some(ClosureAction::Draw(Box::new(cs), summary));
            }
            if ui.button("Add to reports").clicked() {
                let title = if self.title.trim().is_empty() {
                    "Figure".to_string()
                } else {
                    self.title.trim().to_string()
                };
                *action = Some(ClosureAction::Report(
                    format!("Closure {title}"),
                    report(&title, sol, &fig, tolerances, conv),
                ));
            }
        });
    }

    /// Writes solved values back into the form.
    fn fill(&mut self, conv: &Conventions, sol: &Solution) {
        for u in &sol.solved {
            let c = sol.courses[u.course()];
            let row = &mut self.rows[u.course()];
            match u {
                Unknown::Bearing(_) => {
                    row.bearing = c.bearing.map(|b| conv.format_bearing(b)).unwrap_or_default();
                }
                Unknown::Distance(_) => {
                    row.distance = c.distance.map(|d| conv.format_length(d)).unwrap_or_default();
                    if c.arc.is_some() {
                        row.arc_length = c.length().map(|l| conv.format_length(l)).unwrap_or_default();
                    }
                }
            }
        }
        for i in &sol.reversed {
            let c = sol.courses[*i];
            self.rows[*i].bearing = c.bearing.map(|b| conv.format_bearing(b)).unwrap_or_default();
        }
        self.solution = 0;
    }

    fn form(&mut self, ui: &mut egui::Ui, theme: &CadTheme) {
        let field = |ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32| {
            ui.add_sized(
                [width, 22.0],
                egui::TextEdit::singleline(text)
                    .hint_text(hint)
                    .font(fonts::mono(12.0)),
            );
        };
        egui::Grid::new("closure_setup")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Figure");
                field(ui, &mut self.title, "name (optional)", 200.0);
                ui.end_row();
                ui.label("Start");
                field(ui, &mut self.start, "point or coordinates, to draw it", 240.0);
                ui.end_row();
            });
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "Leave a bearing or distance blank to solve it — none blank checks the closure, exactly two are solved.",
            )
            .small()
            .color(theme.text_secondary),
        );
        ui.add_space(4.0);
        let mut remove = None;
        egui::Grid::new("closure_courses")
            .num_columns(8)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                for h in [
                    "#",
                    "Type",
                    "Bearing",
                    "Distance / chord",
                    "Radius",
                    "Arc length",
                    "Turn",
                    "",
                ] {
                    ui.label(RichText::new(h).small().color(theme.text_secondary));
                }
                ui.end_row();
                for (i, r) in self.rows.iter_mut().enumerate() {
                    ui.label(RichText::new((i + 1).to_string()).color(theme.text_secondary));
                    segmented(ui, &mut r.arc, &[(false, "Line"), (true, "Arc")]);
                    field(ui, &mut r.bearing, "", 150.0);
                    field(ui, &mut r.distance, "", 100.0);
                    if r.arc {
                        field(ui, &mut r.radius, "", 80.0);
                        field(ui, &mut r.arc_length, "", 90.0);
                        segmented(ui, &mut r.right, &[(false, "Left"), (true, "Right")]);
                    } else {
                        ui.label("");
                        ui.label("");
                        ui.label("");
                    }
                    if IconButton::new(icons::X, "Remove course")
                        .size(20.0)
                        .show(ui)
                        .clicked()
                    {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = remove {
            self.rows.remove(i);
            if self.rows.is_empty() {
                self.rows.push(Row::default());
            }
        }
        if ui.button(format!("{} Add course", icons::PLUS)).clicked() {
            self.rows.push(Row::default());
        }
        ui.add_space(4.0);
        switch_row(
            ui,
            &mut self.closed_polyline,
            "Draw a closing figure as a closed polyline",
        );
    }
}
