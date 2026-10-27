//! Least-squares network adjustment of saved traverses and imported field
//! data against the project's control.

use std::collections::BTreeSet;

use survey_core::adjustment::{AdjustResult, adjust};
use survey_core::network::{Known, Network, NetworkOptions, Setup, build, setups_from_traverse};
use survey_model::{ChangeSet, Document, Entity, SurveyPoint};

/// Description of the points an adjustment creates.
const CREATED: &str = "Network adjustment";

/// The key of a source: `t:<traverse>` or `f:<field data>`.
pub fn source_key(doc: &Document, name: &str) -> Option<String> {
    if doc.traverses().iter().any(|t| t.name == name) {
        Some(format!("t:{name}"))
    } else if doc.field_data().iter().any(|f| f.name == name) {
        Some(format!("f:{name}"))
    } else {
        None
    }
}

/// Every source in the project.
pub fn all_sources(doc: &Document) -> BTreeSet<String> {
    let traverses = doc.traverses().iter().map(|t| format!("t:{}", t.name));
    let files = doc.field_data().iter().map(|f| format!("f:{}", f.name));
    traverses.chain(files).collect()
}

/// The setups of the chosen sources.
pub fn setups(doc: &Document, sources: &BTreeSet<String>) -> Vec<Setup> {
    let mut out = Vec::new();
    for t in doc.traverses() {
        if sources.contains(&format!("t:{}", t.name)) {
            out.extend(setups_from_traverse(t));
        }
    }
    for f in doc.field_data() {
        if sources.contains(&format!("f:{}", f.name)) {
            out.extend(f.effective_setups());
        }
    }
    out
}

/// Points that exist but are adjusted rather than held fixed unless the
/// user says otherwise: those computed from the very observations being
/// adjusted (placed by reducing the chosen field data, or the stations of
/// the chosen traverses short of the point they close on) and those an
/// earlier adjustment created.
pub fn free_by_default(doc: &Document, sources: &BTreeSet<String>) -> BTreeSet<String> {
    let reduced = doc
        .field_data()
        .iter()
        .filter(|f| sources.contains(&format!("f:{}", f.name)))
        .flat_map(|f| f.reduced.iter().cloned());
    let stations = doc
        .traverses()
        .iter()
        .filter(|t| sources.contains(&format!("t:{}", t.name)))
        .flat_map(|t| {
            t.legs
                .iter()
                .map(|l| l.to.clone())
                .filter(|n| Some(n) != t.close_on.as_ref() && *n != t.start)
        });
    let adjusted = doc.entities().filter_map(|(_, e)| match e {
        Entity::Point(p) if p.description == CREATED => Some(p.name.clone()),
        _ => None,
    });
    reduced.chain(stations).chain(adjusted).collect()
}

/// Builds and adjusts the network: existing points are control, held fixed
/// unless named in `free`.
pub fn run(
    doc: &Document,
    setups: &[Setup],
    free: &BTreeSet<String>,
    options: &NetworkOptions,
) -> Result<(Network, AdjustResult), String> {
    let known = |name: &str| {
        doc.find_point_by_name(name).map(|(_, p)| Known {
            position: p.position,
            height_known: p.height_known,
            fixed: !free.contains(name),
        })
    };
    let net = build(setups, &known, options).map_err(|e| e.to_string())?;
    let r = adjust(&net.stations, &net.observations).map_err(|e| e.to_string())?;
    Ok((net, r))
}

/// Creates the new stations and moves the adjusted ones, keeping the options
/// with the project.
pub fn apply(
    doc: &Document,
    net: &Network,
    r: &AdjustResult,
    options: &NetworkOptions,
) -> (ChangeSet, String) {
    let mut meta = doc.meta().clone();
    meta.network_options = *options;
    let mut cs = ChangeSet::new("Network adjustment").set_meta(meta);
    let (mut created, mut moved) = (0, 0);
    let heights = r.sd.iter().any(|s| s.z > 0.0) || net.stations.iter().any(|s| s.fixed_height);
    for (i, s) in net.stations.iter().enumerate() {
        let plan_adjusted = !s.fixed_plan;
        let height_adjusted = r.sd[i].z > 0.0;
        if !plan_adjusted && !height_adjusted {
            continue;
        }
        let p = r.positions[i];
        match doc.find_point_by_name(&s.name) {
            Some((id, existing)) => {
                let mut q = existing.clone();
                if plan_adjusted {
                    q.position.x = p.x;
                    q.position.y = p.y;
                }
                if height_adjusted {
                    q.position.z = p.z;
                    q.height_known = true;
                }
                cs.push_update_entity(id, Entity::Point(q));
                moved += 1;
            }
            None => {
                cs.push_insert_entity(Entity::Point(SurveyPoint {
                    name: s.name.clone(),
                    position: if height_adjusted {
                        p
                    } else {
                        p.truncate().extend(0.0)
                    },
                    height_known: heights && height_adjusted,
                    code: String::new(),
                    description: CREATED.into(),
                    layer: doc.current_layer(),
                }));
                created += 1;
            }
        }
    }
    (
        cs,
        format!("Network adjustment: {created} points created, {moved} updated"),
    )
}

/// Points the adjustment would create or move.
pub fn updates(net: &Network, r: &AdjustResult) -> usize {
    net.stations
        .iter()
        .enumerate()
        .filter(|(i, s)| !s.fixed_plan || r.sd[*i].z > 0.0)
        .count()
}
