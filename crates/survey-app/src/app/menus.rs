//! The menu bar and its actions.

use egui_cad::ThemeMode;
use egui_cad::chrome::{IconButton, search_button, toolbar_divider};
use egui_cad::viewport::{NavAction, view_controls};

use crate::platform::FileKind;
use crate::settings::Module;
use crate::tools::ToolGroup;

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
    Item("Text", "text"),
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
    Item("Bearing & distance (traverse / lot)", "bd"),
    Submenu(
        "Intersection",
        &[
            Item("Bearing – bearing", "bb"),
            Item("Bearing – distance", "brgdist"),
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
                #[cfg(not(target_arch = "wasm32"))]
                item(ui, "Point clouds (COPC)…", None, Action::Run("copc"), &mut action);
                item(
                    ui,
                    "Import raw data (GSI, JobXML)…",
                    None,
                    Action::Run("rawin"),
                    &mut action,
                );
                item(ui, "Export DXF…", None, Action::ExportDxf, &mut action);
                item(ui, "Export GeoPackage…", None, Action::Run("gpkg"), &mut action);
                ui.separator();
                item(ui, "Plan sheets…", None, Action::Run("sheets"), &mut action);
                item(
                    ui,
                    "Export plan sheet (PDF)…",
                    None,
                    Action::Run("plotpdf"),
                    &mut action,
                );
                #[cfg(not(target_arch = "wasm32"))]
                {
                    ui.separator();
                    item(ui, "Quit", None, Action::Quit, &mut action);
                }
            });
            ui.menu_button("Edit", |ui| {
                let undo = self
                    .session
                    .project
                    .history
                    .undo_label()
                    .map_or("Undo".into(), |l| format!("Undo {l}"));
                let redo = self
                    .session
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
                        .session
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
            let undo = self.session.project.history.undo_label();
            let redo = self.session.project.history.redo_label();
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
                    let a = match self.session.tools.find_command(command) {
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
            Action::Undo => self.session.undo(),
            Action::Redo => self.session.redo(),
            Action::Delete => self.session.delete_selection(),
            Action::SelectNone => self.session.project.selection.clear(),
            Action::ZoomExtents => self.zoom_extents(),
            Action::TogglePlan => self.toggle_plan(),
            Action::ToggleProjection => self.toggle_projection(),
            Action::ToggleGrid => self.settings.show_grid = !self.settings.show_grid,
            Action::View(nav) => self.apply_nav(nav),
            Action::Theme(mode) => self.settings.theme = mode,
            Action::Tool(id) => self.session.set_tool(id),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SESSION_COMMANDS;
    use crate::tools::{CoreModule, DomainModule, ToolRegistry};

    /// Session commands are listed (help, palette, completion) like the others.
    #[test]
    fn session_commands_are_app_commands() {
        for (name, _) in SESSION_COMMANDS {
            assert!(
                APP_COMMANDS.iter().any(|a| a.name == *name),
                "session command '{name}' is missing from APP_COMMANDS"
            );
        }
    }

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
