//! Traverses: resolve a record (stations by name) against the project's
//! points, reduce and adjust it, and create the computed stations.

use glam::DVec3;
use survey_core::traverse::{Closing, Reference, Traverse, TraverseRecord, TraverseResult, compute};
use survey_core::{Angle, Bearing, Conventions, ParseError};
use survey_model::{ChangeSet, Document, Entity, Polyline, SurveyPoint};

/// A point by name, or why not.
fn point<'a>(doc: &'a Document, name: &str) -> Result<&'a SurveyPoint, String> {
    doc.find_point_by_name(name.trim())
        .map(|(_, p)| p)
        .ok_or_else(|| format!("no point named '{}'", name.trim()))
}

/// A point name (if it exists) or a typed bearing; `None` when blank.
pub fn parse_reference(doc: &Document, conv: &Conventions, text: &str) -> Result<Option<Reference>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    if doc.find_point_by_name(t).is_some() {
        return Ok(Some(Reference::Point(t.to_string())));
    }
    conv.parse_bearing(t)
        .map(|b| Some(Reference::Bearing(b)))
        .map_err(|e| format!("'{t}': not a point name or bearing ({e})"))
}

/// A deflection such as `12°30' R`, `L 5.5` or a signed angle (negative = left).
pub fn parse_deflection(conv: &Conventions, text: &str) -> Result<Angle, ParseError> {
    let t = text.trim();
    let upper = t.to_ascii_uppercase();
    let (body, sign) = if let Some(rest) = upper.strip_suffix('R').or_else(|| upper.strip_prefix('R')) {
        (rest.to_string(), 1.0)
    } else if let Some(rest) = upper.strip_suffix('L').or_else(|| upper.strip_prefix('L')) {
        (rest.to_string(), -1.0)
    } else {
        (t.to_string(), 1.0)
    };
    Ok(conv.parse_angle(body.trim())? * sign)
}

/// Bearing from `from` to a reference.
fn reference_bearing(doc: &Document, from: DVec3, r: &Reference) -> Result<Bearing, String> {
    match r {
        Reference::Bearing(b) => Ok(*b),
        Reference::Point(name) => {
            let d = point(doc, name)?.position - from;
            Bearing::from_delta(d.x, d.y).ok_or_else(|| format!("'{name}' is at the station"))
        }
    }
}

/// The traverse a record describes, resolved against the project's points.
pub fn resolve(doc: &Document, rec: &TraverseRecord) -> Result<Traverse, String> {
    if rec.start.trim().is_empty() {
        return Err("Enter the start point and the observations to reduce the traverse.".into());
    }
    let start = point(doc, &rec.start).map_err(|e| format!("Start: {e}"))?;
    let backsight = rec
        .backsight
        .as_ref()
        .map(|r| reference_bearing(doc, start.position, r))
        .transpose()
        .map_err(|e| format!("Backsight: {e}"))?;
    let closing = match &rec.close_on {
        None => None,
        Some(name) => {
            let p = point(doc, name).map_err(|e| format!("Close on: {e}"))?;
            let angle_check = match (&rec.closing_reference, rec.closing_angle) {
                (Some(r), Some(a)) => Some((
                    reference_bearing(doc, p.position, r).map_err(|e| format!("Closing reference: {e}"))?,
                    a,
                )),
                _ => None,
            };
            Some(Closing {
                point: p.position.truncate(),
                height: p.height_known.then_some(p.position.z),
                angle_check,
            })
        }
    };
    Ok(Traverse {
        start_name: start.name.clone(),
        start: start.position,
        start_height_known: start.height_known,
        backsight,
        legs: rec.legs.clone(),
        closing,
        adjustment: rec.adjustment,
        adjust_angles: rec.adjust_angles,
        scale_factor: rec.scale_factor,
    })
}

/// Resolves and computes (reduces and adjusts) a traverse record.
pub fn reduce(doc: &Document, rec: &TraverseRecord) -> Result<(Traverse, TraverseResult), String> {
    let t = resolve(doc, rec)?;
    let r = compute(&t).map_err(|e| e.to_string())?;
    Ok((t, r))
}

/// Points (and, with `polyline`, a line through them) for the computed
/// stations. The last station of a closed traverse is the known point and is
/// not recreated; stations whose names already exist are left as they are.
pub fn create_points(
    doc: &Document,
    t: &Traverse,
    r: &TraverseResult,
    polyline: bool,
) -> (ChangeSet, String) {
    let precision = r.precision().map(|n| format!(", 1:{n:.0}")).unwrap_or_default();
    let mut cs = ChangeSet::new(format!(
        "Traverse from {} ({}{precision})",
        t.start_name,
        r.adjustment.label()
    ));
    let mut created = 0;
    let mut skipped = Vec::new();
    let last = r.stations.len() - 1;
    for (i, st) in r.stations.iter().enumerate() {
        if i == last && t.closing.is_some() {
            continue;
        }
        if doc.find_point_by_name(&st.name).is_some() {
            skipped.push(st.name.clone());
            continue;
        }
        cs.push_insert_entity(Entity::Point(SurveyPoint {
            name: st.name.clone(),
            position: st.position,
            height_known: st.height_known,
            code: "TRAV".into(),
            description: format!("Traverse from {}", t.start_name),
            layer: doc.current_layer(),
        }));
        created += 1;
    }
    if polyline {
        let mut vertices = vec![t.start];
        vertices.extend(r.stations.iter().map(|s| s.position));
        let closed = vertices.len() > 3
            && vertices[0]
                .truncate()
                .distance(vertices[vertices.len() - 1].truncate())
                < 1e-6;
        if closed {
            vertices.pop();
        }
        // A plan-only line unless every station has a height.
        let heights_known = t.start_height_known && r.stations.iter().all(|s| s.height_known);
        cs.push_insert_entity(Entity::Polyline(Polyline {
            name: format!("Traverse {}", t.start_name),
            vertices,
            bulges: Vec::new(),
            closed,
            code: "TRAV".into(),
            layer: doc.current_layer(),
            heights_known,
        }));
    }
    let mut summary = format!("Traverse: created {created} points{precision}");
    if !skipped.is_empty() {
        summary.push_str(&format!(" (existing, not changed: {})", skipped.join(", ")));
    }
    (cs, summary)
}
