//! Field data: raw data files (show, reduce, correct, report) and as-built
//! comparison against the design.

use serde_json::{Value, json};
use survey_app::asbuilt::{Against, compare, labels};
use survey_core::network::{FieldData, RawEdit};
use survey_model::{Document, Entity, EntityId};

use super::{Outcome, Server, ToolResult, str_arg};

fn round(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

/// A 1-based index argument as a 0-based one.
fn index(args: &Value, key: &str) -> Result<Option<usize>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_u64() {
            Some(n) if n >= 1 => Ok(Some(n as usize - 1)),
            _ => Err(format!("{key}: a number from 1")),
        },
    }
}

fn setup_json(f: &FieldData, i: usize, with_shots: bool) -> Value {
    let s = &f.setups[i];
    let se = f.edit_for(i, None);
    let mut out = json!({
        "setup": i + 1,
        "station": se.and_then(|e| e.station.clone()).unwrap_or_else(|| s.station.clone()),
        "instrument_height": se.and_then(|e| e.instrument_height).unwrap_or(s.instrument_height),
        "backsight": se.and_then(|e| e.backsight.clone()).or_else(|| s.backsight.clone()),
        "orientation_degrees": s.orientation.map(|b| b.degrees()),
        "shots": s.observations.len(),
    });
    if let Some(e) = se {
        out["recorded"] = json!({
            "station": s.station,
            "instrument_height": s.instrument_height,
            "backsight": s.backsight,
        });
        out["corrected"] = json!(e.describe(s, None));
    }
    if with_shots {
        let shots: Vec<Value> = s
            .observations
            .iter()
            .enumerate()
            .map(|(k, o)| {
                let e = f.edit_for(i, Some(k));
                let mut row = json!({
                    "shot": k + 1,
                    "target": e.and_then(|e| e.target.clone()).unwrap_or_else(|| o.target.clone()),
                    "code": e.and_then(|e| e.code.clone()).unwrap_or_else(|| o.code.clone()),
                    "target_height": e.and_then(|e| e.target_height).unwrap_or(o.target_height),
                    "hz_degrees": o.hz.map(|a| round(a.degrees() * 1e2) / 1e2),
                    "zenith_degrees": o.zenith.map(|a| round(a.degrees() * 1e2) / 1e2),
                    "distance": o.distance.map(round),
                    "excluded": e.and_then(|e| e.excluded).unwrap_or(false),
                });
                if let Some(e) = e {
                    row["corrected"] = json!(e.describe(s, Some(o)));
                }
                row
            })
            .collect();
        out["shot_list"] = json!(shots);
    }
    out
}

impl Server {
    pub(super) fn raw_data(&mut self, args: &Value) -> Outcome {
        let file = str_arg(args, "file").unwrap_or("");
        let action = str_arg(args, "action").unwrap_or("show");
        let setup = index(args, "setup")?;
        let shot = index(args, "shot")?;
        match action {
            "show" => {
                let f = survey_app::raw::find(self.doc(), file)?;
                let setups: Vec<Value> = match setup {
                    Some(i) if i < f.setups.len() => vec![setup_json(f, i, true)],
                    Some(i) => {
                        return Err(format!(
                            "There is no setup {} (the file has {})",
                            i + 1,
                            f.setups.len()
                        ));
                    }
                    None => (0..f.setups.len()).map(|i| setup_json(f, i, false)).collect(),
                };
                Ok(ToolResult::json(&json!({
                    "file": f.name,
                    "format": f.format,
                    "scale_factor": f.scale_factor.unwrap_or(1.0),
                    "warnings": f.warnings,
                    "corrections": f.edits.len(),
                    "reduced_points": f.reduced.len(),
                    "setups": setups,
                    "hint": if setup.is_none() { "Pass setup (from 1) to list its shots." } else { "" },
                })))
            }
            "reduce" => {
                let summary = self.session.reduce_raw(file)?;
                let doc = self.doc();
                let f = survey_app::raw::find(doc, file)?;
                let r = survey_app::raw::reduce_command(doc, f.id, f.record.clone(), String::new()).reduction;
                Ok(ToolResult::json(&json!({
                    "summary": summary,
                    "points": f.reduced.len(),
                    "setups": r.setups.iter().enumerate().map(|(i, s)| json!({
                        "setup": i + 1,
                        "station": s.station,
                        "oriented_on": match &s.method {
                            Some(survey_core::reduction::Orientation::Recorded) => json!("recorded orientation"),
                            Some(survey_core::reduction::Orientation::Backsight(b)) => json!(format!("backsight {b}")),
                            Some(survey_core::reduction::Orientation::Assumed) => json!("assumed: readings taken as bearings"),
                            None => json!("not reduced: no station coordinates"),
                        },
                        "shots": s.shots,
                    })).collect::<Vec<_>>(),
                    "checks": r.checks.iter().map(|c| json!({
                        "setup": c.setup + 1,
                        "point": c.target,
                        "de": round(c.delta.x),
                        "dn": round(c.delta.y),
                        "plan": round(c.delta.length()),
                        "dz": c.dz.map(round),
                        "backsight": c.backsight,
                    })).collect::<Vec<_>>(),
                    "warnings": f.warnings.iter().chain(&r.warnings).collect::<Vec<_>>(),
                    "next": "Run run_commands 'linework' to string the coded points into lines.",
                })))
            }
            "edit" | "revert" => {
                let setup = setup.ok_or("setup is required (from 1)")?;
                let text = |key: &str| str_arg(args, key).map(str::to_string);
                let num = |key: &str| args.get(key).and_then(Value::as_f64);
                let edit = RawEdit {
                    setup,
                    observation: shot,
                    station: text("station"),
                    instrument_height: num("instrument_height"),
                    backsight: text("backsight"),
                    target: text("target"),
                    code: text("code"),
                    target_height: num("target_height"),
                    excluded: args.get("excluded").and_then(Value::as_bool),
                };
                let revert = action == "revert";
                if !revert
                    && edit
                        == (RawEdit {
                            setup,
                            observation: shot,
                            ..Default::default()
                        })
                {
                    return Err(
                        "Give a field to correct: station, instrument_height, backsight (setup) or target, code, \
                         target_height, excluded (shot)"
                            .into(),
                    );
                }
                let summary = self.session.edit_raw(file, edit, revert)?;
                Ok(ToolResult::text(summary))
            }
            "report" => {
                let (title, text) = self
                    .session
                    .raw_report(file)
                    .ok_or_else(|| survey_app::raw::find(self.doc(), file).err().unwrap_or_default())?;
                Ok(ToolResult::text(format!("{title}\n\n{text}")))
            }
            other => Err(format!(
                "action: '{other}' is not show, reduce, edit, revert or report"
            )),
        }
    }

    pub(super) fn compare_asbuilt(&mut self, args: &Value) -> Outcome {
        let doc = self.doc();
        let shots: Vec<EntityId> = match args.get("shots") {
            Some(Value::Array(list)) => list
                .iter()
                .map(|v| entity(doc, v, "shots", true))
                .collect::<Result<_, _>>()?,
            Some(_) => return Err("shots: a list of point names or ids".into()),
            None => self
                .session
                .project
                .selection
                .ids()
                .iter()
                .copied()
                .filter(|id| matches!(doc.entity(*id), Some(Entity::Point(_))))
                .collect(),
        };
        let against = match str_arg(args, "against").unwrap_or("points") {
            "points" => Against::Points {
                strip: str_arg(args, "strip").unwrap_or("").to_string(),
                radius: args.get("radius").and_then(Value::as_f64),
            },
            "lines" => {
                let lines: Vec<EntityId> = match args.get("lines") {
                    Some(Value::Array(list)) => list
                        .iter()
                        .map(|v| entity(doc, v, "lines", false))
                        .collect::<Result<_, _>>()?,
                    _ => return Err("lines: the design polylines, by id or name".into()),
                };
                Against::Lines(lines)
            }
            "surface" => Against::Surface(self.surface_id(args)?),
            other => return Err(format!("against: '{other}' is not points, lines or surface")),
        };
        let tolerance_h = args.get("tolerance_h").and_then(Value::as_f64).unwrap_or(0.02);
        let tolerance_v = args.get("tolerance_v").and_then(Value::as_f64).unwrap_or(0.02);
        let c = compare(doc, &shots, &against, tolerance_h, tolerance_v)?;
        let rows: Vec<Value> = c
            .rows
            .iter()
            .map(|r| {
                json!({
                    "shot": r.name,
                    "against": r.reference,
                    "de": r.delta.map(|d| round(d.x)),
                    "dn": r.delta.map(|d| round(d.y)),
                    "plan": r.plan().map(round),
                    "chainage": r.chainage.map(round),
                    "offset": r.offset.map(round),
                    "dz": r.dz.map(round),
                    "over": r.over,
                })
            })
            .collect();
        let mut out = json!({
            "summary": c.summary(),
            "against": c.against,
            "tolerance": { "h": tolerance_h, "v": tolerance_v },
            "compared": c.rows.len(),
            "over": c.over(),
            "max_plan": c.max_plan().map(round),
            "max_dz": c.max_dz().map(round),
            "dz_mean_rms": c.dz_stats().map(|(m, r)| [round(m), round(r)]),
            "rows": rows,
            "not_compared": c.unmatched.iter().map(|(n, why)| json!({ "shot": n, "why": why })).collect::<Vec<_>>(),
        });
        if args.get("report").and_then(Value::as_bool).unwrap_or(false) {
            out["report"] = json!(survey_app::asbuilt::report(doc, &c));
        }
        if args.get("label").and_then(Value::as_bool).unwrap_or(false) {
            let layer = str_arg(args, "layer").unwrap_or("AS-BUILT").to_string();
            let cmd = labels(doc, &c, &layer);
            let summary = format!("Labelled {} as-built shots on {layer}", cmd.len());
            self.session.apply(Box::new(cmd), summary.clone());
            out["labelled"] = json!(summary);
        }
        Ok(ToolResult::json(&out))
    }
}

/// A point (`point`) or polyline by id or name.
fn entity(doc: &Document, v: &Value, key: &str, point: bool) -> Result<EntityId, String> {
    let kind = if point { "point" } else { "polyline" };
    let found = match v {
        Value::Number(n) => n
            .as_i64()
            .map(EntityId)
            .and_then(|id| Some((id, doc.entity(id)?))),
        Value::String(name) => doc.entities().find(|(_, e)| match e {
            Entity::Point(p) => point && p.name == *name,
            Entity::Polyline(l) => !point && l.name == *name,
            _ => false,
        }),
        _ => None,
    };
    match found {
        Some((id, Entity::Point(_))) if point => Ok(id),
        Some((id, Entity::Polyline(_))) if !point => Ok(id),
        _ => Err(format!("{key}: no {kind} {v}")),
    }
}
