//! `f64` orbit camera with relative-to-center matrices for jitter-free rendering.
//!
//! World axes: X = easting, Y = northing, Z = up (right-handed).
//!
//! All camera state and maths are `f64`. The GPU only receives
//! [`Camera::chunk_mvp`], a matrix for vertices expressed *relative to a chunk
//! origin*; the large `origin − eye` translation is computed in `f64` before the
//! cast, so `f32` never has to represent values like 7,000,000 m.
//!
//! Depth uses reverse-Z (1 at the near plane, 0 at the far plane) for better
//! precision across very different scales.

use glam::{DMat4, DQuat, DVec2, DVec3, DVec4, Mat4};
use survey_core::Aabb3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    Perspective,
    Orthographic,
}

/// Pitch used for plan (top-down) views.
pub const PLAN_PITCH: f64 = std::f64::consts::FRAC_PI_2;
const MIN_PITCH: f64 = -89.9 * std::f64::consts::PI / 180.0;
const MIN_DISTANCE: f64 = 1e-3;
const MAX_DISTANCE: f64 = 1e8;
/// How far behind the view plane orthographic pick rays start.
const ORTHO_RAY_BACKOFF: f64 = 1e7;

#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    /// Look-at point (also the default orbit pivot).
    pub target: DVec3,
    /// Distance from eye to target. In orthographic mode it sets the view height.
    pub distance: f64,
    /// Heading the camera looks towards, as a bearing (radians clockwise from north).
    /// In plan view this is the direction that points up the screen.
    pub yaw: f64,
    /// Elevation of the eye above the target's horizon; `π/2` looks straight down.
    pub pitch: f64,
    /// Vertical field of view (radians), also used to derive orthographic size.
    pub fov_y: f64,
    pub projection: Projection,
    /// Viewport size in logical pixels.
    pub viewport: DVec2,
    /// Near/far clip distances (positive, along the view direction). See [`Camera::fit_clip`].
    pub near: f64,
    pub far: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: DVec3::ZERO,
            distance: 100.0,
            yaw: 0.0,
            pitch: PLAN_PITCH,
            fov_y: 45f64.to_radians(),
            projection: Projection::Orthographic,
            viewport: DVec2::new(800.0, 600.0),
            near: 0.1,
            far: 1000.0,
        }
    }
}

/// A ray in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: DVec3,
    /// Unit direction.
    pub dir: DVec3,
}

impl Ray {
    pub fn at(&self, t: f64) -> DVec3 {
        self.origin + self.dir * t
    }

    /// Intersection with the horizontal plane `z = height`.
    pub fn intersect_z_plane(&self, height: f64) -> Option<DVec3> {
        if self.dir.z.abs() < 1e-12 {
            return None;
        }
        let t = (height - self.origin.z) / self.dir.z;
        (t >= 0.0).then(|| self.at(t))
    }

    /// Intersection with the plane through `point` with normal `normal`.
    pub fn intersect_plane(&self, point: DVec3, normal: DVec3) -> Option<DVec3> {
        let denom = self.dir.dot(normal);
        if denom.abs() < 1e-12 {
            return None;
        }
        let t = (point - self.origin).dot(normal) / denom;
        (t >= 0.0).then(|| self.at(t))
    }
}

/// A convex region bounded by planes (inward normals); used for picking and snapping.
#[derive(Clone, Debug, PartialEq)]
pub struct Frustum {
    /// `(point_on_plane, inward_normal)`.
    planes: Vec<(DVec3, DVec3)>,
}

impl Frustum {
    /// Conservative test: `false` only if the box is certainly outside.
    pub fn intersects_aabb(&self, b: &Aabb3) -> bool {
        self.planes.iter().all(|(p, n)| {
            let positive = DVec3::new(
                if n.x >= 0.0 { b.max.x } else { b.min.x },
                if n.y >= 0.0 { b.max.y } else { b.min.y },
                if n.z >= 0.0 { b.max.z } else { b.min.z },
            );
            n.dot(positive - *p) >= 0.0
        })
    }

    pub fn contains_point(&self, q: DVec3) -> bool {
        self.planes.iter().all(|(p, n)| n.dot(q - *p) >= 0.0)
    }

    /// The bounding planes as `(point_on_plane, inward_normal)`.
    pub fn planes(&self) -> &[(DVec3, DVec3)] {
        &self.planes
    }
}

impl Camera {
    // ------------------------------------------------------------------ basis

    /// Camera right vector (always horizontal, so it never degenerates in plan view).
    pub fn right(&self) -> DVec3 {
        DVec3::new(self.yaw.cos(), -self.yaw.sin(), 0.0)
    }

    /// Horizontal heading vector.
    pub fn heading(&self) -> DVec3 {
        DVec3::new(self.yaw.sin(), self.yaw.cos(), 0.0)
    }

    /// Unit vector from target towards the eye.
    pub fn back(&self) -> DVec3 {
        DVec3::Z * self.pitch.sin() - self.heading() * self.pitch.cos()
    }

    /// Camera up vector (screen up).
    pub fn up(&self) -> DVec3 {
        self.back().cross(self.right())
    }

    /// Unit view direction (from eye towards target).
    pub fn forward(&self) -> DVec3 {
        -self.back()
    }

    pub fn eye(&self) -> DVec3 {
        self.target + self.back() * self.distance
    }

    pub fn aspect(&self) -> f64 {
        if self.viewport.y > 0.0 {
            self.viewport.x / self.viewport.y
        } else {
            1.0
        }
    }

    /// World-space height of the view at the target depth.
    pub fn view_height(&self) -> f64 {
        2.0 * self.distance * (self.fov_y * 0.5).tan()
    }

    /// World units per logical pixel at the target depth.
    pub fn world_per_pixel(&self) -> f64 {
        self.view_height() / self.viewport.y.max(1.0)
    }

    pub fn is_plan(&self) -> bool {
        (self.pitch - PLAN_PITCH).abs() < 1e-9 && self.projection == Projection::Orthographic
    }

    // ------------------------------------------------------------------ matrices

    /// View rotation (world → camera axes), without translation.
    fn rotation(&self) -> DMat4 {
        let (r, u, b) = (self.right(), self.up(), self.back());
        // Rows are the camera axes.
        DMat4::from_cols(
            DVec4::new(r.x, u.x, b.x, 0.0),
            DVec4::new(r.y, u.y, b.y, 0.0),
            DVec4::new(r.z, u.z, b.z, 0.0),
            DVec4::W,
        )
    }

    /// View matrix for vertices expressed relative to `origin`.
    pub fn view_rtc(&self, origin: DVec3) -> DMat4 {
        // v = R·(origin + local − eye): the translation `origin − eye` is formed in f64.
        let rot = self.rotation();
        let t = rot.transform_vector3(origin - self.eye());
        let mut m = rot;
        m.w_axis = t.extend(1.0);
        m
    }

    /// Reverse-Z projection matrix (depth 1 at near, 0 at far; wgpu clip space).
    pub fn projection_matrix(&self) -> DMat4 {
        let (n, f) = (self.near, self.far.max(self.near + 1e-6));
        match self.projection {
            Projection::Perspective => {
                let fy = 1.0 / (self.fov_y * 0.5).tan();
                let fx = fy / self.aspect();
                DMat4::from_cols(
                    DVec4::new(fx, 0.0, 0.0, 0.0),
                    DVec4::new(0.0, fy, 0.0, 0.0),
                    DVec4::new(0.0, 0.0, n / (f - n), -1.0),
                    DVec4::new(0.0, 0.0, n * f / (f - n), 0.0),
                )
            }
            Projection::Orthographic => {
                let half_h = self.view_height() * 0.5;
                let half_w = half_h * self.aspect();
                DMat4::from_cols(
                    DVec4::new(1.0 / half_w, 0.0, 0.0, 0.0),
                    DVec4::new(0.0, 1.0 / half_h, 0.0, 0.0),
                    DVec4::new(0.0, 0.0, 1.0 / (f - n), 0.0),
                    DVec4::new(0.0, 0.0, f / (f - n), 1.0),
                )
            }
        }
    }

    /// `f64` model-view-projection for a chunk origin.
    pub fn chunk_mvp_f64(&self, origin: DVec3) -> DMat4 {
        self.projection_matrix() * self.view_rtc(origin)
    }

    /// The `f32` matrix uploaded to the GPU for a chunk whose vertices are offsets from `origin`.
    pub fn chunk_mvp(&self, origin: DVec3) -> Mat4 {
        self.chunk_mvp_f64(origin).as_mat4()
    }

    /// Fits near/far to `bounds` (which should include everything that will be drawn).
    pub fn fit_clip(&mut self, bounds: &Aabb3) {
        let eye = self.eye();
        let fwd = self.forward();
        let mut zmin = f64::INFINITY;
        let mut zmax = f64::NEG_INFINITY;
        let corners = if bounds.is_empty() {
            Aabb3::from_point(self.target).corners()
        } else {
            bounds.union(&Aabb3::from_point(self.target)).corners()
        };
        for c in corners {
            let d = (c - eye).dot(fwd);
            zmin = zmin.min(d);
            zmax = zmax.max(d);
        }
        let pad = ((zmax - zmin) * 0.02).max(self.distance * 1e-3).max(1e-3);
        match self.projection {
            Projection::Perspective => {
                self.far = (zmax + pad).max(1e-2);
                // Reverse-Z float depth tolerates large far/near ratios.
                self.near = (zmin - pad).max(self.far * 1e-7).max(1e-4);
            }
            Projection::Orthographic => {
                self.near = zmin - pad;
                self.far = zmax + pad;
            }
        }
    }

    // ------------------------------------------------------------------ projection helpers

    /// Projects a world point to logical pixels (origin top-left) and reverse-Z depth.
    /// Returns `None` for points behind a perspective camera.
    pub fn world_to_screen(&self, p: DVec3) -> Option<(DVec2, f64)> {
        let clip = self.chunk_mvp_f64(p) * DVec4::W;
        if clip.w <= 1e-12 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some((self.ndc_to_screen(ndc.truncate()), ndc.z))
    }

    pub fn ndc_to_screen(&self, ndc: DVec2) -> DVec2 {
        DVec2::new(
            (ndc.x + 1.0) * 0.5 * self.viewport.x,
            (1.0 - ndc.y) * 0.5 * self.viewport.y,
        )
    }

    pub fn screen_to_ndc(&self, px: DVec2) -> DVec2 {
        DVec2::new(
            px.x / self.viewport.x * 2.0 - 1.0,
            1.0 - px.y / self.viewport.y * 2.0,
        )
    }

    /// World-space ray through a pixel.
    pub fn screen_ray(&self, px: DVec2) -> Ray {
        let ndc = self.screen_to_ndc(px);
        let half_h = (self.fov_y * 0.5).tan();
        let half_w = half_h * self.aspect();
        let (r, u, f) = (self.right(), self.up(), self.forward());
        match self.projection {
            Projection::Perspective => Ray {
                origin: self.eye(),
                dir: (f + r * (ndc.x * half_w) + u * (ndc.y * half_h)).normalize(),
            },
            Projection::Orthographic => {
                let offset = r * (ndc.x * half_w * self.distance) + u * (ndc.y * half_h * self.distance);
                Ray {
                    origin: self.eye() + offset - f * ORTHO_RAY_BACKOFF,
                    dir: f,
                }
            }
        }
    }

    /// The world point under a pixel on the plane through the target facing the camera.
    pub fn screen_to_target_plane(&self, px: DVec2) -> DVec3 {
        let ray = self.screen_ray(px);
        ray.intersect_plane(self.target, self.back())
            .unwrap_or(self.target)
    }

    /// A pick volume around a pixel: a thin frustum `half_px` pixels wide.
    pub fn pick_frustum(&self, px: DVec2, half_px: f64) -> Frustum {
        self.rect_frustum(px - DVec2::splat(half_px), px + DVec2::splat(half_px))
    }

    /// The volume seen through a screen rectangle (e.g. a selection window).
    pub fn rect_frustum(&self, min: DVec2, max: DVec2) -> Frustum {
        let (min, max) = (min.min(max), min.max(max).max(min.min(max) + DVec2::splat(1e-6)));
        let corners = [
            DVec2::new(min.x, min.y),
            DVec2::new(max.x, min.y),
            DVec2::new(max.x, max.y),
            DVec2::new(min.x, max.y),
        ]
        .map(|c| self.screen_ray(c));
        let inside = self.screen_ray((min + max) * 0.5).at(1.0);
        let mut planes = Vec::with_capacity(5);
        for i in 0..4 {
            let (a, b) = (corners[i], corners[(i + 1) % 4]);
            let p0 = a.origin;
            let n = (a.dir).cross(b.origin + b.dir - p0);
            let n = if n.dot(inside - p0) < 0.0 { -n } else { n };
            planes.push((p0, n.normalize_or_zero()));
        }
        if self.projection == Projection::Perspective {
            planes.push((self.eye(), self.forward()));
        }
        Frustum { planes }
    }

    // ------------------------------------------------------------------ navigation

    /// Pans by a screen-space delta in logical pixels (content follows the cursor).
    pub fn pan_pixels(&mut self, delta: DVec2) {
        let wpp = self.world_per_pixel();
        self.target += -self.right() * (delta.x * wpp) + self.up() * (delta.y * wpp);
    }

    /// Zooms by `factor` (< 1 zooms in) keeping `anchor` fixed on screen.
    pub fn zoom_about(&mut self, anchor: DVec3, factor: f64) {
        let new_distance = (self.distance * factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
        let k = new_distance / self.distance;
        self.target = anchor + (self.target - anchor) * k;
        self.distance = new_distance;
    }

    /// Orbits about `pivot` by yaw/pitch deltas (radians). Pitch is clamped to
    /// just below the horizon and straight down.
    pub fn orbit(&mut self, pivot: DVec3, d_yaw: f64, d_pitch: f64) {
        let new_pitch = (self.pitch + d_pitch).clamp(MIN_PITCH, PLAN_PITCH);
        let dp = new_pitch - self.pitch;
        let q_pitch = DQuat::from_axis_angle(self.right(), -dp);
        let q_yaw = DQuat::from_rotation_z(-d_yaw);
        let q = q_yaw * q_pitch;
        self.target = pivot + q * (self.target - pivot);
        self.pitch = new_pitch;
        self.yaw = (self.yaw + d_yaw).rem_euclid(std::f64::consts::TAU);
    }

    /// Frames `bounds` in the current view direction.
    pub fn zoom_to_bounds(&mut self, bounds: &Aabb3) {
        if bounds.is_empty() {
            return;
        }
        self.target = bounds.center();
        let size = bounds.size();
        let (r, u) = (self.right(), self.up());
        // Extent of the box projected on the screen axes.
        let mut w: f64 = 0.0;
        let mut h: f64 = 0.0;
        for c in bounds.corners() {
            let d = c - self.target;
            w = w.max(d.dot(r).abs());
            h = h.max(d.dot(u).abs());
        }
        let half = h.max(w / self.aspect()).max(size.length() * 1e-3).max(0.5);
        self.distance = (half * 1.15 / (self.fov_y * 0.5).tan()).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    /// Switches to a top-down orthographic view, keeping the current heading.
    pub fn set_plan(&mut self) {
        self.pitch = PLAN_PITCH;
        self.projection = Projection::Orthographic;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, Vec4};

    fn cam_at(target: DVec3) -> Camera {
        Camera {
            target,
            distance: 50.0,
            viewport: DVec2::new(1920.0, 1080.0),
            ..Default::default()
        }
    }

    fn mga() -> DVec3 {
        DVec3::new(512_345.0, 6_912_345.0, 120.0)
    }

    #[test]
    fn plan_view_is_north_up() {
        let mut cam = cam_at(DVec3::ZERO);
        cam.fit_clip(&Aabb3::from_point(DVec3::ZERO));
        let (c, _) = cam.world_to_screen(DVec3::ZERO).expect("visible");
        let (n, _) = cam.world_to_screen(DVec3::new(0.0, 10.0, 0.0)).expect("visible");
        let (e, _) = cam.world_to_screen(DVec3::new(10.0, 0.0, 0.0)).expect("visible");
        assert!((c - DVec2::new(960.0, 540.0)).length() < 1e-9);
        assert!(n.y < c.y && (n.x - c.x).abs() < 1e-9, "north is up");
        assert!(e.x > c.x && (e.y - c.y).abs() < 1e-9, "east is right");
    }

    #[test]
    fn screen_ray_round_trip_both_projections() {
        for projection in [Projection::Orthographic, Projection::Perspective] {
            let mut cam = cam_at(mga());
            cam.projection = projection;
            cam.orbit(mga(), 0.7, -0.6);
            let p = mga() + DVec3::new(3.0, -4.0, 1.5);
            cam.fit_clip(&Aabb3::from_point(p));
            let (px, depth) = cam.world_to_screen(p).expect("visible");
            assert!((0.0..=1.0).contains(&depth));
            let ray = cam.screen_ray(px);
            let along = (p - ray.origin).dot(ray.dir);
            let miss = (ray.at(along) - p).length();
            assert!(miss < 1e-6, "{projection:?} miss {miss}");
        }
    }

    #[test]
    fn orbit_keeps_pivot_fixed_on_screen() {
        let mut cam = cam_at(mga());
        cam.projection = Projection::Perspective;
        let pivot = mga() + DVec3::new(10.0, 5.0, 0.0);
        cam.fit_clip(&Aabb3::from_points([pivot, mga()]));
        let before = cam.world_to_screen(pivot).expect("visible").0;
        cam.orbit(pivot, 0.3, -0.4);
        cam.orbit(pivot, -1.1, -0.2);
        let after = cam.world_to_screen(pivot).expect("visible").0;
        assert!((before - after).length() < 1e-6, "{before} vs {after}");
        // Basis stays orthonormal.
        assert!(cam.right().dot(cam.up()).abs() < 1e-12);
        assert!((cam.up().length() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn zoom_keeps_anchor_fixed_on_screen() {
        for projection in [Projection::Orthographic, Projection::Perspective] {
            let mut cam = cam_at(mga());
            cam.projection = projection;
            cam.orbit(mga(), 0.2, -0.5);
            let anchor = cam.screen_to_target_plane(DVec2::new(1500.0, 200.0));
            cam.fit_clip(&Aabb3::from_point(anchor));
            let before = cam.world_to_screen(anchor).expect("visible").0;
            cam.zoom_about(anchor, 0.25);
            let after = cam.world_to_screen(anchor).expect("visible").0;
            assert!((before - after).length() < 1e-6, "{projection:?}");
        }
    }

    #[test]
    fn pick_frustum_selects_near_cursor() {
        let mut cam = cam_at(mga());
        cam.projection = Projection::Perspective;
        cam.orbit(mga(), 0.4, -0.3);
        let p = mga() + DVec3::new(1.0, 2.0, 0.0);
        cam.fit_clip(&Aabb3::from_point(p));
        let (px, _) = cam.world_to_screen(p).expect("visible");
        let f = cam.pick_frustum(px, 5.0);
        assert!(f.contains_point(p));
        assert!(f.intersects_aabb(&Aabb3::from_point(p)));
        let (far_px, _) = cam
            .world_to_screen(mga() + DVec3::new(-20.0, -20.0, 0.0))
            .expect("visible");
        assert!((far_px - px).length() > 20.0);
        assert!(!f.contains_point(mga() + DVec3::new(-20.0, -20.0, 0.0)));
    }

    /// Simulates the GPU: f32 matrix × f32 local offset, as the vertex shader does.
    fn gpu_project(cam: &Camera, origin: DVec3, local: Vec3) -> DVec2 {
        let clip: Vec4 = cam.chunk_mvp(origin) * local.extend(1.0);
        let ndc = DVec2::new(f64::from(clip.x / clip.w), f64::from(clip.y / clip.w));
        cam.ndc_to_screen(ndc)
    }

    /// The core precision guarantee: the same scene renders identically at local
    /// coordinates and at MGA/UTM coordinates, after arbitrary orbiting.
    #[test]
    fn relative_to_center_is_jitter_free_at_mga_scale() {
        let local_base = DVec3::new(1_000.0, 5_000.0, 100.0);
        let mga_base = DVec3::new(512_345.0, 6_912_345.0, 100.0);
        let offsets: Vec<Vec3> = (0..50)
            .map(|i| {
                let f = i as f32;
                Vec3::new((f * 0.37).sin() * 3.0, (f * 0.73).cos() * 3.0, f * 0.001)
            })
            .collect();

        for projection in [Projection::Orthographic, Projection::Perspective] {
            let mut a = cam_at(local_base + DVec3::new(0.3, 0.2, 0.0));
            let mut b = cam_at(mga_base + DVec3::new(0.3, 0.2, 0.0));
            for cam in [&mut a, &mut b] {
                cam.projection = projection;
                cam.distance = 5.0; // close-up: millimetres are several pixels
            }
            for step in 0..20 {
                let (dy, dp) = (0.05 * f64::from(step), -0.03);
                let (pa, pb) = (a.target, b.target);
                a.orbit(pa, dy, dp);
                b.orbit(pb, dy, dp);
                a.fit_clip(&Aabb3::from_point(local_base).expanded(5.0));
                b.fit_clip(&Aabb3::from_point(mga_base).expanded(5.0));
                for o in &offsets {
                    let sa = gpu_project(&a, local_base, *o);
                    let sb = gpu_project(&b, mga_base, *o);
                    let err = (sa - sb).length();
                    assert!(err < 0.01, "{projection:?} step {step}: {err} px");
                }
            }
        }
    }

    /// Demonstrates why RTC is needed: casting MGA world coordinates to f32
    /// directly produces errors of many pixels in a close-up view.
    #[test]
    fn naive_f32_world_coordinates_jitter() {
        let base = DVec3::new(512_345.0, 6_912_345.0, 100.0);
        let mut cam = cam_at(base);
        cam.distance = 5.0;
        cam.fit_clip(&Aabb3::from_point(base).expanded(5.0));
        let p = base + DVec3::new(0.123, 0.456, 0.0);
        let exact = cam.world_to_screen(p).expect("visible").0;
        // Naive: whole view-projection in f32 applied to f32 world coordinates.
        let naive_m = cam.chunk_mvp_f64(DVec3::ZERO).as_mat4();
        let clip = naive_m * p.as_vec3().extend(1.0);
        let naive = cam.ndc_to_screen(DVec2::new(f64::from(clip.x / clip.w), f64::from(clip.y / clip.w)));
        assert!((exact - naive).length() > 10.0, "naive error unexpectedly small");
    }
}
