//! Object snapping.
//!
//! The engine is pure `f64` logic over a [`Camera`]; geometry comes from the
//! application through [`SnapProvider`]. Candidates are gathered inside a pick
//! frustum around the cursor, scored by mode priority then screen distance, and
//! can be cycled with Tab when several overlap.

use cad_render::{Camera, Frustum};
use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};
use survey_core::geom::{closest_point_on_segment, closest_ray_segment, line_intersection_2d};

/// Kinds of snap, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SnapMode {
    /// A point object.
    Node,
    /// A vertex of a line or polyline.
    Endpoint,
    Midpoint,
    Intersection,
    /// Foot of the perpendicular from the previous point.
    Perpendicular,
    /// Closest point on a line.
    Nearest,
    Grid,
}

impl SnapMode {
    pub const ALL: [SnapMode; 7] = [
        SnapMode::Node,
        SnapMode::Endpoint,
        SnapMode::Midpoint,
        SnapMode::Intersection,
        SnapMode::Perpendicular,
        SnapMode::Nearest,
        SnapMode::Grid,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SnapMode::Node => "Node",
            SnapMode::Endpoint => "Endpoint",
            SnapMode::Midpoint => "Midpoint",
            SnapMode::Intersection => "Intersection",
            SnapMode::Perpendicular => "Perpendicular",
            SnapMode::Nearest => "Nearest",
            SnapMode::Grid => "Grid",
        }
    }

    /// Short code, also accepted as a one-shot override on the command line.
    pub fn code(self) -> &'static str {
        match self {
            SnapMode::Node => "nod",
            SnapMode::Endpoint => "end",
            SnapMode::Midpoint => "mid",
            SnapMode::Intersection => "int",
            SnapMode::Perpendicular => "per",
            SnapMode::Nearest => "nea",
            SnapMode::Grid => "gri",
        }
    }

    pub fn from_code(code: &str) -> Option<SnapMode> {
        let code = code.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|m| code.len() >= 3 && m.label().to_ascii_lowercase().starts_with(&code))
    }

    /// Lower wins when candidates compete.
    fn priority(self) -> u8 {
        match self {
            // A point object outranks a coincident vertex: it knows whether its height is known.
            SnapMode::Node => 0,
            SnapMode::Endpoint => 1,
            SnapMode::Intersection => 2,
            SnapMode::Midpoint => 3,
            SnapMode::Perpendicular => 4,
            SnapMode::Nearest => 5,
            SnapMode::Grid => 6,
        }
    }

    fn bit(self) -> u32 {
        1 << (self as u32)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapSettings {
    /// Master switch (F3).
    pub enabled: bool,
    modes: u32,
    /// Half-size of the pick box in logical pixels.
    pub aperture_px: f64,
}

impl Default for SnapSettings {
    fn default() -> Self {
        let mut s = Self {
            enabled: true,
            modes: 0,
            aperture_px: 10.0,
        };
        for m in [
            SnapMode::Node,
            SnapMode::Endpoint,
            SnapMode::Midpoint,
            SnapMode::Intersection,
            SnapMode::Perpendicular,
        ] {
            s.set(m, true);
        }
        s
    }
}

impl SnapSettings {
    pub fn is_on(&self, mode: SnapMode) -> bool {
        self.modes & mode.bit() != 0
    }

    pub fn set(&mut self, mode: SnapMode, on: bool) {
        if on {
            self.modes |= mode.bit();
        } else {
            self.modes &= !mode.bit();
        }
    }

    pub fn toggle(&mut self, mode: SnapMode) {
        self.modes ^= mode.bit();
    }

    /// Only `mode` enabled (used for one-shot overrides).
    pub fn only(mode: SnapMode, aperture_px: f64) -> Self {
        let mut s = Self {
            enabled: true,
            modes: 0,
            aperture_px,
        };
        s.set(mode, true);
        s
    }
}

/// Snappable geometry supplied by the application.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SnapGeom {
    Point { p: DVec3, height_known: bool },
    Segment { a: DVec3, b: DVec3 },
}

/// A piece of snappable geometry tagged with an application id (e.g. entity id).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapSource {
    pub geom: SnapGeom,
    pub tag: u64,
}

/// Supplies geometry that may lie inside a pick frustum.
pub trait SnapProvider {
    fn query(&self, frustum: &Frustum, out: &mut Vec<SnapSource>);
}

/// Inputs describing the current pick.
#[derive(Clone, Debug)]
pub struct SnapQuery<'a> {
    pub camera: &'a Camera,
    /// Cursor in logical pixels relative to the viewport.
    pub cursor: DVec2,
    /// The previous point of the active tool (for perpendicular snaps).
    pub base_point: Option<DVec3>,
    /// Height of the working plane used when nothing is snapped.
    pub working_z: f64,
    /// Grid spacing for grid snap (world units).
    pub grid_spacing: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapHit {
    pub point: DVec3,
    pub mode: SnapMode,
    /// Screen position (logical px, viewport-relative).
    pub screen: DVec2,
    pub tag: Option<u64>,
    pub height_known: bool,
}

/// The result of resolving the cursor to a world point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    pub point: DVec3,
    pub height_known: bool,
    pub snap: Option<SnapHit>,
}

/// Maximum segments considered for pairwise intersection tests.
const MAX_INTERSECTION_SEGMENTS: usize = 64;

/// Stateful engine (remembers the Tab-cycle position).
#[derive(Default)]
pub struct SnapEngine {
    candidates: Vec<SnapHit>,
    cycle: usize,
    last_cursor: DVec2,
    sources: Vec<SnapSource>,
}

impl SnapEngine {
    /// Advances to the next overlapping candidate (bound to Tab).
    pub fn cycle_next(&mut self) {
        self.cycle = self.cycle.wrapping_add(1);
    }

    /// All candidates from the last [`SnapEngine::pick`], best first.
    pub fn candidates(&self) -> &[SnapHit] {
        &self.candidates
    }

    /// Resolves the cursor to a world point, snapping if possible.
    pub fn pick(
        &mut self,
        settings: &SnapSettings,
        query: &SnapQuery<'_>,
        provider: &dyn SnapProvider,
    ) -> Pick {
        if query.cursor.distance(self.last_cursor) > 2.0 {
            self.cycle = 0;
        }
        self.last_cursor = query.cursor;
        self.candidates.clear();
        if settings.enabled {
            self.collect(settings, query, provider);
        }
        if let Some(hit) =
            (!self.candidates.is_empty()).then(|| self.candidates[self.cycle % self.candidates.len()])
        {
            return Pick {
                point: hit.point,
                height_known: hit.height_known,
                snap: Some(hit),
            };
        }
        free_point(query)
    }

    fn collect(&mut self, settings: &SnapSettings, q: &SnapQuery<'_>, provider: &dyn SnapProvider) {
        let cam = q.camera;
        let ray = cam.screen_ray(q.cursor);
        let aperture = settings.aperture_px;
        self.sources.clear();
        provider.query(&cam.pick_frustum(q.cursor, aperture), &mut self.sources);

        let mut out: Vec<SnapHit> = Vec::new();
        let mut push = |point: DVec3, mode: SnapMode, tag: Option<u64>, height_known: bool| {
            if let Some((screen, _)) = cam.world_to_screen(point)
                && screen.distance(q.cursor) <= aperture
            {
                out.push(SnapHit {
                    point,
                    mode,
                    screen,
                    tag,
                    height_known,
                });
            }
        };

        let mut segments: Vec<(DVec3, DVec3, u64)> = Vec::new();
        for src in &self.sources {
            let tag = Some(src.tag);
            match src.geom {
                SnapGeom::Point { p, height_known } => {
                    if settings.is_on(SnapMode::Node) {
                        push(p, SnapMode::Node, tag, height_known);
                    }
                }
                SnapGeom::Segment { a, b } => {
                    if settings.is_on(SnapMode::Endpoint) {
                        push(a, SnapMode::Endpoint, tag, true);
                        push(b, SnapMode::Endpoint, tag, true);
                    }
                    if settings.is_on(SnapMode::Midpoint) {
                        push((a + b) * 0.5, SnapMode::Midpoint, tag, true);
                    }
                    if settings.is_on(SnapMode::Nearest) {
                        let (_, s, _) = closest_ray_segment(ray.origin, ray.dir, a, b);
                        push(a.lerp(b, s), SnapMode::Nearest, tag, true);
                    }
                    if settings.is_on(SnapMode::Perpendicular)
                        && let Some(base) = q.base_point
                        && let Some(foot) = perpendicular_foot(base, a, b, cam.is_plan())
                    {
                        push(foot, SnapMode::Perpendicular, tag, true);
                    }
                    if segments.len() < MAX_INTERSECTION_SEGMENTS {
                        segments.push((a, b, src.tag));
                    }
                }
            }
        }

        if settings.is_on(SnapMode::Intersection) {
            for i in 0..segments.len() {
                for j in (i + 1)..segments.len() {
                    let (a, b, tag) = segments[i];
                    let (c, d, _) = segments[j];
                    if let Some(p) = intersect_segments(cam, a, b, c, d) {
                        push(p, SnapMode::Intersection, Some(tag), true);
                    }
                }
            }
        }

        if settings.is_on(SnapMode::Grid) && q.grid_spacing > 0.0 {
            let free = free_point(q);
            let s = q.grid_spacing;
            let g = DVec3::new(
                (free.point.x / s).round() * s,
                (free.point.y / s).round() * s,
                free.point.z,
            );
            push(g, SnapMode::Grid, None, free.height_known);
        }

        out.sort_by(|x, y| {
            x.mode.priority().cmp(&y.mode.priority()).then(
                x.screen
                    .distance(q.cursor)
                    .total_cmp(&y.screen.distance(q.cursor)),
            )
        });
        // Drop duplicates (e.g. shared vertices of adjoining segments).
        out.dedup_by(|x, y| x.mode == y.mode && x.point.distance(y.point) < 1e-9);
        self.candidates = out;
    }
}

/// The working-plane point under the cursor when nothing is snapped.
pub fn free_point(q: &SnapQuery<'_>) -> Pick {
    let ray = q.camera.screen_ray(q.cursor);
    if q.camera.is_plan() {
        let p = ray.intersect_z_plane(q.working_z).unwrap_or(q.camera.target);
        return Pick {
            point: p,
            height_known: false,
            snap: None,
        };
    }
    let point = ray
        .intersect_z_plane(q.working_z)
        .filter(|p| p.distance(ray.origin) < q.camera.far.max(1.0) * 4.0)
        .unwrap_or_else(|| q.camera.screen_to_target_plane(q.cursor));
    Pick {
        point,
        height_known: true,
        snap: None,
    }
}

/// Perpendicular foot from `base` onto segment `a–b` (in plan XY when `plan`).
fn perpendicular_foot(base: DVec3, a: DVec3, b: DVec3, plan: bool) -> Option<DVec3> {
    let mask = if plan {
        DVec3::new(1.0, 1.0, 0.0)
    } else {
        DVec3::ONE
    };
    let ab = (b - a) * mask;
    let len2 = ab.length_squared();
    if len2 == 0.0 {
        return None;
    }
    let t = ((base - a) * mask).dot(ab) / len2;
    (0.0..=1.0).contains(&t).then(|| a.lerp(b, t))
}

/// Apparent intersection of two segments as seen by the camera.
///
/// In plan view this is the exact XY intersection computed in world `f64`
/// (relative to `a` to preserve precision); the height is taken from the first
/// segment. Otherwise it is found in screen space and lifted back onto the first
/// segment with a ray–segment closest approach.
fn intersect_segments(cam: &Camera, a: DVec3, b: DVec3, c: DVec3, d: DVec3) -> Option<DVec3> {
    let in_range = |t: f64| (-1e-9..=1.0 + 1e-9).contains(&t);
    if cam.is_plan() {
        let o = a.truncate();
        let (t, u) = line_intersection_2d(DVec2::ZERO, b.truncate() - o, c.truncate() - o, d.truncate() - o)?;
        return (in_range(t) && in_range(u)).then(|| a.lerp(b, t.clamp(0.0, 1.0)));
    }
    let (sa, _) = cam.world_to_screen(a)?;
    let (sb, _) = cam.world_to_screen(b)?;
    let (sc, _) = cam.world_to_screen(c)?;
    let (sd, _) = cam.world_to_screen(d)?;
    let (t, u) = line_intersection_2d(sa, sb, sc, sd)?;
    if !(in_range(t) && in_range(u)) {
        return None;
    }
    let ray = cam.screen_ray(sa.lerp(sb, t));
    let (_, s, _) = closest_ray_segment(ray.origin, ray.dir, a, b);
    Some(a.lerp(b, s))
}

/// Nearest point on any segment, used for hover/selection hit testing.
pub fn distance_to_segment_px(cam: &Camera, cursor: DVec2, a: DVec3, b: DVec3) -> Option<f64> {
    let (sa, _) = cam.world_to_screen(a)?;
    let (sb, _) = cam.world_to_screen(b)?;
    let (_, p) = closest_point_on_segment(cursor.extend(0.0), sa.extend(0.0), sb.extend(0.0));
    Some(p.truncate().distance(cursor))
}

/// Draws the marker for a snap mode at `pos`.
pub fn draw_snap_glyph(painter: &egui::Painter, pos: egui::Pos2, mode: SnapMode, color: egui::Color32) {
    use egui::{Stroke, StrokeKind, pos2, vec2};
    let s = 6.0;
    let stroke = Stroke::new(2.0, color);
    match mode {
        SnapMode::Node => {
            painter.circle_stroke(pos, s, stroke);
            painter.line_segment([pos + vec2(-s, -s) * 0.7, pos + vec2(s, s) * 0.7], stroke);
            painter.line_segment([pos + vec2(-s, s) * 0.7, pos + vec2(s, -s) * 0.7], stroke);
        }
        SnapMode::Endpoint => {
            painter.rect_stroke(
                egui::Rect::from_center_size(pos, vec2(2.0 * s, 2.0 * s)),
                0.0,
                stroke,
                StrokeKind::Middle,
            );
        }
        SnapMode::Midpoint => {
            let pts = vec![
                pos + vec2(0.0, -s),
                pos + vec2(s, s * 0.8),
                pos + vec2(-s, s * 0.8),
            ];
            painter.add(egui::Shape::closed_line(pts, stroke));
        }
        SnapMode::Intersection => {
            painter.line_segment([pos + vec2(-s, -s), pos + vec2(s, s)], stroke);
            painter.line_segment([pos + vec2(-s, s), pos + vec2(s, -s)], stroke);
        }
        SnapMode::Perpendicular => {
            painter.line_segment([pos + vec2(-s, s), pos + vec2(s, s)], stroke);
            painter.line_segment([pos + vec2(-s, s), pos + vec2(-s, -s)], stroke);
            painter.line_segment([pos + vec2(-s, 0.0), pos + vec2(0.0, 0.0)], stroke);
            painter.line_segment([pos + vec2(0.0, 0.0), pos + vec2(0.0, s)], stroke);
        }
        SnapMode::Nearest => {
            let pts = vec![
                pos + vec2(-s, -s),
                pos + vec2(s, -s),
                pos + vec2(-s, s),
                pos + vec2(s, s),
            ];
            painter.add(egui::Shape::closed_line(pts, stroke));
        }
        SnapMode::Grid => {
            painter.line_segment([pos2(pos.x - s, pos.y), pos2(pos.x + s, pos.y)], stroke);
            painter.line_segment([pos2(pos.x, pos.y - s), pos2(pos.x, pos.y + s)], stroke);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use survey_core::Aabb3;

    struct Geometry(Vec<SnapSource>);

    impl SnapProvider for Geometry {
        fn query(&self, frustum: &Frustum, out: &mut Vec<SnapSource>) {
            for s in &self.0 {
                let b = match s.geom {
                    SnapGeom::Point { p, .. } => Aabb3::from_point(p),
                    SnapGeom::Segment { a, b } => Aabb3::from_points([a, b]),
                };
                if frustum.intersects_aabb(&b) {
                    out.push(*s);
                }
            }
        }
    }

    fn base() -> DVec3 {
        DVec3::new(512_000.0, 6_912_000.0, 50.0)
    }

    fn plan_camera() -> Camera {
        let mut cam = Camera {
            target: base(),
            distance: 20.0,
            viewport: DVec2::new(1000.0, 800.0),
            ..Default::default()
        };
        cam.fit_clip(&Aabb3::from_point(base()).expanded(100.0));
        cam
    }

    fn cross() -> Geometry {
        let b = base();
        Geometry(vec![
            SnapSource {
                geom: SnapGeom::Segment {
                    a: b + DVec3::new(-5.0, -5.0, 0.0),
                    b: b + DVec3::new(5.0, 5.0, 0.0),
                },
                tag: 1,
            },
            SnapSource {
                geom: SnapGeom::Segment {
                    a: b + DVec3::new(-5.0, 5.0, 2.0),
                    b: b + DVec3::new(5.0, -5.0, 2.0),
                },
                tag: 2,
            },
            SnapSource {
                geom: SnapGeom::Point {
                    p: b + DVec3::new(3.0, 0.0, 1.0),
                    height_known: true,
                },
                tag: 3,
            },
        ])
    }

    fn query(cam: &Camera, world: DVec3, base_point: Option<DVec3>) -> SnapQuery<'_> {
        let (cursor, _) = cam.world_to_screen(world).expect("visible");
        SnapQuery {
            camera: cam,
            cursor: cursor + DVec2::new(3.0, -2.0), // slightly off target
            base_point,
            working_z: 0.0,
            grid_spacing: 1.0,
        }
    }

    #[test]
    fn intersection_is_exact_at_mga_coordinates() {
        let cam = plan_camera();
        let mut engine = SnapEngine::default();
        let pick = engine.pick(&SnapSettings::default(), &query(&cam, base(), None), &cross());
        let hit = pick.snap.expect("snapped");
        assert_eq!(hit.mode, SnapMode::Intersection);
        assert!((hit.point.truncate() - base().truncate()).length() < 1e-9);
        assert!((hit.point.z - base().z).abs() < 1e-9, "height from first segment");
    }

    #[test]
    fn endpoint_beats_nearest_and_node_snaps() {
        let cam = plan_camera();
        let mut settings = SnapSettings::default();
        settings.set(SnapMode::Nearest, true);
        let mut engine = SnapEngine::default();
        let end = base() + DVec3::new(5.0, 5.0, 0.0);
        let pick = engine.pick(&settings, &query(&cam, end, None), &cross());
        assert_eq!(pick.snap.expect("hit").mode, SnapMode::Endpoint);
        assert_eq!(pick.point, end);

        let node = base() + DVec3::new(3.0, 0.0, 1.0);
        let pick = engine.pick(&settings, &query(&cam, node, None), &cross());
        let hit = pick.snap.expect("hit");
        assert_eq!((hit.mode, hit.tag), (SnapMode::Node, Some(3)));
    }

    #[test]
    fn perpendicular_from_base_point() {
        let cam = plan_camera();
        let settings = SnapSettings::only(SnapMode::Perpendicular, 10.0);
        let from = base() + DVec3::new(4.0, -4.0, 0.0);
        // Foot of perpendicular from (4,-4) onto the y=x line is the origin.
        let mut engine = SnapEngine::default();
        let pick = engine.pick(&settings, &query(&cam, base(), Some(from)), &cross());
        let hit = pick.snap.expect("hit");
        assert_eq!(hit.mode, SnapMode::Perpendicular);
        assert!((hit.point.truncate() - base().truncate()).length() < 1e-9);
    }

    #[test]
    fn no_snap_falls_back_to_working_plane() {
        let cam = plan_camera();
        let mut engine = SnapEngine::default();
        let target = base() + DVec3::new(-8.0, 3.0, 0.0);
        let pick = engine.pick(&SnapSettings::default(), &query(&cam, target, None), &cross());
        assert!(pick.snap.is_none());
        assert!(!pick.height_known);
        assert!((pick.point.truncate() - target.truncate()).length() < 0.2);
    }

    #[test]
    fn tab_cycles_candidates() {
        let cam = plan_camera();
        let mut settings = SnapSettings::default();
        settings.set(SnapMode::Nearest, true);
        let mut engine = SnapEngine::default();
        let q = query(&cam, base(), None);
        let first = engine.pick(&settings, &q, &cross()).snap.expect("hit");
        assert!(engine.candidates().len() > 1);
        engine.cycle_next();
        let second = engine.pick(&settings, &q, &cross()).snap.expect("hit");
        assert_ne!(first, second);
    }

    #[test]
    fn node_beats_coincident_vertex() {
        let cam = plan_camera();
        let p = base() + DVec3::new(5.0, 5.0, 0.0);
        let geometry = Geometry(vec![
            SnapSource {
                geom: SnapGeom::Segment { a: base(), b: p },
                tag: 1,
            },
            SnapSource {
                geom: SnapGeom::Point {
                    p,
                    height_known: false,
                },
                tag: 2,
            },
        ]);
        let pick = SnapEngine::default().pick(&SnapSettings::default(), &query(&cam, p, None), &geometry);
        let hit = pick.snap.expect("hit");
        assert_eq!((hit.mode, hit.tag), (SnapMode::Node, Some(2)));
        assert!(!pick.height_known, "unknown height is preserved");
    }

    #[test]
    fn mode_codes() {
        assert_eq!(SnapMode::from_code("END"), Some(SnapMode::Endpoint));
        assert_eq!(SnapMode::from_code("perp"), Some(SnapMode::Perpendicular));
        assert_eq!(SnapMode::from_code("en"), None);
    }
}
