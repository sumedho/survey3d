//! Road and construction alignments: the element table (with spiral
//! transitions), points at chainages and offsets, the chainage and offset of
//! points, set-out points, and the centreline drawn with its transitions.

use serde_json::{Value, json};
use survey_app::compute::alignment::{centreline, element_report, geometry, offset_label};
use survey_core::alignment::{ElementKind, Geometry};
use survey_core::{Angle, Conventions};
use survey_model::{AlignmentRecord, ChangeSet, Entity, RecordId, SurveyPoint};

use super::drawing::position;
use super::{Outcome, Server, ToolResult, flag, next_entity_id, str_arg};

fn round(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

/// A number, or text in the project's units.
fn length(conv: &Conventions, v: &Value, key: &str) -> Result<f64, String> {
    match v {
        Value::Number(n) => n.as_f64().ok_or_else(|| format!("{key}: a number")),
        Value::String(s) => conv.parse_length(s).map_err(|e| format!("{key}: {e}")),
        _ => Err(format!("{key}: a length")),
    }
}

fn elements_json(conv: &Conventions, g: &Geometry) -> Vec<Value> {
    g.elements
        .iter()
        .map(|e| {
            let mut row = json!({
                "start_chainage": round(e.start_chainage),
                "end_chainage": round(e.end_chainage()),
                "length": round(e.length),
                "start": [round(e.start.x), round(e.start.y)],
                "end": [round(e.end.x), round(e.end.y)],
            });
            match e.kind {
                ElementKind::Straight { bearing } => {
                    row["kind"] = json!("straight");
                    row["bearing"] = json!(conv.format_bearing(bearing));
                }
                ElementKind::Arc {
                    radius,
                    right,
                    delta,
                    centre,
                    ..
                } => {
                    row["kind"] = json!("arc");
                    row["radius"] = json!(round(radius));
                    row["turn"] = json!(if right { "right" } else { "left" });
                    row["delta"] = json!(conv.format_angle(Angle::from_radians(delta)));
                    row["centre"] = json!([round(centre.x), round(centre.y)]);
                }
                ElementKind::Spiral {
                    radius,
                    right,
                    entering,
                    ..
                } => {
                    let sp = survey_core::spiral::Spiral::new(radius, e.length);
                    row["kind"] = json!(if entering { "spiral_in" } else { "spiral_out" });
                    row["radius"] = json!(round(radius));
                    row["turn"] = json!(if right { "right" } else { "left" });
                    row["a"] = json!(round(sp.a));
                    row["theta_s"] = json!(conv.format_angle(Angle::from_radians(sp.theta)));
                    row["shift_p"] = json!(round(sp.p));
                }
            }
            row
        })
        .collect()
}

impl Server {
    /// The alignment named or numbered by `alignment`, else the only one.
    fn alignment_record(&self, args: &Value) -> Result<(RecordId, AlignmentRecord), String> {
        let list = self.doc().alignments();
        let found = match args.get("alignment") {
            Some(Value::Number(n)) => list.iter().find(|a| Some(a.id.0) == n.as_i64()),
            Some(Value::String(name)) => list.iter().find(|a| a.name == *name),
            None | Some(Value::Null) if list.len() == 1 => list.first(),
            None | Some(Value::Null) => {
                return Err(format!(
                    "alignment: its name or id ({} alignments; make one with run_commands 'mkalign' on a selected polyline)",
                    list.len()
                ));
            }
            Some(_) => return Err("alignment: its name or id".into()),
        };
        found
            .map(|a| (a.id, a.record.clone()))
            .ok_or_else(|| "alignment: no such alignment".to_string())
    }

    pub(super) fn alignment(&mut self, args: &Value) -> Outcome {
        let (id, rec) = self.alignment_record(args)?;
        let doc = self.doc();
        let conv = doc.conventions().clone();
        let g = geometry(doc, &rec).ok_or("The alignment's centreline polyline has gone")??;
        match str_arg(args, "action").unwrap_or("elements") {
            "elements" => {
                let names = g.key_point_names();
                let key_points: Vec<Value> = names
                    .iter()
                    .filter_map(|(ch, name)| {
                        let (p, b) = g.point_at(*ch, 0.0)?;
                        Some(json!({
                            "name": name, "chainage": round(*ch),
                            "e": round(p.x), "n": round(p.y), "bearing": conv.format_bearing(b),
                        }))
                    })
                    .collect();
                Ok(ToolResult::json(&json!({
                    "alignment": rec.name,
                    "start_chainage": round(g.start_chainage),
                    "end_chainage": round(g.end_chainage()),
                    "length": round(g.length()),
                    "transitions": rec.transitions,
                    "elements": elements_json(&conv, &g),
                    "key_points": key_points,
                    "report": element_report(doc, &rec),
                })))
            }
            "at" => {
                let stations = args
                    .get("stations")
                    .and_then(Value::as_array)
                    .ok_or("stations: [[chainage, offset], …] (offset positive right, negative left)")?;
                let mut rows = Vec::new();
                for (i, s) in stations.iter().enumerate() {
                    let (ch, off) = match s {
                        Value::Array(a) => (
                            length(
                                &conv,
                                a.first().unwrap_or(&Value::Null),
                                &format!("stations[{i}]"),
                            )?,
                            a.get(1)
                                .map(|v| length(&conv, v, &format!("stations[{i}]")))
                                .transpose()?
                                .unwrap_or(0.0),
                        ),
                        v => (length(&conv, v, &format!("stations[{i}]"))?, 0.0),
                    };
                    let (p, b) = g
                        .point_at(ch, off)
                        .ok_or_else(|| format!("stations[{i}]: chainage {ch} is off the alignment"))?;
                    rows.push((ch, off, p, b));
                }
                let positions: Vec<Value> = rows
                    .iter()
                    .map(|(ch, off, p, b)| {
                        json!({ "chainage": round(*ch), "offset": round(*off), "e": round(p.x), "n": round(p.y),
                                "z": round(p.z), "bearing": conv.format_bearing(*b) })
                    })
                    .collect();
                if !flag(args, "create") {
                    return Ok(ToolResult::json(&json!({ "positions": positions })));
                }
                let first = next_entity_id(doc);
                let mut names = doc.point_names();
                let code = str_arg(args, "code").unwrap_or("SO").to_string();
                // The centreline's heights carry to the points when it has them.
                let heights = centreline(doc, &rec).is_some_and(|l| l.heights_known);
                let mut cs = ChangeSet::new(format!("{} points on {}", rows.len(), rec.name));
                for (ch, off, p, _) in &rows {
                    cs.push_insert_entity(Entity::Point(SurveyPoint {
                        name: names.next().unwrap_or_default(),
                        position: if heights { *p } else { p.truncate().extend(0.0) },
                        height_known: heights,
                        code: code.clone(),
                        description: format!(
                            "{} Ch {} {}",
                            rec.name,
                            conv.format_chainage(*ch),
                            offset_label(&conv, *off)
                        ),
                        layer: doc.current_layer(),
                    }));
                }
                let n = rows.len();
                self.session
                    .apply(Box::new(cs), format!("Created {n} points on {}", rec.name));
                Ok(ToolResult::json(
                    &json!({ "positions": positions, "created": self.new_entities(first) }),
                ))
            }
            "station" => {
                let list = args
                    .get("points")
                    .and_then(Value::as_array)
                    .ok_or("points: point names or [e, n] positions")?;
                let mut rows = Vec::new();
                for (i, v) in list.iter().enumerate() {
                    let p = position(doc, Some(v), &format!("points[{i}]"))?;
                    let (ch, off) = g
                        .station_offset(p.truncate())
                        .ok_or("The alignment has no elements")?;
                    let mut row = json!({ "point": v, "chainage": round(ch), "offset": round(off) });
                    // Off the ends the nearest point is the start or end, not square to the line.
                    if let Some((q, _)) = g.point_at(ch, off) {
                        let gap = q.truncate().distance(p.truncate());
                        if gap > 1e-3 {
                            let end = if ch - g.start_chainage < g.end_chainage() - ch {
                                "start"
                            } else {
                                "end"
                            };
                            row["beyond"] = json!(end);
                            row["distance_beyond"] = json!(round(gap));
                        }
                    }
                    rows.push(row);
                }
                Ok(ToolResult::json(
                    &json!({ "alignment": rec.name, "stations": rows }),
                ))
            }
            "setout" => {
                let interval = args
                    .get("interval")
                    .map(|v| length(&conv, v, "interval"))
                    .transpose()?
                    .unwrap_or(10.0);
                let offsets: Vec<f64> = match args.get("offsets") {
                    Some(Value::Array(a)) => a
                        .iter()
                        .map(|v| length(&conv, v, "offsets"))
                        .collect::<Result<_, _>>()?,
                    None | Some(Value::Null) => vec![0.0],
                    Some(_) => return Err("offsets: a list (negative left, positive right)".into()),
                };
                let key_points = args.get("key_points").and_then(Value::as_bool).unwrap_or(true);
                let pts = g.setout(interval, &offsets, key_points);
                if pts.is_empty() {
                    return Err("No set-out points: check the interval and offsets".into());
                }
                let rows: Vec<Value> = pts
                    .iter()
                    .map(|p| {
                        json!({ "chainage": round(p.chainage), "offset": round(p.offset), "e": round(p.position.x),
                                "n": round(p.position.y), "z": round(p.position.z), "key_point": p.key_point,
                                "key": (!p.key.is_empty()).then_some(p.key) })
                    })
                    .collect();
                if !flag(args, "create") {
                    return Ok(ToolResult::json(&json!({ "points": rows })));
                }
                let first = next_entity_id(doc);
                let code = str_arg(args, "code").unwrap_or("SO").to_string();
                self.clear_output();
                self.session.create_setout(id, pts, code);
                Ok(ToolResult::json(&json!({
                    "created": self.new_entities(first),
                    "log": self.drain_output(),
                })))
            }
            "draw_centreline" => {
                let first = next_entity_id(doc);
                let summary = self.session.draw_centreline(id)?;
                // The polyline's id and name; its vertices are many (query to see them).
                let created: Vec<Value> = self
                    .new_entities(first)
                    .into_iter()
                    .map(|e| json!({ "id": e["id"], "name": e["name"], "length": e["length"] }))
                    .collect();
                Ok(ToolResult::json(
                    &json!({ "summary": summary, "created": created }),
                ))
            }
            other => Err(format!(
                "action: '{other}' (elements, at, station, setout or draw_centreline)"
            )),
        }
    }
}
