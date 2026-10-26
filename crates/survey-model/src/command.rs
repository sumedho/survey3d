//! Undoable edits.

use serde::Serialize;
use serde_json::{Value, json};

use crate::cloud::{CloudStyle, PointCloud};
use crate::document::{Document, ProjectMeta};
use crate::entity::Entity;
use crate::ids::{EntityId, LayerId, PointCloudId, SurfaceId};
use crate::layer::Layer;
use crate::records::{AnyRecord, Record, RecordId, RecordKind};
use crate::surface::{Surface, SurfaceStyle};

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
    /// Roughly how much memory the command keeps (bytes), for the undo
    /// history's memory budget.
    fn memory_size(&self) -> usize {
        0
    }
}

/// Approximate bytes held by a copy of an entity.
fn entity_size(e: &Entity) -> usize {
    std::mem::size_of::<Entity>()
        + match e {
            Entity::Point(p) => p.name.len() + p.code.len() + p.description.len(),
            Entity::Polyline(l) => l.vertices.len() * 24 + l.bulges.len() * 8 + l.name.len() + l.code.len(),
            Entity::Text(t) => t.text.len(),
            Entity::Dimension(d) => d.text.len(),
        }
}

fn surface_size(s: &Surface) -> usize {
    std::mem::size_of::<Surface>() + s.tin.vertices.len() * 24 + s.tin.triangles.len() * 12
}

fn record_size(r: &AnyRecord) -> usize {
    match r {
        AnyRecord::FieldData(f) => {
            1024 + f.points.len() * 64
                + f.setups
                    .iter()
                    .map(|s| 128 + s.observations.len() * 96)
                    .sum::<usize>()
        }
        _ => 2048,
    }
}

fn meta_size(m: &ProjectMeta) -> usize {
    4096 + m.feature_codes.codes.len() * 256 + m.calibration.pairs.len() * 128
}

impl Op {
    fn memory_size(&self) -> usize {
        let both = |a: Option<usize>, b: Option<usize>| a.unwrap_or(0) + b.unwrap_or(0);
        64 + match self {
            Op::Entity { before, after, .. } => {
                both(before.as_ref().map(entity_size), after.as_ref().map(entity_size))
            }
            Op::Layer { .. } | Op::SurfaceStyle { .. } | Op::CloudStyle { .. } => 128,
            Op::Surface { before, after, .. } => both(
                before.as_deref().map(surface_size),
                after.as_deref().map(surface_size),
            ),
            // Points are shared with the document while the cloud is in it;
            // a deleted cloud lives on only here.
            Op::Cloud { before, after, .. } => match (before, after) {
                (Some(c), None) => c.points.len() * 12 + c.colors.as_ref().map_or(0, |c| c.len() * 3),
                _ => 512,
            },
            Op::Record { before, after, .. } => both(
                before.as_deref().map(|(_, r)| record_size(r)),
                after.as_deref().map(record_size),
            ),
            Op::Meta { before, after } => meta_size(after) + before.as_deref().map_or(0, meta_size),
        }
    }
}

/// Visibility and locking are display state, changed outside the undo
/// history: undoing or redoing a layer's colour or name keeps them as they are.
fn keep_display_state(doc: &Document, layer: &mut Layer) {
    if let Some(current) = doc.layer(layer.id) {
        layer.visible = current.visible;
        layer.locked = current.locked;
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
        /// The layer's place in the list before (restored on undo of a delete).
        at: Option<usize>,
    },
    Surface {
        id: Option<SurfaceId>,
        before: Option<Box<Surface>>,
        after: Option<Box<Surface>>,
    },
    /// Style only, so display changes do not copy the triangles into the history.
    SurfaceStyle {
        id: SurfaceId,
        before: Option<SurfaceStyle>,
        after: SurfaceStyle,
    },
    Cloud {
        id: Option<PointCloudId>,
        before: Option<Box<PointCloud>>,
        after: Option<Box<PointCloud>>,
    },
    CloudStyle {
        id: PointCloudId,
        before: Option<CloudStyle>,
        after: CloudStyle,
    },
    /// One project record (a traverse, parcel, sheet…), so editing one does not
    /// copy the others into the history.
    Record {
        kind: RecordKind,
        id: Option<RecordId>,
        /// The previous value and its position in the list.
        before: Option<Box<(usize, AnyRecord)>>,
        after: Option<Box<AnyRecord>>,
    },
    // Boxed: project settings are much larger than an entity record.
    Meta {
        before: Option<Box<ProjectMeta>>,
        after: Box<ProjectMeta>,
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

    /// The same changes under another label (when it depends on what was collected).
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
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
            at: None,
        });
        self
    }

    pub fn update_layer(mut self, layer: Layer) -> Self {
        self.ops.push(Op::Layer {
            id: Some(layer.id),
            before: None,
            after: Some(layer),
            at: None,
        });
        self
    }

    pub fn delete_layer(mut self, id: LayerId) -> Self {
        self.ops.push(Op::Layer {
            id: Some(id),
            before: None,
            after: None,
            at: None,
        });
        self
    }

    pub fn insert_surface(mut self, surface: Surface) -> Self {
        self.ops.push(Op::Surface {
            id: None,
            before: None,
            after: Some(Box::new(surface)),
        });
        self
    }

    pub fn update_surface(mut self, id: SurfaceId, surface: Surface) -> Self {
        self.ops.push(Op::Surface {
            id: Some(id),
            before: None,
            after: Some(Box::new(surface)),
        });
        self
    }

    pub fn delete_surface(mut self, id: SurfaceId) -> Self {
        self.ops.push(Op::Surface {
            id: Some(id),
            before: None,
            after: None,
        });
        self
    }

    pub fn set_surface_style(mut self, id: SurfaceId, style: SurfaceStyle) -> Self {
        self.ops.push(Op::SurfaceStyle {
            id,
            before: None,
            after: style,
        });
        self
    }

    pub fn insert_cloud(mut self, cloud: PointCloud) -> Self {
        self.ops.push(Op::Cloud {
            id: None,
            before: None,
            after: Some(Box::new(cloud)),
        });
        self
    }

    pub fn delete_cloud(mut self, id: PointCloudId) -> Self {
        self.ops.push(Op::Cloud {
            id: Some(id),
            before: None,
            after: None,
        });
        self
    }

    pub fn set_cloud_style(mut self, id: PointCloudId, style: CloudStyle) -> Self {
        self.ops.push(Op::CloudStyle {
            id,
            before: None,
            after: style,
        });
        self
    }

    /// Ids of inserted surfaces (available after the first apply).
    pub fn inserted_surface_ids(&self) -> Vec<SurfaceId> {
        self.ops
            .iter()
            .filter_map(|op| match op {
                Op::Surface {
                    id: Some(id),
                    before: None,
                    after: Some(_),
                } => Some(*id),
                _ => None,
            })
            .collect()
    }

    /// Adds a record; its id is allocated on first apply (see [`Self::inserted_record_ids`]).
    pub fn insert_record(mut self, record: impl Record) -> Self {
        self.ops.push(Op::Record {
            kind: record.kind_of(),
            id: None,
            before: None,
            after: Some(Box::new(record.into_any())),
        });
        self
    }

    pub fn update_record(mut self, id: RecordId, record: impl Record) -> Self {
        self.ops.push(Op::Record {
            kind: record.kind_of(),
            id: Some(id),
            before: None,
            after: Some(Box::new(record.into_any())),
        });
        self
    }

    pub fn delete_record(mut self, kind: RecordKind, id: RecordId) -> Self {
        self.ops.push(Op::Record {
            kind,
            id: Some(id),
            before: None,
            after: None,
        });
        self
    }

    /// Ids of inserted records (available after the first apply).
    pub fn inserted_record_ids(&self) -> Vec<RecordId> {
        self.ops
            .iter()
            .filter_map(|op| match op {
                Op::Record {
                    id: Some(id),
                    before: None,
                    after: Some(_),
                    ..
                } => Some(*id),
                _ => None,
            })
            .collect()
    }

    pub fn set_meta(mut self, meta: ProjectMeta) -> Self {
        self.ops.push(Op::Meta {
            before: None,
            after: Box::new(meta),
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
                Op::Layer {
                    id,
                    before,
                    after,
                    at,
                } => {
                    let id = *id.get_or_insert_with(|| doc.alloc_layer_id());
                    if let Some(layer) = after {
                        layer.id = id;
                        keep_display_state(doc, layer);
                    }
                    *at = doc.layer_position(id);
                    *before = doc.set_layer(id, after.clone());
                }
                Op::Surface { id, before, after } => {
                    let id = *id.get_or_insert_with(|| doc.alloc_surface_id());
                    *before = doc.set_surface(id, after.as_deref().cloned()).map(Box::new);
                }
                Op::SurfaceStyle { id, before, after } => {
                    *before = doc.set_surface_style(*id, *after);
                }
                Op::Cloud { id, before, after } => {
                    let id = *id.get_or_insert_with(|| doc.alloc_cloud_id());
                    *before = doc.set_cloud(id, after.as_deref().cloned()).map(Box::new);
                }
                Op::CloudStyle { id, before, after } => {
                    *before = doc.set_cloud_style(*id, *after);
                }
                Op::Record {
                    kind,
                    id,
                    before,
                    after,
                } => {
                    let id = *id.get_or_insert_with(|| doc.alloc_record_id());
                    *before = doc
                        .set_record(*kind, id, after.as_deref().cloned(), None)
                        .map(Box::new);
                }
                Op::Meta { before, after } => {
                    *before = Some(Box::new(doc.set_meta((**after).clone())));
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
                    id: Some(id),
                    before,
                    at,
                    ..
                } => {
                    let mut layer = before.clone();
                    if let Some(l) = &mut layer {
                        keep_display_state(doc, l);
                    }
                    doc.set_layer_at(*id, layer, *at);
                }
                Op::Surface {
                    id: Some(id), before, ..
                } => {
                    doc.set_surface(*id, before.as_deref().cloned());
                }
                Op::SurfaceStyle {
                    id,
                    before: Some(before),
                    ..
                } => {
                    doc.set_surface_style(*id, *before);
                }
                Op::Cloud {
                    id: Some(id), before, ..
                } => {
                    doc.set_cloud(*id, before.as_deref().cloned());
                }
                Op::CloudStyle {
                    id,
                    before: Some(before),
                    ..
                } => {
                    doc.set_cloud_style(*id, *before);
                }
                Op::Record {
                    kind,
                    id: Some(id),
                    before,
                    ..
                } => {
                    let (at, record) = match before.as_deref() {
                        Some((at, r)) => (Some(*at), Some(r.clone())),
                        None => (None, None),
                    };
                    doc.set_record(*kind, *id, record, at);
                }
                Op::Meta {
                    before: Some(before), ..
                } => {
                    doc.set_meta((**before).clone());
                }
                _ => {}
            }
        }
    }

    fn memory_size(&self) -> usize {
        self.ops.iter().map(Op::memory_size).sum()
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
    fn record_round_trip_keeps_ids_and_order() {
        use crate::sheet::SheetRecord;
        let sheet = |name: &str| SheetRecord {
            name: name.into(),
            ..Default::default()
        };
        let mut doc = Document::new();
        let mut add = ChangeSet::new("add")
            .insert_record(sheet("A"))
            .insert_record(sheet("B"))
            .insert_record(sheet("C"));
        add.apply(&mut doc);
        let ids = add.inserted_record_ids();
        assert_eq!(ids.len(), 3);
        assert!(doc.dirty().records.contains(&RecordKind::Sheet));

        let mut edit = ChangeSet::new("edit").update_record(ids[1], sheet("B2"));
        edit.apply(&mut doc);
        let mut del = ChangeSet::new("del").delete_record(RecordKind::Sheet, ids[0]);
        del.apply(&mut doc);
        let names = |doc: &Document| doc.sheets().iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&doc), ["B2", "C"]);
        del.revert(&mut doc);
        edit.revert(&mut doc);
        assert_eq!(names(&doc), ["A", "B", "C"]);
        assert_eq!(
            doc.record::<SheetRecord>(ids[2]).map(|s| s.name.as_str()),
            Some("C")
        );

        add.revert(&mut doc);
        assert!(doc.sheets().is_empty());
        add.apply(&mut doc);
        assert_eq!(doc.sheets().iter().map(|s| s.id).collect::<Vec<_>>(), ids);
        // Ids carry on after loaded records.
        let mut more = ChangeSet::new("more").insert_record(sheet("D"));
        more.apply(&mut doc);
        assert!(more.inserted_record_ids()[0] > ids[2]);
    }

    #[test]
    fn layer_undo_keeps_place_and_display_state() {
        use crate::layer::Color;
        let mut doc = Document::new();
        let mut add = ChangeSet::new("add")
            .insert_layer(Layer::new(LayerId(0), "A", Color::WHITE))
            .insert_layer(Layer::new(LayerId(0), "B", Color::WHITE))
            .insert_layer(Layer::new(LayerId(0), "C", Color::WHITE));
        add.apply(&mut doc);
        let names = |doc: &Document| doc.layers().iter().map(|l| l.name.clone()).collect::<Vec<_>>();
        let first = names(&doc);
        let b = doc.layer_by_name("B").expect("b").clone();
        // Deleting B and undoing puts it back where it was, not at the end.
        let mut del = ChangeSet::new("del").delete_layer(b.id);
        del.apply(&mut doc);
        del.revert(&mut doc);
        assert_eq!(names(&doc), first);
        // A colour change, then B is hidden and locked (display state, not
        // undoable): undoing and redoing the colour leaves B hidden and locked.
        let mut recolour = ChangeSet::new("colour").update_layer(Layer {
            color: Color::rgb(1, 2, 3),
            ..b.clone()
        });
        recolour.apply(&mut doc);
        let mut hidden = doc.layer(b.id).expect("b").clone();
        hidden.visible = false;
        hidden.locked = true;
        doc.set_layer(b.id, Some(hidden));
        recolour.revert(&mut doc);
        let l = doc.layer(b.id).expect("b");
        assert_eq!((l.color, l.visible, l.locked), (Color::WHITE, false, true));
        recolour.apply(&mut doc);
        let l = doc.layer(b.id).expect("b");
        assert_eq!((l.color, l.visible, l.locked), (Color::rgb(1, 2, 3), false, true));
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
