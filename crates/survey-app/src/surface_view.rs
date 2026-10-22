//! Drawing surfaces: hill-shaded triangles (coloured by height or a single
//! colour), triangle edges and contours, built into their own render scene so
//! they rebuild only when a surface changes.

use std::sync::Arc;

use cad_render::{Scene, SceneBuilder};
use egui_cad::{CadTheme, ThemeMode};
use glam::DVec3;
use survey_core::contour::{Contour, contours};
use survey_core::tin::TinIndex;
use survey_model::{Document, Surface, SurfaceId};

/// Hypsometric colour stops (low → high).
const RAMP: [[f32; 3]; 6] = [
    [46.0, 110.0, 64.0],
    [96.0, 160.0, 82.0],
    [190.0, 196.0, 110.0],
    [205.0, 160.0, 92.0],
    [160.0, 112.0, 80.0],
    [235.0, 230.0, 220.0],
];

pub fn ramp(t: f64) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0) * (RAMP.len() - 1) as f64;
    let i = (t.floor() as usize).min(RAMP.len() - 2);
    let f = (t - i as f64) as f32;
    let (a, b) = (RAMP[i], RAMP[i + 1]);
    [
        a[0] + (b[0] - a[0]) * f,
        a[1] + (b[1] - a[1]) * f,
        a[2] + (b[2] - a[2]) * f,
    ]
}

/// Cut (negative, blue) to fill (positive, red) through white at zero.
fn isopach(d: f64, max: f64) -> [f32; 3] {
    let t = (d / max.max(1e-9)).clamp(-1.0, 1.0) as f32;
    let white = [235.0, 235.0, 235.0];
    let end = if t < 0.0 {
        [40.0, 110.0, 220.0]
    } else {
        [215.0, 50.0, 45.0]
    };
    let k = t.abs();
    [
        white[0] + (end[0] - white[0]) * k,
        white[1] + (end[1] - white[1]) * k,
        white[2] + (end[2] - white[2]) * k,
    ]
}

/// Lambert shade from a light in the north-west, 45° up (as on hillshade maps).
fn shade(normal: DVec3) -> f32 {
    let light = DVec3::new(-1.0, 1.0, 2f64.sqrt()).normalize();
    let n = if normal.z < 0.0 { -normal } else { normal };
    (0.35 + 0.65 * n.dot(light).max(0.0)) as f32
}

fn rgba(c: [f32; 3], k: f32, alpha: u8) -> [u8; 4] {
    let v = |x: f32| (x * k).round().clamp(0.0, 255.0) as u8;
    [v(c[0]), v(c[1]), v(c[2]), alpha]
}

/// Most contour levels traced per surface (guards against a tiny interval).
pub const MAX_CONTOUR_LEVELS: usize = 2000;

/// Where a contour label goes and the height it shows.
#[derive(Clone, Copy, Debug)]
pub struct ContourLabel {
    pub position: DVec3,
    /// Direction of the contour there (plan), for keeping labels upright.
    pub along: glam::DVec2,
    pub z: f64,
}

/// Rebuilds the surface scene only when surfaces (or the theme) change.
#[derive(Default)]
pub struct SurfaceSceneCache {
    key: Option<(u64, ThemeMode)>,
    revision: u64,
    scene: Option<Arc<Scene>>,
    labels: Vec<ContourLabel>,
}

impl SurfaceSceneCache {
    pub fn get(&mut self, doc: &Document, theme: &CadTheme) -> Option<Arc<Scene>> {
        let key = (doc.surfaces_revision(), theme.mode);
        if self.key != Some(key) {
            self.revision += 1;
            let (scene, labels) = build(doc, theme, self.revision);
            self.scene = scene;
            self.labels = labels;
            self.key = Some(key);
        }
        self.scene.clone()
    }

    /// Label anchors on major contours (from the last [`Self::get`]).
    pub fn labels(&self) -> &[ContourLabel] {
        &self.labels
    }

    pub fn invalidate(&mut self) {
        self.key = None;
    }
}

fn build(doc: &Document, theme: &CadTheme, revision: u64) -> (Option<Arc<Scene>>, Vec<ContourLabel>) {
    let visible: Vec<&Surface> = doc
        .surfaces()
        .map(|(_, s)| s)
        .filter(|s| s.style.visible && !s.tin.triangles.is_empty())
        .collect();
    let mut labels = Vec::new();
    if visible.is_empty() {
        return (None, labels);
    }
    let mut b = SceneBuilder::default();
    for s in visible {
        add_surface(&mut b, s, theme, &mut labels);
    }
    // Offset revisions so they never collide with the drawing scene's.
    (Some(Arc::new(b.build(revision | (1 << 62)))), labels)
}

/// The contours a surface's style asks for.
pub fn surface_contours(s: &Surface) -> Vec<Contour> {
    let c = &s.style.contours;
    contours(&s.tin, c.interval, c.major_every, MAX_CONTOUR_LEVELS)
}

/// Label anchors spread along a contour: its middle, and about every 80 vertices.
fn label_anchors(c: &Contour, out: &mut Vec<ContourLabel>) {
    let n = c.points.len();
    if n < 2 {
        return;
    }
    let step = 80.max(n / 4);
    let mut i = n / 2;
    let mut placed = 0;
    while placed < 1 + n / step {
        let a = c.points[i.min(n - 2)];
        let b = c.points[(i + 1).min(n - 1)];
        out.push(ContourLabel {
            position: (a + b) * 0.5,
            along: (b - a).truncate(),
            z: c.z,
        });
        placed += 1;
        i = (i + step) % (n - 1);
    }
}

fn add_surface(b: &mut SceneBuilder, s: &Surface, theme: &CadTheme, labels: &mut Vec<ContourLabel>) {
    let tin = &s.tin;
    let bounds = tin.bounds();
    let (zmin, zmax) = (bounds.min.z, bounds.max.z);
    let span = (zmax - zmin).max(1e-9);
    let [r, g, bl, _] = s.style.color.0;
    let plain = [f32::from(r), f32::from(g), f32::from(bl)];
    if s.style.shaded {
        for i in 0..tin.triangles.len() {
            let corners = tin.triangle(i);
            let [a, bb, c] = corners;
            // Difference surfaces show cut/fill flat; their heights are not terrain.
            let k = if s.style.isopach {
                1.0
            } else {
                shade((bb - a).cross(c - a).normalize_or_zero())
            };
            let colors = corners.map(|p| {
                let base = if s.style.isopach {
                    isopach(p.z, zmin.abs().max(zmax.abs()))
                } else if s.style.elevation_colors {
                    ramp((p.z - zmin) / span)
                } else {
                    plain
                };
                rgba(base, k, 255)
            });
            b.add_triangle(corners, colors);
        }
    }
    if s.style.triangles {
        let edge = if s.style.shaded {
            // Dark, translucent edges over the shading.
            [20, 24, 28, 90]
        } else {
            let mut c = s.style.color.0;
            if theme.mode == ThemeMode::Light {
                c = [c[0] / 2, c[1] / 2, c[2] / 2, 255];
            }
            c
        };
        for [i, j] in tin.edges() {
            b.add_line(tin.vertices[i as usize], tin.vertices[j as usize], edge, 1.0);
        }
    }
    if s.style.contours.show {
        let (minor, major) = if s.style.shaded {
            ([45, 35, 25, 150], [35, 25, 15, 230])
        } else {
            let c = s.style.color.0;
            let c = if theme.mode == ThemeMode::Light {
                [c[0] / 2, c[1] / 2, c[2] / 2, 255]
            } else {
                c
            };
            ([c[0], c[1], c[2], 150], c)
        };
        for c in surface_contours(s) {
            let (color, width) = if c.major { (major, 1.8) } else { (minor, 1.0) };
            let mut pts = c.points.clone();
            if c.closed {
                pts.push(pts[0]);
            }
            for w in pts.windows(2) {
                b.add_line(w[0], w[1], color, width);
            }
            if c.major && s.style.contours.labels {
                label_anchors(&c, labels);
            }
        }
    }
}

/// Point-location indexes of the visible surfaces, rebuilt when surfaces change.
#[derive(Default)]
pub struct SurfaceIndexes {
    revision: u64,
    indexes: Vec<(SurfaceId, TinIndex)>,
}

impl SurfaceIndexes {
    fn refresh(&mut self, doc: &Document) {
        if self.revision != doc.surfaces_revision()
            || self.indexes.is_empty() && doc.surfaces().next().is_some()
        {
            self.indexes = doc
                .surfaces()
                .filter(|(_, s)| s.style.visible)
                .map(|(id, s)| (id, s.tin.index()))
                .collect();
            self.revision = doc.surfaces_revision();
        }
    }

    /// Height of the first visible surface (the preferred one first) at `p`.
    pub fn height_at(&mut self, doc: &Document, preferred: Option<SurfaceId>, p: glam::DVec2) -> Option<f64> {
        self.refresh(doc);
        let order = self
            .indexes
            .iter()
            .filter(|(id, _)| Some(*id) == preferred)
            .chain(self.indexes.iter().filter(|(id, _)| Some(*id) != preferred));
        for (id, index) in order {
            if let Some(s) = doc.surface(*id)
                && let Some(z) = index.height_at(&s.tin, p)
            {
                return Some(z);
            }
        }
        None
    }

    pub fn invalidate(&mut self) {
        self.indexes.clear();
        self.revision = 0;
    }
}
