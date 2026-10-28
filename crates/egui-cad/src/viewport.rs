//! 3D/plan viewport widget.
//!
//! Navigation (defaults):
//! * wheel / pinch — zoom about the cursor
//! * middle-drag, or Shift + right-drag — pan
//! * right-drag — orbit about the centre of rotation when one is set, else the
//!   point under the cursor (disabled in plan lock)
//! * view cube (bottom-left) — click a face, edge or corner for that view
//!
//! Primary clicks and right clicks without dragging are reported to the
//! application, which owns tools and picking.

use std::sync::Arc;

use cad_render::{Camera, CloudFrame, GridSettings, Projection, Scene, ViewportCallback, ViewportFrame};
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};
use survey_core::Aabb3;

use crate::chrome::{self, IconButton};
use crate::fonts;
use crate::icons;
use crate::theme::CadTheme;

/// Persistent per-viewport state.
#[derive(Clone, Debug)]
pub struct ViewportState {
    pub camera: Camera,
    /// Distinguishes multiple viewports in the renderer.
    pub view_id: u64,
    /// When set the view is top-down orthographic and cannot be orbited.
    pub plan_locked: bool,
    orbit_pivot: Option<DVec3>,
    /// A centre of rotation the user set; orbiting turns about it (instead
    /// of the point under the cursor) until it is cleared.
    pub pivot: Option<DVec3>,
}

impl ViewportState {
    pub fn new(view_id: u64) -> Self {
        Self {
            camera: Camera::default(),
            view_id,
            plan_locked: true,
            orbit_pivot: None,
            pivot: None,
        }
    }

    /// Switches between locked plan view and free 3D. Locking returns to a
    /// north-up plan so grid axes are orthogonal to the screen.
    pub fn set_plan_locked(&mut self, locked: bool) {
        self.plan_locked = locked;
        if locked {
            self.lock_plan();
        } else if self.camera.is_plan() {
            // Tilt a little so it is obvious the view is now 3D.
            let target = self.camera.target;
            self.camera.orbit(target, 0.0, -35f64.to_radians());
        }
    }

    /// Moves the camera to a standard view, keeping the target and distance.
    pub fn apply_preset(&mut self, preset: ViewPreset) {
        if preset == ViewPreset::Plan {
            self.set_plan_locked(true);
            return;
        }
        self.plan_locked = false;
        let (yaw, pitch) = preset.angles();
        self.camera.yaw = yaw;
        self.camera.pitch = pitch;
    }

    pub fn set_projection(&mut self, projection: Projection) {
        if !self.plan_locked {
            self.camera.projection = projection;
        }
    }

    /// Top-down orthographic, north up, centred on the current target.
    fn lock_plan(&mut self) {
        self.camera.yaw = 0.0;
        self.camera.set_plan();
    }
}

/// Saved view settings (no per-frame state), e.g. for app persistence.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NavigationSettings {
    /// Zoom factor per scroll "line".
    pub zoom_speed: f64,
    /// Radians per pixel of orbit drag.
    pub orbit_speed: f64,
}

impl Default for NavigationSettings {
    fn default() -> Self {
        Self {
            zoom_speed: 0.0018,
            orbit_speed: 0.008,
        }
    }
}

/// What happened in the viewport this frame.
pub struct ViewportOutput {
    pub response: egui::Response,
    pub rect: Rect,
    /// Cursor position relative to the viewport (logical px), when hovered.
    pub hover: Option<DVec2>,
    /// A primary click (not a drag) at this position.
    pub clicked: Option<DVec2>,
    /// A secondary click (not a drag).
    pub secondary_clicked: bool,
    /// The camera changed this frame (pan/zoom/orbit).
    pub navigated: bool,
    /// A primary-button drag in progress: `(start, current)` in viewport pixels.
    pub drag: Option<(DVec2, DVec2)>,
    /// A primary-button drag that ended this frame: `(start, end)`.
    pub drag_released: Option<(DVec2, DVec2)>,
    /// A view-control button or gizmo end was clicked. Presets are already
    /// applied to the camera; the others are left to the application.
    pub nav: Option<NavAction>,
}

/// Standard views offered by the gizmo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewPreset {
    Plan,
    /// Camera north of the model, looking south.
    NorthElevation,
    SouthElevation,
    EastElevation,
    WestElevation,
    /// From the south-west, looking north-east and down.
    Isometric,
    /// From a view-cube face, edge or corner: the camera's side of the model
    /// as east, north and up components, each -1, 0 or 1 (`[0, 0, -1]` is
    /// from below).
    From([i8; 3]),
}

impl ViewPreset {
    pub fn label(self) -> String {
        match self {
            ViewPreset::Plan => "Plan (top)".into(),
            ViewPreset::NorthElevation => "North elevation".into(),
            ViewPreset::SouthElevation => "South elevation".into(),
            ViewPreset::EastElevation => "East elevation".into(),
            ViewPreset::WestElevation => "West elevation".into(),
            ViewPreset::Isometric => "Isometric".into(),
            ViewPreset::From([e, n, z]) => {
                let side = match (n, e) {
                    (1, 0) => "north",
                    (-1, 0) => "south",
                    (0, 1) => "east",
                    (0, -1) => "west",
                    (1, 1) => "north-east",
                    (1, -1) => "north-west",
                    (-1, 1) => "south-east",
                    (-1, -1) => "south-west",
                    _ => "",
                };
                match (side, z) {
                    ("", 1) => "View from above".into(),
                    ("", -1) => "View from below".into(),
                    (s, 1) => format!("View from the {s}, above"),
                    (s, -1) => format!("View from the {s}, below"),
                    (s, _) => format!("View from the {s}"),
                }
            }
        }
    }

    /// The preset for a view from `dir` (east, north, up components), using
    /// the named presets where one fits.
    pub fn from_direction(dir: [i8; 3]) -> ViewPreset {
        match dir {
            [0, 0, 1] => ViewPreset::Plan,
            [0, 1, 0] => ViewPreset::NorthElevation,
            [0, -1, 0] => ViewPreset::SouthElevation,
            [1, 0, 0] => ViewPreset::EastElevation,
            [-1, 0, 0] => ViewPreset::WestElevation,
            d => ViewPreset::From(d),
        }
    }

    /// `(yaw, pitch)` in radians; yaw is the bearing the camera looks towards.
    fn angles(self) -> (f64, f64) {
        use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};
        match self {
            ViewPreset::Plan => (0.0, FRAC_PI_2),
            ViewPreset::NorthElevation => (PI, 0.0),
            ViewPreset::SouthElevation => (0.0, 0.0),
            ViewPreset::EastElevation => (1.5 * PI, 0.0),
            ViewPreset::WestElevation => (FRAC_PI_2, 0.0),
            ViewPreset::Isometric => (FRAC_PI_4, (1.0 / 2f64.sqrt()).atan()),
            ViewPreset::From([e, n, z]) => {
                let d = DVec3::new(f64::from(e), f64::from(n), f64::from(z)).normalize_or(DVec3::Z);
                // The camera looks back along `d`; yaw is the bearing it looks towards.
                let yaw = if d.truncate().length() < 1e-9 {
                    0.0
                } else {
                    (-d.x).atan2(-d.y)
                };
                // Straight below is held just off vertical, as orbiting does.
                let pitch = d.z.clamp(-1.0, 1.0).asin().max(-89.9f64.to_radians());
                (yaw, pitch)
            }
        }
    }
}

/// View-control requests from the floating toolbar or gizmo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavAction {
    ZoomExtents,
    TogglePlan,
    ToggleProjection,
    Preset(ViewPreset),
    /// Pick a centre of rotation by clicking in the view (the application
    /// owns picking).
    PickPivot,
}

pub struct Viewport<'a> {
    state: &'a mut ViewportState,
    scene: Arc<Scene>,
    /// Further scenes drawn with the first (e.g. surfaces, point clouds), each uploaded separately.
    overlays: Vec<Arc<Scene>>,
    /// Out-of-core point clouds, drawn node by node.
    clouds: Option<Arc<CloudFrame>>,
    cloud_bounds: Aabb3,
    /// Georeferenced image tiles for this frame.
    images: Option<Arc<cad_render::ImageFrame>>,
    grid: Option<GridSettings>,
    nav: NavigationSettings,
    anchor: Option<DVec3>,
    extra_bounds: Aabb3,
    axis_labels: [String; 3],
    /// Metres per display unit and its suffix, for the scale bar.
    length_unit: (f64, String),
    show_nav: bool,
}

impl<'a> Viewport<'a> {
    pub fn new(state: &'a mut ViewportState, scene: Arc<Scene>) -> Self {
        Self {
            state,
            scene,
            overlays: Vec::new(),
            clouds: None,
            cloud_bounds: Aabb3::EMPTY,
            images: None,
            grid: Some(GridSettings::default()),
            nav: NavigationSettings::default(),
            anchor: None,
            extra_bounds: Aabb3::EMPTY,
            axis_labels: ["X".into(), "Y".into(), "Z".into()],
            length_unit: (1.0, "m".into()),
            show_nav: true,
        }
    }

    /// Another scene drawn with the main one (its own revision, uploaded only when it changes).
    pub fn overlay_scene(mut self, scene: Option<Arc<Scene>>) -> Self {
        self.overlays.extend(scene);
        self
    }

    /// Out-of-core point clouds and their full extent (kept inside the clip planes).
    pub fn clouds(mut self, clouds: Option<Arc<CloudFrame>>, bounds: Aabb3) -> Self {
        self.clouds = clouds;
        self.cloud_bounds = bounds;
        self
    }

    /// Georeferenced image tiles drawn under the drawing.
    pub fn images(mut self, images: Option<Arc<cad_render::ImageFrame>>) -> Self {
        self.images = images;
        self
    }

    pub fn grid(mut self, grid: Option<GridSettings>) -> Self {
        self.grid = grid;
        self
    }

    pub fn navigation(mut self, nav: NavigationSettings) -> Self {
        self.nav = nav;
        self
    }

    /// World point under the cursor (e.g. the current snap), used as the zoom/orbit anchor.
    pub fn anchor(mut self, anchor: Option<DVec3>) -> Self {
        self.anchor = anchor;
        self
    }

    /// Extra geometry (e.g. tool previews) that must not be clipped by near/far.
    pub fn extra_bounds(mut self, bounds: Aabb3) -> Self {
        self.extra_bounds = bounds;
        self
    }

    /// Show the floating view-control toolbar (default on).
    pub fn nav_controls(mut self, show: bool) -> Self {
        self.show_nav = show;
        self
    }

    /// Display unit for the scale bar (e.g. `(0.3048, "ft")`).
    pub fn length_unit(mut self, metres_per_unit: f64, suffix: &str) -> Self {
        self.length_unit = (metres_per_unit, suffix.to_string());
        self
    }

    pub fn axis_labels(mut self, labels: [&str; 3]) -> Self {
        self.axis_labels = labels.map(str::to_string);
        self
    }

    pub fn show(self, ui: &mut egui::Ui) -> ViewportOutput {
        let theme = CadTheme::current(ui.ctx());
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let state = self.state;
        if state.plan_locked && !state.camera.is_plan() {
            state.lock_plan();
        }
        let cam = &mut state.camera;
        cam.viewport = DVec2::new(f64::from(rect.width()), f64::from(rect.height())).max(DVec2::ONE);

        let to_local = |p: Pos2| DVec2::new(f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y));
        let hover = response.hover_pos().map(to_local);
        let mut navigated = false;

        // Zoom.
        if let Some(cursor) = hover {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let mut factor = (-f64::from(scroll) * self.nav.zoom_speed).exp();
            if pinch != 1.0 {
                factor /= f64::from(pinch);
            }
            if (factor - 1.0).abs() > 1e-6 {
                let anchor = self.anchor.unwrap_or_else(|| cam.screen_to_target_plane(cursor));
                cam.zoom_about(anchor, factor);
                navigated = true;
            }
        }

        // Pan / orbit.
        let shift = ui.input(|i| i.modifiers.shift);
        let delta = response.drag_delta();
        let delta = DVec2::new(f64::from(delta.x), f64::from(delta.y));
        let panning = response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged_by(egui::PointerButton::Secondary) && (shift || state.plan_locked));
        let orbiting = response.dragged_by(egui::PointerButton::Secondary) && !shift && !state.plan_locked;
        if response.drag_started_by(egui::PointerButton::Secondary) {
            state.orbit_pivot = state
                .pivot
                .or(self.anchor)
                .or_else(|| hover.map(|c| cam.screen_to_target_plane(c)));
        }
        if panning && delta != DVec2::ZERO {
            cam.pan_pixels(delta);
            navigated = true;
        }
        if orbiting && delta != DVec2::ZERO {
            let pivot = state.orbit_pivot.unwrap_or(cam.target);
            cam.orbit(
                pivot,
                delta.x * self.nav.orbit_speed,
                delta.y * self.nav.orbit_speed,
            );
            navigated = true;
        }
        if response.hovered() && !panning && !orbiting {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        } else if panning {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }

        // Fit clip planes to everything that will be drawn.
        let mut bounds = self
            .scene
            .bounds
            .union(&self.extra_bounds)
            .union(&self.cloud_bounds);
        for o in &self.overlays {
            bounds = bounds.union(&o.bounds);
        }
        if let Some(g) = &self.grid {
            bounds = bounds.union(&g.bounds(cam));
        }
        cam.fit_clip(&bounds);

        let ppp = ui.ctx().pixels_per_point();
        let size_px = [
            (rect.width() * ppp).round().max(1.0) as u32,
            (rect.height() * ppp).round().max(1.0) as u32,
        ];
        let grid = self.grid.map(|mut g| {
            g.minor_color = CadTheme::rgba_f32(theme.grid_minor);
            g.major_color = CadTheme::rgba_f32(theme.grid_major);
            g
        });
        ui.painter()
            .add(cad_render::egui_wgpu::Callback::new_paint_callback(
                rect,
                ViewportCallback {
                    view_id: state.view_id,
                    size_px,
                    pixels_per_point: ppp,
                    frame: ViewportFrame {
                        camera: cam.clone(),
                        scenes: std::iter::once(self.scene.clone())
                            .chain(self.overlays.iter().cloned())
                            .collect(),
                        grid,
                        background: CadTheme::rgba_f32(theme.viewport_bg),
                        clouds: self.clouds.clone(),
                        images: self.images.clone(),
                    },
                    request_frame: Some({
                        let ctx = ui.ctx().clone();
                        std::sync::Arc::new(move || ctx.request_repaint())
                    }),
                },
            ));

        let painter = ui.painter_at(rect);
        if cam.projection == Projection::Orthographic {
            draw_scale_bar(&painter, rect, cam, &self.length_unit, &theme);
        }
        let cam_snapshot = cam.clone();
        if let Some(p) = state.pivot {
            draw_pivot(&painter, rect, &cam_snapshot, p, &theme);
        }
        let mut nav = view_cube(ui, rect, &cam_snapshot, &self.axis_labels, &theme).map(NavAction::Preset);
        if self.show_nav {
            nav = nav.or(nav_controls(ui, rect, state, &theme));
        }
        if let Some(NavAction::Preset(p)) = nav {
            state.apply_preset(p);
        }

        let clicked = response
            .clicked_by(egui::PointerButton::Primary)
            .then(|| response.interact_pointer_pos().map(to_local))
            .flatten();
        let secondary_clicked = response.clicked_by(egui::PointerButton::Secondary);

        let drag_id = response.id.with("primary_drag_start");
        let pointer = response.interact_pointer_pos().map(to_local);
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(p) = ui.input(|i| i.pointer.press_origin()).map(to_local)
        {
            ui.data_mut(|d| d.insert_temp(drag_id, p));
        }
        let start: Option<DVec2> = ui.data(|d| d.get_temp(drag_id));
        let drag = (response.dragged_by(egui::PointerButton::Primary))
            .then(|| start.zip(pointer))
            .flatten();
        let drag_released = if response.drag_stopped_by(egui::PointerButton::Primary) {
            ui.data_mut(|d| d.remove::<DVec2>(drag_id));
            start.zip(pointer.or(hover))
        } else {
            None
        };
        ViewportOutput {
            response,
            rect,
            hover,
            clicked,
            secondary_clicked,
            navigated,
            drag,
            drag_released,
            nav,
        }
    }
}

/// Converts a viewport-relative position to screen space.
pub fn to_screen(rect: Rect, p: DVec2) -> Pos2 {
    pos2(rect.min.x + p.x as f32, rect.min.y + p.y as f32)
}

/// A view-cube face: its outward normal and two in-plane axes (east, north,
/// up components), its label and its tooltip.
struct CubeFace {
    normal: [i8; 3],
    a: [i8; 3],
    c: [i8; 3],
    name: &'static str,
    tip: &'static str,
}

const fn face(normal: [i8; 3], a: [i8; 3], c: [i8; 3], name: &'static str, tip: &'static str) -> CubeFace {
    CubeFace {
        normal,
        a,
        c,
        name,
        tip,
    }
}

const CUBE_FACES: [CubeFace; 6] = [
    face([0, 0, 1], [1, 0, 0], [0, 1, 0], "TOP", "Top (plan)"),
    face([0, 0, -1], [1, 0, 0], [0, -1, 0], "BOTTOM", "Bottom (from below)"),
    face(
        [0, -1, 0],
        [1, 0, 0],
        [0, 0, 1],
        "FRONT",
        "Front (from the south)",
    ),
    face([0, 1, 0], [-1, 0, 0], [0, 0, 1], "BACK", "Back (from the north)"),
    face([1, 0, 0], [0, 1, 0], [0, 0, 1], "RIGHT", "Right (from the east)"),
    face([-1, 0, 0], [0, -1, 0], [0, 0, 1], "LEFT", "Left (from the west)"),
];

/// Share of a face's half-width taken by its edge and corner strips.
const CUBE_EDGE: f32 = 0.6;

fn dvec(v: [i8; 3]) -> DVec3 {
    DVec3::new(f64::from(v[0]), f64::from(v[1]), f64::from(v[2]))
}

/// Interactive view cube (bottom-left). Its faces are the top, bottom and
/// four sides (front = south, right = east); clicking a face, an edge strip
/// or a corner square looks at the model from that side, edge or corner. A
/// compass ring under it marks the easting and northing axes.
fn view_cube(
    ui: &mut egui::Ui,
    rect: Rect,
    cam: &Camera,
    labels: &[String; 3],
    theme: &CadTheme,
) -> Option<ViewPreset> {
    let center = pos2(rect.min.x + 76.0, rect.max.y - 82.0);
    let size = 26.0_f32;
    let (r, u, b) = (cam.right(), cam.up(), cam.back());
    // World direction to screen offset (px) for the cube's half-size.
    let screen = |v: DVec3| vec2(v.dot(r) as f32, -v.dot(u) as f32) * size;
    let painter = ui.painter_at(rect);
    let area = Rect::from_center_size(center, Vec2::splat(size * 2.0 * 1.9));
    let response = ui.interact(area, ui.scope_id().with("view_cube"), Sense::click());

    // Compass ring around the cube's foot, with the easting and northing axes marked.
    let ring = 1.75;
    let ring_pts: Vec<Pos2> = (0..=48)
        .map(|i| {
            let a = f64::from(i) / 48.0 * std::f64::consts::TAU;
            center + screen(DVec3::new(a.cos() * ring, a.sin() * ring, -1.0))
        })
        .collect();
    painter.add(egui::Shape::line(ring_pts, Stroke::new(1.0, theme.border_strong)));
    let short = |s: &str| s.chars().take(2).collect::<String>();
    for (dir, color, label) in [
        (DVec3::X, theme.axis_x, short(&labels[0])),
        (DVec3::Y, theme.axis_y, short(&labels[1])),
    ] {
        let at = center + screen(dir * (ring + 0.45) - DVec3::Z);
        let faded = if dir.dot(b) < -0.2 {
            color.gamma_multiply(0.55)
        } else {
            color
        };
        painter.text(at, egui::Align2::CENTER_CENTER, label, fonts::medium(10.0), faded);
    }

    // Which face region the pointer is over: its view direction and the
    // region's screen quad (for the highlight).
    let hover = response.hover_pos();
    let mut hovered: Option<([i8; 3], [Pos2; 4])> = None;
    let mut faces: Vec<(f64, usize)> = CUBE_FACES
        .iter()
        .enumerate()
        .map(|(i, f)| (dvec(f.normal).dot(b), i))
        .filter(|(facing, _)| *facing > 1e-3)
        .collect();
    faces.sort_by(|x, y| x.0.total_cmp(&y.0));
    for &(facing, i) in &faces {
        let CubeFace {
            normal, a, c, name, ..
        } = &CUBE_FACES[i];
        let (n, av, cv) = (dvec(*normal), screen(dvec(*a)), screen(dvec(*c)));
        let fc = center + screen(n);
        let quad = |a0: f32, a1: f32, c0: f32, c1: f32| {
            [
                fc + av * a0 + cv * c0,
                fc + av * a1 + cv * c0,
                fc + av * a1 + cv * c1,
                fc + av * a0 + cv * c1,
            ]
        };
        let face = quad(-1.0, 1.0, -1.0, 1.0);
        let shade = 0.55 + 0.45 * facing as f32;
        painter.add(egui::Shape::convex_polygon(
            face.to_vec(),
            theme.raised.gamma_multiply(shade.clamp(0.0, 1.0)).to_opaque(),
            Stroke::new(1.0, theme.border_strong),
        ));
        if let Some(p) = hover {
            // Face-local coordinates of the pointer (an affine map in orthographic projection).
            let det = av.x * cv.y - av.y * cv.x;
            if det.abs() > 1e-3 {
                let d = p - fc;
                let ta = (d.x * cv.y - d.y * cv.x) / det;
                let tc = (av.x * d.y - av.y * d.x) / det;
                if ta.abs() <= 1.0 && tc.abs() <= 1.0 {
                    let band = |t: f32| {
                        if t < -CUBE_EDGE {
                            -1
                        } else if t > CUBE_EDGE {
                            1
                        } else {
                            0
                        }
                    };
                    let (ia, ic) = (band(ta), band(tc));
                    let dir = [0, 1, 2].map(|k| normal[k] + ia * a[k] + ic * c[k]);
                    let span = |k: i8| match k {
                        -1 => (-1.0, -CUBE_EDGE),
                        1 => (CUBE_EDGE, 1.0),
                        _ => (-CUBE_EDGE, CUBE_EDGE),
                    };
                    let ((a0, a1), (c0, c1)) = (span(ia), span(ic));
                    hovered = Some((dir, quad(a0, a1, c0, c1)));
                }
            }
        }
        if facing > 0.35 {
            painter.text(
                fc,
                egui::Align2::CENTER_CENTER,
                name,
                fonts::medium(8.5),
                theme
                    .text_secondary
                    .gamma_multiply((facing as f32 * 1.4).min(1.0)),
            );
        }
    }
    let mut chosen = None;
    if let Some((dir, quad)) = hovered {
        painter.add(egui::Shape::convex_polygon(
            quad.to_vec(),
            theme.accent.gamma_multiply(0.45),
            Stroke::new(1.0, theme.accent),
        ));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        let preset = ViewPreset::from_direction(dir);
        let tip = match CUBE_FACES.iter().find(|f| f.normal == dir) {
            Some(f) => f.tip.to_string(),
            None => preset.label(),
        };
        if response.on_hover_text(tip).clicked() {
            chosen = Some(preset);
        }
    }
    chosen
}

/// Marks the centre of rotation: a small ring with a dot.
fn draw_pivot(painter: &egui::Painter, rect: Rect, cam: &Camera, p: DVec3, theme: &CadTheme) {
    if let Some((s, _)) = cam.world_to_screen(p) {
        let at = to_screen(rect, s);
        painter.circle_stroke(at, 7.0, Stroke::new(1.5, theme.accent));
        painter.circle_filled(at, 2.0, theme.accent);
        painter.line_segment(
            [at - vec2(11.0, 0.0), at - vec2(7.0, 0.0)],
            Stroke::new(1.5, theme.accent),
        );
        painter.line_segment(
            [at + vec2(7.0, 0.0), at + vec2(11.0, 0.0)],
            Stroke::new(1.5, theme.accent),
        );
    }
}

/// Floating view controls (top-right of the viewport).
fn nav_controls(ui: &mut egui::Ui, rect: Rect, state: &ViewportState, theme: &CadTheme) -> Option<NavAction> {
    let area = Rect::from_min_max(
        pos2(rect.right() - 260.0, rect.top() + 10.0),
        pos2(rect.right() - 10.0, rect.top() + 48.0),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(area)
            .layout(egui::Layout::right_to_left(egui::Align::Min)),
        |ui| {
            chrome::floating_frame(theme)
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        view_controls(ui, state)
                    })
                    .inner
                })
                .inner
        },
    )
    .inner
}

/// View-control buttons (zoom extents, isometric, plan/3D, perspective) laid
/// out in the current `ui`, e.g. a header toolbar. Presets are not applied;
/// the caller handles the returned action.
pub fn view_controls(ui: &mut egui::Ui, state: &ViewportState) -> Option<NavAction> {
    let mut action = None;
    ui.spacing_mut().item_spacing.x = 2.0;
    if IconButton::new(icons::FRAME_CORNERS, "Zoom extents")
        .description("Fit everything in view")
        .shortcut(Some("Home"))
        .show(ui)
        .clicked()
    {
        action = Some(NavAction::ZoomExtents);
    }
    if IconButton::new(icons::CUBE_FOCUS, "Isometric")
        .description("Isometric view from the south-west")
        .show(ui)
        .clicked()
    {
        action = Some(NavAction::Preset(ViewPreset::Isometric));
    }
    let (icon, title) = if state.plan_locked {
        (icons::CUBE, "3D view")
    } else {
        (icons::MAP_TRIFOLD, "Plan view")
    };
    if IconButton::new(icon, title)
        .description("Switch between locked plan and free 3D")
        .show(ui)
        .clicked()
    {
        action = Some(NavAction::TogglePlan);
    }
    if IconButton::new(icons::TARGET, "Centre of rotation")
        .description("Click a point to orbit about; right-click resets")
        .shortcut(Some("Shift+C"))
        .selected(state.pivot.is_some())
        .enabled(!state.plan_locked)
        .show(ui)
        .clicked()
    {
        action = Some(NavAction::PickPivot);
    }
    let persp = state.camera.projection == Projection::Perspective;
    if IconButton::new(icons::PERSPECTIVE, "Perspective")
        .description("Toggle perspective / orthographic projection")
        .selected(persp)
        .enabled(!state.plan_locked)
        .show(ui)
        .clicked()
    {
        action = Some(NavAction::ToggleProjection);
    }
    action
}

/// The scale bar's on-screen length.
const SCALE_BAR_PX: f32 = 120.0;

/// A scale-bar length to 3 significant figures, in km or mm when metres
/// would be unwieldy.
fn scale_label(length: f64, suffix: &str) -> String {
    let sig3 = |v: f64| {
        let decimals = (2 - v.log10().floor() as i32).clamp(0, 6) as usize;
        let text = format!("{v:.decimals$}");
        (text.parse::<f64>().unwrap_or(v), text)
    };
    let (value, text) = sig3(length);
    if suffix == "m" && value >= 1000.0 {
        format!("{} km", sig3(value / 1000.0).1)
    } else if suffix == "m" && value < 1.0 {
        format!("{} mm", sig3(value * 1000.0).1)
    } else {
        format!("{text} {suffix}")
    }
}

fn draw_scale_bar(painter: &egui::Painter, rect: Rect, cam: &Camera, unit: &(f64, String), theme: &CadTheme) {
    let (metres_per_unit, suffix) = (unit.0.max(1e-12), unit.1.as_str());
    // Work in display units so bar lengths are round numbers of that unit.
    let upp = cam.world_per_pixel() / metres_per_unit;
    if !(upp.is_finite() && upp > 0.0) {
        return;
    }
    // A fixed-length bar, labelled with the distance it spans.
    let px = SCALE_BAR_PX;
    let right = rect.max.x - 16.0;
    let y = rect.max.y - 18.0;
    let left = right - px;
    let stroke = Stroke::new(1.5, theme.text_secondary);
    painter.line_segment([pos2(left, y), pos2(right, y)], stroke);
    for x in [left, (left + right) * 0.5, right] {
        painter.line_segment([pos2(x, y - 4.0), pos2(x, y)], stroke);
    }
    painter.text(
        pos2((left + right) * 0.5, y - 6.0),
        egui::Align2::CENTER_BOTTOM,
        scale_label(upp * f64::from(px), suffix),
        egui::FontId::proportional(11.0),
        theme.text_secondary,
    );
}

/// Draws a polyline given in world coordinates as a screen overlay (e.g. tool previews).
pub fn draw_world_polyline(
    painter: &egui::Painter,
    rect: Rect,
    cam: &Camera,
    points: &[DVec3],
    stroke: Stroke,
    dashed: bool,
) {
    for w in points.windows(2) {
        let (Some((a, _)), Some((b, _))) = (cam.world_to_screen(w[0]), cam.world_to_screen(w[1])) else {
            continue;
        };
        let seg = [to_screen(rect, a), to_screen(rect, b)];
        if dashed {
            painter.extend(egui::Shape::dashed_line(&seg, stroke, 6.0, 4.0));
        } else {
            painter.line_segment(seg, stroke);
        }
    }
}

/// Draws the pick box (aperture) at the cursor.
pub fn draw_pickbox(painter: &egui::Painter, pos: Pos2, half: f32, color: Color32) {
    painter.rect_stroke(
        Rect::from_center_size(pos, Vec2::splat(half * 2.0)),
        0.0,
        Stroke::new(1.0, color),
        egui::StrokeKind::Middle,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_labels_have_three_figures() {
        assert_eq!(scale_label(12.3456, "m"), "12.3 m");
        assert_eq!(scale_label(123.456, "m"), "123 m");
        assert_eq!(scale_label(4.0, "m"), "4.00 m");
        assert_eq!(scale_label(999.7, "m"), "1.00 km");
        assert_eq!(scale_label(1534.0, "m"), "1.53 km");
        assert_eq!(scale_label(0.4567, "m"), "457 mm");
        assert_eq!(scale_label(0.0123, "m"), "12.3 mm");
        assert_eq!(scale_label(58.27, "ft"), "58.3 ft");
    }

    #[test]
    fn returning_to_plan_is_north_up() {
        let mut state = ViewportState::new(1);
        state.camera.target = DVec3::new(512_000.0, 6_912_000.0, 0.0);
        state.set_plan_locked(false);
        let pivot = state.camera.target;
        state.camera.orbit(pivot, 0.8, -0.3);
        state.set_projection(Projection::Perspective);

        state.set_plan_locked(true);
        let cam = &state.camera;
        assert!(cam.is_plan());
        assert!((cam.right() - DVec3::X).length() < 1e-12, "east is screen-right");
        assert!((cam.up() - DVec3::Y).length() < 1e-12, "north is screen-up");
    }
}

#[cfg(test)]
mod preset_tests {
    use super::*;

    #[test]
    fn presets_look_the_right_way() {
        let mut state = ViewportState::new(1);
        state.apply_preset(ViewPreset::NorthElevation);
        // Camera sits north of the target, looking south.
        assert!(state.camera.back().y > 0.99, "{:?}", state.camera.back());
        assert!(!state.plan_locked);
        state.apply_preset(ViewPreset::EastElevation);
        assert!(state.camera.back().x > 0.99);
        state.apply_preset(ViewPreset::Isometric);
        let back = state.camera.back();
        assert!(
            back.x < 0.0 && back.y < 0.0 && back.z > 0.0,
            "south-west and above: {back:?}"
        );
        state.apply_preset(ViewPreset::Plan);
        assert!(state.plan_locked && state.camera.is_plan());
    }

    /// Every cube face, edge and corner puts the camera on that side.
    #[test]
    fn cube_directions_look_from_their_side() {
        for e in -1i8..=1 {
            for n in -1i8..=1 {
                for z in -1i8..=1 {
                    if (e, n, z) == (0, 0, 0) {
                        continue;
                    }
                    let mut state = ViewportState::new(1);
                    state.apply_preset(ViewPreset::from_direction([e, n, z]));
                    let want = dvec([e, n, z]).normalize();
                    let back = state.camera.back();
                    assert!(back.dot(want) > 0.999, "{e} {n} {z}: back {back:?}");
                }
            }
        }
        assert_eq!(
            ViewPreset::From([-1, -1, 1]).label(),
            "View from the south-west, above"
        );
    }
}
