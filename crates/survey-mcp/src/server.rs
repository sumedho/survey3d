//! The tools: one open project in a [`Session`], driven by tool calls.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use egui_cad::command_line::{CommandLineState, LogKind};
use glam::DVec3;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use survey_app::import_command::ImportEntities;
use survey_app::session::Event;
use survey_app::settings::AppSettings;
use survey_app::{Project, Session};
use survey_core::Conventions;
use survey_core::arc::polyline_metrics;
use survey_core::traverse::TraverseRecord;
use survey_io::{CsvLayout, ImportedPoint};
use survey_model::{
    AlignmentRecord, ChangeSet, Color, Document, Entity, EntityId, LayerId, ParcelRecord, Polyline, Record,
    RecordId, SheetRecord, SurveyPoint,
};
use survey_store::ProjectStore;

use crate::render::svg_to_png;

const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 5000;

/// What a tool returns: text and images, and whether it failed.
pub struct ToolResult {
    content: Vec<Content>,
    is_error: bool,
}

enum Content {
    Text(String),
    Png(Vec<u8>),
}

impl ToolResult {
    fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text(text.into())],
            is_error: false,
        }
    }

    fn json(value: &Value) -> Self {
        Self::text(serde_json::to_string_pretty(value).unwrap_or_default())
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text(message.into())],
            is_error: true,
        }
    }

    /// The `tools/call` result object.
    pub fn to_json(&self) -> Value {
        let content: Vec<Value> = self
            .content
            .iter()
            .map(|c| match c {
                Content::Text(text) => json!({ "type": "text", "text": text }),
                Content::Png(bytes) => json!({
                    "type": "image",
                    "data": crate::base64::encode(bytes),
                    "mimeType": "image/png",
                }),
            })
            .collect();
        json!({ "content": content, "isError": self.is_error })
    }
}

type Outcome = Result<ToolResult, String>;

pub struct Options {
    /// Project to open at start (else a new, empty one).
    pub project: Option<PathBuf>,
    /// Never write the project file (exports are still allowed).
    pub read_only: bool,
    /// Recorded in the audit trail for every edit.
    pub user: String,
}

pub struct Server {
    session: Session,
    settings: AppSettings,
    read_only: bool,
}

impl Server {
    pub fn new(options: Options) -> Result<Self, String> {
        let settings = AppSettings {
            user_name: format!("{} (agent via MCP)", options.user),
            ..AppSettings::default()
        };
        let project = match &options.project {
            Some(path) => open(path, &settings)?,
            None => Project::new(&settings).map_err(|e| e.to_string())?,
        };
        Ok(Self {
            session: Session::new(project),
            settings,
            read_only: options.read_only,
        })
    }

    #[cfg(test)]
    pub fn new_for_tests() -> Self {
        Self::new(Options {
            project: None,
            read_only: false,
            user: "tester".into(),
        })
        .expect("server")
    }

    pub fn call(&mut self, name: &str, args: &Value) -> ToolResult {
        let outcome = match name {
            "project_info" => Ok(ToolResult::json(&self.project_info())),
            "run_commands" => self.run_commands(args),
            "query" => self.query(args),
            "select" => self.select(args),
            "add_points" => self.add_points(args),
            "add_polyline" => self.add_polyline(args),
            "set_record" => self.set_record(args),
            "delete_record" => self.delete_record(args),
            "import_file" => self.import_file(args),
            "export_file" => self.export_file(args),
            "render" => self.render(args),
            "new_project" => self.new_project(args),
            "open_project" => self.open_project(args),
            "save_project" => self.save_project(args),
            _ => Err(format!("Unknown tool: {name}")),
        };
        outcome.unwrap_or_else(ToolResult::error)
    }

    fn doc(&self) -> &Document {
        &self.session.project.doc
    }

    /// The command-line output since it was last cleared, and any reports.
    fn drain_output(&mut self) -> String {
        let mut text = String::new();
        for line in self.session.log.lines() {
            let prefix = match line.kind {
                LogKind::Command => "> ",
                LogKind::Error => "error: ",
                LogKind::Warning => "warning: ",
                LogKind::Info | LogKind::Result => "",
            };
            text.push_str(prefix);
            text.push_str(&line.text);
            text.push('\n');
        }
        self.session.log = CommandLineState::default();
        for event in self.session.take_events() {
            if let Event::Report { title, text: report } = event {
                text.push_str(&format!("\n--- Report: {title} ---\n{report}\n"));
            }
        }
        text
    }

    // ------------------------------------------------------------------ project

    fn project_info(&self) -> Value {
        let p = &self.session.project;
        let doc = &p.doc;
        let meta = doc.meta();
        let mut counts = std::collections::HashMap::<LayerId, usize>::new();
        let (mut points, mut polylines) = (0, 0);
        for (_, e) in doc.entities() {
            *counts.entry(e.layer()).or_default() += 1;
            match e {
                Entity::Point(_) => points += 1,
                Entity::Polyline(_) => polylines += 1,
            }
        }
        let bounds = doc.bounds();
        let records = |list: Vec<(RecordId, &str)>| -> Vec<Value> {
            list.into_iter()
                .map(|(id, name)| json!({ "id": id.0, "name": name }))
                .collect()
        };
        json!({
            "file": p.store.file_path().map(|f| f.display().to_string()),
            "unsaved_changes": p.is_dirty(),
            "read_only": self.read_only,
            "name": meta.name,
            "description": meta.description,
            "coordinate_system": meta.coord_system.display_name(),
            "conventions": doc.conventions(),
            "conventions_presets": Conventions::presets().into_iter().map(|c| c.name).collect::<Vec<_>>(),
            "counts": { "points": points, "polylines": polylines },
            "bounds": (!bounds.is_empty()).then(|| json!({
                "min": [bounds.min.x, bounds.min.y, bounds.min.z],
                "max": [bounds.max.x, bounds.max.y, bounds.max.z],
            })),
            "layers": doc.layers().iter().map(|l| json!({
                "id": l.id.0,
                "name": l.name,
                "visible": l.visible,
                "locked": l.locked,
                "current": l.id == doc.current_layer(),
                "objects": counts.get(&l.id).copied().unwrap_or(0),
            })).collect::<Vec<_>>(),
            "surfaces": doc.surfaces().map(|(id, s)| json!({
                "id": id.0, "name": s.name, "points": s.tin.vertices.len(), "triangles": s.tin.triangles.len(),
            })).collect::<Vec<_>>(),
            "clouds": doc.clouds().map(|(id, c)| json!({ "id": id.0, "name": c.name, "points": c.len() })).collect::<Vec<_>>(),
            "records": {
                "sheet": doc.sheets().iter().map(|s| json!({ "id": s.id.0, "name": s.name, "scale": s.scale_label() })).collect::<Vec<_>>(),
                "parcel": records(doc.parcels().iter().map(|r| (r.id, r.name.as_str())).collect()),
                "alignment": records(doc.alignments().iter().map(|r| (r.id, r.name.as_str())).collect()),
                "traverse": records(doc.traverses().iter().map(|r| (r.id, r.name.as_str())).collect()),
                "field_data": records(doc.field_data().iter().map(|r| (r.id, r.name.as_str())).collect()),
            },
            "selected": p.selection.len(),
            "active_tool": self.session.active_tool.id(),
            "prompt": self.session.prompt(),
            "undo": p.history.undo_label(),
            "redo": p.history.redo_label(),
        })
    }

    /// Refuses to drop unsaved work unless asked to.
    fn check_unsaved(&self, args: &Value) -> Result<(), String> {
        if self.session.project.is_dirty() && !flag(args, "discard_changes") {
            return Err(
                "The open project has unsaved changes: save_project first, or pass discard_changes: true"
                    .into(),
            );
        }
        Ok(())
    }

    fn new_project(&mut self, args: &Value) -> Outcome {
        self.check_unsaved(args)?;
        let mut settings = self.settings.clone();
        if let Some(name) = str_arg(args, "conventions") {
            settings.default_conventions = Conventions::presets()
                .into_iter()
                .find(|c| c.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("No conventions preset named '{name}'"))?;
        }
        let project = Project::new(&settings).map_err(|e| e.to_string())?;
        self.session.replace_project(project);
        if let Some(name) = str_arg(args, "name") {
            let mut meta = self.doc().meta().clone();
            meta.name = name.to_string();
            self.session
                .execute(Box::new(ChangeSet::new("Rename project").set_meta(meta)));
        }
        Ok(ToolResult::json(&self.project_info()))
    }

    fn open_project(&mut self, args: &Value) -> Outcome {
        self.check_unsaved(args)?;
        let path = PathBuf::from(required_str(args, "path")?);
        let project = open(&path, &self.settings)?;
        self.session.replace_project(project);
        Ok(ToolResult::json(&self.project_info()))
    }

    fn save_project(&mut self, args: &Value) -> Outcome {
        if self.read_only {
            return Err("This server is read-only (--read-only): the project file is never written".into());
        }
        let project = &mut self.session.project;
        let current = project.store.file_path().map(Path::to_path_buf);
        let saved_to = match str_arg(args, "path") {
            Some(path) => {
                let path = PathBuf::from(path);
                let same = current.as_deref().is_some_and(|c| same_file(c, &path));
                if path.exists() && !same && !flag(args, "overwrite") {
                    return Err(format!(
                        "{} exists: pass overwrite: true to replace it",
                        path.display()
                    ));
                }
                if same {
                    project.save()
                } else {
                    project.save_as(&path)
                }
                .map_err(|e| format!("Save failed: {e}"))?;
                path
            }
            None => {
                let path = current.ok_or("This project has no file yet: give a path (save as)")?;
                project.save().map_err(|e| format!("Save failed: {e}"))?;
                path
            }
        };
        Ok(ToolResult::text(format!("Saved {}", saved_to.display())))
    }

    // ------------------------------------------------------------------ editing

    fn run_commands(&mut self, args: &Value) -> Outcome {
        let lines = args
            .get("lines")
            .and_then(Value::as_array)
            .filter(|l| !l.is_empty())
            .ok_or("lines: give one or more lines to type")?;
        let lines: Vec<&str> = lines
            .iter()
            .map(|l| l.as_str().ok_or("lines: each line is a string"))
            .collect::<Result<_, _>>()?;
        self.session.log = CommandLineState::default();
        for line in lines {
            self.session.run(line);
        }
        let mut text = self.drain_output();
        text.push_str(&format!(
            "\nPrompt: {}\nSelected: {}",
            self.session.prompt(),
            self.session.project.selection.len()
        ));
        Ok(ToolResult::text(text))
    }

    fn select(&mut self, args: &Value) -> Outcome {
        let mode = str_arg(args, "mode").unwrap_or("set");
        if mode == "clear" {
            self.session.project.selection.clear();
            return Ok(ToolResult::json(&json!({ "selected": 0 })));
        }
        let kind = Kind::parse(str_arg(args, "kind").unwrap_or("all"))?;
        let filter = Filter::parse(self.doc(), args.get("filter"))?;
        let doc = self.doc();
        let matched: Vec<EntityId> = doc
            .entities()
            .filter(|(id, e)| kind.accepts(e) && filter.matches(doc, *id, e) && doc.is_entity_editable(*id))
            .map(|(id, _)| id)
            .collect();
        let selection = &mut self.session.project.selection;
        match mode {
            "set" => selection.set(matched.iter().copied()),
            "add" => matched.iter().for_each(|id| selection.insert(*id)),
            "remove" => {
                let drop: HashSet<EntityId> = matched.iter().copied().collect();
                selection.retain(|id| !drop.contains(&id));
            }
            other => return Err(format!("mode: unknown mode '{other}'")),
        }
        Ok(ToolResult::json(
            &json!({ "matched": matched.len(), "selected": selection.len() }),
        ))
    }

    fn add_points(&mut self, args: &Value) -> Outcome {
        let items = args
            .get("points")
            .and_then(Value::as_array)
            .filter(|p| !p.is_empty())
            .ok_or("points: give one or more points")?;
        let mut points = Vec::new();
        for (i, p) in items.iter().enumerate() {
            let num = |key: &str| p.get(key).and_then(Value::as_f64);
            let text = |key: &str| p.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
            let (Some(easting), Some(northing)) = (num("e"), num("n")) else {
                return Err(format!("points[{i}]: e and n are required numbers"));
            };
            points.push(ImportedPoint {
                name: text("name").trim().to_string(),
                easting,
                northing,
                height: num("z"),
                code: text("code"),
                description: text("description"),
            });
        }
        let doc = self.doc();
        let taken: Vec<&str> = points
            .iter()
            .filter(|p| !p.name.is_empty() && doc.find_point_by_name(&p.name).is_some())
            .map(|p| p.name.as_str())
            .collect();
        let warning =
            (!taken.is_empty()).then(|| format!("These names were already in use: {}", taken.join(", ")));
        let first_new = next_entity_id(doc);
        self.session.add_points(&points, str_arg(args, "layer"), "agent");
        let added = self.new_entities(first_new);
        Ok(ToolResult::json(&json!({ "added": added, "warning": warning })))
    }

    fn add_polyline(&mut self, args: &Value) -> Outcome {
        let doc = self.doc();
        let items = args
            .get("vertices")
            .and_then(Value::as_array)
            .filter(|v| v.len() >= 2)
            .ok_or("vertices: give two or more vertices")?;
        let mut vertices = Vec::new();
        for (i, v) in items.iter().enumerate() {
            let position = match v {
                Value::String(name) => {
                    doc.find_point_by_name(name)
                        .ok_or_else(|| format!("vertices[{i}]: no point named '{name}'"))?
                        .1
                        .position
                }
                Value::Array(c) => {
                    let c: Vec<f64> = c.iter().filter_map(Value::as_f64).collect();
                    match c[..] {
                        [e, n] => DVec3::new(e, n, 0.0),
                        [e, n, z] => DVec3::new(e, n, z),
                        _ => return Err(format!("vertices[{i}]: [e, n] or [e, n, z]")),
                    }
                }
                _ => return Err(format!("vertices[{i}]: a point name or [e, n, z]")),
            };
            vertices.push(position);
        }
        let bulges: Vec<f64> = args
            .get("bulges")
            .and_then(Value::as_array)
            .map(|b| b.iter().filter_map(Value::as_f64).collect())
            .unwrap_or_default();
        let line = Polyline {
            name: str_arg(args, "name").unwrap_or_default().to_string(),
            vertices,
            bulges,
            closed: flag(args, "closed"),
            code: str_arg(args, "code").unwrap_or_default().to_string(),
            layer: doc.current_layer(),
        };
        let mut cmd = ImportEntities::new(format!("Add polyline {}", line.name).trim().to_string());
        let layer = cmd.layer_named(doc, str_arg(args, "layer"));
        cmd.add(layer, Entity::Polyline(line));
        let first_new = next_entity_id(doc);
        self.session.execute(Box::new(cmd));
        Ok(ToolResult::json(
            &json!({ "added": self.new_entities(first_new) }),
        ))
    }

    /// Rows for entities created since `first` (ids only grow).
    fn new_entities(&self, first: i64) -> Vec<Value> {
        let doc = self.doc();
        doc.entities()
            .filter(|(id, _)| id.0 >= first)
            .map(|(id, e)| entity_row(doc, id, e))
            .collect()
    }

    fn set_record(&mut self, args: &Value) -> Outcome {
        let kind = required_str(args, "kind")?;
        let id = args.get("id").and_then(Value::as_i64).map(RecordId);
        let patch = args
            .get("record")
            .filter(|r| r.is_object())
            .cloned()
            .ok_or("record: give the record's fields as an object")?;
        let value = match kind {
            "sheet" => self.set_typed::<SheetRecord>(id, patch),
            "parcel" => self.set_typed::<ParcelRecord>(id, patch),
            "alignment" => self.set_typed::<AlignmentRecord>(id, patch),
            "traverse" => self.set_typed::<TraverseRecord>(id, patch),
            other => Err(format!("kind: records of kind '{other}' cannot be set")),
        }?;
        Ok(ToolResult::json(&value))
    }

    /// Creates (`id` `None`) or merges `patch` into a record; returns it with its id.
    fn set_typed<T>(&mut self, id: Option<RecordId>, mut patch: Value) -> Result<Value, String>
    where
        T: Record + Serialize + DeserializeOwned + PartialEq,
    {
        if let Some(fields) = patch.as_object_mut() {
            fields.remove("id");
        }
        let id = match id {
            Some(id) => {
                let current = self
                    .doc()
                    .record::<T>(id)
                    .ok_or_else(|| format!("No {} with id {}", T::KIND.label(), id.0))?;
                let mut value = serde_json::to_value(current).map_err(|e| e.to_string())?;
                merge(&mut value, patch);
                let record: T = serde_json::from_value(value).map_err(|e| format!("record: {e}"))?;
                self.session.update_record(id, record);
                id
            }
            None => {
                let record: T = serde_json::from_value(patch).map_err(|e| format!("record: {e}"))?;
                self.session
                    .add_record(record)
                    .ok_or("The record was not added")?
            }
        };
        let record = self.doc().record::<T>(id).ok_or("The record has gone")?;
        let mut value = serde_json::to_value(record).map_err(|e| e.to_string())?;
        if let Some(fields) = value.as_object_mut() {
            fields.insert("id".into(), json!(id.0));
        }
        Ok(value)
    }

    fn delete_record(&mut self, args: &Value) -> Outcome {
        let kind = required_str(args, "kind")?;
        let id = RecordId(args.get("id").and_then(Value::as_i64).ok_or("id: required")?);
        fn delete<T: Record>(session: &mut Session, id: RecordId) -> Result<String, String> {
            let name = session
                .project
                .doc
                .record::<T>(id)
                .map(|r| r.name().to_string())
                .ok_or_else(|| format!("No {} with id {}", T::KIND.label(), id.0))?;
            session.delete_record::<T>(id);
            Ok(format!("Removed {} {name}", T::KIND.label()))
        }
        let s = &mut self.session;
        let message = match kind {
            "sheet" => delete::<SheetRecord>(s, id),
            "parcel" => delete::<ParcelRecord>(s, id),
            "alignment" => delete::<AlignmentRecord>(s, id),
            "traverse" => delete::<TraverseRecord>(s, id),
            "field_data" => delete::<survey_core::network::FieldData>(s, id),
            other => Err(format!("kind: unknown record kind '{other}'")),
        }?;
        Ok(ToolResult::text(message))
    }

    // ------------------------------------------------------------------ files

    fn import_file(&mut self, args: &Value) -> Outcome {
        let path = PathBuf::from(required_str(args, "path")?);
        let bytes = std::fs::read(&path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let format = match str_arg(args, "format") {
            Some(f) => f.to_string(),
            None => match ext.as_str() {
                "csv" | "txt" | "pts" => "csv",
                "dxf" => "dxf",
                "xml" => "landxml",
                "gsi" | "jxl" => "raw",
                "las" | "xyz" => "cloud",
                _ => return Err(format!("Cannot tell the format of '{name}': pass format")),
            }
            .to_string(),
        };
        self.session.log = CommandLineState::default();
        match format.as_str() {
            "csv" => {
                let layout = CsvLayout::from_letters(str_arg(args, "layout").unwrap_or("PENZC"));
                self.session
                    .import_points_csv(&name, &bytes, layout, str_arg(args, "layer"));
            }
            "dxf" => self.session.import_dxf(&name, &bytes),
            "landxml" => self.session.import_landxml(&name, &bytes),
            "raw" => self.session.import_raw(&name, &bytes),
            "cloud" => self.session.import_cloud(&name, &bytes),
            other => return Err(format!("format: unknown format '{other}'")),
        }
        Ok(ToolResult::text(self.drain_output()))
    }

    /// A record sheet by id, else the first sheet (DXF sizing) or an overview (plots).
    fn sheet(&self, args: &Value, overview: bool) -> Result<Option<SheetRecord>, String> {
        let doc = self.doc();
        match args.get("sheet_id").and_then(Value::as_i64) {
            Some(id) => doc
                .record::<SheetRecord>(RecordId(id))
                .cloned()
                .map(Some)
                .ok_or_else(|| format!("No sheet with id {id}")),
            None if overview => Ok(Some(self.session.overview_sheet())),
            None => Ok(doc.sheets().first().map(|s| s.record.clone())),
        }
    }

    fn export_file(&mut self, args: &Value) -> Outcome {
        let path = PathBuf::from(required_str(args, "path")?);
        if path.exists() && !flag(args, "overwrite") {
            return Err(format!(
                "{} exists: pass overwrite: true to replace it",
                path.display()
            ));
        }
        self.session.log = CommandLineState::default();
        let bytes: Vec<u8> = match required_str(args, "format")? {
            "dxf" => self.session.dxf(self.sheet(args, false)?.as_ref()).0.into_bytes(),
            "landxml" => {
                let (_, xml) = self
                    .session
                    .surfaces_landxml()
                    .ok_or_else(|| self.drain_output())?;
                xml.into_bytes()
            }
            "parcels" => self
                .session
                .parcels_landxml()
                .ok_or_else(|| self.drain_output())?
                .into_bytes(),
            "gpkg" => {
                survey_store::geopackage::export_geopackage(self.doc())
                    .map_err(|e| format!("GeoPackage export failed: {e}"))?
                    .0
            }
            "csv" => {
                let layout = CsvLayout::from_letters(str_arg(args, "layout").unwrap_or("PENZCD"));
                self.session.points_csv(&layout).into_bytes()
            }
            format @ ("pdf" | "svg") => {
                let sheet = self.sheet(args, true)?.ok_or("No sheet")?;
                let plot = self.session.plot(&sheet);
                if format == "pdf" {
                    survey_io::plot::write_pdf(&plot)
                } else {
                    survey_io::plot::write_svg(&plot).into_bytes()
                }
            }
            other => return Err(format!("format: unknown format '{other}'")),
        };
        std::fs::write(&path, &bytes).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
        Ok(ToolResult::text(format!(
            "Wrote {} ({} bytes)",
            path.display(),
            bytes.len()
        )))
    }

    fn render(&mut self, args: &Value) -> Outcome {
        let sheet = self.sheet(args, true)?.ok_or("No sheet")?;
        let width = args
            .get("width")
            .and_then(Value::as_u64)
            .unwrap_or(1600)
            .clamp(200, 4000) as u32;
        let plot = self.session.plot(&sheet);
        let png = svg_to_png(&survey_io::plot::write_svg(&plot), width)?;
        let [w, h] = sheet.page_size();
        Ok(ToolResult {
            content: vec![
                Content::Text(format!(
                    "{}: {}, page {w:.0} × {h:.0} mm",
                    sheet.name,
                    sheet.scale_label()
                )),
                Content::Png(png),
            ],
            is_error: false,
        })
    }

    // ------------------------------------------------------------------ query

    fn query(&mut self, args: &Value) -> Outcome {
        let kind = required_str(args, "kind")?;
        let doc = self.doc();
        let filter = Filter::parse(doc, args.get("filter"))?;
        let rows: Vec<Value> = match kind {
            "points" | "polylines" => {
                let k = Kind::parse(kind)?;
                doc.entities()
                    .filter(|(id, e)| k.accepts(e) && filter.matches(doc, *id, e))
                    .map(|(id, e)| entity_row(doc, id, e))
                    .collect()
            }
            "selection" => self
                .session
                .project
                .selection
                .ids()
                .iter()
                .filter_map(|id| Some((*id, doc.entity(*id)?)))
                .map(|(id, e)| entity_row(doc, id, e))
                .collect(),
            "layers" => doc
                .layers()
                .iter()
                .map(|l| {
                    json!({
                        "id": l.id.0, "name": l.name, "color": color_hex(l.color),
                        "visible": l.visible, "locked": l.locked,
                    })
                })
                .collect(),
            "surfaces" => doc
                .surfaces()
                .map(|(id, s)| {
                    json!({
                        "id": id.0, "name": s.name, "points": s.tin.vertices.len(),
                        "triangles": s.tin.triangles.len(), "definition": s.definition, "style": s.style,
                    })
                })
                .collect(),
            "clouds" => doc
                .clouds()
                .map(|(id, c)| json!({ "id": id.0, "name": c.name, "points": c.len(), "style": c.style }))
                .collect(),
            "sheet" => records::<SheetRecord>(doc, &filter),
            "parcel" => records::<ParcelRecord>(doc, &filter),
            "alignment" => records::<AlignmentRecord>(doc, &filter),
            "traverse" => records::<TraverseRecord>(doc, &filter),
            "field_data" => records::<survey_core::network::FieldData>(doc, &filter),
            "audit" => self.audit_rows()?,
            other => return Err(format!("kind: unknown kind '{other}'")),
        };
        let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_LIMIT, |l| (l as usize).clamp(1, MAX_LIMIT));
        let total = rows.len();
        let page: Vec<Value> = rows.into_iter().skip(offset).take(limit).collect();
        Ok(ToolResult::json(&json!({
            "total": total,
            "offset": offset,
            "returned": page.len(),
            "rows": page,
        })))
    }

    /// Unsaved entries then saved ones, newest first.
    fn audit_rows(&self) -> Result<Vec<Value>, String> {
        let p = &self.session.project;
        let row = |r: &survey_model::AuditRecord| json!({ "time_ms": r.timestamp_ms, "user": r.user, "action": r.action.as_str(), "label": r.label });
        let mut rows: Vec<Value> = p
            .history
            .pending_audit()
            .iter()
            .rev()
            .map(|r| {
                let mut v = row(r);
                v["saved"] = json!(false);
                v
            })
            .collect();
        let saved = p.store.audit_entries(MAX_LIMIT).map_err(|e| e.to_string())?;
        rows.extend(saved.iter().map(|e| {
            let mut v = row(&e.record);
            v["saved"] = json!(true);
            v
        }));
        Ok(rows)
    }
}

// ---------------------------------------------------------------------- helpers

fn open(path: &Path, settings: &AppSettings) -> Result<Project, String> {
    let store =
        ProjectStore::open_file(path).map_err(|e| format!("Could not open {}: {e}", path.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Project::from_store(store, name, settings).map_err(|e| format!("Could not open {}: {e}", path.display()))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    str_arg(args, key).ok_or_else(|| format!("{key}: required"))
}

fn flag(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn next_entity_id(doc: &Document) -> i64 {
    doc.entities().last().map_or(1, |(id, _)| id.0 + 1)
}

/// Merges `patch` into `value`: objects field by field, anything else replaced.
fn merge(value: &mut Value, patch: Value) {
    match (value, patch) {
        (Value::Object(target), Value::Object(patch)) => {
            for (k, v) in patch {
                merge(target.entry(k).or_insert(Value::Null), v);
            }
        }
        (target, patch) => *target = patch,
    }
}

fn color_hex(c: Color) -> String {
    let [r, g, b, _] = c.0;
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn layer_name(doc: &Document, id: LayerId) -> String {
    doc.layer(id).map(|l| l.name.clone()).unwrap_or_default()
}

fn entity_row(doc: &Document, id: EntityId, e: &Entity) -> Value {
    match e {
        Entity::Point(p) => point_row(doc, id, p),
        Entity::Polyline(l) => {
            let (length, length_3d, area) = polyline_metrics(&l.vertices, &l.bulges, l.closed);
            let mut row = Map::new();
            row.insert("type".into(), json!("polyline"));
            row.insert("id".into(), json!(id.0));
            row.insert("name".into(), json!(l.name));
            row.insert("code".into(), json!(l.code));
            row.insert("layer".into(), json!(layer_name(doc, l.layer)));
            row.insert("closed".into(), json!(l.closed));
            row.insert(
                "vertices".into(),
                json!(l.vertices.iter().map(|v| [v.x, v.y, v.z]).collect::<Vec<_>>()),
            );
            if l.bulges.iter().any(|b| *b != 0.0) {
                row.insert("bulges".into(), json!(l.bulges));
            }
            row.insert("length".into(), json!(length));
            row.insert("length_3d".into(), json!(length_3d));
            if l.closed {
                row.insert("area".into(), json!(area.abs()));
            }
            Value::Object(row)
        }
    }
}

fn point_row(doc: &Document, id: EntityId, p: &SurveyPoint) -> Value {
    json!({
        "type": "point",
        "id": id.0,
        "name": p.name,
        "e": p.position.x,
        "n": p.position.y,
        "z": p.height_known.then_some(p.position.z),
        "code": p.code,
        "description": p.description,
        "layer": layer_name(doc, p.layer),
    })
}

fn records<T: Record + Serialize>(doc: &Document, filter: &Filter) -> Vec<Value> {
    doc.record_list::<T>()
        .iter()
        .filter(|r| filter.ids.as_ref().is_none_or(|ids| ids.contains(&r.id.0)))
        .filter(|r| filter.names.as_ref().is_none_or(|n| n.contains(r.record.name())))
        .filter_map(|r| serde_json::to_value(r).ok())
        .collect()
}

#[derive(Clone, Copy)]
enum Kind {
    Points,
    Polylines,
    All,
}

impl Kind {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "points" => Ok(Kind::Points),
            "polylines" => Ok(Kind::Polylines),
            "all" => Ok(Kind::All),
            other => Err(format!("kind: unknown kind '{other}'")),
        }
    }

    fn accepts(self, e: &Entity) -> bool {
        matches!(
            (self, e),
            (Kind::All, _) | (Kind::Points, Entity::Point(_)) | (Kind::Polylines, Entity::Polyline(_))
        )
    }
}

/// Criteria from a `filter` argument.
#[derive(Default)]
struct Filter {
    ids: Option<HashSet<i64>>,
    names: Option<HashSet<String>>,
    code: Option<String>,
    layer: Option<LayerId>,
    within: Option<[f64; 4]>,
}

impl Filter {
    fn parse(doc: &Document, value: Option<&Value>) -> Result<Self, String> {
        let Some(v) = value.filter(|v| !v.is_null()) else {
            return Ok(Self::default());
        };
        let obj = v.as_object().ok_or("filter: an object")?;
        for key in obj.keys() {
            if !["ids", "names", "code", "layer", "within"].contains(&key.as_str()) {
                return Err(format!("filter: unknown field '{key}'"));
            }
        }
        let ids = v
            .get("ids")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_i64).collect());
        let names = v
            .get("names")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
        let layer = match v.get("layer").and_then(Value::as_str) {
            Some(name) => Some(
                doc.layer_by_name(name)
                    .ok_or_else(|| format!("filter: no layer named '{name}'"))?
                    .id,
            ),
            None => None,
        };
        let within = match v.get("within").and_then(Value::as_array) {
            Some(a) => match a.iter().filter_map(Value::as_f64).collect::<Vec<_>>()[..] {
                [x0, y0, x1, y1] => Some([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]),
                _ => return Err("filter.within: [min E, min N, max E, max N]".into()),
            },
            None => None,
        };
        Ok(Self {
            ids,
            names,
            code: v.get("code").and_then(Value::as_str).map(str::to_string),
            layer,
            within,
        })
    }

    fn matches(&self, _doc: &Document, id: EntityId, e: &Entity) -> bool {
        let (name, code) = match e {
            Entity::Point(p) => (&p.name, &p.code),
            Entity::Polyline(l) => (&l.name, &l.code),
        };
        self.ids.as_ref().is_none_or(|ids| ids.contains(&id.0))
            && self.names.as_ref().is_none_or(|n| n.contains(name))
            && self.code.as_ref().is_none_or(|c| c.eq_ignore_ascii_case(code))
            && self.layer.is_none_or(|l| l == e.layer())
            && self.within.is_none_or(|[x0, y0, x1, y1]| {
                let b = e.bounds();
                b.min.x <= x1 && b.max.x >= x0 && b.min.y <= y1 && b.max.y >= y0
            })
    }
}

#[cfg(test)]
mod tests;
