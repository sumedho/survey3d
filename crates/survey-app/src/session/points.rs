//! Point tools: renumbering and raising or lowering by a fixed amount.

use std::collections::{BTreeSet, HashMap};

use survey_model::{ChangeSet, Entity, EntityId};

use super::Session;

/// How `renumber` names the points.
enum Scheme {
    /// From `start`, in steps of `step`, in name order.
    Sequence { start: u64, step: u64 },
    /// Each numeric name plus `delta`.
    Shift(i64),
}

/// Usage, for errors.
pub const RENUMBER_HELP: &str = "renumber <start> [step] (selected points, in name order), \
renumber +1000 / -1000 (shift numeric names), or renumber <from>-<to> <start or ±shift> (points by number)";

/// Sorts names numerically where they are numbers, then as text.
fn name_order(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
        (Err(_), Err(_)) => a.cmp(b),
    }
}

impl Session {
    /// `renumber` with its arguments (see [`RENUMBER_HELP`]); returns the summary.
    pub fn renumber(&mut self, args: &str) -> Result<String, String> {
        let doc = &self.project.doc;
        let mut tokens: Vec<&str> = args.split_whitespace().collect();
        // A leading `from-to` picks points by number instead of the selection.
        let range = tokens.first().and_then(|t| {
            let (a, b) = t.split_once('-')?;
            Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok()?))
        });
        if range.is_some() {
            tokens.remove(0);
        }
        let scheme = match tokens[..] {
            [t] if t.starts_with(['+', '-']) => {
                Scheme::Shift(t.parse().map_err(|_| RENUMBER_HELP.to_string())?)
            }
            [s] => Scheme::Sequence {
                start: s.parse().map_err(|_| RENUMBER_HELP.to_string())?,
                step: 1,
            },
            [s, k] => Scheme::Sequence {
                start: s.parse().map_err(|_| RENUMBER_HELP.to_string())?,
                step: k.parse().ok().filter(|k| *k > 0).ok_or(RENUMBER_HELP)?,
            },
            _ => return Err(RENUMBER_HELP.into()),
        };
        let mut points: Vec<(EntityId, String)> = doc
            .entities()
            .filter_map(|(id, e)| match e {
                Entity::Point(p) => Some((id, p.name.clone())),
                _ => None,
            })
            .filter(|(id, name)| match range {
                Some((a, b)) => name
                    .parse::<u64>()
                    .is_ok_and(|n| (a.min(b)..=a.max(b)).contains(&n)),
                None => self.project.selection.ids().contains(id),
            })
            .collect();
        if points.is_empty() {
            return Err(match range {
                Some((a, b)) => format!("No points numbered {a} to {b}"),
                None => format!("Select the points to renumber, or give a range. {RENUMBER_HELP}"),
            });
        }
        points.sort_by(|a, b| name_order(&a.1, &b.1));
        // Points a raw data reduction made are renamed through their shots.
        let from_raw: Vec<&str> = points
            .iter()
            .map(|(_, n)| n.as_str())
            .filter(|n| from_raw_data(doc, n))
            .collect();
        if !from_raw.is_empty() {
            return Err(format!(
                "{} of these points come from raw data ({}): rename their shots in the raw data editor, \
                 so reducing again keeps the new names",
                from_raw.len(),
                some_names(&from_raw)
            ));
        }
        let mut renames: Vec<(EntityId, String, String)> = Vec::new();
        let mut skipped = 0;
        for (i, (id, old)) in points.iter().enumerate() {
            let new = match scheme {
                Scheme::Sequence { start, step } => (start + step * i as u64).to_string(),
                Scheme::Shift(d) => match old.parse::<i64>() {
                    Ok(n) if n + d >= 0 => (n + d).to_string(),
                    Ok(_) => return Err(format!("{old} would go below zero")),
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                },
            };
            if new != *old {
                renames.push((*id, old.clone(), new));
            }
        }
        // New names must not be taken by points that keep theirs.
        let moving: BTreeSet<EntityId> = renames.iter().map(|r| r.0).collect();
        let mut taken: HashMap<&str, EntityId> = HashMap::new();
        for (id, e) in doc.entities() {
            if let Entity::Point(p) = e
                && !moving.contains(&id)
            {
                taken.insert(p.name.as_str(), id);
            }
        }
        let clashes: Vec<&str> = renames
            .iter()
            .map(|r| r.2.as_str())
            .filter(|n| taken.contains_key(n))
            .collect();
        if !clashes.is_empty() {
            return Err(format!(
                "Those numbers are in use by other points: {}{}",
                clashes[..clashes.len().min(8)].join(", "),
                if clashes.len() > 8 { "…" } else { "" }
            ));
        }
        if renames.is_empty() {
            return Err("Nothing to renumber: the points already have those numbers".into());
        }
        let label = format!(
            "Renumber {} points ({} → {})",
            renames.len(),
            renames[0].1,
            renames[0].2
        );
        let mut cs = ChangeSet::new(label.clone());
        for (id, _, new) in &renames {
            if let Some(Entity::Point(p)) = doc.entity(*id) {
                let mut q = p.clone();
                q.name.clone_from(new);
                cs.push_update_entity(*id, Entity::Point(q));
            }
        }
        self.execute(Box::new(cs));
        let mut summary = label;
        if skipped > 0 {
            summary.push_str(&format!(
                "; {skipped} with names that are not numbers left as they are"
            ));
        }
        self.log.result(summary.clone());
        Ok(summary)
    }

    /// Raises (or with a negative amount, lowers) the selected points and
    /// polylines that have heights by `dz` (project units, as typed).
    pub fn raise_selection(&mut self, args: &str) -> Result<String, String> {
        let doc = &self.project.doc;
        let conv = doc.conventions();
        let dz = conv
            .parse_length(args.trim())
            .map_err(|_| "raise <amount>: e.g. raise 0.150, or raise -0.05 to lower".to_string())?;
        let ids: Vec<EntityId> = self.project.selection.ids().iter().copied().collect();
        if ids.is_empty() {
            return Err("Select the points (and lines) to raise or lower".into());
        }
        let mut cs = ChangeSet::new(format!(
            "Raise {} objects by {}",
            ids.len(),
            conv.format_length(dz)
        ));
        let (mut moved, mut skipped) = (0, 0);
        let mut from_raw = Vec::new();
        for id in ids {
            if !doc.is_entity_editable(id) {
                skipped += 1;
                continue;
            }
            match doc.entity(id) {
                Some(Entity::Point(p)) if p.height_known => {
                    if from_raw_data(doc, &p.name) {
                        from_raw.push(p.name.as_str());
                    }
                    let mut q = p.clone();
                    q.position.z += dz;
                    cs.push_update_entity(id, Entity::Point(q));
                    moved += 1;
                }
                Some(Entity::Polyline(l)) if l.heights_known => {
                    let mut m = l.clone();
                    for v in &mut m.vertices {
                        v.z += dz;
                    }
                    cs.push_update_entity(id, Entity::Polyline(m));
                    moved += 1;
                }
                _ => skipped += 1,
            }
        }
        if moved == 0 {
            return Err("None of the selection has heights to change".into());
        }
        let mut summary = format!(
            "{} {moved} objects by {}",
            if dz >= 0.0 { "Raised" } else { "Lowered" },
            conv.format_length_with_unit(dz.abs())
        );
        if skipped > 0 {
            summary.push_str(&format!(
                " ({skipped} without heights or locked left as they are)"
            ));
        }
        let warning = (!from_raw.is_empty()).then(|| {
            format!(
                "{} of these points come from raw data ({}): reducing it again puts their heights back; \
                 correct the target or instrument height in the raw data editor to keep the change",
                from_raw.len(),
                some_names(&from_raw)
            )
        });
        self.execute(Box::new(cs));
        self.log.result(summary.clone());
        if let Some(w) = warning {
            self.log.warn(w.clone());
            summary = format!("{summary}. Warning: {w}");
        }
        Ok(summary)
    }
}

/// Whether a raw data reduction placed the point named `name`.
fn from_raw_data(doc: &survey_model::Document, name: &str) -> bool {
    doc.field_data()
        .iter()
        .any(|f| f.reduced.iter().any(|r| r == name))
}

/// Up to five names, with … when there are more.
fn some_names(names: &[&str]) -> String {
    let more = if names.len() > 5 { "…" } else { "" };
    format!("{}{more}", names[..names.len().min(5)].join(", "))
}
