//! Bridges the document to the renderer and the snapping engine.

use std::sync::Arc;

use std::collections::HashMap;

use cad_render::{Camera, Frustum, Scene, SceneBuilder};
use egui::Color32;
use egui_cad::selection::SelectionBox;
use egui_cad::snap::{SnapGeom, SnapProvider, SnapSource, distance_to_segment_px};
use egui_cad::{CadTheme, ThemeMode};
use glam::DVec2;
use survey_core::Aabb3;
use survey_core::arc::Arc as ArcSegment;
use survey_core::field_to_finish::FeatureLibrary;
use survey_core::style::{CodeStyle, Symbol};
use survey_model::{Document, Entity, EntityId, Selection};

pub(crate) const POINT_SIZE: f32 = 7.0;
pub(crate) const LINE_WIDTH: f32 = 1.5;
/// Chord tolerance for drawing arcs (metres). Fine enough for survey-scale zooms;
/// tessellation is capped per arc, so large radii stay cheap.
const ARC_TOLERANCE: f64 = 0.001;

/// Rebuilds the drawing's GPU scene only when the document or theme change.
#[derive(Default)]
pub struct SceneCache {
    key: Option<(u64, ThemeMode)>,
    revision: u64,
    scene: Arc<Scene>,
}

impl SceneCache {
    pub fn get(&mut self, doc: &Document, theme: &CadTheme) -> Arc<Scene> {
        let key = (doc.revision(), theme.mode);
        if self.key != Some(key) {
            self.revision += 1;
            self.scene = Arc::new(build_scene(doc, None, theme, self.revision));
            self.key = Some(key);
        }
        self.scene.clone()
    }

    /// Forces a rebuild (e.g. after opening another project).
    pub fn invalidate(&mut self) {
        self.key = None;
    }
}

/// The selected objects again, highlighted and drawn over the drawing's scene,
/// so selecting rebuilds only the selection, not the whole drawing.
#[derive(Default)]
pub struct SelectionCache {
    key: Option<(u64, u64, ThemeMode)>,
    revision: u64,
    scene: Option<Arc<Scene>>,
}

impl SelectionCache {
    /// `None` when nothing is selected.
    pub fn get(&mut self, doc: &Document, selection: &Selection, theme: &CadTheme) -> Option<Arc<Scene>> {
        let key = (doc.revision(), selection.revision(), theme.mode);
        if self.key != Some(key) {
            self.revision += 1;
            self.scene = (!selection.is_empty()).then(|| {
                // A revision of its own (see the surface and cloud scenes).
                let mut scene = build_scene(doc, Some(selection), theme, self.revision | (1 << 60));
                scene.on_top = true;
                Arc::new(scene)
            });
            self.key = Some(key);
        }
        self.scene.clone()
    }

    pub fn invalidate(&mut self) {
        self.key = None;
    }
}

/// Style of a code (from the feature library) and the colour it draws in,
/// cached per distinct code text.
struct Styles<'a> {
    doc: &'a Document,
    library: &'a FeatureLibrary,
    cache: HashMap<&'a str, Option<(CodeStyle, [u8; 4])>>,
}

impl<'a> Styles<'a> {
    fn get(&mut self, code: &'a str) -> Option<(CodeStyle, [u8; 4])> {
        if code.is_empty() {
            return None;
        }
        let (doc, library) = (self.doc, self.library);
        *self.cache.entry(code).or_insert_with(|| {
            library.feature_for(code).map(|f| {
                let [r, g, b] = f.color;
                // "By layer" is the feature's own layer, wherever the object currently sits.
                let color = match doc.layer_by_name(&f.layer) {
                    Some(layer) if f.style.by_layer => layer.color.0,
                    _ => [r, g, b, 255],
                };
                (f.style, color)
            })
        })
    }
}

/// The drawing (`highlight` is `None`) or the highlighted copy of the selected objects.
fn build_scene(doc: &Document, highlight: Option<&Selection>, theme: &CadTheme, revision: u64) -> Scene {
    let mut b = SceneBuilder::default();
    let selected = theme.selection.to_srgba_unmultiplied();
    let mut styles = Styles {
        doc,
        library: &doc.meta().feature_codes,
        cache: HashMap::new(),
    };
    let entities: Box<dyn Iterator<Item = (EntityId, &Entity)>> = match highlight {
        None => Box::new(doc.entities()),
        Some(sel) => Box::new(sel.ids().iter().filter_map(|id| Some((*id, doc.entity(*id)?)))),
    };
    let is_selected = highlight.is_some();
    for (_, entity) in entities {
        let Some(layer) = doc.layer(entity.layer()) else {
            continue;
        };
        if !layer.visible {
            continue;
        }
        // Text and dimensions are sized on paper and drawn over the scene (see
        // the viewport overlays).
        if matches!(entity, Entity::Text(_) | Entity::Dimension(_)) {
            continue;
        }
        let code = entity.code();
        let style = styles.get(code);
        let color = if is_selected {
            selected
        } else {
            match style {
                Some((_, c)) => layer_color(c, theme),
                None => layer_color(layer.color.0, theme),
            }
        };
        match entity {
            Entity::Point(p) => {
                // Uncoded points: a dot when the height is known, a square when not.
                let (symbol, size) = match style {
                    Some((s, _)) => (s.symbol, s.symbol_size),
                    None if p.height_known => (Symbol::Dot, POINT_SIZE),
                    None => (Symbol::Square, POINT_SIZE),
                };
                let size = if is_selected { size + 3.0 } else { size };
                b.add_point(p.position, color, size, symbol);
            }
            Entity::Polyline(l) => {
                // A code's style wins; otherwise the layer's.
                let (mut line_style, weight) = style
                    .map_or((layer.line_style, layer.line_weight), |(s, _)| {
                        (s.line_style, s.line_weight)
                    });
                // Highlights are solid: a wider dashed copy would not line up with
                // the dashes of the line underneath.
                if is_selected {
                    line_style = Default::default();
                }
                let width = if is_selected { weight + 1.5 } else { weight };
                // Distance along the polyline, so dash patterns run on across vertices.
                let mut along = 0.0;
                for part in l.tessellate(ARC_TOLERANCE) {
                    for w in part.windows(2) {
                        b.add_styled_line(w[0], w[1], color, width, line_style, along);
                        along += w[0].distance(w[1]);
                    }
                }
            }
            Entity::Text(_) | Entity::Dimension(_) => {}
        }
    }
    b.build(revision)
}

/// Near-white layer colours are drawn dark on a light background so they stay visible.
/// Display colour for a layer colour on the current canvas. Stored colours are
/// never changed; on a light canvas, near-white becomes the text colour and
/// other very bright colours (DXF yellow, green, cyan…) are darkened so they
/// stay readable — the same adaptation CAD packages apply to white/black.
pub(crate) fn layer_color(c: [u8; 4], theme: &CadTheme) -> [u8; 4] {
    if theme.mode != ThemeMode::Light {
        return c;
    }
    if c[0] > 200 && c[1] > 200 && c[2] > 200 {
        let t = theme.text.to_srgba_unmultiplied();
        return [t[0], t[1], t[2], c[3]];
    }
    // Rec. 709 luma of the sRGB values is enough for this heuristic.
    let luma = 0.2126 * f32::from(c[0]) + 0.7152 * f32::from(c[1]) + 0.0722 * f32::from(c[2]);
    if luma > 140.0 {
        let k = 110.0 / luma;
        let d = |v: u8| (f32::from(v) * k).round() as u8;
        [d(c[0]), d(c[1]), d(c[2]), c[3]]
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
                Entity::Text(t) => out.push(SnapSource {
                    geom: SnapGeom::Point {
                        p: t.position,
                        height_known: false,
                    },
                    tag,
                }),
                // Dimensions measure what is drawn; they are not snapped to.
                Entity::Dimension(_) => {}
                Entity::Polyline(l) => {
                    for (a, b, bulge) in l.segments() {
                        let (geom, bounds) = match ArcSegment::from_bulge(a, b, bulge) {
                            Some(arc) => (
                                SnapGeom::Arc { a, b, bulge },
                                Aabb3::from_points(arc.tessellate(arc.radius * 0.01)),
                            ),
                            None => (SnapGeom::Segment { a, b }, Aabb3::from_points([a, b])),
                        };
                        if frustum.intersects_aabb(&bounds) {
                            out.push(SnapSource { geom, tag });
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
    for &id in &ids {
        if !doc.is_entity_editable(id) {
            continue;
        }
        let d = match doc.entity(id) {
            Some(Entity::Point(p)) => cam
                .world_to_screen(p.position)
                .map(|(s, _)| s.distance(cursor) - 1.0),
            Some(Entity::Polyline(l)) => l
                .tessellate(cam.world_per_pixel() * 0.5)
                .iter()
                .flat_map(|part| part.windows(2))
                .filter_map(|w| distance_to_segment_px(cam, cursor, w[0], w[1]))
                .reduce(f64::min),
            Some(Entity::Dimension(d)) => {
                let [a, b] = d.line().map(|p| p.extend(d.line_at.z));
                [[d.from, a], [a, b], [b, d.to]]
                    .iter()
                    .filter_map(|[p, q]| distance_to_segment_px(cam, cursor, *p, *q))
                    .reduce(f64::min)
            }
            // Text is found below, by its whole box.
            Some(Entity::Text(_)) | None => None,
        };
        if let Some(d) = d
            && d <= aperture_px
            && best.is_none_or(|(bd, _)| d < bd)
        {
            best = Some((d, id));
        }
    }
    // Text reaches beyond its insertion point (the only place indexed), so look
    // through the notes in view. Over the letters counts as 2 px away: a point
    // or line right under the cursor still wins, anything further does not.
    let view = cam.rect_frustum(DVec2::ZERO, cam.viewport);
    ids.clear();
    doc.query(&|b| view.intersects_aabb(b), &mut ids);
    for id in ids {
        let Some(Entity::Text(t)) = doc.entity(id) else {
            continue;
        };
        if doc.is_entity_editable(id)
            && let Some(place) = text_placement(cam, t)
            && let d = place.distance(cursor).max(2.0)
            && d <= aperture_px
            && best.is_none_or(|(bd, _)| d < bd)
        {
            best = Some((d, id));
        }
    }
    best.map(|(_, id)| id)
}

/// Screen size of text: pixels per paper millimetre of capital height. Text
/// keeps this size at any zoom, as point labels do.
pub const TEXT_PX_PER_MM: f64 = 4.4;

/// Where a text note sits on screen (viewport pixels, y down).
pub struct TextPlacement {
    /// The insertion point.
    pub anchor: DVec2,
    /// Unit vectors along the reading direction and towards the top of the letters.
    pub along: DVec2,
    pub up: DVec2,
    /// Font size (em) in pixels.
    pub font_px: f64,
    /// Box corners: bottom-left, bottom-right, top-right, top-left.
    pub corners: [DVec2; 4],
}

impl TextPlacement {
    /// Pixels from `p` to the box (0 inside).
    pub fn distance(&self, p: DVec2) -> f64 {
        let c = &self.corners;
        let local = DVec2::new((p - c[0]).dot(self.along), (p - c[0]).dot(self.up));
        let size = DVec2::new((c[1] - c[0]).length(), (c[3] - c[0]).length());
        (local - local.clamp(DVec2::ZERO, size)).length()
    }
}

/// Line height as a multiple of the font size, for multi-line text.
pub const TEXT_LINE_SPACING: f64 = 1.25;

/// The note's place on screen, or `None` when its insertion point is not in front of the camera.
pub fn text_placement(cam: &Camera, t: &survey_model::TextNote) -> Option<TextPlacement> {
    let (anchor, _) = cam.world_to_screen(t.position)?;
    // The reading direction as seen: project a short step along it.
    let (s, c) = t.rotation.to_radians().sin_cos();
    let step = glam::DVec3::new(c, s, 0.0) * cam.world_per_pixel() * 20.0;
    let along = cam
        .world_to_screen(t.position + step)
        .map(|(p, _)| p - anchor)
        .filter(|d| d.length() > 1e-6)
        .map_or(DVec2::X, |d| d.normalize());
    let up = DVec2::new(along.y, -along.x);
    let font_px = t.height * TEXT_PX_PER_MM / survey_io::plot::CAP_HEIGHT;
    let lines: Vec<&str> = t.text.lines().collect();
    let width = lines
        .iter()
        .map(|l| survey_io::plot::text_width(l, font_px, false))
        .fold(0.0, f64::max);
    let height = font_px * TEXT_LINE_SPACING * lines.len().max(1) as f64;
    let left = -width * align_fraction(t.align);
    let corner = |x: f64, y: f64| anchor + along * x + up * y;
    let corners = [
        corner(left, -height / 2.0),
        corner(left + width, -height / 2.0),
        corner(left + width, height / 2.0),
        corner(left, height / 2.0),
    ];
    Some(TextPlacement {
        anchor,
        along,
        up,
        font_px,
        corners,
    })
}

/// How far along the text its insertion point is (0 left, ½ centre, 1 right).
pub fn align_fraction(align: survey_model::TextAlign) -> f64 {
    match align {
        survey_model::TextAlign::Left => 0.0,
        survey_model::TextAlign::Centre => 0.5,
        survey_model::TextAlign::Right => 1.0,
    }
}

/// Editable entities chosen by a window/crossing box.
pub fn select_in_box(doc: &Document, cam: &Camera, sel: &SelectionBox) -> Vec<EntityId> {
    let frustum = cam.rect_frustum(sel.min, sel.max);
    let mut ids = Vec::new();
    doc.query(&|b| frustum.intersects_aabb(b), &mut ids);
    let tolerance = cam.world_per_pixel() * 0.5;
    ids.retain(|id| {
        doc.is_entity_editable(*id)
            && match doc.entity(*id) {
                Some(Entity::Point(p)) => sel.selects(cam, &[vec![p.position]]),
                Some(Entity::Polyline(l)) => sel.selects(cam, &l.tessellate(tolerance)),
                Some(Entity::Text(t)) => sel.selects(cam, &[vec![t.position]]),
                Some(Entity::Dimension(d)) => {
                    let [a, b] = d.line().map(|p| p.extend(d.line_at.z));
                    sel.selects(cam, &[vec![d.from, a, b, d.to]])
                }
                None => false,
            }
    });
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bright_colours_are_darkened_on_light_canvas() {
        let light = CadTheme::light();
        let green = layer_color([0, 255, 0, 255], &light);
        assert!(green[1] <= 160, "{green:?}");
        assert_eq!(layer_color([200, 0, 0, 255], &light), [200, 0, 0, 255]);
        assert_eq!(layer_color([0, 255, 0, 255], &CadTheme::dark()), [0, 255, 0, 255]);
    }

    #[test]
    fn selecting_rebuilds_only_the_selection() {
        use survey_model::{ChangeSet, Command, SurveyPoint};
        let mut doc = Document::new();
        let mut add = ChangeSet::new("add");
        for i in 0..50 {
            add.push_insert_entity(Entity::Point(SurveyPoint {
                name: i.to_string(),
                position: glam::DVec3::new(f64::from(i), 0.0, 0.0),
                height_known: true,
                code: String::new(),
                description: String::new(),
                layer: doc.current_layer(),
            }));
        }
        add.apply(&mut doc);
        let theme = CadTheme::dark();
        let (mut drawing, mut highlight) = (SceneCache::default(), SelectionCache::default());
        let mut selection = Selection::default();

        let base = drawing.get(&doc, &theme);
        assert_eq!(base.point_count(), 50);
        assert!(highlight.get(&doc, &selection, &theme).is_none());

        let ids: Vec<EntityId> = doc.entities().map(|(id, _)| id).take(3).collect();
        selection.set(ids);
        assert!(
            Arc::ptr_eq(&base, &drawing.get(&doc, &theme)),
            "the drawing is not rebuilt"
        );
        let sel = highlight.get(&doc, &selection, &theme).expect("highlight");
        assert_eq!(sel.point_count(), 3);
        assert!(sel.on_top && !base.on_top);
        assert_ne!(sel.revision, base.revision);
        assert!(Arc::ptr_eq(
            &sel,
            &highlight.get(&doc, &selection, &theme).expect("cached")
        ));
    }
}
