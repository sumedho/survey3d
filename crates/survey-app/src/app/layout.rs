//! Panels, viewport overlays and dialogs of [`SurveyApp`].

use std::collections::HashMap;

use egui::{Color32, RichText, Stroke};
use egui_cad::chrome::empty_state;
use egui_cad::chrome::{IconButton, palette_divider, search_button, toolbar_divider};
use egui_cad::hud::{HudLine, draw_cursor_hud};
use egui_cad::labels::LabelPlacer;
use egui_cad::selection::SelectionBox;
use egui_cad::viewport::NavAction;

use crate::platform::FileKind;
use crate::scene_build::select_in_box;
use egui_cad::ThemeMode;

use super::*;

/// Menu and toolbar actions, applied after the UI closure to keep borrows simple.
#[derive(Clone, Copy)]
enum Action {
    New,
    Open,
    Save,
    #[cfg(not(target_arch = "wasm32"))]
    SaveAs,
    Import,
    ImportDxf,
    ExportDxf,
    Labels,
    Palette,
    #[cfg(not(target_arch = "wasm32"))]
    Quit,
    Undo,
    Redo,
    Delete,
    SelectNone,
    ZoomExtents,
    TogglePlan,
    ToggleProjection,
    ToggleGrid,
    Theme(ThemeMode),
    Tool(&'static str),
    Conventions,
    DraftingSettings,
    Settings,
    Audit,
    Help,
    About,
}

/// Horizontal gap between a point marker and its label (px).
const LABEL_GAP: f32 = 7.0;

/// Grips within this many pixels of the cursor can be picked.
const GRIP_APERTURE: f64 = 7.0;

/// Palette for new layers.
const LAYER_COLORS: [Color; 8] = [
    Color::rgb(0x7d, 0xd3, 0xfc),
    Color::rgb(0xfb, 0xbf, 0x24),
    Color::rgb(0x4a, 0xde, 0x80),
    Color::rgb(0xf4, 0x72, 0xb6),
    Color::rgb(0xa7, 0x8b, 0xfa),
    Color::rgb(0xf8, 0x71, 0x71),
    Color::rgb(0x2d, 0xd4, 0xbf),
    Color::rgb(0xfd, 0xba, 0x74),
];

impl SurveyApp {
    // ------------------------------------------------------------------ menu & toolbar

    pub(super) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        let ctx = ui.ctx().clone();
        let shortcut = |m: Modifiers, k: Key| ctx.format_shortcut(&KeyboardShortcut::new(m, k));
        let item = |ui: &mut egui::Ui,
                    label: &str,
                    shortcut: Option<String>,
                    a: Action,
                    action: &mut Option<Action>| {
            let mut b = egui::Button::new(label);
            if let Some(s) = shortcut {
                b = b.shortcut_text(s);
            }
            if ui.add(b).clicked() {
                *action = Some(a);
            }
        };
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                item(
                    ui,
                    "New project",
                    Some(shortcut(Modifiers::COMMAND, Key::N)),
                    Action::New,
                    &mut action,
                );
                item(
                    ui,
                    "Open…",
                    Some(shortcut(Modifiers::COMMAND, Key::O)),
                    Action::Open,
                    &mut action,
                );
                item(
                    ui,
                    "Save",
                    Some(shortcut(Modifiers::COMMAND, Key::S)),
                    Action::Save,
                    &mut action,
                );
                #[cfg(not(target_arch = "wasm32"))]
                item(
                    ui,
                    "Save as…",
                    Some(shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::S)),
                    Action::SaveAs,
                    &mut action,
                );
                ui.separator();
                item(ui, "Import points (CSV/TXT)…", None, Action::Import, &mut action);
                item(ui, "Import DXF…", None, Action::ImportDxf, &mut action);
                item(ui, "Export DXF…", None, Action::ExportDxf, &mut action);
                #[cfg(not(target_arch = "wasm32"))]
                {
                    ui.separator();
                    item(ui, "Quit", None, Action::Quit, &mut action);
                }
            });
            ui.menu_button("Edit", |ui| {
                let undo = self
                    .project
                    .history
                    .undo_label()
                    .map_or("Undo".into(), |l| format!("Undo {l}"));
                let redo = self
                    .project
                    .history
                    .redo_label()
                    .map_or("Redo".into(), |l| format!("Redo {l}"));
                item(
                    ui,
                    &undo,
                    Some(shortcut(Modifiers::COMMAND, Key::Z)),
                    Action::Undo,
                    &mut action,
                );
                item(
                    ui,
                    &redo,
                    Some(shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)),
                    Action::Redo,
                    &mut action,
                );
                ui.separator();
                item(ui, "Delete", Some("Del".into()), Action::Delete, &mut action);
                item(
                    ui,
                    "Select none",
                    Some("Esc".into()),
                    Action::SelectNone,
                    &mut action,
                );
            });
            ui.menu_button("View", |ui| {
                item(
                    ui,
                    "Zoom extents",
                    Some("Home".into()),
                    Action::ZoomExtents,
                    &mut action,
                );
                let plan = if self.viewport.plan_locked {
                    "Switch to 3D"
                } else {
                    "Lock to plan (2D)"
                };
                item(ui, plan, None, Action::TogglePlan, &mut action);
                item(
                    ui,
                    "Toggle perspective",
                    None,
                    Action::ToggleProjection,
                    &mut action,
                );
                item(
                    ui,
                    "Toggle grid",
                    Some("F7".into()),
                    Action::ToggleGrid,
                    &mut action,
                );
                item(
                    ui,
                    "Snap & drafting…",
                    None,
                    Action::DraftingSettings,
                    &mut action,
                );
                item(ui, "Labels…", None, Action::Labels, &mut action);
                ui.separator();
                item(
                    ui,
                    "Dark theme",
                    None,
                    Action::Theme(ThemeMode::Dark),
                    &mut action,
                );
                item(
                    ui,
                    "Light theme",
                    None,
                    Action::Theme(ThemeMode::Light),
                    &mut action,
                );
            });
            ui.menu_button("Tools", |ui| {
                for t in self.tools.tools() {
                    item(
                        ui,
                        t.name,
                        Some(t.commands[0].into()),
                        Action::Tool(t.id),
                        &mut action,
                    );
                }
            });
            ui.menu_button("Project", |ui| {
                item(ui, "Conventions…", None, Action::Conventions, &mut action);
                item(ui, "Settings…", None, Action::Settings, &mut action);
                item(ui, "Audit trail…", None, Action::Audit, &mut action);
            });
            ui.menu_button("Help", |ui| {
                item(ui, "Commands", None, Action::Help, &mut action);
                item(ui, "About survey3d", None, Action::About, &mut action);
            });

            toolbar_divider(ui);
            let sc = |m: Modifiers, k: Key| Some(ctx.format_shortcut(&KeyboardShortcut::new(m, k)));
            let buttons: [(&str, &str, Option<String>, Action); 4] = [
                (
                    icons::FILE_PLUS,
                    "New project",
                    sc(Modifiers::COMMAND, Key::N),
                    Action::New,
                ),
                (
                    icons::FOLDER_OPEN,
                    "Open project",
                    sc(Modifiers::COMMAND, Key::O),
                    Action::Open,
                ),
                (
                    icons::FLOPPY_DISK,
                    "Save project",
                    sc(Modifiers::COMMAND, Key::S),
                    Action::Save,
                ),
                (icons::FILE_CSV, "Import points", None, Action::Import),
            ];
            for (icon, title, shortcut, a) in buttons {
                if IconButton::new(icon, title)
                    .shortcut(shortcut.as_deref())
                    .show(ui)
                    .clicked()
                {
                    action = Some(a);
                }
            }
            toolbar_divider(ui);
            let undo = self.project.history.undo_label();
            let redo = self.project.history.redo_label();
            let undo_sc = sc(Modifiers::COMMAND, Key::Z);
            let redo_sc = sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
            if IconButton::new(icons::ARROW_U_UP_LEFT, "Undo")
                .description(undo.as_deref().unwrap_or("Nothing to undo"))
                .shortcut(undo_sc.as_deref())
                .enabled(undo.is_some())
                .show(ui)
                .clicked()
            {
                action = Some(Action::Undo);
            }
            if IconButton::new(icons::ARROW_U_UP_RIGHT, "Redo")
                .description(redo.as_deref().unwrap_or("Nothing to redo"))
                .shortcut(redo_sc.as_deref())
                .enabled(redo.is_some())
                .show(ui)
                .clicked()
            {
                action = Some(Action::Redo);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if IconButton::new(icons::GEAR, "Settings")
                    .description("Drafting, display, conventions and coordinates")
                    .show(ui)
                    .clicked()
                {
                    action = Some(Action::Settings);
                }
                // Search box look-alike that opens the command palette.
                let palette_sc = ctx.format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::K));
                let search = search_button(ui, "Search commands", &palette_sc, 240.0);
                if search.clicked() {
                    action = Some(Action::Palette);
                }
            });
        });

        let ctx = ui.ctx().clone();
        if let Some(a) = action {
            self.apply_action(&ctx, a);
        }
    }

    fn apply_action(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::New => self.request_discard(ctx, Discard::New),
            Action::Open => self.request_discard(ctx, Discard::Open),
            Action::Save => self.save(ctx),
            #[cfg(not(target_arch = "wasm32"))]
            Action::SaveAs => self.save_as(),
            Action::Import => self.platform.pick_file(ctx, FileKind::Points),
            Action::ImportDxf => self.platform.pick_file(ctx, FileKind::Dxf),
            Action::ExportDxf => self.export_dxf(),
            Action::Labels => self.settings_window.open_at_display(),
            Action::Palette => self.palette.open(),
            #[cfg(not(target_arch = "wasm32"))]
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::Delete => self.delete_selection(),
            Action::SelectNone => self.project.selection.clear(),
            Action::ZoomExtents => self.zoom_extents(),
            Action::TogglePlan => self.toggle_plan(),
            Action::ToggleProjection => self.toggle_projection(),
            Action::ToggleGrid => self.settings.show_grid = !self.settings.show_grid,
            Action::Theme(mode) => self.settings.theme = mode,
            Action::Tool(id) => self.set_tool(id),
            Action::Conventions => self.settings_window.open_at_conventions(),
            Action::DraftingSettings => self.settings_window.open_at_drafting(),
            Action::Settings => self.settings_window.open = true,
            Action::Audit => self.audit_window.open = true,
            Action::Help => {
                self.run_app_command(ctx, "help");
            }
            Action::About => self.about_open = true,
        }
    }

    // ------------------------------------------------------------------ status bar

    /// Read-only feedback only; all settings live in the Settings panel.
    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme.clone();
        let conv = self.project.doc.conventions();
        ui.horizontal(|ui| {
            let coord = self
                .pick
                .map_or_else(|| "—".to_string(), |p| conv.format_coord(p.point, p.height_known));
            ui.add_sized(
                [340.0, 18.0],
                egui::Label::new(RichText::new(coord).monospace().color(theme.text)).truncate(),
            );
            if let Some(hit) = self.pick.and_then(|p| p.snap) {
                ui.label(RichText::new(hit.mode.label()).small().color(theme.snap));
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.project.is_dirty() {
                    ui.label(RichText::new("● unsaved").small().color(theme.warning));
                }
                let sel = self.project.selection.len();
                if sel > 0 {
                    ui.label(
                        RichText::new(format!("{sel} selected"))
                            .small()
                            .color(theme.selection),
                    );
                }
                // Only states that change how picks behave, and only when non-default.
                for (active, text, tip) in [
                    (!self.settings.snap.enabled, "snap off", "Object snap is off (F3)"),
                    (
                        self.settings.ortho.enabled,
                        "ortho",
                        "Ortho / polar constraint is on (F8)",
                    ),
                ] {
                    if active {
                        ui.label(RichText::new(text).small().color(theme.accent))
                            .on_hover_text(tip);
                    }
                }
            });
        });
    }

    // ------------------------------------------------------------------ command line

    pub(super) fn command_line_ui(&mut self, ui: &mut egui::Ui) {
        let mut prompt = self.active_tool.prompt(self.project.doc.conventions());
        if let Some(mode) = self.snap_override {
            prompt = format!("[{}] {prompt}", mode.code().to_uppercase());
        }
        let event = CommandLine::new(&mut self.command_line, &prompt)
            .completions(&self.completions)
            .log_rows(4)
            .show(ui);
        let ctx = ui.ctx().clone();
        match event {
            Some(CommandLineEvent::Submit(text)) => self.submit(&ctx, text),
            Some(CommandLineEvent::Cancel) => {
                self.snap_override = None;
                self.dispatch(ToolEvent::Cancel);
            }
            None => {}
        }
    }

    // ------------------------------------------------------------------ tool palette

    pub(super) fn tool_palette(&mut self, ui: &mut egui::Ui) {
        let mut tools: Vec<_> = self
            .tools
            .tools()
            .iter()
            .filter(|t| t.palette)
            .map(|t| (t.group, t.id, t.icon, t.name, t.description, t.commands[0]))
            .collect();
        tools.sort_by_key(|t| t.0);
        let active = self.active_tool.id();
        let mut chosen = None;
        ui.vertical_centered(|ui| {
            ui.add_space(2.0);
            let mut group = None;
            for (g, id, icon, name, description, command) in tools {
                if group.is_some_and(|prev| prev != g) {
                    palette_divider(ui);
                }
                group = Some(g);
                if IconButton::new(icon, name)
                    .description(description)
                    .shortcut(Some(command))
                    .selected(active == id)
                    .size(32.0)
                    .show(ui)
                    .clicked()
                {
                    chosen = Some(id);
                }
            }
        });
        if let Some(id) = chosen {
            self.set_tool(id);
        }
    }

    // ------------------------------------------------------------------ side panel

    pub(super) fn side_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let rows = self.layer_rows();
            let mut add = false;
            let actions = section(
                ui,
                "Layers",
                true,
                |ui| {
                    add = IconButton::new(icons::PLUS, "New layer")
                        .description("Adds a layer and makes it current")
                        .size(20.0)
                        .show(ui)
                        .clicked();
                },
                |ui| layer_panel(ui, &rows),
            )
            .unwrap_or_default();
            if add {
                self.apply_layer_action(LayerAction::Add);
            }
            for a in actions {
                self.apply_layer_action(a);
            }
            let p = &self.project;
            let edit = section(
                ui,
                "Properties",
                true,
                |_| {},
                |ui| properties_panel(ui, &p.doc, &p.selection),
            );
            if let Some(cmd) = edit.flatten() {
                self.execute(cmd);
            }
        });
    }

    fn layer_rows(&mut self) -> Vec<LayerRow> {
        let doc = &self.project.doc;
        if self.layer_counts.0 != doc.revision() {
            let mut counts: HashMap<LayerId, usize> = HashMap::new();
            for (_, e) in doc.entities() {
                *counts.entry(e.layer()).or_default() += 1;
            }
            self.layer_counts = (doc.revision(), counts);
        }
        doc.layers()
            .iter()
            .map(|l| LayerRow {
                id: l.id.0,
                name: l.name.clone(),
                color: color32(l.color),
                visible: l.visible,
                locked: l.locked,
                current: l.id == doc.current_layer(),
                count: self.layer_counts.1.get(&l.id).copied().unwrap_or(0),
            })
            .collect()
    }

    fn apply_layer_action(&mut self, action: LayerAction) {
        let doc = &mut self.project.doc;
        let layer = |doc: &survey_model::Document, id: i64| doc.layer(LayerId(id)).cloned();
        match action {
            LayerAction::SetCurrent(id) => doc.set_current_layer(LayerId(id)),
            // Visibility and locking are display state: saved, but not undoable or audited.
            LayerAction::SetVisible(id, visible) => {
                if let Some(mut l) = layer(doc, id) {
                    l.visible = visible;
                    doc.set_layer(l.id, Some(l));
                    let doc = &self.project.doc;
                    self.project.selection.retain(|e| doc.is_entity_editable(e));
                }
            }
            LayerAction::SetLocked(id, locked) => {
                if let Some(mut l) = layer(doc, id) {
                    l.locked = locked;
                    doc.set_layer(l.id, Some(l));
                    let doc = &self.project.doc;
                    self.project.selection.retain(|e| doc.is_entity_editable(e));
                }
            }
            LayerAction::SetColor(id, c) => {
                if let Some(mut l) = layer(doc, id) {
                    l.color = Color(c.to_srgba_unmultiplied());
                    let label = format!("Change colour of layer {}", l.name);
                    self.execute(Box::new(ChangeSet::new(label).update_layer(l)));
                }
            }
            LayerAction::Rename(id, name) => {
                if let Some(mut l) = layer(doc, id) {
                    let label = format!("Rename layer {} to {name}", l.name);
                    l.name = name;
                    self.execute(Box::new(ChangeSet::new(label).update_layer(l)));
                }
            }
            LayerAction::Add => {
                let n = doc.layers().len();
                let name = (n..)
                    .map(|i| format!("Layer {i}"))
                    .find(|name| doc.layer_by_name(name).is_none())
                    .unwrap_or_default();
                let color = LAYER_COLORS[n % LAYER_COLORS.len()];
                let layer = Layer::new(LayerId(0), name.clone(), color);
                self.execute(Box::new(
                    ChangeSet::new(format!("Add layer {name}")).insert_layer(layer),
                ));
                if let Some(id) = self.project.doc.layer_by_name(&name).map(|l| l.id) {
                    self.project.doc.set_current_layer(id);
                }
            }
            LayerAction::Delete(id) => {
                if let Some(l) = layer(doc, id) {
                    self.execute(Box::new(
                        ChangeSet::new(format!("Delete layer {}", l.name)).delete_layer(l.id),
                    ));
                }
            }
        }
    }

    // ------------------------------------------------------------------ viewport

    pub(super) fn viewport_ui(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme.clone();
        let scene = self
            .scene_cache
            .get(&self.project.doc, &self.project.selection, &theme);
        let conv = self.project.doc.conventions().clone();
        let grid = self.settings.show_grid.then(|| GridSettings {
            elevation: self.settings.working_z,
            ..Default::default()
        });

        // Keep the tool preview inside the clip range.
        let mut preview = Preview::default();
        self.active_tool.preview(&conv, self.pick.as_ref(), &mut preview);
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
            .grid(grid)
            .anchor(anchor)
            .extra_bounds(extra)
            .axis_labels(labels)
            .length_unit(conv.linear_unit.metres_per_unit(), conv.linear_unit.suffix())
            .show(ui);

        self.pick = out.hover.map(|c| self.compute_pick(c, grid.as_ref()));
        self.hovered_entity = match out.hover {
            Some(c) if !self.active_tool.wants_point() => hit_test(
                &self.project.doc,
                &self.viewport.camera,
                c,
                self.settings.snap.aperture_px,
            ),
            _ => None,
        };
        let idle_select = self.active_tool.id() == SELECT_TOOL_ID && !self.active_tool.wants_point();
        self.hovered_grip = match out.hover {
            Some(c) if idle_select => self.grip_at(c),
            _ => None,
        };

        // Recompute the preview against this frame's cursor.
        let mut preview = Preview::default();
        self.active_tool.preview(&conv, self.pick.as_ref(), &mut preview);
        let painter = ui.painter_at(out.rect);
        if self.settings.labels.any() {
            self.draw_labels(&painter, out.rect, &theme);
        }
        self.draw_overlays(&painter, out.rect, &preview, out.hover, &theme);
        if self.settings.dynamic_input
            && let Some(c) = out.hover
        {
            self.draw_hud(&painter, out.rect, to_screen(out.rect, c), &preview, &conv);
        }
        if self.project.doc.entity_count() == 0 && out.drag.is_none() {
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
            Some(NavAction::ZoomExtents) => self.zoom_extents(),
            Some(NavAction::TogglePlan) => self.toggle_plan(),
            Some(NavAction::ToggleProjection) => self.toggle_projection(),
            Some(NavAction::Preset(p)) => self.command_line.info(p.label()),
            None => {}
        }

        // Window / crossing selection with the select tool.
        if !self.active_tool.wants_point() {
            if let Some((a, b)) = out.drag {
                SelectionBox::from_drag(a, b).draw(&painter, out.rect, &theme);
            }
            if let Some((a, b)) = out.drag_released
                && a.distance(b) > 3.0
            {
                let ids = select_in_box(
                    &self.project.doc,
                    &self.viewport.camera,
                    &SelectionBox::from_drag(a, b),
                );
                self.dispatch(ToolEvent::Region(ids));
            }
        }

        if let Some(c) = out.clicked {
            let pick = self.compute_pick(c, grid.as_ref());
            self.dispatch(ToolEvent::Point(pick));
        }
        if out.secondary_clicked {
            self.dispatch(ToolEvent::Finish);
        }
    }

    /// Grips of the current selection (cached).
    fn current_grips(&mut self) -> &[Grip] {
        let key = (self.project.doc.revision(), self.project.selection.revision());
        if self.grips.0 != Some(key) {
            self.grips = (
                Some(key),
                crate::tools::grips::grips(&self.project.doc, &self.project.selection),
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
        let base = self.active_tool.base_point();
        let query = SnapQuery {
            camera: cam,
            cursor,
            base_point: base.map(|b| b.position),
            working_z: self.settings.working_z,
            grid_spacing: grid.map_or(0.0, |g| g.spacing(cam).0),
        };
        if !self.active_tool.wants_point() {
            return free_point(&query);
        }
        let settings = match self.snap_override {
            Some(mode) => SnapSettings::only(mode, self.settings.snap.aperture_px),
            None => self.settings.snap.clone(),
        };
        let provider = DocSnapProvider {
            doc: &self.project.doc,
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
        if self.active_tool.id() == SELECT_TOOL_ID && !self.active_tool.wants_point() {
            return;
        }
        let mut lines = vec![HudLine::Prompt(self.active_tool.prompt(conv))];
        if let Some((_, _, text)) = preview.labels.first() {
            lines.push(HudLine::Value(text.clone()));
        } else if let Some(p) = self.pick {
            lines.push(HudLine::Value(conv.format_coord(p.point, p.height_known)));
        }
        if !self.command_line.input.is_empty() {
            lines.push(HudLine::Input(self.command_line.input.clone()));
        }
        draw_cursor_hud(painter, rect, cursor, &lines);
    }

    fn draw_overlays(
        &mut self,
        painter: &egui::Painter,
        rect: egui::Rect,
        preview: &Preview,
        hover: Option<glam::DVec2>,
        theme: &CadTheme,
    ) {
        let cam = &self.viewport.camera;
        // Hover highlight (selection tool).
        if let Some(id) = self.hovered_entity {
            match self.project.doc.entity(id) {
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
                None => {}
            }
        }

        // Grips of the selection while the select tool is idle.
        if self.active_tool.id() == SELECT_TOOL_ID {
            let hovered = self.hovered_grip;
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
            && !self.active_tool.wants_point()
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
    fn draw_labels(&self, painter: &egui::Painter, rect: egui::Rect, theme: &CadTheme) {
        let cam = &self.viewport.camera;
        let doc = &self.project.doc;
        let conv = doc.conventions();
        let opts = &self.settings.labels;
        let frustum = cam.rect_frustum(glam::DVec2::ZERO, cam.viewport);
        let mut ids = Vec::new();
        doc.query(&|b| frustum.intersects_aabb(b), &mut ids);
        let mut placer = LabelPlacer::new(rect, 3000);
        let font = egui::FontId::proportional(opts.size);
        let small = egui::FontId::proportional((opts.size - 1.5).max(7.0));
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

            let mut job = egui::text::LayoutJob::default();
            let mut line = |text: &str, font: &egui::FontId, color: Color32| {
                if !job.text.is_empty() {
                    job.append("\n", 0.0, egui::TextFormat::simple(font.clone(), color));
                }
                job.append(text, 0.0, egui::TextFormat::simple(font.clone(), color));
            };
            if opts.names && !p.name.is_empty() {
                line(&p.name, &font, theme.text);
            }
            if opts.codes && !p.code.is_empty() {
                line(&p.code, &small, theme.accent);
            }
            if opts.heights && p.height_known {
                line(&conv.format_length(p.position.z), &small, theme.text_secondary);
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

    // ------------------------------------------------------------------ dialogs

    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        #[cfg(target_arch = "wasm32")]
        self.recovery_prompt(ctx);

        if let Some(dialog) = &mut self.csv_dialog {
            match dialog.show(ctx, &self.project.doc) {
                CsvDialogResult::Open => {}
                CsvDialogResult::Cancelled => {
                    self.settings.csv_layout = dialog.layout().clone();
                    self.csv_dialog = None;
                }
                CsvDialogResult::Import(cmd, n) => {
                    self.settings.csv_layout = dialog.layout().clone();
                    self.csv_dialog = None;
                    self.execute(cmd);
                    self.command_line.info(format!("Imported {n} points"));
                    self.toasts.success(format!("Imported {n} points"));
                    self.zoom_extents();
                }
            }
        }

        if let Some(cmd) = self
            .settings_window
            .show(ctx, &mut self.settings, &self.project.doc)
        {
            self.execute(cmd);
        }
        self.audit_window.show(ctx, &self.project);
        match self.traverse_window.show(ctx, &self.project.doc) {
            Some(TraverseAction::Create(cmd, summary)) => {
                self.execute(cmd);
                self.command_line.result(summary.clone());
                self.toasts.success(summary);
            }
            Some(TraverseAction::Report(title, text)) => self.reports.add(title, text),
            None => {}
        }
        let has_selection = !self.project.selection.is_empty();
        match self.codes_window.show(ctx, &self.project.doc, has_selection) {
            Some(CodesAction::Apply(library)) => self.apply_feature_codes(library),
            Some(CodesAction::Process {
                library,
                only_selected,
                assign_layers,
            }) => self.process_linework(library, only_selected, assign_layers),
            Some(CodesAction::Import) => self.platform.pick_file(ctx, FileKind::Codes),
            Some(CodesAction::Export(text)) => {
                match export(&self.platform, FileKind::Codes, "feature-codes", text.as_bytes()) {
                    Ok(Some(where_)) => self.command_line.info(format!("Saved code sheet: {where_}")),
                    Ok(None) => {}
                    Err(e) => self.command_line.error(format!("Could not save code sheet: {e}")),
                }
            }
            None => {}
        }
        if let Some(ReportAction::Export(title, text)) = self.reports.show(ctx) {
            match export(
                &self.platform,
                FileKind::Report,
                &file_stem(&title),
                text.as_bytes(),
            ) {
                Ok(Some(where_)) => self.command_line.info(format!("Saved report: {where_}")),
                Ok(None) => {}
                Err(e) => self.command_line.error(format!("Could not save report: {e}")),
            }
        }

        if self.about_open {
            egui::Window::new("About survey3d")
                .open(&mut self.about_open)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.heading(format!("survey3d {APP_VERSION}"));
                    ui.label("Surveying CAD for cadastral, construction and mining work.");
                    ui.label("Built with Rust, egui and wgpu.");
                    ui.add_space(6.0);
                    ui.label(RichText::new("Icons: Phosphor (MIT)").small());
                });
        }

        if let Some(action) = self.confirm_discard {
            let mut decision: Option<bool> = None;
            let mut save_first = false;
            egui::Modal::new(egui::Id::unique("confirm_discard")).show(ctx, |ui| {
                ui.set_width(320.0);
                ui.heading("Unsaved changes");
                ui.label(format!("{} has unsaved changes.", self.project.display_name));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        save_first = true;
                    }
                    if ui
                        .button(RichText::new("Discard").color(self.theme.error))
                        .clicked()
                    {
                        decision = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decision = Some(false);
                    }
                });
            });
            if save_first {
                self.save(ctx);
                // On native "save" may be cancelled in the file dialog; only continue if it worked.
                decision = Some(!self.project.is_dirty()).filter(|ok| *ok).or(Some(false));
            }
            match decision {
                Some(true) => {
                    self.confirm_discard = None;
                    self.perform_discard(ctx, action);
                }
                Some(false) => self.confirm_discard = None,
                None => {}
            }
        }
    }
}

/// Draws text in a small box near `pos`, kept inside `bounds`.
#[cfg(target_arch = "wasm32")]
impl SurveyApp {
    /// Offers to restore unsaved work found in browser storage.
    fn recovery_prompt(&mut self, ctx: &egui::Context) {
        if self.recovery.is_none() {
            return;
        }
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("recovery")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.heading("Recover unsaved work?");
            ui.label("survey3d found work from a previous session that was not saved.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Recover").clicked() {
                    choice = Some(true);
                }
                if ui
                    .button(RichText::new("Discard").color(self.theme.error))
                    .clicked()
                {
                    choice = Some(false);
                }
            });
        });
        let Some(recover) = choice else {
            return;
        };
        let bytes = self.recovery.take().unwrap_or_default();
        if recover {
            let result = ProjectStore::from_bytes(&bytes)
                .and_then(|store| Project::from_store(store, "Recovered".into(), &self.settings));
            match result {
                Ok(mut p) => {
                    // Not saved anywhere the user controls yet.
                    p.doc.mark_all_dirty();
                    let n = p.doc.entity_count();
                    self.replace_project(p);
                    self.command_line
                        .warn(format!("Recovered {n} objects — save to keep them (Ctrl+S)"));
                }
                Err(e) => self.command_line.error(format!("Recovery failed: {e}")),
            }
        } else {
            wasm_bindgen_futures::spawn_local(async {
                let _ = crate::platform::opfs::remove(crate::platform::opfs::AUTOSAVE).await;
            });
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
