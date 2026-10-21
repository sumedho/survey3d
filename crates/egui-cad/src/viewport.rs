//! 3D/plan viewport widget.
//!
//! Navigation (defaults):
//! * wheel / pinch — zoom about the cursor
//! * middle-drag, or Shift + right-drag — pan
//! * right-drag — orbit about the point under the cursor (disabled in plan lock)
//!
//! Primary clicks and right clicks without dragging are reported to the
//! application, which owns tools and picking.

use std::sync::Arc;

use cad_render::{Camera, GridSettings, Projection, Scene, ViewportCallback, ViewportFrame};
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};
use survey_core::Aabb3;

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
}

impl ViewportState {
    pub fn new(view_id: u64) -> Self {
        Self {
            camera: Camera::default(),
            view_id,
            plan_locked: true,
            orbit_pivot: None,
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
}

pub struct Viewport<'a> {
    state: &'a mut ViewportState,
    scene: Arc<Scene>,
    grid: Option<GridSettings>,
    nav: NavigationSettings,
    anchor: Option<DVec3>,
    extra_bounds: Aabb3,
    axis_labels: [String; 3],
}

impl<'a> Viewport<'a> {
    pub fn new(state: &'a mut ViewportState, scene: Arc<Scene>) -> Self {
        Self {
            state,
            scene,
            grid: Some(GridSettings::default()),
            nav: NavigationSettings::default(),
            anchor: None,
            extra_bounds: Aabb3::EMPTY,
            axis_labels: ["X".into(), "Y".into(), "Z".into()],
        }
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
            state.orbit_pivot = self
                .anchor
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
        let mut bounds = self.scene.bounds.union(&self.extra_bounds);
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
                        scene: self.scene.clone(),
                        grid,
                        background: CadTheme::rgba_f32(theme.viewport_bg),
                    },
                },
            ));

        let painter = ui.painter_at(rect);
        draw_axis_gizmo(&painter, rect, cam, &self.axis_labels, &theme);
        if cam.projection == Projection::Orthographic {
            draw_scale_bar(&painter, rect, cam, &theme);
        }

        let clicked = response
            .clicked_by(egui::PointerButton::Primary)
            .then(|| response.interact_pointer_pos().map(to_local))
            .flatten();
        let secondary_clicked = response.clicked_by(egui::PointerButton::Secondary);
        ViewportOutput {
            response,
            rect,
            hover,
            clicked,
            secondary_clicked,
            navigated,
        }
    }
}

/// Converts a viewport-relative position to screen space.
pub fn to_screen(rect: Rect, p: DVec2) -> Pos2 {
    pos2(rect.min.x + p.x as f32, rect.min.y + p.y as f32)
}

fn draw_axis_gizmo(
    painter: &egui::Painter,
    rect: Rect,
    cam: &Camera,
    labels: &[String; 3],
    theme: &CadTheme,
) {
    let center = pos2(rect.min.x + 42.0, rect.max.y - 42.0);
    let len = 26.0;
    painter.circle_filled(center, len + 10.0, theme.panel.gamma_multiply(0.75));
    let (r, u, b) = (cam.right(), cam.up(), cam.back());
    let mut axes: Vec<(DVec3, Color32, &str)> = vec![
        (DVec3::X, theme.axis_x, &labels[0]),
        (DVec3::Y, theme.axis_y, &labels[1]),
        (DVec3::Z, theme.axis_z, &labels[2]),
    ];
    // Draw far axes first.
    axes.sort_by(|a, c| a.0.dot(b).total_cmp(&c.0.dot(b)));
    for (axis, color, label) in axes {
        let v = vec2(axis.dot(r) as f32, -axis.dot(u) as f32) * len;
        let end = center + v;
        let faded = if axis.dot(b) < -0.1 {
            color.gamma_multiply(0.45)
        } else {
            color
        };
        painter.line_segment([center, end], Stroke::new(2.0, faded));
        if v.length() > 4.0 {
            painter.text(
                center + v * 1.32,
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(11.0),
                faded,
            );
        }
    }
}

fn draw_scale_bar(painter: &egui::Painter, rect: Rect, cam: &Camera, theme: &CadTheme) {
    let wpp = cam.world_per_pixel();
    if !(wpp.is_finite() && wpp > 0.0) {
        return;
    }
    // Pick a 1/2/5 × 10^k length close to 120 px.
    let target = wpp * 120.0;
    let pow = 10f64.powf(target.log10().floor());
    let length = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * pow)
        .min_by(|a, b| (a - target).abs().total_cmp(&(b - target).abs()))
        .unwrap_or(pow);
    let px = (length / wpp) as f32;
    let right = rect.max.x - 16.0;
    let y = rect.max.y - 18.0;
    let left = right - px;
    let stroke = Stroke::new(1.5, theme.text_dim);
    painter.line_segment([pos2(left, y), pos2(right, y)], stroke);
    for x in [left, (left + right) * 0.5, right] {
        painter.line_segment([pos2(x, y - 4.0), pos2(x, y)], stroke);
    }
    let text = if length >= 1000.0 {
        format!("{} km", length / 1000.0)
    } else if length >= 1.0 {
        format!("{length} m")
    } else {
        format!("{} mm", (length * 1000.0).round())
    };
    painter.text(
        pos2((left + right) * 0.5, y - 6.0),
        egui::Align2::CENTER_BOTTOM,
        text,
        egui::FontId::proportional(11.0),
        theme.text_dim,
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
