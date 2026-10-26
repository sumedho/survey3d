//! Raw data in the project: reducing a file's shots to coded points, office
//! corrections to the observations (re-reduced in the same undoable step, so
//! the audit trail holds both), and the reduction report.

use std::fmt::Write as _;

use survey_core::network::{FieldData, Known, RawEdit};
use survey_core::reduction::{Orientation, Reduction, reduce};
use survey_model::{Color, Document, Entity, Keyed, RecordId, SurveyPoint};

use crate::import_command::{ImportEntities, LayerRef};

/// A reduction ready to execute.
pub struct Reduced {
    pub command: ImportEntities,
    pub reduction: Reduction,
    pub summary: String,
}

/// The raw data file called `name` (any case); with `name` empty, the only one.
pub fn find<'a>(doc: &'a Document, name: &str) -> Result<&'a Keyed<FieldData>, String> {
    let files = doc.field_data();
    let name = name.trim();
    if name.is_empty() {
        return match files {
            [one] => Ok(one),
            [] => Err("There is no raw data in the project: import a GSI, JobXML or RW5 file".into()),
            _ => Err(format!(
                "Name the raw data file: {}",
                files
                    .iter()
                    .map(|f| f.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        };
    }
    files
        .iter()
        .find(|f| f.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            format!(
                "No raw data file '{name}' (the project has {})",
                if files.is_empty() {
                    "none".to_string()
                } else {
                    files
                        .iter()
                        .map(|f| f.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            )
        })
}

/// Coordinates the reduction may use: every point except the ones this
/// file's last reduction made (those are what it recomputes).
fn known<'a>(doc: &'a Document, data: &'a FieldData) -> impl Fn(&str) -> Option<Known> + 'a {
    move |name: &str| {
        if data.reduced.iter().any(|r| r == name) {
            return None;
        }
        doc.find_point_by_name(name).map(|(_, p)| Known {
            position: p.position,
            height_known: p.height_known,
            fixed: true,
        })
    }
}

/// Reduces `data` (the record `id`, possibly with new corrections) and builds
/// one command that adds or updates its points, removes the ones it no
/// longer makes, and stores `data` with the new list of reduced points.
pub fn reduce_command(doc: &Document, id: RecordId, mut data: FieldData, label: String) -> Reduced {
    let reduction = reduce(&data, &known(doc, &data));
    let mut cmd = ImportEntities::new(label);
    let library = &doc.meta().feature_codes;
    let layer_for = |cmd: &mut ImportEntities, code: &str| match library.feature_for(code) {
        Some(f) if !f.layer.trim().is_empty() => {
            let [r, g, b] = f.color;
            Some(cmd.layer_in(doc, &f.layer, Some(Color::rgb(r, g, b))))
        }
        _ => None,
    };
    let description = format!("{} ({})", data.name, data.format);
    for p in &reduction.points {
        match doc
            .find_point_by_name(&p.name)
            .filter(|_| data.reduced.contains(&p.name))
        {
            Some((eid, old)) => {
                let layer = if old.code == p.code {
                    LayerRef::Existing(old.layer)
                } else {
                    layer_for(&mut cmd, &p.code).unwrap_or(LayerRef::Existing(old.layer))
                };
                let point = SurveyPoint {
                    position: p.position,
                    height_known: p.height_known,
                    code: p.code.clone(),
                    ..old.clone()
                };
                if point != *old {
                    cmd.update(eid, layer, Entity::Point(point));
                }
            }
            None => {
                let layer = layer_for(&mut cmd, &p.code).unwrap_or(LayerRef::Existing(doc.current_layer()));
                cmd.add(
                    layer,
                    Entity::Point(SurveyPoint {
                        name: p.name.clone(),
                        position: p.position,
                        height_known: p.height_known,
                        code: p.code.clone(),
                        description: description.clone(),
                        layer: doc.current_layer(),
                    }),
                );
            }
        }
    }
    for old in &data.reduced {
        if !reduction.points.iter().any(|p| p.name == *old)
            && let Some((eid, _)) = doc.find_point_by_name(old)
        {
            cmd.delete(eid);
        }
    }
    data.reduced = reduction.points.iter().map(|p| p.name.clone()).collect();
    let (added, updated, removed) = (cmd.len(), cmd.updated(), cmd.deleted());
    cmd.update_field_data(id, data);
    let mut summary = format!(
        "{added} new point{}, {updated} updated",
        if added == 1 { "" } else { "s" }
    );
    if removed > 0 {
        let _ = write!(summary, ", {removed} removed");
    }
    let checks = reduction.checks.len();
    if let Some((h, v)) = reduction.worst_check() {
        let _ = write!(
            summary,
            "; {checks} check{} (worst {:.3} m plan{})",
            if checks == 1 { "" } else { "s" },
            h,
            v.map(|v| format!(", {v:.3} m height")).unwrap_or_default()
        );
    }
    Reduced {
        command: cmd,
        reduction,
        summary,
    }
}

/// Reduces the file called `name` as it stands.
pub fn reduce_file(doc: &Document, name: &str) -> Result<Reduced, String> {
    let f = find(doc, name)?;
    let mut r = reduce_command(doc, f.id, f.record.clone(), format!("Reduce raw data {}", f.name));
    r.summary = format!("Reduced {}: {}", f.name, r.summary);
    Ok(r)
}

/// Corrects one setup or shot. A file that has been reduced is reduced again
/// in the same step, so its points follow the correction.
pub fn edit_file(doc: &Document, name: &str, edit: RawEdit, revert: bool) -> Result<Reduced, String> {
    let f = find(doc, name)?;
    let mut data = f.record.clone();
    let (s, k) = (edit.setup, edit.observation);
    let setup = data.setups.get(s).cloned();
    data.apply_edit(edit.clone(), revert).map_err(|e| e.to_string())?;
    let setup = setup.expect("checked by apply_edit");
    let what = match k {
        Some(k) => format!(
            "setup {} shot {} ({})",
            s + 1,
            k + 1,
            setup.observations[k].target
        ),
        None => format!("setup {} ({})", s + 1, setup.station),
    };
    let change = if revert {
        "back to the recorded values".to_string()
    } else {
        edit.describe(&setup, k.map(|k| &setup.observations[k]))
            .join(", ")
    };
    let label = format!("Raw edit {} {what}: {change}", f.name);
    if data.reduced.is_empty() {
        let mut command = ImportEntities::new(label.clone());
        command.update_field_data(f.id, data);
        return Ok(Reduced {
            command,
            reduction: Reduction::default(),
            summary: label,
        });
    }
    let mut r = reduce_command(doc, f.id, data, label.clone());
    r.summary = format!("{label}; re-reduced: {}", r.summary);
    Ok(r)
}

/// A plain-text report of a reduction: setups and their orientation, checks
/// to known points, warnings and the corrections in force.
pub fn report(doc: &Document, data: &FieldData, r: &Reduction) -> String {
    let conv = doc.conventions();
    let mut out = String::new();
    let _ = writeln!(out, "RAW DATA REDUCTION — {} ({})", data.name, data.format);
    let _ = writeln!(
        out,
        "{} setups, {} points placed, {} checks; scale factor {:.8}",
        data.setups.len(),
        r.points.len(),
        r.checks.len(),
        data.scale_factor.unwrap_or(1.0)
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "SETUPS");
    let effective = data.effective_setups();
    for (i, (s, e)) in r.setups.iter().zip(&effective).enumerate() {
        let how = match &s.method {
            Some(Orientation::Recorded) => "recorded orientation".to_string(),
            Some(Orientation::Backsight(b)) => format!("backsight {b}"),
            Some(Orientation::Assumed) => "ASSUMED (no backsight)".to_string(),
            None => "NOT REDUCED (no station coordinates)".to_string(),
        };
        let _ = writeln!(
            out,
            "  {:>3}  {:<12} HI {:>7.3}  {:<28} zero {:>14}  {} shots",
            i + 1,
            s.station,
            e.instrument_height,
            how,
            s.orientation
                .map(|b| conv.format_bearing(b))
                .unwrap_or_else(|| "-".into()),
            s.shots
        );
    }
    if !r.checks.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "CHECKS (reduced − known)");
        let _ = writeln!(out, "  setup  point          dE        dN       plan        dZ");
        for c in &r.checks {
            let _ = writeln!(
                out,
                "  {:>5}  {:<10} {:>9.4} {:>9.4} {:>9.4} {:>9}{}",
                c.setup + 1,
                c.target,
                c.delta.x,
                c.delta.y,
                c.delta.length(),
                c.dz.map(|v| format!("{v:.4}")).unwrap_or_else(|| "-".into()),
                if c.backsight { "  (backsight)" } else { "" }
            );
        }
    }
    let warnings: Vec<&String> = data.warnings.iter().chain(&r.warnings).collect();
    if !warnings.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "WARNINGS");
        for w in warnings {
            let _ = writeln!(out, "  {w}");
        }
    }
    if !data.edits.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "OFFICE CORRECTIONS");
        for e in &data.edits {
            let Some(s) = data.setups.get(e.setup) else {
                continue;
            };
            let obs = e.observation.and_then(|k| s.observations.get(k));
            let what = match (e.observation, obs) {
                (Some(k), Some(o)) => format!("setup {} shot {} ({})", e.setup + 1, k + 1, o.target),
                _ => format!("setup {} ({})", e.setup + 1, s.station),
            };
            let _ = writeln!(out, "  {what}: {}", e.describe(s, obs).join(", "));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use survey_core::Angle;
    use survey_core::network::{RawObservation, Setup};
    use survey_model::{ChangeSet, History};

    fn shot(target: &str, hz: f64, d: f64, code: &str) -> RawObservation {
        RawObservation {
            target: target.into(),
            hz: Some(Angle::from_degrees(hz)),
            zenith: Some(Angle::from_degrees(90.0)),
            distance: Some(d),
            target_height: 1.5,
            code: code.into(),
            ..Default::default()
        }
    }

    #[test]
    fn reduce_edit_and_undo() {
        let mut doc = Document::new();
        let mut history = History::new("t");
        let mut cs = ChangeSet::new("control");
        for (n, p) in [
            ("1", DVec3::new(0.0, 0.0, 10.0)),
            ("2", DVec3::new(0.0, 100.0, 10.0)),
        ] {
            cs.push_insert_entity(Entity::Point(SurveyPoint {
                name: n.into(),
                position: p,
                height_known: true,
                code: String::new(),
                description: String::new(),
                layer: doc.current_layer(),
            }));
        }
        let data = FieldData {
            name: "day1".into(),
            format: "test".into(),
            setups: vec![Setup {
                station: "1".into(),
                instrument_height: 1.5,
                observations: vec![
                    shot("2", 0.0, 100.0, ""),
                    shot("10", 90.0, 20.0, "TR"),
                    shot("11", 180.0, 5.0, "KB"),
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        history.execute(Box::new(cs.insert_record(data)), &mut doc);

        let r = reduce_file(&doc, "").expect("reduce");
        assert!(r.summary.contains("2 new points"), "{}", r.summary);
        history.execute(Box::new(r.command), &mut doc);
        let p10 = doc.find_point_by_name("10").expect("10").1.clone();
        assert!((p10.position - DVec3::new(20.0, 0.0, 10.0)).length() < 1e-9);
        assert_eq!(p10.code, "TR");
        assert_eq!(doc.field_data()[0].reduced, ["10", "11"]);

        // Reducing again changes nothing but is allowed.
        let r = reduce_file(&doc, "DAY1").expect("again");
        assert!(r.summary.contains("0 new points, 0 updated"), "{}", r.summary);

        // A correction re-reduces: the rod was 2.0 m on 10, and 11 is excluded.
        let r = edit_file(
            &doc,
            "day1",
            RawEdit {
                setup: 0,
                observation: Some(1),
                target_height: Some(2.0),
                ..Default::default()
            },
            false,
        )
        .expect("edit");
        assert!(r.summary.contains("target height 1.500 → 2.000"), "{}", r.summary);
        history.execute(Box::new(r.command), &mut doc);
        let r = edit_file(
            &doc,
            "day1",
            RawEdit {
                setup: 0,
                observation: Some(2),
                excluded: Some(true),
                ..Default::default()
            },
            false,
        )
        .expect("edit");
        history.execute(Box::new(r.command), &mut doc);
        assert!((doc.find_point_by_name("10").expect("10").1.position.z - 9.5).abs() < 1e-9);
        assert!(doc.find_point_by_name("11").is_none());
        let f = &doc.field_data()[0];
        assert_eq!(f.edits.len(), 2);
        let text = report(&doc, f, &survey_core::reduction::reduce(f, &known(&doc, f)));
        assert!(
            text.contains("backsight 2") && text.contains("OFFICE CORRECTIONS"),
            "{text}"
        );

        history.undo(&mut doc);
        assert!(doc.find_point_by_name("11").is_some());
        history.undo(&mut doc);
        assert!((doc.find_point_by_name("10").expect("10").1.position.z - 10.0).abs() < 1e-9);
        assert!(find(&doc, "nope").is_err());
    }
}
