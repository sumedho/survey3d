//! Site calibration: match GNSS-measured points to local control, check the
//! residuals live, and apply the fit to the measured data.

use std::collections::BTreeSet;

use egui::RichText;
use egui_cad::chrome::{IconButton, segmented};
use egui_cad::{CadTheme, fonts, icons};
use survey_core::calibration::{PairSetup, SiteCalibration, VerticalMethod};
use survey_model::{ChangeSet, Command, Document, Entity, EntityId};

use crate::calibration::{SiteFit, Target, apply, compute, format_rotation, report};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ApplyTo {
    AllExceptControl,
    Selection,
}

pub struct CalibrationWindow {
    pub open: bool,
    /// The setup was loaded from the project since the window opened.
    loaded: bool,
    setup: SiteCalibration,
    apply_to: ApplyTo,
}

impl Default for CalibrationWindow {
    fn default() -> Self {
        Self {
            open: false,
            loaded: false,
            setup: SiteCalibration::default(),
            apply_to: ApplyTo::AllExceptControl,
        }
    }
}

pub enum CalibrationAction {
    /// Apply (or save) the calibration: a change and its summary.
    Apply(Box<dyn Command>, String),
    Report(String, String),
}

fn new_pair() -> PairSetup {
    PairSetup {
        horizontal: true,
        vertical: true,
        ..Default::default()
    }
}

impl CalibrationWindow {
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        selection: &BTreeSet<EntityId>,
    ) -> Option<CalibrationAction> {
        if !self.open {
            self.loaded = false;
            return None;
        }
        if !self.loaded {
            self.loaded = true;
            self.setup = doc.meta().calibration.clone();
            self.setup.applied = None;
            if self.setup.pairs.is_empty() {
                self.setup.pairs = vec![new_pair(); 3];
            }
        }
        let theme = CadTheme::current(ctx);
        let mut action = None;
        let mut open = self.open;
        egui::Window::new("Site calibration")
            .open(&mut open)
            .default_size([720.0, 640.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    ui.label(
                        RichText::new(
                            "Pair each GNSS-measured point with the local control point it should \
                             match (a point name, or type E,N[,Z]). Residuals update as you type; \
                             untick H or V to leave a doubtful pair out. Use 3 or more pairs \
                             spread around the site, enclosing the work.",
                        )
                        .small()
                        .color(theme.text_secondary),
                    );
                    ui.add_space(6.0);
                    let site = compute(doc, &self.setup);
                    self.pairs_table(ui, &theme, doc, site.as_ref().ok(), selection);
                    ui.add_space(8.0);
                    self.options(ui, &theme);
                    ui.add_space(6.0);
                    ui.separator();
                    match &site {
                        Ok(site) => {
                            results(ui, &theme, doc, site, &self.setup);
                            ui.add_space(8.0);
                            action = self.apply_row(ui, &theme, doc, site, selection);
                        }
                        Err(e) => {
                            ui.label(RichText::new(e).color(theme.text_secondary));
                        }
                    }
                });
            });
        self.open = open;
        action
    }

    fn pairs_table(
        &mut self,
        ui: &mut egui::Ui,
        theme: &CadTheme,
        doc: &Document,
        site: Option<&SiteFit>,
        selection: &BTreeSet<EntityId>,
    ) {
        let conv = doc.conventions();
        let (ea, nb) = conv.horizontal_labels();
        let opts = self.setup.options;
        let field = |ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32, known: bool| {
            let mut edit = egui::TextEdit::singleline(text)
                .hint_text(hint)
                .font(fonts::mono(12.0));
            if !known {
                edit = edit.text_color(theme.error);
            }
            ui.add_sized([width, 22.0], edit);
        };
        let mut remove = None;
        egui::Grid::new("calibration_pairs")
            .num_columns(9)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                for h in [
                    "#".to_string(),
                    "Measured (GNSS)".into(),
                    "Control".into(),
                    "H".into(),
                    "V".into(),
                    format!("d{ea}"),
                    format!("d{nb}"),
                    "dH".into(),
                    String::new(),
                ] {
                    ui.label(RichText::new(h).small().color(theme.text_secondary));
                }
                ui.end_row();
                let mut resolved = site.map(|s| {
                    s.pairs
                        .iter()
                        .zip(&s.fit.residuals)
                        .enumerate()
                        .map(|(i, (p, r))| (p, r, i))
                });
                for (i, row) in self.setup.pairs.iter_mut().enumerate() {
                    let blank = row.measured.trim().is_empty() && row.control.trim().is_empty();
                    // Rows line up with the fit's pairs (blank rows are skipped there).
                    let fitted = if blank {
                        None
                    } else {
                        resolved.as_mut().and_then(|r| r.next())
                    };
                    ui.label(RichText::new((i + 1).to_string()).color(theme.text_secondary));
                    let measured_known = row.measured.trim().is_empty()
                        || doc.find_point_by_name(row.measured.trim()).is_some();
                    field(ui, &mut row.measured, "point name", 110.0, measured_known);
                    let control = row.control.trim();
                    let control_known = control.is_empty()
                        || doc.find_point_by_name(control).is_some()
                        || conv.parse_coord(control).is_ok();
                    field(ui, &mut row.control, "name or E,N,Z", 170.0, control_known);
                    ui.checkbox(&mut row.horizontal, "")
                        .on_hover_text("Use in the horizontal fit");
                    let heights = fitted.is_none_or(|(p, _, _)| p.has_heights);
                    ui.add_enabled(heights, egui::Checkbox::without_text(&mut row.vertical))
                        .on_hover_text("Use in the height fit")
                        .on_disabled_hover_text("Both points need a height to be used vertically");
                    // Left-out pairs show their misfit too, dimmed.
                    let cell = |ui: &mut egui::Ui, v: Option<f64>, used: bool, over: bool| {
                        let text = v.map(|v| format!("{v:+.3}")).unwrap_or_default();
                        let color = match (used, over) {
                            (true, true) => theme.error,
                            (true, false) => theme.text,
                            (false, _) => theme.text_disabled,
                        };
                        let label = ui.label(RichText::new(text).font(fonts::mono(12.0)).color(color));
                        if !used && v.is_some() {
                            label.on_hover_text("Not used in the fit: how far this pair is from it");
                        }
                    };
                    let (h, v) = fitted.as_ref().map_or((None, None), |(_, _, i)| {
                        site.map_or((None, None), |s| s.check(*i))
                    });
                    let (h_used, v_used) = fitted
                        .as_ref()
                        .map_or((false, false), |(p, _, _)| (p.pair.horizontal, p.pair.vertical));
                    let h_over = h.is_some_and(|d| d.length() > opts.tolerance_h);
                    let hv = h.map(|d| conv.horizontal_values(d.extend(0.0)));
                    cell(ui, hv.map(|x| x.0), h_used, h_over);
                    cell(ui, hv.map(|x| x.1), h_used, h_over);
                    cell(ui, v, v_used, v.is_some_and(|d| d.abs() > opts.tolerance_v));
                    if IconButton::new(icons::X, "Remove pair")
                        .size(20.0)
                        .show(ui)
                        .clicked()
                    {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = remove {
            self.setup.pairs.remove(i);
            if self.setup.pairs.is_empty() {
                self.setup.pairs.push(new_pair());
            }
        }
        ui.horizontal(|ui| {
            if ui.button(format!("{} Add pair", icons::PLUS)).clicked() {
                self.setup.pairs.push(new_pair());
            }
            let picked: Vec<String> = selection
                .iter()
                .filter_map(|id| match doc.entity(*id) {
                    Some(Entity::Point(p)) => Some(p.name.clone()),
                    _ => None,
                })
                .filter(|n| !self.setup.pairs.iter().any(|r| r.measured.trim() == n))
                .collect();
            let add = ui
                .add_enabled(
                    !picked.is_empty(),
                    egui::Button::new(format!("Add selected points ({})", picked.len())),
                )
                .on_hover_text("A pair for each selected point, as the measured point")
                .on_disabled_hover_text("Select GNSS-measured points in the drawing to add them as pairs");
            if add.clicked() {
                self.setup
                    .pairs
                    .retain(|r| !(r.measured.trim().is_empty() && r.control.trim().is_empty()));
                for name in picked {
                    self.setup.pairs.push(PairSetup {
                        measured: name,
                        ..new_pair()
                    });
                }
            }
        });
    }

    fn options(&mut self, ui: &mut egui::Ui, theme: &CadTheme) {
        let opts = &mut self.setup.options;
        egui::Grid::new("calibration_options")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Scale");
                ui.horizontal(|ui| {
                    segmented(
                        ui,
                        &mut opts.fixed_scale,
                        &[(false, "Free"), (true, "Fixed at 1")],
                    );
                    ui.label(
                        RichText::new("fixed keeps distances as measured")
                            .small()
                            .color(theme.text_secondary),
                    );
                });
                ui.end_row();
                ui.label("Heights");
                let methods: Vec<(VerticalMethod, &str)> =
                    VerticalMethod::ALL.iter().map(|m| (*m, m.label())).collect();
                segmented(ui, &mut opts.vertical, &methods);
                ui.end_row();
                ui.label("Residual limits");
                ui.horizontal(|ui| {
                    ui.label(RichText::new("H").small());
                    ui.add(
                        egui::DragValue::new(&mut opts.tolerance_h)
                            .range(0.001..=10.0)
                            .speed(0.001)
                            .fixed_decimals(3),
                    );
                    ui.label(RichText::new("V").small());
                    ui.add(
                        egui::DragValue::new(&mut opts.tolerance_v)
                            .range(0.001..=10.0)
                            .speed(0.001)
                            .fixed_decimals(3),
                    );
                    ui.label(RichText::new("m").small().color(theme.text_secondary));
                });
                ui.end_row();
            });
    }

    fn apply_row(
        &mut self,
        ui: &mut egui::Ui,
        theme: &CadTheme,
        doc: &Document,
        site: &SiteFit,
        selection: &BTreeSet<EntityId>,
    ) -> Option<CalibrationAction> {
        let mut action = None;
        let target = match self.apply_to {
            ApplyTo::AllExceptControl => Target::AllExceptControl,
            ApplyTo::Selection => Target::Objects(selection.iter().copied().collect()),
        };
        let count = crate::calibration::targets(doc, site, &target).len();
        ui.horizontal(|ui| {
            ui.label("Apply to");
            segmented(
                ui,
                &mut self.apply_to,
                &[
                    (ApplyTo::AllExceptControl, "Everything except control points"),
                    (ApplyTo::Selection, "Selected objects"),
                ],
            );
            ui.label(
                RichText::new(format!("{count} objects"))
                    .small()
                    .color(theme.text_secondary),
            );
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let apply_button = ui
                .add_enabled(count > 0, egui::Button::new(RichText::new("Apply calibration").strong()))
                .on_hover_text("Moves the objects onto the local control (one undoable step) and saves the calibration with the project");
            if apply_button.clicked() {
                match apply(doc, &self.setup, &target) {
                    Ok((cs, summary)) => action = Some(CalibrationAction::Apply(Box::new(cs), summary)),
                    Err(e) => action = Some(CalibrationAction::Report("Site calibration".into(), e)),
                }
            }
            if ui
                .button("Save pairs")
                .on_hover_text("Keep these pairs and options with the project without applying")
                .clicked()
            {
                let mut meta = doc.meta().clone();
                meta.calibration = SiteCalibration {
                    applied: meta.calibration.applied,
                    ..self.setup.clone()
                };
                action = Some(CalibrationAction::Apply(
                    Box::new(ChangeSet::new("Save site calibration pairs").set_meta(meta)),
                    "Site calibration pairs saved".into(),
                ));
            }
            if ui.button("Add to reports").clicked() {
                action = Some(CalibrationAction::Report(
                    "Site calibration".into(),
                    report(doc.conventions(), site, &self.setup),
                ));
            }
        });
        if let Some(applied) = doc.meta().calibration.applied {
            let rotation = format_rotation(doc.conventions(), applied.rotation());
            let scale = applied.horizontal.map_or(1.0, |h| h.scale());
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "A calibration (scale {scale:.8}, rotation {rotation}) has been applied in this project. \
                     Once applied, the measured points sit on the control, so refitting shows a scale of \
                     about 1 and no rotation."
                ))
                .small()
                .color(theme.text_secondary),
            );
        }
        action
    }
}

fn results(ui: &mut egui::Ui, theme: &CadTheme, doc: &Document, site: &SiteFit, setup: &SiteCalibration) {
    let conv = doc.conventions();
    let line = |ui: &mut egui::Ui, label: &str, value: String| {
        ui.horizontal(|ui| {
            ui.add_sized(
                [130.0, 16.0],
                egui::Label::new(RichText::new(label).color(theme.text_secondary)),
            );
            ui.label(RichText::new(value).font(fonts::mono(12.0)));
        });
    };
    let fit = &site.fit;
    if let Some(h) = fit.calibration.horizontal {
        line(
            ui,
            "Scale",
            format!("{:.8}  ({:+.1} ppm)", h.scale(), (h.scale() - 1.0) * 1e6),
        );
        line(ui, "Rotation", format_rotation(conv, h.rotation()));
        let (a, b) = conv.horizontal_values(h.shift().extend(0.0));
        let (ea, nb) = conv.horizontal_labels();
        line(
            ui,
            "Shift",
            format!("{ea} {}  {nb} {}", conv.format_length(a), conv.format_length(b)),
        );
    }
    if let Some(v) = fit.calibration.vertical {
        line(ui, "Height shift", conv.format_length_with_unit(v.constant));
        if fit.vertical_method == Some(VerticalMethod::InclinedPlane) {
            line(
                ui,
                "Tilt",
                format!(
                    "N {:+.1} ppm  E {:+.1} ppm",
                    v.slope_north * 1e6,
                    v.slope_east * 1e6
                ),
            );
        }
    }
    let max = |m: Option<f64>, limit: f64| {
        m.map(|m| {
            let mark = if m > limit { "  over limit" } else { "" };
            format!("{}{mark}", conv.format_length_with_unit(m))
        })
    };
    if let Some(m) = max(fit.max_horizontal(), setup.options.tolerance_h) {
        line(ui, "Max H residual", m);
    }
    if let Some(m) = max(fit.max_vertical(), setup.options.tolerance_v) {
        line(ui, "Max V residual", m);
    }
    if fit.warnings.is_empty() {
        ui.label(RichText::new(format!("{} Within limits", icons::CHECK)).color(theme.success));
    }
    for w in &fit.warnings {
        ui.label(RichText::new(format!("{} {w}", icons::WARNING)).color(theme.warning));
    }
}
