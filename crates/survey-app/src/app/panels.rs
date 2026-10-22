//! Status bar, command line, tool palette and side panel (layers, surfaces, properties).

use std::collections::HashMap;

use egui::RichText;
use egui_cad::chrome::{IconButton, palette_divider, status_toggle};

use crate::ui::surfaces::{SurfaceAction, surfaces_panel};

use super::*;

impl SurveyApp {
    // ------------------------------------------------------------------ status bar

    /// Read-only feedback only; all settings live in the Settings panel.
    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme.clone();
        ui.horizontal(|ui| {
            self.coordinate_readout(ui, &theme);
            ui.add_space(8.0);
            ui.separator();
            self.snap_toggles(ui);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.session.project.is_dirty() {
                    ui.label(RichText::new("● unsaved").small().color(theme.warning));
                }
                let sel = self.session.project.selection.len();
                if sel > 0 {
                    ui.label(
                        RichText::new(format!("{sel} selected"))
                            .small()
                            .color(theme.selection),
                    );
                }
                if let Some(hit) = self.pick.and_then(|p| p.snap) {
                    ui.label(RichText::new(hit.mode.label()).small().color(theme.snap));
                }
            });
        });
    }

    /// Cursor coordinates in a fixed-width, left-aligned field sized for grid
    /// coordinates (7-digit northings), so it never shifts as the values change.
    fn coordinate_readout(&self, ui: &mut egui::Ui, theme: &CadTheme) {
        const HORIZONTAL: usize = 12;
        const HEIGHT: usize = 10;
        let conv = self.session.project.doc.conventions();
        let (ea, nb) = conv.horizontal_labels();
        let z_label = conv.axis_labels.height.as_str();
        let text = match self.pick {
            Some(p) => {
                let (a, b) = conv.horizontal_values(p.point);
                let z = if p.height_known {
                    conv.format_length(p.point.z)
                } else {
                    "—".to_string()
                };
                format!(
                    "{ea} {:>HORIZONTAL$}  {nb} {:>HORIZONTAL$}  {z_label} {z:>HEIGHT$}",
                    conv.format_length(a),
                    conv.format_length(b),
                )
            }
            None => format!(
                "{ea} {:>HORIZONTAL$}  {nb} {:>HORIZONTAL$}  {z_label} {:>HEIGHT$}",
                "—", "—", "—"
            ),
        };
        // Width of the longest possible text in the monospace font.
        let font = egui::FontId::monospace(12.0);
        let chars =
            ea.chars().count() + nb.chars().count() + z_label.chars().count() + 2 * HORIZONTAL + HEIGHT + 7;
        let width = ui.fonts_mut(|f| f.glyph_width(&font, '0')) * chars as f32;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 18.0), egui::Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            text,
            font,
            theme.text,
        );
    }

    /// Snap and drafting toggles: object snap master and modes, grid snap, ortho.
    fn snap_toggles(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.x = 2.0;
        let snap_on = self.settings.snap.enabled;
        if status_toggle(ui, snap_on, "SNAP", "Object snap on/off (F3)").clicked() {
            self.toggle_snap();
        }
        ui.add_enabled_ui(snap_on, |ui| {
            for mode in SnapMode::ALL.into_iter().filter(|m| *m != SnapMode::Grid) {
                let on = self.settings.snap.is_on(mode);
                let tip = format!("{} snap (type {} for a one-shot snap)", mode.label(), mode.code());
                if status_toggle(ui, on, &mode.code().to_uppercase(), &tip).clicked() {
                    self.settings.snap.toggle(mode);
                }
            }
        });
        ui.add_space(6.0);
        let grid = self.settings.snap.is_on(SnapMode::Grid);
        if status_toggle(ui, grid, "GRID", "Snap to grid (F9)").clicked() {
            self.toggle_grid_snap();
        }
        if status_toggle(
            ui,
            self.settings.surface_heights,
            "SURF",
            "Picks that do not snap to an object take their height from the surface",
        )
        .clicked()
        {
            self.settings.surface_heights = !self.settings.surface_heights;
        }
        if status_toggle(
            ui,
            self.settings.ortho.enabled,
            "ORTHO",
            "Ortho / polar constraint (F8)",
        )
        .clicked()
        {
            self.toggle_ortho();
        }
        if status_toggle(
            ui,
            self.settings.dynamic_input,
            "DYN",
            "Dynamic input at the cursor (F12)",
        )
        .clicked()
        {
            self.settings.dynamic_input = !self.settings.dynamic_input;
        }
    }

    // ------------------------------------------------------------------ command line

    pub(super) fn command_line_ui(&mut self, ui: &mut egui::Ui) {
        let mut prompt = self
            .session
            .active_tool
            .prompt(self.session.project.doc.conventions());
        if let Some(mode) = self.session.snap_override {
            prompt = format!("[{}] {prompt}", mode.code().to_uppercase());
        }
        let event = CommandLine::new(&mut self.session.log, &prompt)
            .completions(&self.completions)
            .log_rows(4)
            .show(ui);
        let ctx = ui.ctx().clone();
        match event {
            Some(CommandLineEvent::Submit(text)) => self.submit(&ctx, text),
            Some(CommandLineEvent::Cancel) => {
                self.session.snap_override = None;
                self.session.dispatch(ToolEvent::Cancel);
            }
            None => {}
        }
    }

    // ------------------------------------------------------------------ tool palette

    pub(super) fn tool_palette(&mut self, ui: &mut egui::Ui) {
        let mut tools: Vec<_> = self
            .session
            .tools
            .tools()
            .iter()
            .filter(|t| t.palette)
            .map(|t| (t.group, t.id, t.icon, t.name, t.description, t.commands[0]))
            .collect();
        tools.sort_by_key(|t| t.0);
        let active = self.session.active_tool.id();
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
            self.session.set_tool(id);
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
            let mut new_surface = false;
            let mut active = self.session.active_surface;
            let surface_action = section(
                ui,
                "Surfaces",
                true,
                |ui| {
                    new_surface = IconButton::new(icons::PLUS, "New surface")
                        .description("Build a surface from points and breaklines")
                        .size(20.0)
                        .show(ui)
                        .clicked();
                },
                |ui| surfaces_panel(ui, &self.session.project.doc, &mut active),
            )
            .flatten();
            self.session.active_surface = active;
            if new_surface {
                self.apply_surface_action(SurfaceAction::New);
            }
            if let Some(a) = surface_action {
                self.apply_surface_action(a);
            }
            if self.session.project.doc.clouds().next().is_some() {
                let doc = &self.session.project.doc;
                let panel = &mut self.cloud_panel;
                let cloud_action =
                    section(ui, "Point clouds", true, |_| {}, |ui| panel.show(ui, doc)).flatten();
                if let Some(a) = cloud_action {
                    self.apply_cloud_action(a);
                }
            }
            let p = &self.session.project;
            let edit = section(
                ui,
                "Properties",
                true,
                |_| {},
                |ui| properties_panel(ui, &p.doc, &p.selection),
            );
            if let Some(cmd) = edit.flatten() {
                self.session.execute(cmd);
            }
        });
    }

    fn layer_rows(&mut self) -> Vec<LayerRow> {
        let doc = &self.session.project.doc;
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
        let doc = &mut self.session.project.doc;
        let layer = |doc: &survey_model::Document, id: i64| doc.layer(LayerId(id)).cloned();
        match action {
            LayerAction::SetCurrent(id) => doc.set_current_layer(LayerId(id)),
            // Visibility and locking are display state: saved, but not undoable or audited.
            LayerAction::SetVisible(id, visible) => {
                if let Some(mut l) = layer(doc, id) {
                    l.visible = visible;
                    doc.set_layer(l.id, Some(l));
                    let doc = &self.session.project.doc;
                    self.session
                        .project
                        .selection
                        .retain(|e| doc.is_entity_editable(e));
                }
            }
            LayerAction::SetLocked(id, locked) => {
                if let Some(mut l) = layer(doc, id) {
                    l.locked = locked;
                    doc.set_layer(l.id, Some(l));
                    let doc = &self.session.project.doc;
                    self.session
                        .project
                        .selection
                        .retain(|e| doc.is_entity_editable(e));
                }
            }
            LayerAction::SetColor(id, c) => {
                if let Some(mut l) = layer(doc, id) {
                    l.color = Color(c.to_srgba_unmultiplied());
                    let label = format!("Change colour of layer {}", l.name);
                    self.session
                        .execute(Box::new(ChangeSet::new(label).update_layer(l)));
                }
            }
            LayerAction::Rename(id, name) => {
                if let Some(mut l) = layer(doc, id) {
                    let label = format!("Rename layer {} to {name}", l.name);
                    l.name = name;
                    self.session
                        .execute(Box::new(ChangeSet::new(label).update_layer(l)));
                }
            }
            LayerAction::Add => {
                let n = doc.layers().len();
                let name = (n..)
                    .map(|i| format!("Layer {i}"))
                    .find(|name| doc.layer_by_name(name).is_none())
                    .unwrap_or_default();
                let color = Layer::palette_color(n);
                let layer = Layer::new(LayerId(0), name.clone(), color);
                self.session.execute(Box::new(
                    ChangeSet::new(format!("Add layer {name}")).insert_layer(layer),
                ));
                if let Some(id) = self.session.project.doc.layer_by_name(&name).map(|l| l.id) {
                    self.session.project.doc.set_current_layer(id);
                }
            }
            LayerAction::Delete(id) => {
                if let Some(l) = layer(doc, id) {
                    self.session.execute(Box::new(
                        ChangeSet::new(format!("Delete layer {}", l.name)).delete_layer(l.id),
                    ));
                }
            }
        }
    }
}
