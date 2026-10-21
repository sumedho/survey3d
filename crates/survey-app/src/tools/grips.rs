//! Grips: editable handles on selected objects.
//!
//! Points have one grip; polylines have a grip on every vertex and one at the
//! middle of every segment. Moving a vertex grip stretches the polyline; moving a
//! segment's middle grip inserts a vertex on a straight segment, or reshapes an
//! arc to pass through the new position.

use egui_cad::snap::Pick;
use glam::DVec3;
use survey_core::arc::Arc;
use survey_model::{Document, Entity, EntityId, Selection};

/// Beyond this many grips none are shown (keeps large selections responsive).
pub const MAX_GRIPS: usize = 4000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GripKind {
    Point,
    Vertex(usize),
    /// Middle of segment `i`.
    Midpoint(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grip {
    pub entity: EntityId,
    pub kind: GripKind,
    pub position: DVec3,
}

impl Grip {
    pub fn describe(&self) -> &'static str {
        match self.kind {
            GripKind::Point => "Move point",
            GripKind::Vertex(_) => "Stretch vertex",
            GripKind::Midpoint(_) => "Reshape segment",
        }
    }
}

/// Grips of the editable selected objects (empty if there would be too many).
pub fn grips(doc: &Document, selection: &Selection) -> Vec<Grip> {
    let mut out = Vec::new();
    for &id in selection.ids() {
        if !doc.is_entity_editable(id) {
            continue;
        }
        match doc.entity(id) {
            Some(Entity::Point(p)) => out.push(Grip {
                entity: id,
                kind: GripKind::Point,
                position: p.position,
            }),
            Some(Entity::Polyline(l)) => {
                for (i, v) in l.vertices.iter().enumerate() {
                    out.push(Grip {
                        entity: id,
                        kind: GripKind::Vertex(i),
                        position: *v,
                    });
                }
                for (i, (a, b, bulge)) in l.segments().enumerate() {
                    let position = Arc::from_bulge(a, b, bulge).map_or((a + b) * 0.5, |arc| arc.midpoint());
                    out.push(Grip {
                        entity: id,
                        kind: GripKind::Midpoint(i),
                        position,
                    });
                }
            }
            None => {}
        }
        if out.len() > MAX_GRIPS {
            return Vec::new();
        }
    }
    out
}

/// Plan position from the pick; height from the pick only when it is known
/// (snapped to something with a height), otherwise `keep`.
fn place(to: &Pick, keep: f64) -> DVec3 {
    let z = if to.height_known { to.point.z } else { keep };
    to.point.truncate().extend(z)
}

/// The entity after moving `grip` to `to`; `None` if the grip does not apply.
pub fn apply_grip(entity: &Entity, grip: &Grip, to: &Pick) -> Option<Entity> {
    let mut e = entity.clone();
    match (&mut e, grip.kind) {
        (Entity::Point(p), GripKind::Point) => {
            p.position = place(to, p.position.z);
            p.height_known |= to.height_known;
        }
        (Entity::Polyline(l), GripKind::Vertex(i)) => {
            let v = l.vertices.get_mut(i)?;
            *v = place(to, v.z);
        }
        (Entity::Polyline(l), GripKind::Midpoint(i)) => {
            let (a, b, bulge) = l.segments().nth(i)?;
            if bulge == 0.0 {
                let at = place(to, (a.z + b.z) * 0.5);
                l.vertices.insert(i + 1, at);
                if !l.bulges.is_empty() {
                    let mut bulges = l.padded_bulges();
                    bulges.insert(i + 1, 0.0);
                    l.bulges = bulges;
                }
            } else {
                let mut bulges = l.padded_bulges();
                bulges[i] = Arc::bulge_through(a, to.point, b).unwrap_or(0.0);
                l.bulges = bulges;
            }
        }
        _ => return None,
    }
    Some(e)
}

/// The polyline without the vertex under `grip`; `None` if it would have fewer than two.
pub fn delete_vertex(entity: &Entity, grip: &Grip) -> Option<Entity> {
    let (Entity::Polyline(l), GripKind::Vertex(i)) = (entity, grip.kind) else {
        return None;
    };
    if l.vertices.len() <= 2 || i >= l.vertices.len() {
        return None;
    }
    let mut l = l.clone();
    l.vertices.remove(i);
    if !l.bulges.is_empty() {
        let mut bulges = l.padded_bulges();
        bulges.remove(i);
        l.bulges = bulges;
    }
    if l.closed && l.vertices.len() < 3 && l.bulges.iter().all(|b| *b == 0.0) {
        l.closed = false;
    }
    Some(Entity::Polyline(l))
}

#[cfg(test)]
mod tests {
    use super::*;
    use survey_model::{LayerId, Polyline};

    fn pick(x: f64, y: f64, z: Option<f64>) -> Pick {
        Pick {
            point: DVec3::new(x, y, z.unwrap_or(0.0)),
            height_known: z.is_some(),
            snap: None,
        }
    }

    fn line() -> Entity {
        Entity::Polyline(Polyline {
            name: String::new(),
            vertices: vec![DVec3::new(0.0, 0.0, 5.0), DVec3::new(10.0, 0.0, 7.0)],
            bulges: Vec::new(),
            closed: false,
            code: String::new(),
            layer: LayerId(1),
        })
    }

    fn grip(kind: GripKind) -> Grip {
        Grip {
            entity: EntityId(1),
            kind,
            position: DVec3::ZERO,
        }
    }

    #[test]
    fn stretching_keeps_heights_unless_snapped() {
        let Some(Entity::Polyline(l)) =
            apply_grip(&line(), &grip(GripKind::Vertex(1)), &pick(12.0, 3.0, None))
        else {
            panic!("polyline")
        };
        assert_eq!(l.vertices[1], DVec3::new(12.0, 3.0, 7.0));
        let Some(Entity::Polyline(l)) =
            apply_grip(&line(), &grip(GripKind::Vertex(0)), &pick(1.0, 1.0, Some(9.0)))
        else {
            panic!("polyline")
        };
        assert_eq!(l.vertices[0].z, 9.0);
    }

    #[test]
    fn midpoint_grips_insert_or_bend() {
        let Some(Entity::Polyline(l)) =
            apply_grip(&line(), &grip(GripKind::Midpoint(0)), &pick(5.0, 2.0, None))
        else {
            panic!("polyline")
        };
        assert_eq!(l.vertices.len(), 3);
        assert_eq!(l.vertices[1], DVec3::new(5.0, 2.0, 6.0));

        let mut arc = line();
        if let Entity::Polyline(l) = &mut arc {
            l.bulges = vec![0.5, 0.0];
        }
        let Some(Entity::Polyline(l)) =
            apply_grip(&arc, &grip(GripKind::Midpoint(0)), &pick(5.0, -5.0, None))
        else {
            panic!("polyline")
        };
        // Through (5, -5) from (0, 0) to (10, 0): a semicircle, anticlockwise.
        assert!((l.bulges[0] - 1.0).abs() < 1e-9, "{:?}", l.bulges);
        assert!(delete_vertex(&line(), &grip(GripKind::Vertex(0))).is_none());
    }
}
