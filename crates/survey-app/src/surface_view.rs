//! Drawing surfaces: hill-shaded triangles (coloured by height or a single
//! colour), triangle edges and contours, built into their own render scene so
//! they rebuild only when a surface changes.

use std::sync::Arc;

use cad_render::{Scene, SceneBuilder};
use egui_cad::{CadTheme, ThemeMode};
use glam::DVec3;
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

fn ramp(t: f64) -> [f32; 3] {
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

/// Rebuilds the surface scene only when surfaces (or the theme) change.
#[derive(Default)]
pub struct SurfaceSceneCache {
    key: Option<(u64, ThemeMode)>,
    revision: u64,
    scene: Option<Arc<Scene>>,
}

impl SurfaceSceneCache {
    pub fn get(&mut self, doc: &Document, theme: &CadTheme) -> Option<Arc<Scene>> {
        let key = (doc.surfaces_revision(), theme.mode);
        if self.key != Some(key) {
            self.revision += 1;
            self.scene = build(doc, theme, self.revision);
            self.key = Some(key);
        }
        self.scene.clone()
    }

    pub fn invalidate(&mut self) {
        self.key = None;
    }
}

fn build(doc: &Document, theme: &CadTheme, revision: u64) -> Option<Arc<Scene>> {
    let visible: Vec<&Surface> = doc
        .surfaces()
        .map(|(_, s)| s)
        .filter(|s| s.style.visible && !s.tin.triangles.is_empty())
        .collect();
    if visible.is_empty() {
        return None;
    }
    let mut b = SceneBuilder::default();
    for s in visible {
        add_surface(&mut b, s, theme);
    }
    // Offset revisions so they never collide with the drawing scene's.
    Some(Arc::new(b.build(revision | (1 << 62))))
}

fn add_surface(b: &mut SceneBuilder, s: &Surface, theme: &CadTheme) {
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
            let k = shade((bb - a).cross(c - a).normalize_or_zero());
            let colors = corners.map(|p| {
                let base = if s.style.elevation_colors {
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
