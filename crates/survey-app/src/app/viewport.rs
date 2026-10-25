//! The viewport: navigation, picking and snapping, tool input and the cursor HUD.

use egui_cad::chrome::empty_state;
use egui_cad::hud::{HudLine, draw_cursor_hud};
use egui_cad::selection::SelectionBox;
use egui_cad::viewport::NavAction;

use crate::scene_build::select_in_box;

use super::*;

/// Grips within this many pixels of the cursor can be picked.
const GRIP_APERTURE: f64 = 7.0;

impl SurveyApp {
    // ------------------------------------------------------------------ viewport

    pub(super) fn viewport_ui(&mut self, ui: &mut egui::Ui) {
        #[cfg(not(target_arch = "wasm32"))]
        let cloud_frame = self.stream_clouds(ui.ctx());
        #[cfg(target_arch = "wasm32")]
        let cloud_frame = None;
        // Out-of-core clouds stay inside the clip planes even where nothing is loaded yet.
        let cloud_bounds = self
            .session
            .project
            .doc
            .clouds()
            .filter(|(_, c)| c.style.visible && c.is_out_of_core())
            .fold(Aabb3::EMPTY, |b, (_, c)| b.union(&c.full_bounds()));
        let theme = self.theme.clone();
        let doc = &self.session.project.doc;
        let scene = self.scene_cache.get(doc, &theme);
        let selection = self
            .selection_cache
            .get(doc, &self.session.project.selection, &theme);
        let surfaces = self.surface_cache.get(&self.session.project.doc, &theme);
        let clouds = self.cloud_cache.get(&self.session.project.doc, &theme, ui.ctx());
        let conv = self.session.project.doc.conventions().clone();
        let grid = self.settings.show_grid.then(|| GridSettings {
            elevation: self.settings.working_z,
            ..Default::default()
        });

        // Keep the tool preview inside the clip range.
        let mut preview = Preview::default();
        self.session
            .active_tool
            .preview(&conv, self.pick.as_ref(), &mut preview);
        let mut extra = Aabb3::EMPTY;
        for p in preview
            .polylines
            .iter()
            .flatten()
            .chain(preview.rubber.iter().flatten())
        {
            extra.extend(*p);
        }

        let anchor = self.pick.and_then(|p| p.snap).map(|s| s.point);
        let labels = [
            conv.axis_labels.easting.as_str(),
            conv.axis_labels.northing.as_str(),
            conv.axis_labels.height.as_str(),
        ];
        let out = Viewport::new(&mut self.viewport, scene)
            .overlay_scene(surfaces)
            .overlay_scene(clouds)
            .overlay_scene(selection)
            .clouds(cloud_frame, cloud_bounds)
            .grid(grid)
            .anchor(anchor)
            .extra_bounds(extra)
            .axis_labels(labels)
            .length_unit(conv.linear_unit.metres_per_unit(), conv.linear_unit.suffix())
            .nav_controls(false)
            .show(ui);

        self.pick = out.hover.map(|c| self.compute_pick(c, grid.as_ref()));
        self.session.pointer.hovered = match out.hover {
            Some(c) if !self.session.active_tool.wants_point() => hit_test(
                &self.session.project.doc,
                &self.viewport.camera,
                c,
                self.settings.snap.aperture_px,
            ),
            _ => None,
        };
        let idle_select =
            self.session.active_tool.id() == SELECT_TOOL_ID && !self.session.active_tool.wants_point();
        self.session.pointer.hovered_grip = match out.hover {
            Some(c) if idle_select => self.grip_at(c),
            _ => None,
        };

        // Recompute the preview against this frame's cursor.
        let mut preview = Preview::default();
        self.session
            .active_tool
            .preview(&conv, self.pick.as_ref(), &mut preview);
        let painter = ui.painter_at(out.rect);
        let code_labels = self
            .session
            .project
            .doc
            .meta()
            .feature_codes
            .codes
            .iter()
            .any(|c| c.style.labels.is_some());
        self.draw_texts(&painter, out.rect, &theme);
        if self.settings.labels.any() || code_labels {
            self.draw_labels(&painter, out.rect, &theme);
        }
        if self
            .session
            .project
            .doc
            .alignments()
            .iter()
            .any(|a| a.show_chainages)
        {
            self.draw_chainages(&painter, out.rect, &theme);
        }
        if self.network_window.open && self.network_window.show_ellipses {
            self.draw_error_ellipses(&painter, out.rect, &theme);
        }
        if self.session.project.doc.parcels().iter().any(|p| p.annotate) {
            self.draw_parcel_annotations(&painter, out.rect, &theme);
        }
        if !self.surface_cache.labels().is_empty() {
            self.draw_contour_labels(&painter, out.rect, &theme);
        }
        if let Some(sheet) = self.sheets_window.active(&self.session.project.doc) {
            self.draw_sheet_frame(&painter, out.rect, &sheet, &theme);
        }
        self.draw_overlays(&painter, out.rect, &preview, out.hover, &theme);
        if self.settings.dynamic_input
            && let Some(c) = out.hover
        {
            self.draw_hud(&painter, out.rect, to_screen(out.rect, c), &preview, &conv);
        }
        let doc_empty = {
            let d = &self.session.project.doc;
            d.entity_count() == 0 && d.surfaces().next().is_none() && d.clouds().next().is_none()
        };
        if doc_empty && out.drag.is_none() {
            let search = ui
                .ctx()
                .format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::K));
            empty_state(
                &painter,
                out.rect,
                icons::MAP_TRIFOLD,
                "Start a survey",
                &[
                    (search.as_str(), "Search every command"),
                    ("point", "Place points — pick or type E,N[,Z]"),
                    ("import", "Import points from CSV / TXT"),
                    ("dxfin", "Import a DXF drawing"),
                ],
            );
        }
        self.toasts.show(&painter, out.rect);
        match out.nav {
            // The viewport has already applied gizmo presets.
            Some(NavAction::Preset(p)) => self.session.log.info(p.label()),
            Some(nav) => self.apply_nav(nav),
            None => {}
        }

        // Window / crossing selection with the select tool.
        if !self.session.active_tool.wants_point() {
            if let Some((a, b)) = out.drag {
                SelectionBox::from_drag(a, b).draw(&painter, out.rect, &theme);
            }
            if let Some((a, b)) = out.drag_released
                && a.distance(b) > 3.0
            {
                let ids = select_in_box(
                    &self.session.project.doc,
                    &self.viewport.camera,
                    &SelectionBox::from_drag(a, b),
                );
                self.session.dispatch(ToolEvent::Region(ids));
            }
        }

        if let Some(c) = out.clicked {
            let pick = self.compute_pick(c, grid.as_ref());
            self.session.dispatch(ToolEvent::Point(pick));
        }
        if out.secondary_clicked {
            self.session.dispatch(ToolEvent::Finish);
        }
    }

    /// Grips of the current selection (cached).
    pub(super) fn current_grips(&mut self) -> &[Grip] {
        let key = (
            self.session.project.doc.revision(),
            self.session.project.selection.revision(),
        );
        if self.grips.0 != Some(key) {
            self.grips = (
                Some(key),
                crate::tools::grips::grips(&self.session.project.doc, &self.session.project.selection),
            );
        }
        &self.grips.1
    }

    /// The grip nearest the cursor within [`GRIP_APERTURE`].
    fn grip_at(&mut self, cursor: glam::DVec2) -> Option<Grip> {
        let cam = self.viewport.camera.clone();
        self.current_grips()
            .iter()
            .filter_map(|g| {
                let (s, _) = cam.world_to_screen(g.position)?;
                let d = s.distance(cursor);
                (d <= GRIP_APERTURE).then_some((d, *g))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, g)| g)
    }

    fn compute_pick(&mut self, cursor: glam::DVec2, grid: Option<&GridSettings>) -> Pick {
        let cam = &self.viewport.camera;
        let base = self.session.active_tool.base_point();
        // Grid snap uses a fixed spacing, or the spacing the grid has (or would have) at this zoom.
        let grid_spacing = self
            .settings
            .grid_snap_spacing
            .filter(|s| *s > 0.0)
            .unwrap_or_else(|| grid.copied().unwrap_or_default().spacing(cam).0);
        let query = SnapQuery {
            camera: cam,
            cursor,
            base_point: base.map(|b| b.position),
            working_z: self.settings.working_z,
            grid_spacing,
        };
        if !self.session.active_tool.wants_point() {
            return free_point(&query);
        }
        // Grid snap is its own toggle (F9), independent of object snap (F3): object
        // snaps win, then the grid.
        let settings = match self.session.snap_override {
            Some(mode) => SnapSettings::only(mode, self.settings.snap.aperture_px),
            None => {
                let mut s = self.settings.snap.clone();
                s.set(SnapMode::Grid, false);
                s
            }
        };
        let provider = DocSnapProvider {
            doc: &self.session.project.doc,
        };
        let mut pick = self.snap_engine.pick(&settings, &query, &provider);
        if self.settings.ortho.enabled
            && pick.snap.is_none()
            && let Some(b) = base
        {
            let reference = if cam.is_plan() { cam.yaw } else { 0.0 };
            pick.point = constrain_direction(
                b.position,
                pick.point,
                self.settings.ortho.increment_deg.to_radians(),
                reference,
            );
        }
        if self.session.snap_override.is_none()
            && self.settings.snap.is_on(SnapMode::Grid)
            && pick.snap.is_none()
        {
            let s = grid_spacing;
            let point = glam::DVec3::new(
                (pick.point.x / s).round() * s,
                (pick.point.y / s).round() * s,
                pick.point.z,
            );
            if let Some((screen, _)) = cam.world_to_screen(point) {
                pick = Pick {
                    point,
                    height_known: pick.height_known,
                    snap: Some(SnapHit {
                        point,
                        mode: SnapMode::Grid,
                        screen,
                        tag: None,
                        height_known: pick.height_known,
                    }),
                };
            }
        }
        // Drape free and grid picks on a surface.
        let draped = pick.snap.is_none_or(|h| h.mode == SnapMode::Grid);
        if self.settings.surface_heights && draped && self.session.project.doc.surfaces().next().is_some() {
            let doc = &self.session.project.doc;
            let preferred = self.session.active_surface;
            if cam.is_plan() {
                if let Some(z) = self
                    .surface_indexes
                    .height_at(doc, preferred, pick.point.truncate())
                {
                    pick.point.z = z;
                    pick.height_known = true;
                }
            } else if pick.snap.is_none() {
                // Along the view ray: alternate between the surface height and the ray.
                let ray = cam.screen_ray(cursor);
                let mut p = pick.point;
                let mut hit = None;
                for _ in 0..8 {
                    let Some(z) = self.surface_indexes.height_at(doc, preferred, p.truncate()) else {
                        break;
                    };
                    match ray.intersect_z_plane(z) {
                        Some(q) => {
                            hit = Some(q.truncate().extend(z));
                            if q.truncate().distance(p.truncate()) < 1e-4 {
                                break;
                            }
                            p = q;
                        }
                        None => break,
                    }
                }
                if let Some(h) = hit
                    && let Some(z) = self.surface_indexes.height_at(doc, preferred, h.truncate())
                {
                    pick.point = h.truncate().extend(z);
                    pick.height_known = true;
                }
            }
        }
        pick
    }

    /// Dynamic input: prompt, live measurement and typed text beside the cursor.
    fn draw_hud(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        cursor: egui::Pos2,
        preview: &Preview,
        conv: &survey_core::Conventions,
    ) {
        if self.session.active_tool.id() == SELECT_TOOL_ID && !self.session.active_tool.wants_point() {
            return;
        }
        let mut lines = vec![HudLine::Prompt(self.session.active_tool.prompt(conv))];
        if let Some((_, _, text)) = preview.labels.first() {
            lines.push(HudLine::Value(text.clone()));
        } else if let Some(p) = self.pick {
            lines.push(HudLine::Value(conv.format_coord(p.point, p.height_known)));
        }
        if !self.session.log.input.is_empty() {
            lines.push(HudLine::Input(self.session.log.input.clone()));
        }
        draw_cursor_hud(painter, rect, cursor, &lines);
    }

    pub(super) fn apply_nav(&mut self, nav: NavAction) {
        match nav {
            NavAction::ZoomExtents => self.zoom_extents(),
            NavAction::TogglePlan => self.toggle_plan(),
            NavAction::ToggleProjection => self.toggle_projection(),
            NavAction::Preset(p) => {
                self.viewport.apply_preset(p);
                self.session.log.info(p.label());
            }
        }
    }
}
