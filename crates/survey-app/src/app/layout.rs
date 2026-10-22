//! Panels, viewport overlays and dialogs of [`SurveyApp`].

use std::collections::HashMap;

use egui::{Color32, RichText, Stroke};
use egui_cad::chrome::empty_state;
use egui_cad::chrome::{IconButton, palette_divider, search_button, status_toggle, toolbar_divider};
use egui_cad::hud::{HudLine, draw_cursor_hud};
use egui_cad::labels::LabelPlacer;
use egui_cad::selection::SelectionBox;
use egui_cad::viewport::{NavAction, view_controls};

use crate::platform::FileKind;
use crate::scene_build::select_in_box;
use crate::settings::Module;
use crate::tools::ToolGroup;
use crate::ui::alignments::AlignmentsAction;
use crate::ui::parcels::ParcelsAction;
use crate::ui::sections::SectionsAction;
use crate::ui::surfaces::{SurfaceAction, SurfaceWindowAction, surfaces_panel};
use crate::ui::volumes::VolumesAction;
use egui_cad::ThemeMode;
use survey_core::style::LabelSet;

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
    Points,
    Reports,
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
    /// A request from the view controls in the header.
    View(NavAction),
    Theme(ThemeMode),
    Tool(&'static str),
    /// An application command by name.
    Run(&'static str),
    Conventions,
    DraftingSettings,
    Settings,
    Audit,
    Help,
    About,
}

/// A menu entry: a tool or app command (by command name) with its label.
enum MenuEntry {
    Item(&'static str, &'static str),
    Separator,
    Submenu(&'static str, &'static [MenuEntry]),
}

use MenuEntry::{Item, Separator, Submenu};

const DRAW_MENU: &[MenuEntry] = &[
    Item("Point", "point"),
    Item("Polyline", "polyline"),
    Item("Arc (three points)", "arc"),
    Item("Circle", "circle"),
    Separator,
    Item("Point along line", "along"),
    Item("Divide polyline", "divide"),
];

const MODIFY_MENU: &[MenuEntry] = &[
    Item("Move", "move"),
    Item("Copy", "copy"),
    Item("Rotate", "rotate"),
    Item("Offset", "offset"),
    Item("Fillet", "fillet"),
    Separator,
    Item("Explode", "explode"),
    Item("Join", "join"),
    Item("Reverse direction", "reverse"),
    Separator,
    Item("Delete", "delete"),
];

const COGO_MENU: &[MenuEntry] = &[
    Item("Inverse", "inverse"),
    Item("Area", "area"),
    Item("List selection", "list"),
    Separator,
    Item("Radiation", "radiate"),
    Submenu(
        "Intersection",
        &[
            Item("Bearing – bearing", "bb"),
            Item("Bearing – distance", "bd"),
            Item("Distance – distance", "dd"),
        ],
    ),
    Item("Chainage & offset", "chainage"),
    Item("Average points", "average"),
    Separator,
    Item("COGO calculator…", "calc"),
];

const SURFACE_MENU: &[MenuEntry] = &[
    Item("Create surface…", "surface"),
    Separator,
    Item("Volumes…", "volume"),
    Item("Sections…", "sections"),
    Item("Contours to polylines", "contours"),
    Separator,
    Item("Import LandXML…", "landxmlin"),
    Item("Export LandXML…", "landxmlout"),
];

const SURVEY_MENU: &[MenuEntry] = &[
    Item("Traverse…", "traverse"),
    Item("Closure…", "closure"),
    Item("Resection…", "resection"),
    Item("Subdivide area…", "subdivide"),
    Separator,
    Item("Import raw data (GSI, JobXML)…", "rawin"),
    Item("Network adjustment…", "netadj"),
    Item("Projection calculator…", "projcalc"),
    Separator,
    Item("Feature codes…", "codes"),
    Item("Process linework", "linework"),
];

const CADASTRAL_MENU: &[MenuEntry] = &[
    Item("Parcels…", "parcels"),
    Item("Parcels from selected polylines", "mkparcel"),
    Separator,
    Item("Closure…", "closure"),
    Item("Subdivide area…", "subdivide"),
    Separator,
    Item("Export parcels to LandXML…", "parcelxml"),
];

const CONSTRUCTION_MENU: &[MenuEntry] = &[
    Item("Alignments…", "alignments"),
    Item("Alignment from selected polyline", "mkalign"),
    Separator,
    Item("Chainage & offset", "chainage"),
    Item("Stake-out sheet…", "stakeout"),
    Item("Sections…", "sections"),
];

const MINING_MENU: &[MenuEntry] = &[
    Item("Import point cloud…", "cloudin"),
    Separator,
    Item("Stockpile volume…", "stockpile"),
    Item("Period volumes…", "volume"),
    Separator,
    Item("Create surface…", "surface"),
    Item("Sections…", "sections"),
];

/// A menu after View.
struct Menu {
    title: &'static str,
    entries: &'static [MenuEntry],
    /// Tool groups whose unlisted tools the menu collects.
    groups: &'static [ToolGroup],
    /// Module that must be switched on to show the menu.
    module: Option<Module>,
}

const fn menu(
    title: &'static str,
    entries: &'static [MenuEntry],
    groups: &'static [ToolGroup],
    module: Option<Module>,
) -> Menu {
    Menu {
        title,
        entries,
        groups,
        module,
    }
}

const MENUS: &[Menu] = &[
    menu("Draw", DRAW_MENU, &[ToolGroup::Draw], None),
    menu("Modify", MODIFY_MENU, &[ToolGroup::Modify], None),
    menu("COGO", COGO_MENU, &[ToolGroup::Cogo, ToolGroup::Measure], None),
    menu("Survey", SURVEY_MENU, &[], None),
    menu("Surface", SURFACE_MENU, &[], None),
    menu("Cadastral", CADASTRAL_MENU, &[], Some(Module::Cadastral)),
    menu("Construction", CONSTRUCTION_MENU, &[], Some(Module::Construction)),
    menu("Mining", MINING_MENU, &[], Some(Module::Mining)),
];

/// Whether any menu table lists `command`.
fn menu_lists(command: &str) -> bool {
    fn walk(entries: &[MenuEntry], command: &str) -> bool {
        entries.iter().any(|e| match e {
            Item(_, c) => *c == command,
            Separator => false,
            Submenu(_, inner) => walk(inner, command),
        })
    }
    MENUS.iter().any(|m| walk(m.entries, command))
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
                item(
                    ui,
                    "Import point cloud…",
                    None,
                    Action::Run("cloudin"),
                    &mut action,
                );
                item(
                    ui,
                    "Import raw data (GSI, JobXML)…",
                    None,
                    Action::Run("rawin"),
                    &mut action,
                );
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
                item(ui, "Points table", None, Action::Points, &mut action);
                item(ui, "Reports", None, Action::Reports, &mut action);
                item(ui, "Legend", None, Action::Run("legend"), &mut action);
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
            for m in MENUS {
                if m.module
                    .is_some_and(|module| !self.settings.modules.is_on(module))
                {
                    continue;
                }
                let groups = m.groups;
                ui.menu_button(m.title, |ui| {
                    self.menu_entries(ui, m.entries, &mut action);
                    // Tools registered by modules but not placed in a menu table.
                    let extra: Vec<_> = self
                        .tools
                        .tools()
                        .iter()
                        .filter(|t| groups.contains(&t.group) && !menu_lists(t.commands[0]))
                        .collect();
                    if !extra.is_empty() {
                        ui.separator();
                        for t in extra {
                            item(
                                ui,
                                t.name,
                                Some(t.commands[0].into()),
                                Action::Tool(t.id),
                                &mut action,
                            );
                        }
                    }
                });
            }
            ui.menu_button("Project", |ui| {
                item(ui, "Conventions…", None, Action::Conventions, &mut action);
                item(ui, "Accuracy limits…", None, Action::Run("accuracy"), &mut action);
                item(
                    ui,
                    "Projection calculator…",
                    None,
                    Action::Run("projcalc"),
                    &mut action,
                );
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
            toolbar_divider(ui);
            if let Some(nav) = view_controls(ui, &self.viewport) {
                action = Some(Action::View(nav));
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

    /// Menu entries from a table: tools by command name, otherwise app commands.
    fn menu_entries(&self, ui: &mut egui::Ui, entries: &[MenuEntry], action: &mut Option<Action>) {
        for entry in entries {
            match entry {
                MenuEntry::Item(label, command) => {
                    let a = match self.tools.find_command(command) {
                        Some(t) => Action::Tool(t.id),
                        None => Action::Run(command),
                    };
                    if ui
                        .add(egui::Button::new(*label).shortcut_text(*command))
                        .clicked()
                    {
                        *action = Some(a);
                    }
                }
                MenuEntry::Separator => {
                    ui.separator();
                }
                MenuEntry::Submenu(label, inner) => {
                    ui.menu_button(*label, |ui| self.menu_entries(ui, inner, action));
                }
            }
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
            Action::Points => self.points_window.open = true,
            Action::Reports => self.reports.open = true,
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
            Action::View(nav) => self.apply_nav(nav),
            Action::Theme(mode) => self.settings.theme = mode,
            Action::Tool(id) => self.set_tool(id),
            Action::Run(command) => {
                self.run_app_command(ctx, command);
            }
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

    fn apply_nav(&mut self, nav: NavAction) {
        match nav {
            NavAction::ZoomExtents => self.zoom_extents(),
            NavAction::TogglePlan => self.toggle_plan(),
            NavAction::ToggleProjection => self.toggle_projection(),
            NavAction::Preset(p) => {
                self.viewport.apply_preset(p);
                self.command_line.info(p.label());
            }
        }
    }

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
        let conv = self.project.doc.conventions();
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
            let mut new_surface = false;
            let mut active = self.active_surface;
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
                |ui| surfaces_panel(ui, &self.project.doc, &mut active),
            )
            .flatten();
            self.active_surface = active;
            if new_surface {
                self.apply_surface_action(SurfaceAction::New);
            }
            if let Some(a) = surface_action {
                self.apply_surface_action(a);
            }
            if self.project.doc.clouds().next().is_some() {
                let doc = &self.project.doc;
                let panel = &mut self.cloud_panel;
                let cloud_action =
                    section(ui, "Point clouds", true, |_| {}, |ui| panel.show(ui, doc)).flatten();
                if let Some(a) = cloud_action {
                    self.apply_cloud_action(a);
                }
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
        let surfaces = self.surface_cache.get(&self.project.doc, &theme);
        let clouds = self.cloud_cache.get(&self.project.doc, &theme);
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
            .overlay_scene(surfaces)
            .overlay_scene(clouds)
            .grid(grid)
            .anchor(anchor)
            .extra_bounds(extra)
            .axis_labels(labels)
            .length_unit(conv.linear_unit.metres_per_unit(), conv.linear_unit.suffix())
            .nav_controls(false)
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
        let code_labels = self
            .project
            .doc
            .meta()
            .feature_codes
            .codes
            .iter()
            .any(|c| c.style.labels.is_some());
        if self.settings.labels.any() || code_labels {
            self.draw_labels(&painter, out.rect, &theme);
        }
        if self
            .project
            .doc
            .meta()
            .alignments
            .iter()
            .any(|a| a.show_chainages)
        {
            self.draw_chainages(&painter, out.rect, &theme);
        }
        if self.network_window.open && self.network_window.show_ellipses {
            self.draw_error_ellipses(&painter, out.rect, &theme);
        }
        if self.project.doc.meta().parcels.iter().any(|p| p.annotate) {
            self.draw_parcel_annotations(&painter, out.rect, &theme);
        }
        if !self.surface_cache.labels().is_empty() {
            self.draw_contour_labels(&painter, out.rect, &theme);
        }
        self.draw_overlays(&painter, out.rect, &preview, out.hover, &theme);
        if self.settings.dynamic_input
            && let Some(c) = out.hover
        {
            self.draw_hud(&painter, out.rect, to_screen(out.rect, c), &preview, &conv);
        }
        let doc_empty = {
            let d = &self.project.doc;
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
            Some(NavAction::Preset(p)) => self.command_line.info(p.label()),
            Some(nav) => self.apply_nav(nav),
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
        if !self.active_tool.wants_point() {
            return free_point(&query);
        }
        // Grid snap is its own toggle (F9), independent of object snap (F3): object
        // snaps win, then the grid.
        let settings = match self.snap_override {
            Some(mode) => SnapSettings::only(mode, self.settings.snap.aperture_px),
            None => {
                let mut s = self.settings.snap.clone();
                s.set(SnapMode::Grid, false);
                s
            }
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
        if self.snap_override.is_none() && self.settings.snap.is_on(SnapMode::Grid) && pick.snap.is_none() {
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
        if self.settings.surface_heights && draped && self.project.doc.surfaces().next().is_some() {
            let doc = &self.project.doc;
            let preferred = self.active_surface;
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
    fn draw_contour_labels(&self, painter: &egui::Painter, rect: egui::Rect, theme: &CadTheme) {
        let cam = &self.viewport.camera;
        let conv = self.project.doc.conventions();
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
    fn draw_error_ellipses(&self, painter: &egui::Painter, rect: egui::Rect, theme: &CadTheme) {
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

    fn draw_chainages(&self, painter: &egui::Painter, rect: egui::Rect, theme: &CadTheme) {
        let doc = &self.project.doc;
        let conv = doc.conventions();
        let cam = &self.viewport.camera;
        let font = egui::FontId::proportional(10.0);
        let screen = |p: glam::DVec3| cam.world_to_screen(p).map(|(s, _)| to_screen(rect, s));
        let stroke = egui::Stroke::new(1.2, theme.accent);
        for a in doc.meta().alignments.iter().filter(|a| a.show_chainages) {
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

    /// Cadastral annotation for parcels that ask for it: bearing above and
    /// distance below each line (radius and arc length on arcs), and the lot's
    /// name and area at its centre.
    fn draw_parcel_annotations(&self, painter: &egui::Painter, rect: egui::Rect, theme: &CadTheme) {
        let doc = &self.project.doc;
        let conv = doc.conventions();
        let cam = &self.viewport.camera;
        let font = egui::FontId::proportional(10.5);
        let bold = egui::FontId::new(12.0, egui::FontFamily::Proportional);
        let screen = |p: glam::DVec3| cam.world_to_screen(p).map(|(s, _)| to_screen(rect, s));
        // A boundary shared by two lots is annotated once.
        let mut done: std::collections::HashSet<[(i64, i64); 2]> = std::collections::HashSet::new();
        let key = |p: glam::DVec3| ((p.x * 1000.0).round() as i64, (p.y * 1000.0).round() as i64);
        for parcel in doc.meta().parcels.iter().filter(|p| p.annotate) {
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
            Some(TraverseAction::Save(record)) => {
                let mut meta = self.project.doc.meta().clone();
                let name = record.name.clone();
                match meta.traverses.iter_mut().find(|r| r.name == name) {
                    Some(existing) => *existing = record,
                    None => meta.traverses.push(record),
                }
                self.execute(Box::new(
                    ChangeSet::new(format!("Save traverse {name}")).set_meta(meta),
                ));
                self.command_line
                    .result(format!("Traverse '{name}' saved with the project"));
            }
            Some(TraverseAction::Delete(name)) => {
                let mut meta = self.project.doc.meta().clone();
                meta.traverses.retain(|r| r.name != name);
                self.execute(Box::new(
                    ChangeSet::new(format!("Delete traverse {name}")).set_meta(meta),
                ));
            }
            None => {}
        }
        let selected_parcel = self
            .subdivide_window
            .open
            .then(|| self.selected_polylines().into_iter().next())
            .flatten();
        match self.subdivide_window.show(
            ctx,
            &self.project.doc,
            selected_parcel.as_ref().map(|(id, l)| (*id, l)),
        ) {
            Some(SubdivideAction::Draw(cmd, summary)) => {
                self.execute(cmd);
                self.command_line.result(summary);
            }
            Some(SubdivideAction::Report(title, text)) => self.reports.add(title, text),
            None => {}
        }
        let closed: Vec<EntityId> =
            if self.surface_window.open || self.volumes_window.open || self.stockpile_window.open {
                self.selected_polylines()
                    .into_iter()
                    .filter(|(_, l)| l.closed)
                    .map(|(id, _)| id)
                    .collect()
            } else {
                Vec::new()
            };
        if let Some(a) = self.surface_window.show(ctx, &self.project.doc, &closed) {
            self.apply_surface_window(a);
        }
        let can_create = self.parcels_window.open && self.selected_polylines().iter().any(|(_, l)| l.closed);
        match self.parcels_window.show(ctx, &self.project.doc, can_create) {
            Some(ParcelsAction::Update(i, record)) => {
                let mut meta = self.project.doc.meta().clone();
                if let Some(slot) = meta.parcels.get_mut(i) {
                    let label = format!("Edit parcel {}", record.name);
                    *slot = record;
                    self.execute(Box::new(ChangeSet::new(label).set_meta(meta)));
                }
            }
            Some(ParcelsAction::Delete(i)) => {
                let mut meta = self.project.doc.meta().clone();
                if i < meta.parcels.len() {
                    let p = meta.parcels.remove(i);
                    self.execute(Box::new(
                        ChangeSet::new(format!("Remove parcel {}", p.name)).set_meta(meta),
                    ));
                }
            }
            Some(ParcelsAction::Select(id)) if self.project.doc.contains(id) => {
                self.project.selection.set([id]);
            }
            Some(ParcelsAction::Report(title, text)) => self.reports.add(title, text),
            Some(ParcelsAction::CreateFromSelection) => self.parcels_from_selection(),
            Some(ParcelsAction::ExportLandXml) => self.export_parcels_landxml(),
            Some(ParcelsAction::Select(_)) | None => {}
        }
        let one_line = self.alignments_window.open && !self.selected_polylines().is_empty();
        match self.alignments_window.show(ctx, &self.project.doc, one_line) {
            Some(AlignmentsAction::Create) => self.alignment_from_selection(),
            Some(AlignmentsAction::Update(i, record)) => {
                let mut meta = self.project.doc.meta().clone();
                if let Some(slot) = meta.alignments.get_mut(i) {
                    let label = format!("Edit alignment {}", record.name);
                    *slot = record;
                    self.execute(Box::new(ChangeSet::new(label).set_meta(meta)));
                }
            }
            Some(AlignmentsAction::Delete(i)) => {
                let mut meta = self.project.doc.meta().clone();
                if i < meta.alignments.len() {
                    let a = meta.alignments.remove(i);
                    self.execute(Box::new(
                        ChangeSet::new(format!("Remove alignment {}", a.name)).set_meta(meta),
                    ));
                }
            }
            Some(AlignmentsAction::Select(id)) if self.project.doc.contains(id) => {
                self.project.selection.set([id]);
            }
            Some(AlignmentsAction::Report(title, text)) => self.reports.add(title, text),
            Some(AlignmentsAction::SetOut(i, pts, code)) => self.create_setout(i, pts, code),
            Some(AlignmentsAction::Select(_)) | None => {}
        }
        let targets: Vec<EntityId> = if self.stakeout_window.open {
            self.project.selection.ids().iter().copied().collect()
        } else {
            Vec::new()
        };
        if let Some((title, text)) = self.stakeout_window.show(ctx, &self.project.doc, &targets) {
            self.reports.add(title, text);
        }
        let selected_line = self
            .sections_window
            .open
            .then(|| self.selected_polylines().first().map(|(id, _)| *id))
            .flatten();
        if let Some(SectionsAction::Report(title, text)) =
            self.sections_window.show(ctx, &self.project.doc, selected_line)
        {
            self.reports.add(title, text);
        }
        let selected_point = self
            .project
            .selection
            .ids()
            .iter()
            .copied()
            .find(|id| matches!(self.project.doc.entity(*id), Some(Entity::Point(_))));
        match self.network_window.show(ctx, &self.project.doc) {
            Some(NetworkAction::Apply(cmd, summary)) => {
                self.execute(cmd);
                self.command_line.result(summary.clone());
                self.toasts.success(summary);
            }
            Some(NetworkAction::Report(title, text)) => self.reports.add(title, text),
            None => {}
        }
        match self
            .projection_window
            .show(ctx, &self.project.doc, selected_point)
        {
            Some(ProjectionAction::Create(cmd, summary)) => {
                self.execute(cmd);
                self.command_line.result(summary);
            }
            Some(ProjectionAction::Report(title, text)) => self.reports.add(title, text),
            None => {}
        }
        if let Some((title, text)) = self.stockpile_window.show(ctx, &self.project.doc, &closed) {
            self.reports.add(title, text);
        }
        match self.volumes_window.show(ctx, &self.project.doc, &closed) {
            Some(VolumesAction::Report(title, text)) => self.reports.add(title, text),
            Some(VolumesAction::CreateSurface(surface)) => {
                self.apply_surface_window(SurfaceWindowAction::Create(*surface));
            }
            None => {}
        }
        self.legend.show(ctx, &self.project.doc);
        if let Some((title, text)) = self.calculator.show(ctx, self.project.doc.conventions()) {
            self.reports.add(title, text);
        }
        match self.resection_window.show(ctx, &self.project.doc) {
            Some(ResectionAction::Create(cmd, summary)) => {
                self.execute(cmd);
                self.command_line.result(summary);
            }
            Some(ResectionAction::Report(title, text)) => self.reports.add(title, text),
            None => {}
        }
        let selected_line = self
            .closure_window
            .open
            .then(|| self.selected_polylines().into_iter().next().map(|(_, l)| l))
            .flatten();
        match self
            .closure_window
            .show(ctx, &self.project.doc, selected_line.as_ref())
        {
            Some(ClosureAction::Draw(cmd, summary)) => {
                self.execute(cmd);
                self.command_line.result(summary);
            }
            Some(ClosureAction::Report(title, text)) => self.reports.add(title, text),
            None => {}
        }
        match self
            .points_window
            .show(ctx, &self.project.doc, &self.project.selection)
        {
            Some(PointsAction::Select { id, toggle }) => {
                if toggle {
                    self.project.selection.toggle(id);
                } else {
                    self.project.selection.set([id]);
                }
            }
            Some(PointsAction::ZoomTo(id)) => {
                if let Some(e) = self.project.doc.entity(id) {
                    self.viewport.camera.target = e.bounds().center();
                    self.project.selection.set([id]);
                }
            }
            Some(PointsAction::Edit(cmd)) => self.execute(cmd),
            Some(PointsAction::Error(e)) => self.command_line.error(e),
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
                    let count =
                        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
                    let mut parts = vec![count(p.doc.entity_count(), "object", "objects")];
                    let surfaces = p.doc.surfaces().count();
                    if surfaces > 0 {
                        parts.push(count(surfaces, "surface", "surfaces"));
                    }
                    let clouds = p.doc.clouds().count();
                    if clouds > 0 {
                        parts.push(count(clouds, "point cloud", "point clouds"));
                    }
                    self.replace_project(p);
                    self.command_line.warn(format!(
                        "Recovered {} — save to keep them (Ctrl+S)",
                        parts.join(", ")
                    ));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every menu entry is a registered tool or a known app command.
    #[test]
    fn menu_commands_exist() {
        let mut tools = ToolRegistry::default();
        CoreModule.register(&mut tools);
        fn check(entries: &[MenuEntry], tools: &ToolRegistry) {
            for e in entries {
                match e {
                    Item(label, c) => assert!(
                        tools.find_command(c).is_some() || APP_COMMANDS.iter().any(|a| a.name == *c),
                        "menu item '{label}' runs unknown command '{c}'"
                    ),
                    Separator => {}
                    Submenu(_, inner) => check(inner, tools),
                }
            }
        }
        for m in MENUS {
            check(m.entries, &tools);
        }
        // Every core tool except Select has a place in the menus.
        for t in tools.tools() {
            assert!(
                t.id == SELECT_TOOL_ID || menu_lists(t.commands[0]),
                "tool '{}' is not in a menu",
                t.name
            );
        }
    }
}
