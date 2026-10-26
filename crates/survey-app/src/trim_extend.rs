//! Trim, extend and apparent intersections on the drawing's polylines, as
//! undoable changes (shared by the tools and the MCP server).

use glam::{DVec2, DVec3};
use survey_core::trim::{self, Segment};
use survey_model::{ChangeSet, Document, Entity, EntityId, Polyline};

/// The boundary edges: the segments of `boundaries` (every visible polyline
/// but `target` when empty).
fn edges(doc: &Document, target: EntityId, boundaries: &[EntityId]) -> Vec<Segment> {
    let lines: Vec<&Polyline> = if boundaries.is_empty() {
        doc.entities()
            .filter(|(id, e)| *id != target && doc.is_layer_visible(e.layer()))
            .filter_map(|(_, e)| match e {
                Entity::Polyline(l) => Some(l),
                _ => None,
            })
            .collect()
    } else {
        boundaries
            .iter()
            .filter(|id| **id != target)
            .filter_map(|id| match doc.entity(*id) {
                Some(Entity::Polyline(l)) => Some(l),
                _ => None,
            })
            .collect()
    };
    lines
        .into_iter()
        .flat_map(|l| l.segments().collect::<Vec<_>>())
        .collect()
}

fn editable_polyline(doc: &Document, id: EntityId) -> Result<&Polyline, String> {
    match doc.entity(id) {
        Some(Entity::Polyline(l)) if doc.is_entity_editable(id) => Ok(l),
        Some(Entity::Polyline(_)) => Err("That polyline is on a locked layer".into()),
        _ => Err("Pick a polyline".into()),
    }
}

fn piece(template: &Polyline, (vertices, bulges): trim::Piece, closed: bool) -> Polyline {
    Polyline {
        vertices,
        bulges,
        closed,
        ..template.clone()
    }
}

/// Whether a parcel uses the polyline as its boundary.
fn is_parcel_boundary(doc: &Document, id: EntityId) -> bool {
    doc.parcels().iter().any(|p| p.boundary == id)
}

/// Trims the part of `target` around `at` back to the boundary crossings
/// either side of it.
pub fn trim_command(
    doc: &Document,
    target: EntityId,
    boundaries: &[EntityId],
    at: DVec2,
) -> Result<(ChangeSet, String), String> {
    let l = editable_polyline(doc, target)?;
    let edges = edges(doc, target, boundaries);
    let cuts = trim::crossings(&l.vertices, &l.bulges, l.closed, &edges);
    let pick =
        trim::nearest_param(&l.vertices, &l.bulges, l.closed, at).ok_or("The polyline has no length")?;
    let pieces = trim::trim(&l.vertices, &l.bulges, l.closed, &cuts, pick).map_err(|e| e.to_string())?;
    let mut cs = ChangeSet::new("Trim polyline");
    let mut pieces = pieces.into_iter();
    match pieces.next() {
        Some(first) => cs.push_update_entity(target, Entity::Polyline(piece(l, first, false))),
        None => cs.push_delete_entity(target),
    }
    let mut extra = 0;
    for p in pieces {
        // The second piece is a new line of the same kind, without the name.
        let mut line = piece(l, p, false);
        line.name.clear();
        cs.push_insert_entity(Entity::Polyline(line));
        extra += 1;
    }
    let mut summary = if extra > 0 {
        "Trimmed: the polyline is now in two pieces".to_string()
    } else {
        "Trimmed".to_string()
    };
    if l.closed {
        summary.push_str("; it is no longer closed");
        if is_parcel_boundary(doc, target) {
            summary.push_str(" (it was a parcel's boundary)");
        }
    }
    Ok((cs, summary))
}

/// Extends the end of `target` nearer `at` to the nearest boundary it meets.
pub fn extend_command(
    doc: &Document,
    target: EntityId,
    boundaries: &[EntityId],
    at: DVec2,
) -> Result<(ChangeSet, String), String> {
    let l = editable_polyline(doc, target)?;
    if l.closed {
        return Err("Closed polylines cannot be extended".into());
    }
    let (Some(first), Some(last)) = (l.vertices.first(), l.vertices.last()) else {
        return Err("The polyline has no vertices".into());
    };
    let at_start = first.truncate().distance(at) < last.truncate().distance(at);
    let edges = edges(doc, target, boundaries);
    let (vertices, bulges) =
        trim::extend(&l.vertices, &l.bulges, at_start, &edges).map_err(|e| e.to_string())?;
    let moved = if at_start {
        (vertices[0] - *first).truncate().length()
    } else {
        (vertices[vertices.len() - 1] - *last).truncate().length()
    };
    let conv = doc.conventions();
    let summary = format!(
        "Extended the {} by {}",
        if at_start { "start" } else { "end" },
        conv.format_length_with_unit(moved)
    );
    let cs = ChangeSet::new("Extend polyline").update_entity(
        target,
        Entity::Polyline(Polyline {
            vertices,
            bulges,
            ..l.clone()
        }),
    );
    Ok((cs, summary))
}

/// The segment of `id` (a polyline) nearest `at`.
pub fn segment_near(doc: &Document, id: EntityId, at: DVec2) -> Option<Segment> {
    let Some(Entity::Polyline(l)) = doc.entity(id) else {
        return None;
    };
    let s = trim::nearest_param(&l.vertices, &l.bulges, l.closed, at)?;
    let segs: Vec<Segment> = l.segments().collect();
    let i = (s.floor() as usize).min(segs.len().checked_sub(1)?);
    segs.get(i).copied()
}

/// Where two segments would meet, extended; with two answers (an arc), the
/// one nearer `near`.
pub fn apparent_intersection(a: &Segment, b: &Segment, near: DVec2) -> Option<DVec3> {
    trim::intersect_extended(a, b)
        .into_iter()
        .min_by(|p, q| p.distance(near).total_cmp(&q.distance(near)))
        .map(|p| p.extend(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use survey_model::{History, LayerId};

    fn line(v: &[(f64, f64)], closed: bool, layer: LayerId) -> Entity {
        Entity::Polyline(Polyline {
            name: "KB.1".into(),
            vertices: v.iter().map(|&(x, y)| DVec3::new(x, y, 0.0)).collect(),
            bulges: Vec::new(),
            closed,
            code: "KB".into(),
            layer,
            heights_known: false,
        })
    }

    #[test]
    fn trim_and_extend_in_the_drawing() {
        let mut doc = Document::new();
        let mut history = History::new("t");
        let layer = doc.current_layer();
        let cs = ChangeSet::new("setup")
            .insert_entity(line(&[(0.0, 0.0), (20.0, 0.0)], false, layer))
            .insert_entity(line(&[(5.0, -5.0), (5.0, 5.0)], false, layer))
            .insert_entity(line(&[(15.0, -5.0), (15.0, 5.0)], false, layer))
            .insert_entity(line(&[(30.0, -5.0), (30.0, 5.0)], false, layer));
        history.execute(Box::new(cs), &mut doc);
        let ids: Vec<EntityId> = doc.entities().map(|(id, _)| id).collect();
        let (kerb, a, b, far) = (ids[0], ids[1], ids[2], ids[3]);
        // Every line is a boundary: the middle goes, leaving two pieces.
        let (cs, summary) = trim_command(&doc, kerb, &[], DVec2::new(10.0, 0.0)).expect("trim");
        assert!(summary.contains("two pieces"), "{summary}");
        history.execute(Box::new(cs), &mut doc);
        assert_eq!(doc.entity_count(), 5);
        let Some(Entity::Polyline(l)) = doc.entity(kerb) else {
            panic!()
        };
        assert_eq!(l.vertices[1], DVec3::new(5.0, 0.0, 0.0));
        // Only the far line as a boundary: the first piece's end runs out to it,
        // past the crossing lines.
        let (cs, summary) = extend_command(&doc, kerb, &[far], DVec2::new(5.0, 0.0)).expect("extend");
        assert!(summary.contains("end by 25.000"), "{summary}");
        history.execute(Box::new(cs), &mut doc);
        let Some(Entity::Polyline(l)) = doc.entity(kerb) else {
            panic!()
        };
        assert_eq!(l.vertices[1], DVec3::new(30.0, 0.0, 0.0));
        // The apparent corner of the two crossing lines' extensions.
        let s1 = segment_near(&doc, a, DVec2::new(5.0, 0.0)).expect("a");
        let s2 = (DVec3::new(0.0, 8.0, 0.0), DVec3::new(1.0, 8.0, 0.0), 0.0);
        assert_eq!(
            apparent_intersection(&s1, &s2, DVec2::ZERO),
            Some(DVec3::new(5.0, 8.0, 0.0))
        );
        assert!(
            extend_command(&doc, b, &[a], DVec2::new(15.0, 5.0)).is_err(),
            "parallel"
        );
    }
}
