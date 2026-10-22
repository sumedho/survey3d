//! Text, and changing objects already in the drawing.

use glam::DVec3;
use serde_json::{Value, json};
use survey_app::import_command::ImportEntities;
use survey_model::{Entity, EntityId, TextAlign, TextNote};

use super::{Filter, Kind, Outcome, Server, ToolResult, next_entity_id, str_arg};

/// Fields `edit_entities` can set, and the kinds of object each applies to.
const EDITABLE: &[(&str, &str)] = &[
    ("name", "points, polylines"),
    ("code", "points, polylines"),
    ("description", "points"),
    ("closed", "polylines"),
    ("layer", "all"),
    ("text", "text"),
    ("height_mm", "text"),
    ("rotation", "text"),
    ("align", "text"),
    ("e", "one point or text"),
    ("n", "one point or text"),
    ("z", "one point (null: height unknown)"),
];

impl Server {
    pub(super) fn add_text(&mut self, args: &Value) -> Outcome {
        let items = args
            .get("texts")
            .and_then(Value::as_array)
            .filter(|t| !t.is_empty())
            .ok_or("texts: give one or more texts")?;
        let doc = self.doc();
        let mut notes = Vec::new();
        for (i, t) in items.iter().enumerate() {
            let text = t
                .get("text")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| format!("texts[{i}].text: required"))?;
            let position = match t.get("at") {
                Some(Value::String(name)) => {
                    doc.find_point_by_name(name)
                        .ok_or_else(|| format!("texts[{i}].at: no point named '{name}'"))?
                        .1
                        .position
                }
                Some(Value::Array(c)) => match c.iter().filter_map(Value::as_f64).collect::<Vec<_>>()[..] {
                    [e, n] => DVec3::new(e, n, 0.0),
                    [e, n, z] => DVec3::new(e, n, z),
                    _ => return Err(format!("texts[{i}].at: [e, n] or a point name")),
                },
                _ => return Err(format!("texts[{i}].at: [e, n] or a point name")),
            };
            let height = t
                .get("height_mm")
                .and_then(Value::as_f64)
                .unwrap_or(TextNote::DEFAULT_HEIGHT);
            if !(0.5..=50.0).contains(&height) {
                return Err(format!("texts[{i}].height_mm: between 0.5 and 50"));
            }
            let align = match t.get("align").and_then(Value::as_str) {
                Some(a) => {
                    TextAlign::parse(a).ok_or_else(|| format!("texts[{i}].align: left, centre or right"))?
                }
                None => TextAlign::Centre,
            };
            notes.push(TextNote {
                text: text.to_string(),
                position,
                height,
                rotation: t
                    .get("rotation")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
                    .rem_euclid(360.0),
                align,
                layer: doc.current_layer(),
            });
        }
        let label = match &notes[..] {
            [one] => format!("Add text \"{}\"", one.text.lines().next().unwrap_or_default()),
            many => format!("Add {} texts", many.len()),
        };
        let mut cmd = ImportEntities::new(label);
        let layer = cmd.layer_named(doc, str_arg(args, "layer"));
        for note in notes {
            cmd.add(layer, Entity::Text(note));
        }
        let first_new = next_entity_id(doc);
        self.session.execute(Box::new(cmd));
        Ok(ToolResult::json(
            &json!({ "added": self.new_entities(first_new) }),
        ))
    }

    pub(super) fn edit_entities(&mut self, args: &Value) -> Outcome {
        let set = args
            .get("set")
            .and_then(Value::as_object)
            .filter(|s| !s.is_empty())
            .ok_or("set: give the fields to change")?;
        for key in set.keys() {
            if !EDITABLE.iter().any(|(k, _)| k == key) {
                let fields: Vec<String> = EDITABLE
                    .iter()
                    .map(|(k, applies)| format!("{k} ({applies})"))
                    .collect();
                return Err(format!(
                    "set: unknown field '{key}'; fields are {}",
                    fields.join(", ")
                ));
            }
        }
        if args
            .get("filter")
            .is_none_or(|f| f.as_object().is_none_or(|o| o.is_empty()))
        {
            return Err("filter: say which objects to change (ids, names, code, layer or within)".into());
        }
        let doc = self.doc();
        let kind = Kind::parse(str_arg(args, "kind").unwrap_or("all"))?;
        let filter = Filter::parse(doc, args.get("filter"))?;
        let targets: Vec<(EntityId, Entity)> = doc
            .entities()
            .filter(|(id, e)| kind.accepts(e) && filter.matches(doc, *id, e))
            .map(|(id, e)| (id, e.clone()))
            .collect();
        if targets.is_empty() {
            return Err("No objects match the filter".into());
        }
        let moving = ["e", "n", "z"].iter().any(|k| set.contains_key(*k));
        if moving && targets.len() != 1 {
            return Err(format!(
                "e, n and z move one object; the filter matches {} (use run_commands 'move' for several)",
                targets.len()
            ));
        }
        let text = |key: &str| set.get(key).and_then(Value::as_str).map(str::to_string);
        let number = |key: &str| -> Result<Option<f64>, String> {
            match set.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v.as_f64().map(Some).ok_or_else(|| format!("set.{key}: a number")),
            }
        };
        let align = match text("align") {
            Some(a) => Some(TextAlign::parse(&a).ok_or("set.align: left, centre or right")?),
            None => None,
        };
        let height = number("height_mm")?;
        if height.is_some_and(|h| !(0.5..=50.0).contains(&h)) {
            return Err("set.height_mm: between 0.5 and 50".into());
        }
        let (e, n, rotation) = (number("e")?, number("n")?, number("rotation")?);
        let closed = set.get("closed").and_then(Value::as_bool);

        let label = match &targets[..] {
            [(_, one)] => format!("Edit {} {}", one.kind_name().to_lowercase(), one.name()),
            many => format!("Edit {} objects", many.len()),
        };
        let mut cmd = ImportEntities::new(label.trim().to_string());
        let layer_name = text("layer");
        let mut changed = 0;
        for (id, mut entity) in targets.iter().cloned() {
            let before = entity.clone();
            match &mut entity {
                Entity::Point(p) => {
                    if let Some(v) = text("name") {
                        p.name = v.trim().to_string();
                    }
                    if let Some(v) = text("code") {
                        p.code = v.trim().to_string();
                    }
                    if let Some(v) = text("description") {
                        p.description = v;
                    }
                    p.position.x = e.unwrap_or(p.position.x);
                    p.position.y = n.unwrap_or(p.position.y);
                    match set.get("z") {
                        Some(Value::Null) => {
                            p.height_known = false;
                            p.position.z = 0.0;
                        }
                        Some(z) => {
                            p.position.z = z.as_f64().ok_or("set.z: a number, or null for unknown")?;
                            p.height_known = true;
                        }
                        None => {}
                    }
                }
                Entity::Polyline(l) => {
                    if let Some(v) = text("name") {
                        l.name = v.trim().to_string();
                    }
                    if let Some(v) = text("code") {
                        l.code = v.trim().to_string();
                    }
                    if let Some(c) = closed {
                        if c && l.vertices.len() < 3 {
                            return Err(format!("Polyline {} has too few vertices to close", id.0));
                        }
                        l.closed = c;
                    }
                }
                Entity::Text(t) => {
                    if let Some(v) = text("text").filter(|v| !v.trim().is_empty()) {
                        t.text = v;
                    }
                    t.height = height.unwrap_or(t.height);
                    t.rotation = rotation.map_or(t.rotation, |r| r.rem_euclid(360.0));
                    t.align = align.unwrap_or(t.align);
                    t.position.x = e.unwrap_or(t.position.x);
                    t.position.y = n.unwrap_or(t.position.y);
                }
            }
            let layer = match &layer_name {
                Some(name) => cmd.layer_named(self.doc(), Some(name)),
                None => survey_app::import_command::LayerRef::Existing(entity.layer()),
            };
            let moves_layer = match layer {
                survey_app::import_command::LayerRef::Existing(l) => l != before.layer(),
                survey_app::import_command::LayerRef::New(_) => true,
            };
            if entity != before || moves_layer {
                cmd.update(id, layer, entity);
                changed += 1;
            }
        }
        if changed > 0 {
            self.session.execute(Box::new(cmd));
        }
        let doc = self.doc();
        let rows: Vec<Value> = targets
            .iter()
            .filter_map(|(id, _)| Some(super::entity_row(doc, *id, doc.entity(*id)?)))
            .collect();
        Ok(ToolResult::json(&json!({ "changed": changed, "objects": rows })))
    }
}
