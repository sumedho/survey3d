//! Windows and dialogs, and what their actions do.

use egui::RichText;

use crate::platform::FileKind;
use crate::ui::alignments::AlignmentsAction;
use crate::ui::parcels::ParcelsAction;
use crate::ui::sections::SectionsAction;
use crate::ui::sheets::SheetsAction;
use crate::ui::volumes::VolumesAction;

use super::*;

impl SurveyApp {
    // ------------------------------------------------------------------ dialogs

    /// Raw data, stake-out files and as-built checks.
    fn field_windows(&mut self, ctx: &egui::Context) {
        match self.raw_window.show(ctx, &self.session.project.doc) {
            Some(RawAction::Edit(file, edit, revert)) => {
                if let Err(e) = self.session.edit_raw(&file, edit, revert) {
                    self.session.log.error(e);
                }
            }
            Some(RawAction::Reduce(file)) => match self.session.reduce_raw(&file) {
                Ok(summary) => self.toasts.success(summary),
                Err(e) => self.session.log.error(e),
            },
            Some(RawAction::Report(file)) => {
                if let Some((title, text)) = self.session.raw_report(&file) {
                    self.reports.add(title, text);
                }
            }
            Some(RawAction::Network(file)) => {
                self.network_window.open_with(&self.session.project.doc);
                self.network_window.select_only(format!("f:{file}"));
            }
            None => {}
        }
        let selection: Vec<EntityId> = self.session.project.selection.ids().iter().copied().collect();
        if self.stake_export_window.open {
            let doc = &self.session.project.doc;
            let count = |f: fn(&Entity) -> bool| {
                selection
                    .iter()
                    .filter(|id| doc.entity(**id).is_some_and(f))
                    .count()
            };
            let points = count(|e| matches!(e, Entity::Point(_)));
            let lines = count(|e| matches!(e, Entity::Polyline(_)));
            if let Some(format) = self.stake_export_window.show(ctx, points, lines) {
                self.export_stakeout(format, &selection);
            }
        }
        match self
            .asbuilt_window
            .show(ctx, &self.session.project.doc, &selection)
        {
            Some(AsBuiltAction::Report(title, text)) => self.reports.add(title, text),
            Some(AsBuiltAction::Label(cmd, summary)) => self.session.apply_with_toast(cmd, summary),
            None => {}
        }
    }

    pub(super) fn export_stakeout(&mut self, format: survey_io::stakeout::StakeFormat, ids: &[EntityId]) {
        use survey_io::stakeout::StakeFormat;
        let (text, summary) = match self.session.stakeout_file(format, ids) {
            Ok(v) => v,
            Err(e) => {
                self.session.log.error(e);
                return;
            }
        };
        let kind = match format {
            StakeFormat::JobXml => FileKind::StakeJobXml,
            StakeFormat::Rw5 => FileKind::StakeRw5,
            StakeFormat::Gsi => FileKind::StakeGsi,
            StakeFormat::Csv => FileKind::Points,
        };
        let stem = format!("{}-stakeout", file_stem(&self.session.project.doc.meta().name));
        match export(&self.platform, kind, &stem, text.as_bytes()) {
            Ok(Some(where_)) => self.session.log.result(format!("{summary}: {where_}")),
            Ok(None) => {}
            Err(e) => self
                .session
                .log
                .error(format!("Could not save the stake-out file: {e}")),
        }
    }

    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        self.recovery_prompt(ctx);

        if let Some(dialog) = &mut self.csv_dialog {
            match dialog.show(ctx, &self.session.project.doc) {
                CsvDialogResult::Open => {}
                CsvDialogResult::Cancelled => {
                    self.settings.csv_layout = dialog.layout().clone();
                    self.csv_dialog = None;
                }
                CsvDialogResult::Import(cmd, n) => {
                    self.settings.csv_layout = dialog.layout().clone();
                    self.csv_dialog = None;
                    self.session.execute(cmd);
                    self.session.log.info(format!("Imported {n} points"));
                    self.toasts.success(format!("Imported {n} points"));
                    self.zoom_extents();
                }
            }
        }

        if let Some(cmd) = self.settings_window.show(ctx, &mut self.settings, &self.session) {
            self.session.execute(cmd);
        }
        if std::mem::take(&mut self.settings_window.retry_geoid) {
            self.session.retry_geoid();
        }
        self.audit_window.show(ctx, &self.session.project);
        match self.traverse_window.show(ctx, &self.session.project.doc) {
            Some(TraverseAction::Create(cmd, summary)) => self.session.apply_with_toast(cmd, summary),
            Some(TraverseAction::Report(title, text)) => self.reports.add(title, text),
            Some(TraverseAction::Save(record)) => self.session.save_traverse(record),
            Some(TraverseAction::Delete(name)) => self.session.delete_traverse(&name),
            None => {}
        }
        let selected_parcel = self
            .subdivide_window
            .open
            .then(|| self.session.selected_polylines().into_iter().next())
            .flatten();
        match self.subdivide_window.show(
            ctx,
            &self.session.project.doc,
            selected_parcel.as_ref().map(|(id, l)| (*id, l)),
        ) {
            Some(SubdivideAction::Draw(cmd, summary)) => self.session.apply(cmd, summary),
            Some(SubdivideAction::Report(title, text)) => self.reports.add(title, text),
            None => {}
        }
        let closed: Vec<EntityId> =
            if self.surface_window.open || self.volumes_window.open || self.stockpile_window.open {
                self.session.selected_closed()
            } else {
                Vec::new()
            };
        if let Some(a) = self.surface_window.show(ctx, &self.session.project.doc, &closed) {
            self.apply_surface_window(a);
        }
        let can_create = self.parcels_window.open && !self.session.selected_closed().is_empty();
        match self
            .parcels_window
            .show(ctx, &self.session.project.doc, can_create)
        {
            Some(ParcelsAction::Update(id, record)) => self.session.update_record(id, record),
            Some(ParcelsAction::Delete(id)) => self.session.delete_record::<ParcelRecord>(id),
            Some(ParcelsAction::Select(id)) if self.session.project.doc.contains(id) => {
                self.session.project.selection.set([id]);
            }
            Some(ParcelsAction::Report(title, text)) => self.reports.add(title, text),
            Some(ParcelsAction::CreateFromSelection) => self.session.parcels_from_selection(),
            Some(ParcelsAction::ExportLandXml) => self.export_parcels_landxml(),
            Some(ParcelsAction::Select(_)) | None => {}
        }
        let one_line = self.alignments_window.open && !self.session.selected_polylines().is_empty();
        match self
            .alignments_window
            .show(ctx, &self.session.project.doc, one_line)
        {
            Some(AlignmentsAction::Create) => self.session.alignment_from_selection(),
            Some(AlignmentsAction::Update(id, record)) => self.session.update_record(id, record),
            Some(AlignmentsAction::Delete(id)) => self.session.delete_record::<AlignmentRecord>(id),
            Some(AlignmentsAction::Select(id)) if self.session.project.doc.contains(id) => {
                self.session.project.selection.set([id]);
            }
            Some(AlignmentsAction::Report(title, text)) => self.reports.add(title, text),
            Some(AlignmentsAction::SetOut(id, pts, code)) => self.session.create_setout(id, pts, code),
            Some(AlignmentsAction::DrawCentreline(id)) => {
                if let Err(e) = self.session.draw_centreline(id) {
                    self.session.log.error(e);
                }
            }
            Some(AlignmentsAction::Select(_)) | None => {}
        }
        let targets: Vec<EntityId> = if self.stakeout_window.open {
            self.session.project.selection.ids().iter().copied().collect()
        } else {
            Vec::new()
        };
        if let Some((title, text)) = self
            .stakeout_window
            .show(ctx, &self.session.project.doc, &targets)
        {
            self.reports.add(title, text);
        }
        self.field_windows(ctx);
        let selected_line = self
            .sections_window
            .open
            .then(|| self.session.selected_polylines().first().map(|(id, _)| *id))
            .flatten();
        if let Some(SectionsAction::Report(title, text)) =
            self.sections_window
                .show(ctx, &self.session.project.doc, selected_line)
        {
            self.reports.add(title, text);
        }
        let selected_point = self
            .session
            .project
            .selection
            .ids()
            .iter()
            .copied()
            .find(|id| matches!(self.session.project.doc.entity(*id), Some(Entity::Point(_))));
        match self.network_window.show(ctx, &self.session.project.doc) {
            Some(NetworkAction::Apply(cmd, summary)) => self.session.apply_with_toast(cmd, summary),
            Some(NetworkAction::Report(title, text)) => self.reports.add(title, text),
            None => {}
        }
        match self
            .projection_window
            .show(ctx, &self.session.project.doc, selected_point)
        {
            Some(ProjectionAction::Create(cmd, summary)) => self.session.apply(cmd, summary),
            Some(ProjectionAction::Report(title, text)) => self.reports.add(title, text),
            None => {}
        }
        if let Some((title, text)) = self
            .stockpile_window
            .show(ctx, &self.session.project.doc, &closed)
        {
            self.reports.add(title, text);
        }
        match self.volumes_window.show(ctx, &self.session.project.doc, &closed) {
            Some(VolumesAction::Report(title, text)) => self.reports.add(title, text),
            Some(VolumesAction::CreateSurface(surface)) => self.session.create_surface(*surface),
            None => {}
        }
        self.legend.show(ctx, &self.session.project.doc);
        let view = self.view_info();
        let labels = self.plan_labels();
        match self
            .sheets_window
            .show(ctx, &self.session.project.doc, &view, labels)
        {
            Some(SheetsAction::Add(sheet)) => {
                if let Some(id) = self.session.add_record(sheet) {
                    self.sheets_window.current = Some(id);
                }
            }
            Some(SheetsAction::Update(id, sheet)) => self.session.update_record(id, sheet),
            Some(SheetsAction::Delete(id)) => self.session.delete_record::<SheetRecord>(id),
            Some(SheetsAction::Export(sheet, format)) => self.export_sheet(&sheet, format),
            None => {}
        }
        if let Some((title, text)) = self.calculator.show(ctx, self.session.project.doc.conventions()) {
            self.reports.add(title, text);
        }
        match self.calibration_window.show(
            ctx,
            &self.session.project.doc,
            self.session.project.selection.ids(),
        ) {
            Some(CalibrationAction::Apply(cmd, summary)) => self.session.apply(cmd, summary),
            Some(CalibrationAction::Report(title, text)) => self.reports.add(title, text),
            None => {}
        }
        match self.resection_window.show(ctx, &self.session.project.doc) {
            Some(ResectionAction::Create(cmd, summary)) => self.session.apply(cmd, summary),
            Some(ResectionAction::Report(title, text)) => self.reports.add(title, text),
            None => {}
        }
        let selected_line = self
            .closure_window
            .open
            .then(|| {
                self.session
                    .selected_polylines()
                    .into_iter()
                    .next()
                    .map(|(_, l)| l)
            })
            .flatten();
        match self
            .closure_window
            .show(ctx, &self.session.project.doc, selected_line.as_ref())
        {
            Some(ClosureAction::Draw(cmd, summary)) => self.session.apply(cmd, summary),
            Some(ClosureAction::Report(title, text)) => self.reports.add(title, text),
            None => {}
        }
        match self
            .points_window
            .show(ctx, &self.session.project.doc, &self.session.project.selection)
        {
            Some(PointsAction::Select { id, toggle }) => {
                if toggle {
                    self.session.project.selection.toggle(id);
                } else {
                    self.session.project.selection.set([id]);
                }
            }
            Some(PointsAction::ZoomTo(id)) => {
                if let Some(e) = self.session.project.doc.entity(id) {
                    self.viewport.camera.target = e.bounds().center();
                    self.session.project.selection.set([id]);
                }
            }
            Some(PointsAction::Edit(cmd)) => self.session.execute(cmd),
            Some(PointsAction::Error(e)) => self.session.log.error(e),
            None => {}
        }
        let has_selection = !self.session.project.selection.is_empty();
        match self
            .codes_window
            .show(ctx, &self.session.project.doc, has_selection)
        {
            Some(CodesAction::Apply(library)) => self.session.apply_feature_codes(library),
            Some(CodesAction::Process {
                library,
                only_selected,
                assign_layers,
            }) => self
                .session
                .process_linework(library, only_selected, assign_layers),
            Some(CodesAction::Import) => self.platform.pick_file(ctx, FileKind::Codes),
            Some(CodesAction::Export(text)) => {
                match export(&self.platform, FileKind::Codes, "feature-codes", text.as_bytes()) {
                    Ok(Some(where_)) => self.session.log.info(format!("Saved code sheet: {where_}")),
                    Ok(None) => {}
                    Err(e) => self.session.log.error(format!("Could not save code sheet: {e}")),
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
                Ok(Some(where_)) => self.session.log.info(format!("Saved report: {where_}")),
                Ok(None) => {}
                Err(e) => self.session.log.error(format!("Could not save report: {e}")),
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        if self.copc_window.open {
            self.copc_window.project_wkt = self.session.project.doc.meta().coord_system.wkt();
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(action) = self.copc_window.show(ctx, self.settings.data_dir.as_deref()) {
            match action {
                crate::ui::copc::CopcAction::SetDataDir(dir) => {
                    self.session
                        .log
                        .info(format!("Data directory: {}", dir.display()));
                    self.settings.data_dir = Some(dir);
                }
                crate::ui::copc::CopcAction::Load(path) => self.session.import_copc(&path),
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
                ui.label(format!(
                    "{} has unsaved changes.",
                    self.session.project.display_name
                ));
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
                decision = Some(!self.session.project.is_dirty())
                    .filter(|ok| *ok)
                    .or(Some(false));
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
            self.restore_snapshot(&bytes, None);
        } else {
            wasm_bindgen_futures::spawn_local(async {
                let _ = crate::platform::opfs::remove(crate::platform::opfs::AUTOSAVE).await;
            });
        }
    }
}

impl SurveyApp {
    /// Replaces the project with a crash-recovery snapshot (unsaved work).
    /// `from` names the project file the work was on, if any.
    fn restore_snapshot(&mut self, bytes: &[u8], from: Option<&str>) {
        let name = match from {
            Some(f) => format!(
                "Recovered {}",
                std::path::Path::new(f)
                    .file_name()
                    .map_or_else(|| f.to_string(), |n| n.to_string_lossy().into_owned())
            ),
            None => "Recovered".into(),
        };
        let result = ProjectStore::from_bytes(bytes)
            .and_then(|store| Project::from_store(store, name, &self.settings));
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
                let hint = match from {
                    Some(f) => format!(" — save to keep them (Save As over {f} to replace that file)"),
                    None => " — save to keep them (Ctrl+S)".into(),
                };
                self.session
                    .log
                    .warn(format!("Recovered {}{hint}", parts.join(", ")));
            }
            Err(e) => self.session.log.error(format!("Recovery failed: {e}")),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl SurveyApp {
    /// Offers back unsaved work from a session that stopped unexpectedly
    /// (one snapshot at a time, newest first).
    fn recovery_prompt(&mut self, ctx: &egui::Context) {
        let Some(found) = self.crash.pending.first().cloned() else {
            return;
        };
        let what = match &found.source {
            Some(f) => format!(
                "unsaved changes to {}",
                crate::platform::file_name(std::path::Path::new(f))
            ),
            None => "an unsaved new project".into(),
        };
        let ago = found
            .modified
            .elapsed()
            .map(|d| match d.as_secs() {
                s if s < 120 => "moments ago".to_string(),
                s if s < 7200 => format!("{} minutes ago", s / 60),
                s if s < 172_800 => format!("{} hours ago", s / 3600),
                s => format!("{} days ago", s / 86_400),
            })
            .unwrap_or_default();
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("crash_recovery")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading("Recover unsaved work?");
            ui.label(format!(
                "survey3d closed unexpectedly with {what} (last snapshot {ago})."
            ));
            if let Some(f) = &found.source {
                ui.label(
                    RichText::new(format!("The file {f} itself was not changed."))
                        .small()
                        .color(self.theme.text_secondary),
                );
            }
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
        self.crash.pending.remove(0);
        if recover {
            match found.read() {
                Ok(bytes) => self.restore_snapshot(&bytes, found.source.as_deref()),
                Err(e) => self.session.log.error(format!("Recovery failed: {e}")),
            }
        }
        found.discard();
    }
}
