//! Viewport overlays drawn over the 3D scene: labels, annotations, chainages, sheet frame.

use std::collections::HashMap;

use egui::{Color32, Stroke};
use egui_cad::labels::LabelPlacer;
use survey_core::style::LabelSet;

use super::*;

/// Horizontal gap between a point marker and its label (px).
const LABEL_GAP: f32 = 7.0;

impl SurveyApp {
    /// Text notes in view, at their paper size, in their layer's colour.
    pub(super) fn draw_texts(&self, painter: &egui::Painter, rect: egui::Rect, theme: &CadTheme) {
        let cam = &self.viewport.camera;
        let doc = &self.session.project.doc;
        let selection = &self.session.project.selection;
        let view = cam.rect_frustum(glam::DVec2::ZERO, cam.viewport);
        let mut ids = Vec::new();
        doc.query(&|b| view.intersects_aabb(b), &mut ids);
        for id in ids {
            let Some(Entity::Text(t)) = doc.entity(id) else {
                continue;
            };
            let Some(layer) = doc.layer(t.layer).filter(|l| l.visible) else {
                continue;
            };
            let Some(place) = crate::scene_build::text_placement(cam, t) else {
                continue;
            };
            let color = if selection.contains(id) {
                theme.selection
            } else {
                let [r, g, b, a] = crate::scene_build::layer_color(layer.color.0, theme);
                Color32::from_rgba_unmultiplied(r, g, b, a)
            };
            let font = egui::FontId::proportional(place.font_px as f32);
            let angle = place.along.y.atan2(place.along.x) as f32;
            let lines: Vec<&str> = t.text.lines().collect();
            let step = place.font_px * crate::scene_build::TEXT_LINE_SPACING;
            let along = crate::scene_build::align_fraction(t.align);
            for (i, line) in lines.iter().enumerate() {
                let width = f64::from(
                    painter
                        .layout_no_wrap(line.to_string(), font.clone(), color)
                        .size()
                        .x,
                );
                let shift = ((lines.len() - 1) as f64 / 2.0 - i as f64) * step;
                let centre = place.anchor + place.along * (width * (0.5 - along)) + place.up * shift;
                halo_text(
                    painter,
                    to_screen(rect, centre),
                    line.to_string(),
                    &font,
                    angle,
                    color,
                    theme.viewport_bg,
                );
            }
        }
    }

    pub(super) fn draw_overlays(
        &mut self,
        painter: &egui::Painter,
        rect: egui::Rect,
        preview: &Preview,
        hover: Option<glam::DVec2>,
        theme: &CadTheme,
    ) {
        let cam = &self.viewport.camera;
        // Hover highlight (selection tool).
        if let Some(id) = self.session.pointer.hovered {
            match self.session.project.doc.entity(id) {
                Some(Entity::Point(p)) => {
                    if let Some((s, _)) = cam.world_to_screen(p.position) {
                        painter.circle_stroke(to_screen(rect, s), 8.0, Stroke::new(2.0, theme.hover));
                    }
                }
                Some(Entity::Polyline(l)) => {
                    let stroke = Stroke::new(3.0, theme.hover.gamma_multiply(0.8));
                    for part in l.tessellate(cam.world_per_pixel() * 0.1) {
                        draw_world_polyline(painter, rect, cam, &part, stroke, false);
                    }
                }
                Some(Entity::Text(t)) => {
                    if let Some(corners) = text_corners(cam, rect, t) {
                        let stroke = Stroke::new(1.5, theme.hover);
                        painter.add(egui::Shape::closed_line(corners.to_vec(), stroke));
                    }
                }
                None => {}
            }
        }

        // Grips of the selection while the select tool is idle.
        if self.session.active_tool.id() == SELECT_TOOL_ID {
            let hovered = self.session.pointer.hovered_grip;
            let cam = cam.clone();
            for g in self.current_grips() {
                let Some((s, _)) = cam.world_to_screen(g.position) else {
                    continue;
                };
                let vertex = !matches!(g.kind, crate::tools::grips::GripKind::Midpoint(_));
                let half = if vertex { 4.0 } else { 3.0 };
                let r = egui::Rect::from_center_size(to_screen(rect, s), egui::Vec2::splat(half * 2.0));
                let fill = if hovered == Some(*g) {
                    theme.snap
                } else if vertex {
                    theme.accent
                } else {
                    theme.accent.gamma_multiply(0.55)
                };
                painter.rect_filled(r, 1.0, fill);
                painter.rect_stroke(r, 1.0, Stroke::new(1.0, theme.bg), egui::StrokeKind::Outside);
            }
            self.draw_vertex_numbers(painter, rect, &cam, theme);
        }
        let cam = &self.viewport.camera;

        // Tool preview.
        for p in &preview.points {
            if let Some((s, _)) = cam.world_to_screen(*p) {
                painter.circle_stroke(to_screen(rect, s), 3.5, Stroke::new(1.5, theme.preview));
            }
        }
        for pl in &preview.polylines {
            draw_world_polyline(painter, rect, cam, pl, Stroke::new(1.5, theme.preview), false);
        }
        for r in &preview.rubber {
            draw_world_polyline(painter, rect, cam, r, Stroke::new(1.2, theme.preview), true);
        }
        // With dynamic input on, measurements appear in the cursor HUD instead.
        let labels = if self.settings.dynamic_input {
            &[][..]
        } else {
            &preview.labels[..]
        };
        for (p, offset, text) in labels {
            if let Some((s, _)) = cam.world_to_screen(*p) {
                let pos = to_screen(rect, s) + egui::vec2(offset.x as f32, offset.y as f32);
                label_box(painter, rect, pos, text, theme);
            }
        }

        // Snap marker, or pick box for selection.
        if let Some(hit) = self.pick.and_then(|p| p.snap) {
            let pos = to_screen(rect, hit.screen);
            draw_snap_glyph(painter, pos, hit.mode, theme.snap);
            painter.text(
                pos + egui::vec2(10.0, -10.0),
                egui::Align2::LEFT_BOTTOM,
                hit.mode.label(),
                egui::FontId::proportional(11.0),
                theme.snap,
            );
        } else if let Some(c) = hover
            && !self.session.active_tool.wants_point()
        {
            draw_pickbox(
                painter,
                to_screen(rect, c),
                self.settings.snap.aperture_px as f32,
                theme.text_secondary,
            );
        }
    }

    /// Point labels (name / code / height), skipping any that would overlap.
    pub(super) fn draw_labels(&self, painter: &egui::Painter, rect: egui::Rect, theme: &CadTheme) {
        let cam = &self.viewport.camera;
        let doc = &self.session.project.doc;
        let conv = doc.conventions();
        let opts = &self.settings.labels;
        let frustum = cam.rect_frustum(glam::DVec2::ZERO, cam.viewport);
        let mut ids = Vec::new();
        doc.query(&|b| frustum.intersects_aabb(b), &mut ids);
        let mut placer = LabelPlacer::new(rect, 3000);
        let font = egui::FontId::proportional(opts.size);
        let small = egui::FontId::proportional((opts.size - 1.5).max(7.0));
        let library = &doc.meta().feature_codes;
        let global = LabelSet {
            name: opts.names,
            code: opts.codes,
            height: opts.heights,
            description: false,
        };
        let mut code_labels: HashMap<&str, LabelSet> = HashMap::new();
        for id in ids {
            if placer.is_full() {
                break;
            }
            let Some(Entity::Point(p)) = doc.entity(id) else {
                continue;
            };
            if !doc.is_layer_visible(p.layer) {
                continue;
            }
            let Some((s, _)) = cam.world_to_screen(p.position) else {
                continue;
            };
            let anchor = to_screen(rect, s);
            // Keep the marker clear; labels start beyond it (the placer pads by 2 px).
            placer.reserve(egui::Rect::from_center_size(anchor, egui::vec2(6.0, 6.0)));

            // A feature code's own label choice overrides the global settings.
            let set = *code_labels.entry(p.code.as_str()).or_insert_with(|| {
                library
                    .feature_for(&p.code)
                    .and_then(|f| f.style.labels)
                    .unwrap_or(global)
            });
            let mut job = egui::text::LayoutJob::default();
            let mut line = |text: &str, font: &egui::FontId, color: Color32| {
                if !job.text.is_empty() {
                    job.append("\n", 0.0, egui::TextFormat::simple(font.clone(), color));
                }
                job.append(text, 0.0, egui::TextFormat::simple(font.clone(), color));
            };
            if set.name && !p.name.is_empty() {
                line(&p.name, &font, theme.text);
            }
            if set.code && !p.code.is_empty() {
                line(&p.code, &small, theme.accent);
            }
            if set.height && p.height_known {
                line(&conv.format_length(p.position.z), &small, theme.text_secondary);
            }
            if set.description && !p.description.is_empty() {
                line(&p.description, &small, theme.text_secondary);
            }
            if job.text.is_empty() {
                continue;
            }
            let galley = painter.layout_job(job);
            let size = galley.size();
            if let Some(r) = placer.place(anchor, egui::vec2(LABEL_GAP, -size.y * 0.5), size) {
                painter.galley(r.min, galley, theme.text);
            }
        }
    }

    /// Heights on major contours, rotated along the line and kept upright.
    pub(super) fn draw_contour_labels(&self, painter: &egui::Painter, rect: egui::Rect, theme: &CadTheme) {
        let cam = &self.viewport.camera;
        let conv = self.session.project.doc.conventions();
        let font = egui::FontId::proportional(10.5);
        let mut placed: Vec<egui::Pos2> = Vec::new();
        for label in self.surface_cache.labels() {
            let Some((s, _)) = cam.world_to_screen(label.position) else {
                continue;
            };
            let Some((s2, _)) =
                cam.world_to_screen(label.position + (label.along.normalize_or_zero()).extend(0.0))
            else {
                continue;
            };
            let center = to_screen(rect, s);
            if !rect.shrink(20.0).contains(center) || placed.iter().any(|p| p.distance(center) < 70.0) {
                continue;
            }
            placed.push(center);
            let d = to_screen(rect, s2) - center;
            let mut angle = d.y.atan2(d.x);
            if angle > std::f32::consts::FRAC_PI_2 {
                angle -= std::f32::consts::PI;
            } else if angle < -std::f32::consts::FRAC_PI_2 {
                angle += std::f32::consts::PI;
            }
            let text = conv.format_length(label.z);
            let text = if text.contains('.') {
                text.trim_end_matches('0').trim_end_matches('.').to_string()
            } else {
                text
            };
            halo_text(painter, center, text, &font, angle, theme.text, theme.viewport_bg);
        }
    }

    /// Chainage ticks along alignments, labelled where there is room.
    /// 95% error ellipses of the last network adjustment, exaggerated.
    pub(super) fn draw_error_ellipses(&self, painter: &egui::Painter, rect: egui::Rect, theme: &CadTheme) {
        let Some(Ok((_, r))) = &self.network_window.result else {
            return;
        };
        let cam = &self.viewport.camera;
        let scale = self.network_window.exaggeration;
        let stroke = egui::Stroke::new(1.5, theme.error);
        for (i, e) in r.ellipses.iter().enumerate() {
            let Some(e) = e else { continue };
            let c = r.positions[i];
            let (s, co) = e.bearing.radians().sin_cos();
            let major = glam::DVec3::new(s, co, 0.0) * e.semi_major * scale;
            let minor = glam::DVec3::new(co, -s, 0.0) * e.semi_minor * scale;
            let pts: Vec<egui::Pos2> = (0..=48)
                .filter_map(|k| {
                    let t = k as f64 / 48.0 * std::f64::consts::TAU;
                    let p = c + major * t.cos() + minor * t.sin();
                    cam.world_to_screen(p).map(|(s, _)| to_screen(rect, s))
                })
                .collect();
            if pts.len() > 2 && pts.iter().any(|p| rect.contains(*p)) {
                painter.add(egui::Shape::line(pts, stroke));
            }
            if let Some((sp, _)) = cam.world_to_screen(c) {
                let sp = to_screen(rect, sp);
                if rect.contains(sp) {
                    painter.text(
                        sp + egui::vec2(6.0, 6.0),
                        egui::Align2::LEFT_TOP,
                        format!("{:.1} mm", e.semi_major * 1000.0),
                        egui::FontId::proportional(10.0),
                        theme.error,
                    );
                }
            }
        }
    }

    pub(super) fn draw_chainages(&self, painter: &egui::Painter, rect: egui::Rect, theme: &CadTheme) {
        let doc = &self.session.project.doc;
        let conv = doc.conventions();
        let cam = &self.viewport.camera;
        let font = egui::FontId::proportional(10.0);
        let screen = |p: glam::DVec3| cam.world_to_screen(p).map(|(s, _)| to_screen(rect, s));
        let stroke = egui::Stroke::new(1.2, theme.accent);
        for a in doc.alignments().iter().filter(|a| a.show_chainages) {
            let Some(l) = crate::ui::alignments::centreline(doc, a) else {
                continue;
            };
            if !doc.is_layer_visible(l.layer) {
                continue;
            }
            let (length, _, _) = survey_core::arc::polyline_metrics(&l.vertices, &l.bulges, l.closed);
            let step = a.tick_interval.max(0.01);
            // Pixels per tick decide how often to label.
            let px_per_m = 1.0 / cam.world_per_pixel().max(1e-9);
            let every = (70.0 / (step * px_per_m)).ceil().max(1.0) as i64;
            let first = (a.start_chainage / step).ceil() as i64;
            let last = ((a.start_chainage + length) / step).floor() as i64;
            if last - first > 5000 {
                continue;
            }
            let mut ticks: Vec<(f64, bool)> = (first..=last)
                .map(|k| (k as f64 * step, k % every == 0))
                .collect();
            ticks.push((a.start_chainage, true));
            ticks.push((a.start_chainage + length, true));
            for (ch, label) in ticks {
                let along = ch - a.start_chainage;
                let (Some(p), Some(q)) = (
                    survey_core::cogo::point_at_station(&l.vertices, &l.bulges, l.closed, along, 0.0),
                    survey_core::cogo::point_at_station(&l.vertices, &l.bulges, l.closed, along, 1.0),
                ) else {
                    continue;
                };
                let (Some(sp), Some(sq)) = (screen(p), screen(q)) else {
                    continue;
                };
                if !rect.contains(sp) {
                    continue;
                }
                // Towards the right of the alignment on screen.
                let right = (sq - sp).normalized();
                let len = if label { 9.0 } else { 5.0 };
                painter.line_segment([sp - right * len, sp + right * len], stroke);
                if label {
                    let mut angle = right.y.atan2(right.x);
                    if angle > std::f32::consts::FRAC_PI_2 {
                        angle -= std::f32::consts::PI;
                    } else if angle < -std::f32::consts::FRAC_PI_2 {
                        angle += std::f32::consts::PI;
                    }
                    let text = conv.format_chainage(ch);
                    let w = painter
                        .layout_no_wrap(text.clone(), font.clone(), theme.text)
                        .size()
                        .x;
                    halo_text(
                        painter,
                        sp + right * (len + 4.0 + w / 2.0),
                        text,
                        &font,
                        angle,
                        theme.accent,
                        theme.viewport_bg,
                    );
                }
            }
        }
    }

    /// The plan sheet being edited: its map frame (dashed) and title block.
    pub(super) fn draw_sheet_frame(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        sheet: &survey_model::SheetRecord,
        theme: &CadTheme,
    ) {
        let cam = &self.viewport.camera;
        let z = cam.target.z;
        let screen = |p: glam::DVec2| cam.world_to_screen(p.extend(z)).map(|(s, _)| to_screen(rect, s));
        let (frame, title) = crate::plan::sheet_outline(sheet);
        let ring = |corners: [glam::DVec2; 4]| -> Option<Vec<egui::Pos2>> {
            let mut pts: Vec<egui::Pos2> = corners.iter().map(|c| screen(*c)).collect::<Option<_>>()?;
            pts.push(pts[0]);
            Some(pts)
        };
        let stroke = Stroke::new(1.5, theme.accent);
        if let Some(pts) = ring(frame) {
            painter.extend(egui::Shape::dashed_line(&pts, stroke, 8.0, 5.0));
            painter.text(
                pts[3] + egui::vec2(6.0, 6.0),
                egui::Align2::LEFT_TOP,
                format!("{} · {}", sheet.name, sheet.scale_label()),
                egui::FontId::proportional(12.0),
                theme.accent,
            );
        }
        if let Some(pts) = ring(title) {
            painter.add(egui::Shape::line(
                pts,
                Stroke::new(1.0, theme.accent.gamma_multiply(0.6)),
            ));
        }
    }

    /// Cadastral annotation for parcels that ask for it: bearing above and
    /// distance below each line (radius and arc length on arcs), and the lot's
    /// name and area at its centre.
    pub(super) fn draw_parcel_annotations(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        theme: &CadTheme,
    ) {
        let doc = &self.session.project.doc;
        let conv = doc.conventions();
        let cam = &self.viewport.camera;
        let font = egui::FontId::proportional(10.5);
        let bold = egui::FontId::new(12.0, egui::FontFamily::Proportional);
        let screen = |p: glam::DVec3| cam.world_to_screen(p).map(|(s, _)| to_screen(rect, s));
        // A boundary shared by two lots is annotated once.
        let mut done: std::collections::HashSet<[(i64, i64); 2]> = std::collections::HashSet::new();
        let key = |p: glam::DVec3| ((p.x * 1000.0).round() as i64, (p.y * 1000.0).round() as i64);
        for parcel in doc.parcels().iter().filter(|p| p.annotate) {
            let Some(l) = crate::ui::parcels::boundary(doc, parcel) else {
                continue;
            };
            if !doc.is_layer_visible(l.layer) {
                continue;
            }
            for (a, b, bulge) in l.segments() {
                let (ka, kb) = (key(a), key(b));
                if !done.insert(if ka < kb { [ka, kb] } else { [kb, ka] }) {
                    continue;
                }
                let (mid, tangent, top, bottom) = match survey_core::arc::Arc::from_bulge(a, b, bulge) {
                    Some(arc) => {
                        let t = arc.tangent_at(0.5);
                        (
                            arc.midpoint(),
                            t.extend(0.0),
                            format!("R {}", conv.format_length(arc.radius)),
                            format!("A {}", conv.format_length(arc.length_2d())),
                        )
                    }
                    None => {
                        let d = (b - a).truncate();
                        let Some(brg) = survey_core::Bearing::from_delta(d.x, d.y) else {
                            continue;
                        };
                        (
                            (a + b) * 0.5,
                            (b - a),
                            conv.format_bearing(brg),
                            conv.format_length(d.length()),
                        )
                    }
                };
                let (Some(m), Some(m2), Some(sa), Some(sb)) = (
                    screen(mid),
                    screen(mid + tangent.normalize_or_zero()),
                    screen(a),
                    screen(b),
                ) else {
                    continue;
                };
                if !rect.contains(m) {
                    continue;
                }
                // Only where the line is long enough on screen for the text.
                let width = painter
                    .layout_no_wrap(top.clone(), font.clone(), theme.text)
                    .size()
                    .x
                    .max(
                        painter
                            .layout_no_wrap(bottom.clone(), font.clone(), theme.text)
                            .size()
                            .x,
                    );
                if sa.distance(sb) < width + 16.0 {
                    continue;
                }
                let d = m2 - m;
                let mut angle = d.y.atan2(d.x);
                if angle > std::f32::consts::FRAC_PI_2 {
                    angle -= std::f32::consts::PI;
                } else if angle < -std::f32::consts::FRAC_PI_2 {
                    angle += std::f32::consts::PI;
                }
                // "Up" relative to the upright text.
                let up = egui::vec2(angle.sin(), -angle.cos());
                halo_text(
                    painter,
                    m + up * 8.0,
                    top,
                    &font,
                    angle,
                    theme.text,
                    theme.viewport_bg,
                );
                halo_text(
                    painter,
                    m - up * 8.0,
                    bottom,
                    &font,
                    angle,
                    theme.text_secondary,
                    theme.viewport_bg,
                );
            }
            // Name and area at the area-weighted centre.
            let ring: Vec<glam::DVec2> = l
                .tessellate(0.05)
                .into_iter()
                .flatten()
                .map(|p| p.truncate())
                .collect();
            let (mut area, mut c) = (0.0, glam::DVec2::ZERO);
            for i in 0..ring.len() {
                let (p, q) = (ring[i], ring[(i + 1) % ring.len()]);
                let cross = p.perp_dot(q);
                area += cross;
                c += (p + q) * cross;
            }
            if area.abs() < 1e-9 {
                continue;
            }
            let centre = (c / (3.0 * area)).extend(l.vertices[0].z);
            if let Some(p) = screen(centre).filter(|p| rect.contains(*p)) {
                halo_text(
                    painter,
                    p - egui::vec2(0.0, 7.0),
                    parcel.name.clone(),
                    &bold,
                    0.0,
                    theme.accent,
                    theme.viewport_bg,
                );
                halo_text(
                    painter,
                    p + egui::vec2(0.0, 8.0),
                    conv.format_area((area / 2.0).abs()),
                    &font,
                    0.0,
                    theme.text,
                    theme.viewport_bg,
                );
            }
        }
    }
}

/// Text centred at `center`, rotated by `angle`, with a one-pixel halo in `halo`
/// so it stays readable over lines and shading.
fn halo_text(
    painter: &egui::Painter,
    center: egui::Pos2,
    text: String,
    font: &egui::FontId,
    angle: f32,
    color: Color32,
    halo: Color32,
) {
    let galley = painter.layout_no_wrap(text, font.clone(), color);
    let rot = egui::emath::Rot2::from_angle(angle);
    let origin = center - rot * (galley.size() * 0.5);
    for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        painter.add(
            egui::epaint::TextShape::new(origin + egui::vec2(dx, dy), galley.clone(), halo)
                .with_override_text_color(halo)
                .with_angle(angle),
        );
    }
    painter.add(egui::epaint::TextShape::new(origin, galley, color).with_angle(angle));
}

/// A text note's box on screen.
fn text_corners(
    cam: &cad_render::Camera,
    rect: egui::Rect,
    t: &survey_model::TextNote,
) -> Option<[egui::Pos2; 4]> {
    let place = crate::scene_build::text_placement(cam, t)?;
    Some(place.corners.map(|c| to_screen(rect, c)))
}

/// Draws text in a small box near `pos`, kept inside `bounds`.
/// Vertex numbers beside a selected polyline's grips (its point sequence)
/// are left out when a selection has more than this many vertices.
const MAX_NUMBERED_VERTICES: usize = 500;

impl SurveyApp {
    /// Numbers each vertex of the selected polylines in drawing order, the
    /// first one marked as the start; labels that would overlap are skipped.
    fn draw_vertex_numbers(
        &mut self,
        painter: &egui::Painter,
        rect: egui::Rect,
        cam: &cad_render::Camera,
        theme: &CadTheme,
    ) {
        use crate::tools::grips::GripKind;
        let vertices: Vec<(glam::DVec3, usize)> = self
            .current_grips()
            .iter()
            .filter_map(|g| match g.kind {
                GripKind::Vertex(i) => Some((g.position, i)),
                _ => None,
            })
            .collect();
        if vertices.len() > MAX_NUMBERED_VERTICES {
            return;
        }
        let font = egui::FontId::monospace(10.5);
        let mut placed: Vec<egui::Pos2> = Vec::new();
        for (p, i) in vertices {
            let Some((s, _)) = cam.world_to_screen(p) else {
                continue;
            };
            let at = to_screen(rect, s) + egui::vec2(7.0, -7.0);
            if !rect.contains(at) || placed.iter().any(|q| q.distance(at) < 14.0) {
                continue;
            }
            placed.push(at);
            let text = if i == 0 {
                "1 start".to_string()
            } else {
                (i + 1).to_string()
            };
            let galley = painter.layout_no_wrap(text, font.clone(), theme.text);
            let r =
                egui::Rect::from_min_size(at - egui::vec2(0.0, galley.size().y), galley.size()).expand(2.0);
            painter.rect_filled(r, 2.0, theme.panel.gamma_multiply(0.85));
            let color = if i == 0 {
                theme.accent
            } else {
                theme.text_secondary
            };
            painter.galley_with_override_text_color(r.min + egui::vec2(2.0, 2.0), galley, color);
        }
    }
}

fn label_box(painter: &egui::Painter, bounds: egui::Rect, pos: egui::Pos2, text: &str, theme: &CadTheme) {
    let galley = painter.layout_no_wrap(text.to_string(), egui::FontId::monospace(11.5), theme.text);
    let size = galley.size();
    let margin = 6.0;
    let pos = egui::pos2(
        pos.x
            .min(bounds.right() - size.x - margin)
            .max(bounds.left() + margin),
        pos.y
            .min(bounds.bottom() - size.y - margin)
            .max(bounds.top() + margin),
    );
    let rect = egui::Rect::from_min_size(pos, size).expand(4.0);
    painter.rect_filled(rect, 3.0, theme.panel.gamma_multiply(0.92));
    painter.rect_stroke(
        rect,
        3.0,
        Stroke::new(1.0, theme.border),
        egui::StrokeKind::Inside,
    );
    painter.galley(pos, galley, Color32::WHITE);
}
