//! Undoable edits.

use serde::Serialize;
use serde_json::{Value, json};

use crate::document::{Document, ProjectMeta};
use crate::entity::Entity;
use crate::ids::{EntityId, LayerId};
use crate::layer::Layer;

/// An undoable edit to a [`Document`].
///
/// `apply` is called for the first execution and every redo; `revert` for every undo.
/// Implementations capture whatever state they need to revert during `apply`.
pub trait Command: Send {
    /// Short human-readable name, e.g. "Add point 1001".
    fn label(&self) -> String;
    fn apply(&mut self, doc: &mut Document);
    fn revert(&mut self, doc: &mut Document);
    /// Machine-readable description of what changed, recorded in the audit trail.
    fn audit_detail(&self) -> Value {
        Value::Null
    }
}

/// Above this many operations the audit detail is summarised instead of listing every record.
const AUDIT_DETAIL_LIMIT: usize = 500;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "op")]
enum Op {
    Entity {
        /// `None` until first applied for insertions.
        id: Option<EntityId>,
        before: Option<Entity>,
        after: Option<Entity>,
    },
    Layer {
        id: Option<LayerId>,
        before: Option<Layer>,
        after: Option<Layer>,
    },
    Meta {
        before: Option<ProjectMeta>,
        after: ProjectMeta,
    },
}

/// A general-purpose command: an ordered list of inserts, updates and deletes.
///
/// Previous values are captured when applied, so builders only need the new state.
/// Ids for inserted records are allocated on first apply and reused on redo.
#[derive(Clone, Debug)]
pub struct ChangeSet {
    label: String,
    ops: Vec<Op>,
}

impl ChangeSet {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ops: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn insert_entity(mut self, entity: Entity) -> Self {
        self.push_insert_entity(entity);
        self
    }

    pub fn push_insert_entity(&mut self, entity: Entity) {
        self.ops.push(Op::Entity {
            id: None,
            before: None,
            after: Some(entity),
        });
    }

    pub fn update_entity(mut self, id: EntityId, entity: Entity) -> Self {
        self.push_update_entity(id, entity);
        self
    }

    pub fn push_update_entity(&mut self, id: EntityId, entity: Entity) {
        self.ops.push(Op::Entity {
            id: Some(id),
            before: None,
            after: Some(entity),
        });
    }

    pub fn delete_entity(mut self, id: EntityId) -> Self {
        self.push_delete_entity(id);
        self
    }

    pub fn push_delete_entity(&mut self, id: EntityId) {
        self.ops.push(Op::Entity {
            id: Some(id),
            before: None,
            after: None,
        });
    }

    /// Inserts a new layer; its `id` field is replaced by a freshly allocated id.
    pub fn insert_layer(mut self, layer: Layer) -> Self {
        self.ops.push(Op::Layer {
            id: None,
            before: None,
            after: Some(layer),
        });
        self
    }

    pub fn update_layer(mut self, layer: Layer) -> Self {
        self.ops.push(Op::Layer {
            id: Some(layer.id),
            before: None,
            after: Some(layer),
        });
        self
    }

    pub fn delete_layer(mut self, id: LayerId) -> Self {
        self.ops.push(Op::Layer {
            id: Some(id),
            before: None,
            after: None,
        });
        self
    }

    pub fn set_meta(mut self, meta: ProjectMeta) -> Self {
        self.ops.push(Op::Meta {
            before: None,
            after: meta,
        });
        self
    }

    /// Ids of inserted entities (available after the first apply).
    pub fn inserted_entity_ids(&self) -> Vec<EntityId> {
        self.ops
            .iter()
            .filter_map(|op| match op {
                Op::Entity {
                    id: Some(id),
                    before: None,
                    after: Some(_),
                } => Some(*id),
                _ => None,
            })
            .collect()
    }
}

impl Command for ChangeSet {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn apply(&mut self, doc: &mut Document) {
        for op in &mut self.ops {
            match op {
                Op::Entity { id, before, after } => {
                    let id = *id.get_or_insert_with(|| doc.alloc_entity_id());
                    *before = doc.set_entity(id, after.clone());
                }
                Op::Layer { id, before, after } => {
                    let id = *id.get_or_insert_with(|| doc.alloc_layer_id());
                    if let Some(layer) = after {
                        layer.id = id;
                    }
                    *before = doc.set_layer(id, after.clone());
                }
                Op::Meta { before, after } => {
                    *before = Some(doc.set_meta(after.clone()));
                }
            }
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        for op in self.ops.iter().rev() {
            match op {
                Op::Entity {
                    id: Some(id), before, ..
                } => {
                    doc.set_entity(*id, before.clone());
                }
                Op::Layer {
                    id: Some(id), before, ..
                } => {
                    doc.set_layer(*id, before.clone());
                }
                Op::Meta {
                    before: Some(before), ..
                } => {
                    doc.set_meta(before.clone());
                }
                _ => {}
            }
        }
    }

    fn audit_detail(&self) -> Value {
        if self.ops.len() <= AUDIT_DETAIL_LIMIT {
            return serde_json::to_value(&self.ops).unwrap_or(Value::Null);
        }
        let (mut inserted, mut updated, mut deleted) = (0usize, 0usize, 0usize);
        for op in &self.ops {
            match op {
                Op::Entity { before: None, .. } => inserted += 1,
                Op::Entity { after: None, .. } => deleted += 1,
                _ => updated += 1,
            }
        }
        let ids: Vec<i64> = self
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Entity { id: Some(id), .. } => Some(id.0),
                _ => None,
            })
            .collect();
        json!({ "summary": { "inserted": inserted, "updated": updated, "deleted": deleted }, "entity_ids": ids })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::SurveyPoint;
    use glam::DVec3;

    fn point(name: &str, x: f64) -> Entity {
        Entity::Point(SurveyPoint {
            name: name.into(),
            position: DVec3::new(x, 0.0, 0.0),
            height_known: false,
            code: String::new(),
            description: String::new(),
            layer: LayerId(1),
        })
    }

    #[test]
    fn insert_update_delete_round_trip() {
        let mut doc = Document::new();
        let mut add = ChangeSet::new("add").insert_entity(point("1", 1.0));
        add.apply(&mut doc);
        let id = add.inserted_entity_ids()[0];
        assert_eq!(doc.entity_count(), 1);

        let mut upd = ChangeSet::new("move").update_entity(id, point("1", 5.0));
        upd.apply(&mut doc);
        assert_eq!(doc.bounds().min.x, 5.0);
        upd.revert(&mut doc);
        assert_eq!(doc.bounds().min.x, 1.0);

        let mut del = ChangeSet::new("del").delete_entity(id);
        del.apply(&mut doc);
        assert_eq!(doc.entity_count(), 0);
        del.revert(&mut doc);
        assert_eq!(doc.entity(id), Some(&point("1", 1.0)));

        add.revert(&mut doc);
        assert_eq!(doc.entity_count(), 0);
        // Redo reuses the same id.
        add.apply(&mut doc);
        assert!(doc.contains(id));
    }

    #[test]
    fn layer_ids_are_allocated() {
        let mut doc = Document::new();
        let mut cmd = ChangeSet::new("layer").insert_layer(Layer::new(
            LayerId(0),
            "Boundary",
            crate::layer::Color::WHITE,
        ));
        cmd.apply(&mut doc);
        let layer = doc.layer_by_name("boundary").expect("layer added");
        assert_ne!(layer.id, LayerId(0));
        cmd.revert(&mut doc);
        assert!(doc.layer_by_name("boundary").is_none());
    }
}
