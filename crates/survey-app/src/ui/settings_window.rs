//! Settings: user preferences (app-wide) and project conventions / coordinate system.

use egui::RichText;
use egui_cad::chrome::{section_header, segmented, switch_row};
use egui_cad::snap::SnapMode;
use egui_cad::{Accent, CadTheme, Density, ThemeMode};
use survey_core::accuracy::Tolerances;
use survey_core::conventions::{
    AngleFormat, AxisLabels, BearingStyle, ChainageStyle, CoordOrder, LinearUnit,
};
use survey_core::{Angle, Bearing, Conventions, DVec3};
use survey_model::{ChangeSet, Command, CoordinateSystem, DistanceKind, GroundFactor, GroundSettings};

use crate::session::Session;
use crate::settings::AppSettings;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Drafting,
    Display,
    Conventions,
    Coordinates,
    Accuracy,
    General,
}

#[derive(Default)]
pub struct SettingsWindow {
    pub open: bool,
    tab: Option<Tab>,
    /// Draft project settings, applied together as one audited edit.
    conventions: Option<Conventions>,
    coord_system: Option<CoordinateSystem>,
    ground: Option<GroundSettings>,
    tolerances: Option<Tolerances>,
    /// Geoid files found in the geoid folder (rescanned when it changes).
    geoid_files: Option<(Option<std::path::PathBuf>, Vec<String>)>,
}

impl SettingsWindow {
    pub fn open_at_drafting(&mut self) {
        self.open = true;
        self.tab = Some(Tab::Drafting);
    }

    pub fn open_at_display(&mut self) {
        self.open = true;
        self.tab = Some(Tab::Display);
    }

    pub fn open_at_accuracy(&mut self) {
        self.open = true;
        self.tab = Some(Tab::Accuracy);
    }

    pub fn open_at_conventions(&mut self) {
        self.open = true;
        self.tab = Some(Tab::Conventions);
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        settings: &mut AppSettings,
        session: &Session,
    ) -> Option<Box<dyn Command>> {
        let doc = &session.project.doc;
        if !self.open {
            self.conventions = None;
            self.coord_system = None;
            self.ground = None;
            self.tolerances = None;
            self.geoid_files = None;
            return None;
        }
        let mut open = self.open;
        let mut command = None;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .default_width(500.0)
            .show(ctx, |ui| {
                let tab = self.tab.get_or_insert(Tab::Drafting);
                ui.horizontal(|ui| {
                    for (t, label) in [
                        (Tab::Drafting, "Drafting"),
                        (Tab::Display, "Display"),
                        (Tab::Conventions, "Conventions"),
                        (Tab::Coordinates, "Coordinates"),
                        (Tab::Accuracy, "Accuracy"),
                        (Tab::General, "General"),
                    ] {
                        ui.selectable_value(tab, t, label);
                    }
                });
                ui.separator();
                match *tab {
                    Tab::General => general(ui, settings),
                    Tab::Drafting => drafting(ui, settings),
                    Tab::Display => display(ui, settings),
                    Tab::Conventions => {
                        let draft = self.conventions.get_or_insert_with(|| doc.conventions().clone());
                        conventions_editor(ui, draft);
                        ui.separator();
                        ui.horizontal(|ui| {
                            let changed = draft != doc.conventions();
                            if ui
                                .add_enabled(changed, egui::Button::new("Apply to project"))
                                .clicked()
                            {
                                let mut meta = doc.meta().clone();
                                meta.conventions = draft.clone();
                                command = Some(Box::new(ChangeSet::new("Change conventions").set_meta(meta))
                                    as Box<dyn Command>);
                            }
                            if ui
                                .button("Use as default for new projects")
                                .on_hover_text("Saved in your user settings")
                                .clicked()
                            {
                                settings.default_conventions = draft.clone();
                            }
                        });
                    }
                    Tab::Accuracy => {
                        let draft = self.tolerances.get_or_insert(doc.meta().tolerances);
                        tolerances_editor(ui, draft, doc.conventions());
                        ui.separator();
                        if ui
                            .add_enabled(
                                *draft != doc.meta().tolerances,
                                egui::Button::new("Apply to project"),
                            )
                            .clicked()
                        {
                            let mut meta = doc.meta().clone();
                            meta.tolerances = *draft;
                            command = Some(Box::new(ChangeSet::new("Change accuracy limits").set_meta(meta))
                                as Box<dyn Command>);
                        }
                    }
                    Tab::Coordinates => {
                        let dir = settings.geoid_dir.clone();
                        if self.geoid_files.as_ref().is_none_or(|(d, _)| *d != dir) {
                            let files = dir
                                .as_deref()
                                .map(crate::session::geoid_files)
                                .unwrap_or_default();
                            self.geoid_files = Some((dir, files));
                        }
                        let files = self
                            .geoid_files
                            .as_ref()
                            .map(|(_, f)| f.clone())
                            .unwrap_or_default();
                        let draft = self
                            .coord_system
                            .get_or_insert_with(|| doc.meta().coord_system.clone());
                        coord_system_editor(ui, draft, &files);
                        if matches!(draft, CoordinateSystem::Projected { .. }) {
                            geoid_folder(ui, settings, &mut self.geoid_files, &session.geoid_status());
                        }
                        let ground = self.ground.get_or_insert(doc.meta().ground);
                        ground_editor(ui, ground, draft.projection().is_some());
                        ui.separator();
                        let changed = *draft != doc.meta().coord_system || *ground != doc.meta().ground;
                        if ui
                            .add_enabled(changed, egui::Button::new("Apply to project"))
                            .clicked()
                        {
                            let mut meta = doc.meta().clone();
                            meta.coord_system = draft.clone();
                            meta.ground = *ground;
                            command = Some(
                                Box::new(ChangeSet::new("Change coordinate system").set_meta(meta))
                                    as Box<dyn Command>,
                            );
                        }
                    }
                }
            });
        self.open = open;
        if command.is_some() {
            self.conventions = None;
            self.coord_system = None;
            self.ground = None;
            self.tolerances = None;
        }
        command
    }
}

fn general(ui: &mut egui::Ui, settings: &mut AppSettings) {
    section_header(ui, "Modules");
    for m in crate::settings::Module::ALL {
        switch_row(ui, settings.modules.get_mut(m), m.name());
    }
    ui.label(
        RichText::new("Modules add their own menu. Their commands can still be typed when switched off.")
            .small()
            .weak(),
    );
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        ui.label("User name");
        ui.text_edit_singleline(&mut settings.user_name)
            .on_hover_text("Recorded with every edit in the project audit trail");
    });
}

/// How things look: theme, grid and point labels.
fn display(ui: &mut egui::Ui, settings: &mut AppSettings) {
    section_header(ui, "Appearance");
    egui::Grid::new("appearance")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Theme");
            segmented(
                ui,
                &mut settings.theme,
                &[(ThemeMode::Dark, "Dark"), (ThemeMode::Light, "Light")],
            );
            ui.end_row();
            ui.label("Accent");
            let accents: Vec<(Accent, &str)> = Accent::ALL.iter().map(|a| (*a, a.label())).collect();
            segmented(ui, &mut settings.accent, &accents);
            ui.end_row();
            ui.label("Density");
            let densities: Vec<(Density, &str)> = Density::ALL.iter().map(|d| (*d, d.label())).collect();
            segmented(ui, &mut settings.density, &densities);
            ui.end_row();
        });
    ui.add_space(4.0);
    switch_row(ui, &mut settings.show_grid, "Grid (F7)");
    switch_row(
        ui,
        &mut settings.dynamic_input,
        "Dynamic input beside the cursor (F12)",
    );
    switch_row(ui, &mut settings.cloud_edl, "Eye-dome lighting on point clouds");

    ui.add_space(8.0);
    section_header(ui, "Point labels");
    let labels = &mut settings.labels;
    switch_row(ui, &mut labels.names, "Name");
    switch_row(ui, &mut labels.codes, "Code");
    switch_row(ui, &mut labels.heights, "Height");
    ui.add_enabled(
        labels.any(),
        egui::Slider::new(&mut labels.size, 8.0..=18.0).text("text size"),
    );
    ui.label(
        RichText::new("Overlapping labels are hidden automatically; zoom in to see more.")
            .small()
            .weak(),
    );
}

/// Everything that affects picking in the viewport.
fn drafting(ui: &mut egui::Ui, settings: &mut AppSettings) {
    section_header(ui, "Object snap");
    let snap = &mut settings.snap;
    switch_row(ui, &mut snap.enabled, "Snap to objects (F3)");
    ui.add_space(2.0);
    ui.add_enabled_ui(snap.enabled, |ui| {
        egui::Grid::new("snap_modes")
            .num_columns(3)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                let modes = SnapMode::ALL.into_iter().filter(|m| *m != SnapMode::Grid);
                for (i, mode) in modes.enumerate() {
                    let mut on = snap.is_on(mode);
                    if ui.checkbox(&mut on, mode.label()).changed() {
                        snap.set(mode, on);
                    }
                    if i % 3 == 2 {
                        ui.end_row();
                    }
                }
            });
        ui.add(egui::Slider::new(&mut snap.aperture_px, 4.0..=30.0).text("pick aperture (px)"));
    });
    ui.label(
        RichText::new(
            "During a command, type end, mid, int, per, cen, nea, nod or gri for a one-shot snap; Tab cycles candidates.",
        )
        .small()
        .weak(),
    );

    ui.add_space(8.0);
    section_header(ui, "Grid snap");
    let mut grid_snap = settings.snap.is_on(SnapMode::Grid);
    switch_row(ui, &mut grid_snap, "Snap to grid (F9)");
    settings.snap.set(SnapMode::Grid, grid_snap);
    ui.horizontal(|ui| {
        let mut fixed = settings.grid_snap_spacing.is_some();
        segmented(
            ui,
            &mut fixed,
            &[(false, "Follow visible grid"), (true, "Fixed spacing")],
        );
        if fixed {
            let spacing = settings.grid_snap_spacing.get_or_insert(1.0);
            ui.add(
                egui::DragValue::new(spacing)
                    .range(0.001..=100_000.0)
                    .speed(0.1)
                    .suffix(" m"),
            );
        } else {
            settings.grid_snap_spacing = None;
        }
    });
    ui.label(
        RichText::new("Object snaps take priority; the grid applies when nothing else is under the cursor.")
            .small()
            .weak(),
    );

    ui.add_space(8.0);
    section_header(ui, "Direction constraint");
    switch_row(ui, &mut settings.ortho.enabled, "Ortho / polar (F8)");
    ui.add_enabled(
        settings.ortho.enabled,
        egui::Slider::new(&mut settings.ortho.increment_deg, 5.0..=90.0)
            .step_by(5.0)
            .text("increment (°)"),
    );

    ui.add_space(8.0);
    section_header(ui, "Working plane");
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(&mut settings.working_z)
                .speed(0.1)
                .suffix(" m"),
        );
        ui.label("working elevation")
            .on_hover_text("Height of the plane used for picks that do not snap to anything");
    });
}

fn conventions_editor(ui: &mut egui::Ui, c: &mut Conventions) {
    ui.horizontal(|ui| {
        ui.label("Preset");
        egui::ComboBox::from_id_salt("conv_preset")
            .selected_text(c.name.clone())
            .show_ui(ui, |ui| {
                for preset in Conventions::presets() {
                    if ui.selectable_label(c.name == preset.name, &preset.name).clicked() {
                        *c = preset;
                    }
                }
            });
    });
    let before = c.clone();
    egui::Grid::new("conv_grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Angles");
            combo(ui, "angle_fmt", &mut c.angle_format, &AngleFormat::ALL, |f| {
                f.label()
            });
            ui.end_row();
            ui.label("Angle decimals");
            ui.add(egui::DragValue::new(&mut c.angle_decimals).range(0..=4));
            ui.end_row();
            ui.label("Bearings");
            combo(ui, "brg_style", &mut c.bearing_style, &BearingStyle::ALL, |f| {
                f.label()
            });
            ui.end_row();
            ui.label("Coordinate order");
            combo(ui, "coord_order", &mut c.coord_order, &CoordOrder::ALL, |f| {
                f.label()
            });
            ui.end_row();
            ui.label("Distance unit");
            combo(ui, "lin_unit", &mut c.linear_unit, &LinearUnit::ALL, |f| {
                f.label()
            });
            ui.end_row();
            ui.label("Distance decimals");
            ui.add(egui::DragValue::new(&mut c.linear_decimals).range(0..=6));
            ui.end_row();
            ui.label("Chainage");
            ui.horizontal(|ui| {
                let mut station = matches!(c.chainage_style, ChainageStyle::Station { .. });
                ui.selectable_value(&mut station, false, "1234.5");
                ui.selectable_value(&mut station, true, "Station (+)");
                c.chainage_style = match (station, c.chainage_style) {
                    (false, _) => ChainageStyle::Plain,
                    (true, ChainageStyle::Station { interval }) => {
                        let mut interval = interval;
                        egui::ComboBox::from_id_salt("station_interval")
                            .selected_text(interval.to_string())
                            .width(70.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut interval, 100, "100");
                                ui.selectable_value(&mut interval, 1000, "1000");
                            });
                        ChainageStyle::Station { interval }
                    }
                    (true, ChainageStyle::Plain) => ChainageStyle::Station { interval: 1000 },
                };
            });
            ui.end_row();
            let AxisLabels {
                easting,
                northing,
                height,
            } = &mut c.axis_labels;
            ui.label("Axis labels");
            ui.horizontal(|ui| {
                for v in [easting, northing, height] {
                    ui.add(egui::TextEdit::singleline(v).desired_width(48.0));
                }
            });
            ui.end_row();
        });
    if *c != before && c.name == before.name && Conventions::presets().iter().any(|p| p.name == c.name) {
        c.name = format!("{} (custom)", before.name);
    }

    // Live examples so the effect of each option is obvious.
    let theme = CadTheme::current(ui.ctx());
    ui.add_space(6.0);
    let example_bearing = Bearing::new(Angle::from_dms(false, 123.0, 45.0, 30.5));
    let example = [
        ("Bearing", c.format_bearing(example_bearing)),
        ("Distance", c.format_length_with_unit(1234.5678)),
        (
            "Coordinate",
            c.format_coord(DVec3::new(512_345.678, 6_912_345.678, 101.25), true),
        ),
        ("Chainage", c.format_chainage(1234.5678)),
    ];
    egui::Frame::new()
        .fill(theme.bg)
        .inner_margin(8)
        .corner_radius(4)
        .show(ui, |ui| {
            for (k, v) in example {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{k}:")).color(theme.text_secondary));
                    ui.label(RichText::new(v).monospace());
                });
            }
        });
}

fn combo<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut T,
    options: &[T],
    label: fn(T) -> &'static str,
) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(label(*value))
        .show_ui(ui, |ui| {
            for &o in options {
                ui.selectable_value(value, o, label(o));
            }
        });
}

/// The folder geoid models are kept in, and what is loaded.
fn geoid_folder(
    ui: &mut egui::Ui,
    settings: &mut AppSettings,
    files: &mut Option<(Option<std::path::PathBuf>, Vec<String>)>,
    status: &str,
) {
    #[cfg(not(target_arch = "wasm32"))]
    ui.horizontal(|ui| {
        ui.label("Geoid folder");
        let shown = settings
            .geoid_dir
            .as_ref()
            .map_or_else(|| "(none set)".to_string(), |p| p.display().to_string());
        ui.label(RichText::new(shown).monospace().small());
        if ui
            .button("Change…")
            .on_hover_text("The folder you download geoid models into")
            .clicked()
            && let Some(dir) = rfd::FileDialog::new().pick_folder()
        {
            settings.geoid_dir = Some(dir);
        }
        if ui.small_button("Rescan").clicked() {
            *files = None;
        }
    });
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (settings, files);
        ui.label(RichText::new("Geoid models are available in the desktop app.").small());
    }
    ui.label(RichText::new(status).small());
    ui.label(
        RichText::new(
            "Download a model into the folder: PROJ's GeoTIFF grids from cdn.proj.org \
             (au_ga_AUSGeoid2020_20180201.tif, us_noaa_g2018u0.tif, us_nga_egm08_25.tif…), \
             Geoscience Australia's AUSGeoid2020 .gsb, NGS GEOID18 .bin, VDatum .gtx or GeographicLib .pgm.",
        )
        .small()
        .weak(),
    );
}

/// Grid or ground distances.
fn ground_editor(ui: &mut egui::Ui, g: &mut GroundSettings, projection: bool) {
    section_header(ui, "Distances");
    segmented(
        ui,
        &mut g.distances,
        &[(DistanceKind::Grid, "Grid"), (DistanceKind::Ground, "Ground")],
    );
    ui.label(
        RichText::new(
            "Coordinates stay on the grid. Ground shows distances in inverses, listings, set-out and on plans \
             as grid ÷ the combined scale factor, and takes distances typed in bd as ground.",
        )
        .small(),
    );
    if g.distances == DistanceKind::Ground {
        let mut fixed = matches!(g.factor, GroundFactor::Fixed(_));
        ui.horizontal(|ui| {
            ui.label("Combined factor");
            ui.radio_value(&mut fixed, false, "From the projection, along each line");
            ui.radio_value(&mut fixed, true, "Fixed");
        });
        match (fixed, &mut g.factor) {
            (false, f) => {
                *f = GroundFactor::Projection;
                if !projection {
                    ui.label(
                        RichText::new(
                            "Set a projection above, or a fixed factor: without one, ground = grid.",
                        )
                        .small(),
                    );
                }
            }
            (true, GroundFactor::Fixed(k)) => {
                ui.add(
                    egui::DragValue::new(k)
                        .speed(0.00001)
                        .max_decimals(9)
                        .range(0.9..=1.1),
                );
            }
            (true, f) => *f = GroundFactor::Fixed(1.0),
        }
    }
}

fn coord_system_editor(ui: &mut egui::Ui, cs: &mut CoordinateSystem, geoid_files: &[String]) {
    use survey_core::projection::{Ellipsoid, TransverseMercator};
    #[derive(PartialEq, Clone, Copy)]
    enum Kind {
        Unspecified,
        Projected,
        Local,
    }
    let kind = match cs {
        CoordinateSystem::Unspecified => Kind::Unspecified,
        CoordinateSystem::Projected { .. } => Kind::Projected,
        CoordinateSystem::LocalGrid { .. } => Kind::Local,
    };
    let mut new_kind = kind;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut new_kind, Kind::Unspecified, "Unspecified");
        ui.selectable_value(&mut new_kind, Kind::Projected, "Projected grid");
        ui.selectable_value(&mut new_kind, Kind::Local, "Local grid");
    });
    if new_kind != kind {
        *cs = match new_kind {
            Kind::Unspecified => CoordinateSystem::Unspecified,
            Kind::Projected => CoordinateSystem::Projected {
                name: String::new(),
                epsg: None,
                zone: None,
                projection: None,
                geoid_separation: 0.0,
                geoid: None,
            },
            Kind::Local => CoordinateSystem::LocalGrid {
                name: "Site grid".into(),
                description: String::new(),
            },
        };
    }
    match cs {
        CoordinateSystem::Unspecified => {
            ui.label("Coordinates are stored as entered; no system is recorded.");
        }
        CoordinateSystem::Projected {
            name,
            epsg,
            zone,
            projection,
            geoid_separation,
            geoid,
        } => {
            #[derive(PartialEq, Clone, Copy)]
            enum Preset {
                None,
                UtmWgs84,
                UtmGrs80,
                Custom,
            }
            let preset = match projection {
                None => Preset::None,
                Some(tm)
                    if tm.scale == 0.9996
                        && tm.false_easting == 500_000.0
                        && tm.latitude_of_origin == 0.0 =>
                {
                    if tm.ellipsoid == Ellipsoid::WGS84 {
                        Preset::UtmWgs84
                    } else {
                        Preset::UtmGrs80
                    }
                }
                Some(_) => Preset::Custom,
            };
            let mut new_preset = preset;
            egui::Grid::new("crs_grid")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Projection");
                    egui::ComboBox::from_id_salt("crs_preset")
                        .selected_text(match preset {
                            Preset::None => "Not set (record only)",
                            Preset::UtmWgs84 => "UTM zone (WGS84)",
                            Preset::UtmGrs80 => "UTM zone (GRS80: MGA, ETRS89, NAD83…)",
                            Preset::Custom => "Custom Transverse Mercator",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut new_preset, Preset::None, "Not set (record only)");
                            ui.selectable_value(&mut new_preset, Preset::UtmWgs84, "UTM zone (WGS84)");
                            ui.selectable_value(
                                &mut new_preset,
                                Preset::UtmGrs80,
                                "UTM zone (GRS80: MGA, ETRS89, NAD83…)",
                            );
                            ui.selectable_value(
                                &mut new_preset,
                                Preset::Custom,
                                "Custom Transverse Mercator",
                            );
                        });
                    ui.end_row();
                    if new_preset != preset {
                        let z = zone.unwrap_or(31);
                        let south = projection.is_some_and(|p| p.false_northing > 0.0);
                        *projection = match new_preset {
                            Preset::None => None,
                            Preset::UtmWgs84 => Some(TransverseMercator::utm(z, south, Ellipsoid::WGS84)),
                            Preset::UtmGrs80 => Some(TransverseMercator::utm(z, south, Ellipsoid::GRS80)),
                            Preset::Custom => Some(projection.unwrap_or(TransverseMercator::utm(
                                z,
                                south,
                                Ellipsoid::GRS80,
                            ))),
                        };
                        if matches!(new_preset, Preset::UtmWgs84 | Preset::UtmGrs80) {
                            *zone = Some(z);
                        }
                    }
                    match (new_preset, projection.as_mut()) {
                        (Preset::UtmWgs84 | Preset::UtmGrs80, Some(tm)) => {
                            ui.label("Zone");
                            ui.horizontal(|ui| {
                                let mut z = zone.unwrap_or(TransverseMercator::utm_zone(tm.central_meridian));
                                let mut south = tm.false_northing > 0.0;
                                let changed = ui.add(egui::DragValue::new(&mut z).range(1..=60)).changed()
                                    | ui.checkbox(&mut south, "Southern hemisphere").changed();
                                if changed {
                                    *tm = TransverseMercator::utm(z, south, tm.ellipsoid);
                                    *zone = Some(z);
                                }
                                ui.label(RichText::new(format!("CM {}°", tm.central_meridian)).small());
                            });
                            ui.end_row();
                        }
                        (Preset::Custom, Some(tm)) => {
                            ui.label("Ellipsoid");
                            egui::ComboBox::from_id_salt("crs_ellipsoid")
                                .selected_text(tm.ellipsoid.name)
                                .show_ui(ui, |ui| {
                                    for e in Ellipsoid::ALL {
                                        ui.selectable_value(&mut tm.ellipsoid, e, e.name);
                                    }
                                });
                            ui.end_row();
                            for (label, value, speed) in [
                                ("Central meridian (°)", &mut tm.central_meridian, 0.1),
                                ("Latitude of origin (°)", &mut tm.latitude_of_origin, 0.1),
                                ("Scale factor", &mut tm.scale, 0.0001),
                                ("False easting (m)", &mut tm.false_easting, 1000.0),
                                ("False northing (m)", &mut tm.false_northing, 1000.0),
                            ] {
                                ui.label(label);
                                ui.add(egui::DragValue::new(value).speed(speed).max_decimals(8));
                                ui.end_row();
                            }
                        }
                        _ => {}
                    }
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(name).hint_text("e.g. MGA2020 zone 56"));
                    ui.end_row();
                    ui.label("EPSG code");
                    optional_u32(ui, epsg);
                    ui.end_row();
                    ui.label("Geoid model");
                    let none = "None (constant separation)";
                    egui::ComboBox::from_id_salt("crs_geoid")
                        .selected_text(geoid.as_deref().unwrap_or(none))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(geoid, None, none);
                            for f in geoid_files {
                                ui.selectable_value(geoid, Some(f.clone()), f);
                            }
                            if let Some(g) = geoid.clone()
                                && !geoid_files.contains(&g)
                            {
                                ui.selectable_value(
                                    geoid,
                                    Some(g.clone()),
                                    format!("{g} (not in the folder)"),
                                );
                            }
                        });
                    ui.end_row();
                    ui.label(if geoid.is_some() {
                        "Separation outside it"
                    } else {
                        "Geoid separation N"
                    });
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(geoid_separation).speed(0.01).suffix(" m"));
                        ui.label(RichText::new("ellipsoidal height = height + N").small());
                    });
                    ui.end_row();
                });
            ui.label(
                RichText::new(
                    "With a projection set, the projection calculator converts to and from latitude/longitude, \
                     and scale factors (point, height and combined) are available for traverses.",
                )
                .small(),
            );
        }
        CoordinateSystem::LocalGrid { name, description } => {
            egui::Grid::new("crs_grid")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(name);
                    ui.end_row();
                    ui.label("Description");
                    ui.text_edit_multiline(description);
                    ui.end_row();
                });
        }
    }
}

fn tolerances_editor(ui: &mut egui::Ui, t: &mut Tolerances, conv: &Conventions) {
    section_header(ui, "Misclosure limits");
    ui.label(
        RichText::new(
            "Limits depend on your jurisdiction and survey class, so none are set by default. \
             Traverse and closure results show PASS or FAIL against the limits switched on here.",
        )
        .small(),
    );
    ui.add_space(4.0);
    let unit = conv.linear_unit.suffix();
    egui::Grid::new("tolerances")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Angular misclosure");
            optional_f64(ui, &mut t.angular_seconds, 10.0, 0.0..=3600.0, "″ × √n angles");
            ui.end_row();
            ui.label("Minimum precision");
            optional_f64(ui, &mut t.min_precision, 10_000.0, 1.0..=10_000_000.0, "1 : N");
            ui.end_row();
            ui.label("Linear allowance");
            let mpu = conv.linear_unit.metres_per_unit();
            let mut floor = t.linear_floor.map(|m| m / mpu);
            optional_f64(
                ui,
                &mut floor,
                0.02 / mpu,
                0.0..=10.0,
                &format!("{unit} (passes regardless of ratio)"),
            );
            t.linear_floor = floor.map(|v| v * mpu);
            ui.end_row();
            ui.label("Vertical misclosure");
            optional_f64(ui, &mut t.vertical_mm_per_root_km, 12.0, 0.0..=1000.0, "mm × √km");
            ui.end_row();
        });
}

/// A checkbox enabling an optional number, with a unit hint after it.
fn optional_f64(
    ui: &mut egui::Ui,
    value: &mut Option<f64>,
    default: f64,
    range: std::ops::RangeInclusive<f64>,
    suffix: &str,
) {
    let mut on = value.is_some();
    ui.horizontal(|ui| {
        ui.checkbox(&mut on, "");
        let mut v = value.unwrap_or(default);
        ui.add_enabled(on, egui::DragValue::new(&mut v).range(range).max_decimals(4));
        ui.label(RichText::new(suffix).small());
        *value = on.then_some(v);
    });
}

fn optional_u32(ui: &mut egui::Ui, value: &mut Option<u32>) {
    let mut on = value.is_some();
    ui.horizontal(|ui| {
        ui.checkbox(&mut on, "");
        let mut v = value.unwrap_or(0);
        ui.add_enabled(on, egui::DragValue::new(&mut v).range(0..=999_999));
        *value = on.then_some(v);
    });
}
