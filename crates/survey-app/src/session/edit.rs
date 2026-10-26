//! Edits: selection operations, project records, surfaces and point clouds.

use glam::DVec3;
use survey_core::field_to_finish::FeatureLibrary;
use survey_core::traverse::TraverseRecord;
use survey_model::{
    AlignmentRecord, ChangeSet, CloudStyle, Color, Document, Entity, EntityId, Layer, ParcelRecord,
    PointCloudId, Polyline, Record, RecordId, Surface, SurfaceDefinition, SurfaceId, SurfaceStyle,
    SurveyPoint,
};

use super::{Event, Panel, Session, Toast};
use crate::import_command::{ImportEntities, LayerRef};

/// Polyline ends closer than this (plan, metres) are joined.
const JOIN_TOLERANCE: f64 = 0.001;

/// A change to a layer's name, colour or line style as one undoable step.
/// Feature codes and surface definitions name their layers, so a rename is
/// carried into them too (otherwise linework would recreate the old layer and
/// surfaces would lose their points). Visibility and locking are display state
/// and are set directly, not through this.
pub fn layer_change(doc: &Document, layer: Layer) -> Result<ChangeSet, String> {
    let name = layer.name.trim().to_string();
    if name.is_empty() {
        return Err("A layer needs a name".into());
    }
    if doc.layer_by_name(&name).is_some_and(|other| other.id != layer.id) {
        return Err(format!("There is already a layer called {name}"));
    }
    let layer = Layer { name, ..layer };
    let Some(old) = doc.layer(layer.id).map(|l| l.name.clone()) else {
        return Ok(ChangeSet::new(format!("Add layer {}", layer.name)).insert_layer(layer));
    };
    let renamed = old != layer.name;
    let label = if renamed {
        format!("Rename layer {old} to {}", layer.name)
    } else {
        format!("Edit layer {}", layer.name)
    };
    let new = layer.name.clone();
    let mut cs = ChangeSet::new(label).update_layer(layer);
    if !renamed {
        return Ok(cs);
    }
    let same = |n: &String| n.eq_ignore_ascii_case(&old);
    let mut meta = doc.meta().clone();
    let mut codes_changed = false;
    for code in &mut meta.feature_codes.codes {
        if same(&code.layer) {
            code.layer = new.clone();
            codes_changed = true;
        }
    }
    if codes_changed {
        cs = cs.set_meta(meta);
    }
    for (id, surface) in doc.surfaces() {
        let Some(def) = &surface.definition else { continue };
        if def.point_layers.iter().chain(&def.breakline_layers).any(same) {
            let mut surface = surface.clone();
            if let Some(def) = &mut surface.definition {
                for n in def.point_layers.iter_mut().chain(&mut def.breakline_layers) {
                    if same(n) {
                        *n = new.clone();
                    }
                }
            }
            cs = cs.update_surface(id, surface);
        }
    }
    Ok(cs)
}

impl Session {
    // ------------------------------------------------------------------ selection

    /// Editable selected polylines.
    pub fn selected_polylines(&self) -> Vec<(EntityId, Polyline)> {
        let doc = &self.project.doc;
        self.project
            .selection
            .ids()
            .iter()
            .filter(|id| doc.is_entity_editable(**id))
            .filter_map(|id| match doc.entity(*id) {
                Some(Entity::Polyline(l)) => Some((*id, l.clone())),
                _ => None,
            })
            .collect()
    }

    /// Closed selected polylines (boundaries, toes).
    pub fn selected_closed(&self) -> Vec<EntityId> {
        self.selected_polylines()
            .into_iter()
            .filter(|(_, l)| l.closed)
            .map(|(id, _)| id)
            .collect()
    }

    pub fn delete_selection(&mut self) {
        let doc = &self.project.doc;
        let ids: Vec<EntityId> = self
            .project
            .selection
            .ids()
            .iter()
            .copied()
            .filter(|id| doc.is_entity_editable(*id))
            .collect();
        if ids.is_empty() {
            self.log.warn("Select objects to delete first");
            return;
        }
        let mut cs = ChangeSet::new(format!("Delete {} objects", ids.len()));
        for id in &ids {
            cs.push_delete_entity(*id);
        }
        self.execute(Box::new(cs));
        self.log.info(format!("Deleted {} objects", ids.len()));
    }

    pub fn explode_selection(&mut self) {
        let lines = self.selected_polylines();
        if lines.is_empty() {
            self.log.warn("Select the polylines to explode first");
            return;
        }
        let mut cs = ChangeSet::new(format!("Explode {} polylines", lines.len()));
        let mut parts = 0;
        for (id, l) in &lines {
            cs.push_delete_entity(*id);
            for part in l.explode() {
                cs.push_insert_entity(Entity::Polyline(part));
                parts += 1;
            }
        }
        self.execute(Box::new(cs));
        self.log.result(format!(
            "Exploded {} polylines into {parts} segments",
            lines.len()
        ));
    }

    pub fn join_selection(&mut self) {
        let lines = self.selected_polylines();
        if lines.len() < 2 {
            self.log.warn("Select two or more polylines to join");
            return;
        }
        let polylines: Vec<_> = lines.iter().map(|(_, l)| l.clone()).collect();
        let joined = survey_model::join_polylines(&polylines, JOIN_TOLERANCE);
        if joined.len() == lines.len() {
            self.log.warn("None of the selected polylines meet end to end");
            return;
        }
        let mut cs = ChangeSet::new(format!("Join {} polylines", lines.len()));
        for (id, _) in &lines {
            cs.push_delete_entity(*id);
        }
        for l in &joined {
            cs.push_insert_entity(Entity::Polyline(l.clone()));
        }
        self.execute(Box::new(cs));
        self.log
            .result(format!("Joined {} polylines into {}", lines.len(), joined.len()));
    }

    pub fn reverse_selection(&mut self) {
        let lines = self.selected_polylines();
        if lines.is_empty() {
            self.log.warn("Select the polylines to reverse first");
            return;
        }
        let mut cs = ChangeSet::new(format!("Reverse {} polylines", lines.len()));
        for (id, mut l) in lines {
            l.reverse();
            cs.push_update_entity(id, Entity::Polyline(l));
        }
        self.execute(Box::new(cs));
    }

    /// A report describing the selection: polyline legs (bearing, distance, arcs)
    /// and point coordinates.
    pub fn list_selection(&mut self) {
        let doc = &self.project.doc;
        let conv = doc.conventions();
        if self.project.selection.is_empty() {
            self.log.warn("Select objects to list first");
            return;
        }
        let mut text = String::new();
        for id in self.project.selection.ids() {
            match doc.entity(*id) {
                Some(Entity::Point(p)) => {
                    text.push_str(&format!(
                        "POINT {:<10} {}  {}  {}\n",
                        p.name,
                        conv.format_coord(p.position, p.height_known),
                        p.code,
                        p.description
                    ));
                }
                Some(Entity::Polyline(l)) => {
                    text.push_str(&crate::listing::polyline_listing(l, conv));
                }
                Some(Entity::Text(t)) => {
                    text.push_str(&format!(
                        "TEXT  \"{}\" at {}\n",
                        t.text.replace('\n', " / "),
                        conv.format_coord(t.position, false)
                    ));
                }
                None => {}
            }
        }
        let n = self.project.selection.len();
        self.report(format!("Listing ({})", crate::tools::objects(n)), text);
    }

    /// Creates the mean of the selected points and reports their spread.
    pub fn average_selection(&mut self) {
        let doc = &self.project.doc;
        let points: Vec<(DVec3, bool)> = self
            .project
            .selection
            .ids()
            .iter()
            .filter_map(|id| match doc.entity(*id) {
                Some(Entity::Point(p)) => Some((p.position, p.height_known)),
                _ => None,
            })
            .collect();
        if points.len() < 2 {
            self.log.warn("Select two or more points to average");
            return;
        }
        let Some(avg) = survey_core::cogo::average(&points) else {
            return;
        };
        let conv = doc.conventions();
        let name = doc.next_point_name();
        let len = |v: f64| conv.format_length(v);
        let (ea, nb) = conv.horizontal_labels();
        let mut spread = format!(
            "spread {} plan (sd {ea} {} {nb} {})",
            len(avg.max_plan),
            len(avg.sd.x),
            len(avg.sd.y)
        );
        if avg.height_known {
            spread.push_str(&format!(
                ", {} height (sd {})",
                len(avg.max_height),
                len(avg.sd.z)
            ));
        }
        let summary = format!(
            "Average of {} points → {name}: {}; {spread}",
            points.len(),
            conv.format_coord(avg.position, avg.height_known)
        );
        let point = SurveyPoint {
            name: name.clone(),
            position: avg.position,
            height_known: avg.height_known,
            code: "AVG".into(),
            description: format!("Average of {} points", points.len()),
            layer: doc.current_layer(),
        };
        self.execute(Box::new(
            ChangeSet::new(format!("Average point {name}")).insert_entity(Entity::Point(point)),
        ));
        self.log.result(summary);
    }

    // ------------------------------------------------------------------ field to finish

    /// Stores `library` in the project if it differs (one undoable change).
    pub fn apply_feature_codes(&mut self, library: FeatureLibrary) {
        if self.project.doc.meta().feature_codes == library {
            return;
        }
        let mut meta = self.project.doc.meta().clone();
        meta.feature_codes = library;
        self.execute(Box::new(ChangeSet::new("Edit feature codes").set_meta(meta)));
    }

    /// Stores `library` and strings the coded points into linework.
    pub fn process_linework(&mut self, library: FeatureLibrary, only_selected: bool, assign_layers: bool) {
        if library.codes.is_empty() {
            self.log
                .warn("The feature code library is empty — open it with 'codes'");
            self.emit(Event::Open(Panel::FeatureCodes));
            return;
        }
        self.apply_feature_codes(library);
        let selection = self.project.selection.ids().clone();
        let options = crate::field_to_finish::LineworkOptions {
            only: only_selected.then_some(&selection),
            assign_layers,
        };
        let (cmd, summary) = crate::field_to_finish::build_linework(
            &self.project.doc,
            &self.project.doc.meta().feature_codes,
            &options,
        );
        if !cmd.is_empty() {
            self.execute(Box::new(cmd));
        }
        self.log.result(summary.describe());
        if summary.unknown.is_empty() {
            self.emit(Event::Toast(Toast::Success, format!("{} strings", summary.lines)));
        } else {
            self.emit(Event::Toast(
                Toast::Warning,
                "Some codes are not in the library — see the command line".into(),
            ));
        }
    }

    // ------------------------------------------------------------------ records

    /// Adds a record (a sheet, parcel…); returns its id.
    pub fn add_record<T: Record>(&mut self, record: T) -> Option<RecordId> {
        let label = format!("Add {} {}", T::KIND.label(), record.name());
        self.execute(Box::new(ChangeSet::new(label).insert_record(record)));
        // New records go at the end of their list.
        self.project.doc.record_list::<T>().last().map(|r| r.id)
    }

    /// Replaces a record; nothing happens if it has gone or is unchanged.
    pub fn update_record<T: Record + PartialEq>(&mut self, id: RecordId, record: T) {
        if self.project.doc.record::<T>(id).is_none_or(|old| *old == record) {
            return;
        }
        let label = format!("Edit {} {}", T::KIND.label(), record.name());
        self.execute(Box::new(ChangeSet::new(label).update_record(id, record)));
    }

    pub fn delete_record<T: Record>(&mut self, id: RecordId) {
        let Some(old) = self.project.doc.record::<T>(id) else {
            return;
        };
        let label = format!("Remove {} {}", T::KIND.label(), old.name());
        self.execute(Box::new(ChangeSet::new(label).delete_record(T::KIND, id)));
    }

    /// Saves a traverse under its name, replacing one of the same name.
    pub fn save_traverse(&mut self, record: TraverseRecord) {
        let name = record.name.clone();
        let existing = self
            .project
            .doc
            .traverses()
            .iter()
            .find(|r| r.name == name)
            .map(|r| r.id);
        let cs = ChangeSet::new(format!("Save traverse {name}"));
        let cs = match existing {
            Some(id) => cs.update_record(id, record),
            None => cs.insert_record(record),
        };
        self.execute(Box::new(cs));
        self.log
            .result(format!("Traverse '{name}' saved with the project"));
    }

    pub fn delete_traverse(&mut self, name: &str) {
        if let Some(id) = self
            .project
            .doc
            .traverses()
            .iter()
            .find(|r| r.name == name)
            .map(|r| r.id)
        {
            self.delete_record::<TraverseRecord>(id);
        }
    }

    /// Makes a parcel of each selected closed polyline that is not one already.
    pub fn parcels_from_selection(&mut self) {
        let doc = &self.project.doc;
        let existing: Vec<EntityId> = doc.parcels().iter().map(|p| p.boundary).collect();
        let lines: Vec<EntityId> = self
            .selected_closed()
            .into_iter()
            .filter(|id| !existing.contains(id))
            .collect();
        if lines.is_empty() {
            self.log
                .warn("Select closed polylines (lot boundaries) that are not parcels yet");
            return;
        }
        // Continue the "Lot n" numbering.
        let mut next = doc
            .parcels()
            .iter()
            .filter_map(|p| {
                p.name
                    .strip_prefix("Lot ")
                    .and_then(|n| n.trim().parse::<u32>().ok())
            })
            .max()
            .unwrap_or(0)
            + 1;
        let n = lines.len();
        let mut cs = ChangeSet::new(format!("Create {n} parcels"));
        for id in lines {
            let name = match doc.entity(id) {
                Some(Entity::Polyline(l)) if !l.name.trim().is_empty() => l.name.trim().to_string(),
                _ => {
                    let name = format!("Lot {next}");
                    next += 1;
                    name
                }
            };
            cs = cs.insert_record(ParcelRecord {
                name,
                boundary: id,
                ..Default::default()
            });
        }
        self.execute(Box::new(cs));
        self.emit(Event::Open(Panel::Parcels));
        self.log
            .result(format!("Created {n} parcel{}", if n == 1 { "" } else { "s" }));
    }

    /// Makes an alignment of the first selected polyline.
    pub fn alignment_from_selection(&mut self) {
        let Some((id, line)) = self.selected_polylines().into_iter().next() else {
            self.log.warn("Select the centreline polyline first");
            return;
        };
        self.emit(Event::Open(Panel::Alignments));
        let doc = &self.project.doc;
        if doc.alignments().iter().any(|a| a.polyline == id) {
            self.log.info("That polyline is already an alignment");
            return;
        }
        let name = if line.name.trim().is_empty() {
            format!("MC{:02}", doc.alignments().len() + 1)
        } else {
            line.name.trim().to_string()
        };
        self.execute(Box::new(
            ChangeSet::new(format!("Create alignment {name}")).insert_record(AlignmentRecord {
                name: name.clone(),
                polyline: id,
                ..Default::default()
            }),
        ));
        self.log.result(format!("Alignment {name} created"));
    }

    /// Adds set-out points along an alignment, with their table as a report.
    pub fn create_setout(
        &mut self,
        alignment: RecordId,
        pts: Vec<survey_core::alignment::SetoutPoint>,
        code: String,
    ) {
        let doc = &self.project.doc;
        let Some(a) = doc.record::<AlignmentRecord>(alignment) else {
            return;
        };
        let Some(line) = crate::ui::alignments::centreline(doc, a) else {
            return;
        };
        let conv = doc.conventions();
        let heights = line.vertices.iter().any(|v| v.z != 0.0);
        let mut names = doc.point_names();
        let mut cs = ChangeSet::new(format!("Set out {} ({} points)", a.name, pts.len()));
        let mut point_names = Vec::new();
        for p in &pts {
            let name = names.next().unwrap_or_default();
            cs.push_insert_entity(Entity::Point(SurveyPoint {
                name: name.clone(),
                position: if heights {
                    p.position
                } else {
                    p.position.truncate().extend(0.0)
                },
                height_known: heights,
                code: code.clone(),
                description: format!(
                    "{} Ch {} {}",
                    a.name,
                    conv.format_chainage(p.chainage),
                    crate::ui::alignments::offset_label(conv, p.offset)
                ),
                layer: doc.current_layer(),
            }));
            point_names.push(name);
        }
        let text = crate::ui::alignments::setout_report(conv, &a.name, &pts, &point_names);
        let title = format!("Set-out {}", a.name);
        let n = pts.len();
        self.execute(Box::new(cs));
        self.report(title, text);
        self.log
            .result(format!("Set out {n} points (table added to the reports)"));
    }

    // ------------------------------------------------------------------ surfaces

    /// The surface commands act on: the active one, else the only one.
    pub fn target_surface(&mut self) -> Option<SurfaceId> {
        let doc = &self.project.doc;
        if let Some(id) = self.active_surface.filter(|id| doc.surface(*id).is_some()) {
            return Some(id);
        }
        let mut ids = doc.surfaces().map(|(id, _)| id);
        match (ids.next(), ids.next()) {
            (Some(id), None) => Some(id),
            (None, _) => {
                self.log
                    .warn("Create a surface first (Surface → Create surface…)");
                None
            }
            _ => {
                self.log.warn("Select a surface in the Surfaces panel first");
                None
            }
        }
    }

    /// Adds a surface and makes it the active one.
    pub fn create_surface(&mut self, surface: Surface) {
        let summary = format!(
            "Surface {}: {} points, {} triangles",
            surface.name,
            surface.tin.vertices.len(),
            surface.tin.triangles.len()
        );
        self.warn_surface(&surface);
        let cmd = ChangeSet::new(format!("Create surface {}", surface.name)).insert_surface(surface);
        self.execute(Box::new(cmd));
        // New surfaces get the highest id.
        self.active_surface = self.project.doc.surfaces().last().map(|(id, _)| id);
        self.success(summary);
    }

    /// Builds a surface from `definition` and adds it as the active surface.
    pub fn build_surface(
        &mut self,
        name: &str,
        definition: SurfaceDefinition,
        style: SurfaceStyle,
    ) -> Result<SurfaceId, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A surface needs a name".into());
        }
        if self.project.doc.surface_by_name(name).is_some() {
            return Err(format!("There is already a surface called {name}"));
        }
        let tin = crate::ui::surfaces::build_from(&self.project.doc, &definition)
            .map_err(|e| format!("Surface {name}: {e}"))?;
        self.create_surface(Surface {
            name: name.to_string(),
            definition: Some(definition),
            tin,
            style,
        });
        self.active_surface
            .ok_or_else(|| "The surface was not added".into())
    }

    /// Renames, restyles or redefines a surface (rebuilding it from its
    /// definition when that changes, or when `rebuild`) in one undo step.
    pub fn change_surface(
        &mut self,
        id: SurfaceId,
        name: Option<String>,
        definition: Option<SurfaceDefinition>,
        style: Option<SurfaceStyle>,
        rebuild: bool,
    ) -> Result<(), String> {
        let doc = &self.project.doc;
        let mut surface = doc.surface(id).ok_or("No such surface")?.clone();
        if let Some(name) = name.map(|n| n.trim().to_string()).filter(|n| *n != surface.name) {
            if doc.surface_by_name(&name).is_some_and(|(other, _)| other != id) {
                return Err(format!("There is already a surface called {name}"));
            }
            surface.name = name;
        }
        let rebuild = rebuild || definition.is_some();
        if let Some(definition) = definition {
            surface.definition = Some(definition);
        }
        if rebuild {
            let definition = surface
                .definition
                .as_ref()
                .ok_or("This surface was imported, not built from the drawing, so it cannot be rebuilt")?;
            surface.tin = crate::ui::surfaces::build_from(doc, definition)
                .map_err(|e| format!("Surface {}: {e}", surface.name))?;
        }
        if let Some(style) = style {
            surface.style = style;
        }
        if rebuild {
            self.warn_surface(&surface);
        }
        let doc = &self.project.doc;
        if doc.surface(id) != Some(&surface) {
            let label = format!("Change surface {}", surface.name);
            self.execute(Box::new(ChangeSet::new(label).update_surface(id, surface)));
        }
        Ok(())
    }

    /// Logs what a surface's definition selects but could not use (missing
    /// heights, a deleted boundary…).
    fn warn_surface(&mut self, surface: &Surface) {
        if let Some(def) = &surface.definition {
            for w in def.warnings(&self.project.doc) {
                self.log.warn(format!("Surface {}: {w}", surface.name));
            }
        }
    }

    pub fn update_surface(&mut self, id: SurfaceId, surface: Surface) {
        self.warn_surface(&surface);
        let label = format!("Rebuild surface {}", surface.name);
        self.execute(Box::new(ChangeSet::new(label).update_surface(id, surface)));
    }

    fn surface_name(&self, id: SurfaceId) -> String {
        self.project
            .doc
            .surface(id)
            .map(|s| s.name.clone())
            .unwrap_or_default()
    }

    pub fn delete_surface(&mut self, id: SurfaceId) {
        let name = self.surface_name(id);
        self.execute(Box::new(
            ChangeSet::new(format!("Delete surface {name}")).delete_surface(id),
        ));
    }

    pub fn set_surface_style(&mut self, id: SurfaceId, style: SurfaceStyle) {
        let name = self.surface_name(id);
        self.execute(Box::new(
            ChangeSet::new(format!("Surface {name} display")).set_surface_style(id, style),
        ));
    }

    /// Rebuilds a surface from its definition (after the points or breaklines changed).
    pub fn rebuild_surface(&mut self, id: SurfaceId) {
        let doc = &self.project.doc;
        let Some(s) = doc.surface(id) else {
            return;
        };
        let Some(def) = &s.definition else {
            return;
        };
        match crate::ui::surfaces::build_from(doc, def) {
            Ok(tin) => {
                let summary = format!(
                    "Surface {}: {} points, {} triangles",
                    s.name,
                    tin.vertices.len(),
                    tin.triangles.len()
                );
                let mut surface = s.clone();
                surface.tin = tin;
                self.update_surface(id, surface);
                self.log.result(summary);
            }
            Err(e) => self.log.error(format!("Surface {}: {e}", s.name)),
        }
    }

    /// Adds the active surface's contours (at its contour interval) as polylines.
    pub fn contours_to_polylines(&mut self) {
        let Some(id) = self.target_surface() else {
            return;
        };
        let doc = &self.project.doc;
        let Some(s) = doc.surface(id) else {
            return;
        };
        let contours = crate::surface_view::surface_contours(s);
        if contours.is_empty() {
            self.log.warn("No contours at this interval");
            return;
        }
        let mut cmd = ImportEntities::new(format!("Contours of {}", s.name));
        let mut layer = |name: &str, color: Color| match doc.layer_by_name(name) {
            Some(l) => LayerRef::Existing(l.id),
            None => cmd.add_layer(name.to_string(), color, true),
        };
        let minor = layer("CONTOUR MINOR", Color::rgb(160, 130, 100));
        let major = layer("CONTOUR MAJOR", Color::rgb(200, 160, 110));
        let n = contours.len();
        for c in contours {
            let line = Polyline {
                name: format!("Contour {}", doc.conventions().format_length(c.z)),
                vertices: c.points,
                bulges: Vec::new(),
                closed: c.closed,
                code: String::new(),
                layer: doc.current_layer(),
                heights_known: true,
            };
            cmd.add(if c.major { major } else { minor }, Entity::Polyline(line));
        }
        let summary = format!("Contours: added {n} polylines from {}", s.name);
        self.execute(Box::new(cmd));
        self.log.result(summary);
    }

    // ------------------------------------------------------------------ point clouds

    fn cloud_name(&self, id: PointCloudId) -> String {
        self.project
            .doc
            .cloud(id)
            .map(|c| c.name.clone())
            .unwrap_or_default()
    }

    pub fn set_cloud_style(&mut self, id: PointCloudId, style: CloudStyle) {
        let name = self.cloud_name(id);
        self.execute(Box::new(
            ChangeSet::new(format!("Point cloud {name} display")).set_cloud_style(id, style),
        ));
    }

    pub fn delete_cloud(&mut self, id: PointCloudId) {
        let name = self.cloud_name(id);
        self.execute(Box::new(
            ChangeSet::new(format!("Delete point cloud {name}")).delete_cloud(id),
        ));
    }

    /// Replaces a cloud with one point per `spacing` cube.
    pub fn thin_cloud(&mut self, id: PointCloudId, spacing: f64) {
        let Some(c) = self.project.doc.cloud(id) else {
            return;
        };
        let t = c.thinned(spacing);
        let summary = format!("Thinned {} to {} points ({} m)", c.name, t.len(), spacing);
        self.execute(Box::new(
            ChangeSet::new(format!("Thin point cloud {}", c.name))
                .delete_cloud(id)
                .insert_cloud(t),
        ));
        if let Some((id, _)) = self.project.doc.clouds().last() {
            self.emit(Event::SelectCloud(id));
        }
        self.success(summary);
    }

    /// A surface from a cloud thinned to a grid of `cell` metres.
    pub fn surface_from_cloud(
        &mut self,
        id: PointCloudId,
        cell: f64,
        thin: survey_core::tin::Thin,
        max_edge: Option<f64>,
    ) {
        let Some(c) = self.project.doc.cloud(id) else {
            return;
        };
        let points = survey_core::tin::thin_to_grid(c.iter(), cell, thin);
        let n = points.len();
        let input = survey_core::tin::TinInput {
            points,
            max_edge,
            ..Default::default()
        };
        match survey_core::tin::build(&input) {
            Ok(tin) => {
                let surface = Surface {
                    name: format!("{} surface", c.name),
                    definition: None,
                    tin,
                    style: SurfaceStyle::default(),
                };
                self.log.info(format!(
                    "Thinned {} cloud points to {n} on a {cell} m grid",
                    c.len()
                ));
                self.create_surface(surface);
            }
            Err(e) => self.log.error(format!("Surface from {}: {e}", c.name)),
        }
    }
}
