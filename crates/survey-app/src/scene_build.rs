//! Bridges the document to the renderer and the snapping engine.

use std::sync::Arc;

use cad_render::scene::PointShape;
use cad_render::{Camera, Frustum, Scene, SceneBuilder};
use egui::Color32;
use egui_cad::snap::{SnapGeom, SnapProvider, SnapSource, distance_to_segment_px};
use egui_cad::{CadTheme, ThemeMode};
use glam::DVec2;
use survey_core::Aabb3;
use survey_core::geom::segments;
use survey_model::{Document, Entity, EntityId, Selection};

const POINT_SIZE: f32 = 7.0;
const LINE_WIDTH: f32 = 1.5;

/// Rebuilds the GPU scene only when the document, selection or theme change.
#[derive(Default)]
pub struct SceneCache {
    key: Option<(u64, u64, ThemeMode)>,
    revision: u64,
    scene: Arc<Scene>,
}

impl SceneCache {
    pub fn get(&mut self, doc: &Document, selection: &Selection, theme: &CadTheme) -> Arc<Scene> {
        let key = (doc.revision(), selection.revision(), theme.mode);
        if self.key != Some(key) {
            self.revision += 1;
            self.scene = Arc::new(build_scene(doc, selection, theme, self.revision));
            self.key = Some(key);
        }
        self.scene.clone()
    }

    /// Forces a rebuild (e.g. after opening another project).
    pub fn invalidate(&mut self) {
        self.key = None;
    }
}

fn build_scene(doc: &Document, selection: &Selection, theme: &CadTheme, revision: u64) -> Scene {
    let mut b = SceneBuilder::default();
    let selected = theme.selection.to_srgba_unmultiplied();
    for (id, entity) in doc.entities() {
        let Some(layer) = doc.layer(entity.layer()) else {
            continue;
        };
        if !layer.visible {
            continue;
        }
        let is_selected = selection.contains(id);
        let color = if is_selected {
            selected
        } else {
            layer_color(layer.color.0, theme)
        };
        match entity {
            Entity::Point(p) => {
                let shape = if p.height_known {
                    PointShape::Circle
                } else {
                    PointShape::Square
                };
                let size = if is_selected { POINT_SIZE + 3.0 } else { POINT_SIZE };
                b.add_point(p.position, color, size, shape);
            }
            Entity::Polyline(l) => {
                let width = if is_selected { LINE_WIDTH + 1.5 } else { LINE_WIDTH };
                for (a, c) in segments(&l.vertices, l.closed) {
                    b.add_line(a, c, color, width);
                }
            }
        }
    }
    b.build(revision)
}

/// Near-white layer colours are drawn dark on a light background so they stay visible.
fn layer_color(c: [u8; 4], theme: &CadTheme) -> [u8; 4] {
    if theme.mode == ThemeMode::Light && c[0] > 200 && c[1] > 200 && c[2] > 200 {
        let t = theme.text.to_srgba_unmultiplied();
        [t[0], t[1], t[2], c[3]]
    } else {
        c
    }
}

pub fn color32(c: survey_model::Color) -> Color32 {
    let [r, g, b, a] = c.0;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Snap geometry from visible layers.
pub struct DocSnapProvider<'a> {
    pub doc: &'a Document,
}

impl SnapProvider for DocSnapProvider<'_> {
    fn query(&self, frustum: &Frustum, out: &mut Vec<SnapSource>) {
        let mut ids = Vec::new();
        self.doc.query(&|b| frustum.intersects_aabb(b), &mut ids);
        for id in ids {
            let Some(entity) = self.doc.entity(id) else {
                continue;
            };
            if !self.doc.is_layer_visible(entity.layer()) {
                continue;
            }
            let tag = id.0 as u64;
            match entity {
                Entity::Point(p) => out.push(SnapSource {
                    geom: SnapGeom::Point {
                        p: p.position,
                        height_known: p.height_known,
                    },
                    tag,
                }),
                Entity::Polyline(l) => {
                    for (a, b) in segments(&l.vertices, l.closed) {
                        if frustum.intersects_aabb(&Aabb3::from_points([a, b])) {
                            out.push(SnapSource {
                                geom: SnapGeom::Segment { a, b },
                                tag,
                            });
                        }
                    }
                }
            }
        }
    }
}

/// The selectable entity nearest the cursor within `aperture_px` (points win ties).
pub fn hit_test(doc: &Document, cam: &Camera, cursor: DVec2, aperture_px: f64) -> Option<EntityId> {
    let frustum = cam.pick_frustum(cursor, aperture_px);
    let mut ids = Vec::new();
    doc.query(&|b| frustum.intersects_aabb(b), &mut ids);
    let mut best: Option<(f64, EntityId)> = None;
    for id in ids {
        if !doc.is_entity_editable(id) {
            continue;
        }
        let d = match doc.entity(id) {
            Some(Entity::Point(p)) => cam
                .world_to_screen(p.position)
                .map(|(s, _)| s.distance(cursor) - 1.0),
            Some(Entity::Polyline(l)) => segments(&l.vertices, l.closed)
                .filter_map(|(a, b)| distance_to_segment_px(cam, cursor, a, b))
                .reduce(f64::min),
            None => None,
        };
        if let Some(d) = d
            && d <= aperture_px
            && best.is_none_or(|(bd, _)| d < bd)
        {
            best = Some((d, id));
        }
    }
    best.map(|(_, id)| id)
}
