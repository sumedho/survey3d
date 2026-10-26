//! Imports (and other bulk edits such as field-to-finish) as a single undoable,
//! audited command — including creating any new layers the entities need.

use glam::DVec3;
use serde_json::Value;
use survey_core::network::FieldData;
use survey_io::ImportedPoint;
use survey_model::{
    ChangeSet, Color, Command, Document, Entity, EntityId, Layer, LayerId, RecordId, SurveyPoint,
};

/// Which layer an imported entity goes on.
#[derive(Clone, Copy, Debug)]
pub enum LayerRef {
    Existing(LayerId),
    /// Index into [`ImportEntities`]' new layers.
    New(usize),
}

enum Item {
    Insert(LayerRef, Entity),
    Update(EntityId, LayerRef, Entity),
    Delete(EntityId),
}

pub struct ImportEntities {
    label: String,
    new_layers: Vec<(String, Color, bool)>,
    items: Vec<Item>,
    /// Raw data records changed in the same step (a reduction's bookkeeping).
    field_data: Vec<(RecordId, FieldData)>,
    /// Built on first apply (when ids are allocated) and replayed on redo.
    inner: Option<ChangeSet>,
}

impl ImportEntities {
    /// Points read from a file (or given by a script). They go on the layer
    /// named `layer`, created if the project has none of that name. Without
    /// one, a point whose code is in the feature library goes on the code's
    /// layer (created in the code's colour, as linework does) and others on the
    /// current layer. Points without a name are numbered on from the largest
    /// number in use, in the project or among `points`.
    pub fn points(doc: &Document, label: String, points: &[ImportedPoint], layer: Option<&str>) -> Self {
        let mut cmd = Self::new(label);
        let chosen = layer.map(str::trim).filter(|n| !n.is_empty());
        let fixed = cmd.layer_named(doc, chosen);
        let library = &doc.meta().feature_codes;
        let in_batch = points.iter().filter_map(|p| p.name.parse::<u64>().ok()).max();
        let in_doc: u64 = doc.next_point_name().parse().unwrap_or(1);
        let mut next = in_doc.max(in_batch.map_or(0, |n| n + 1));
        for p in points {
            let name = if p.name.is_empty() {
                next += 1;
                (next - 1).to_string()
            } else {
                p.name.clone()
            };
            let point = SurveyPoint {
                name,
                position: DVec3::new(p.easting, p.northing, p.height.unwrap_or(0.0)),
                height_known: p.height.is_some(),
                code: p.code.clone(),
                description: p.description.clone(),
                layer: doc.current_layer(),
            };
            let layer = match library.feature_for(&p.code) {
                Some(f) if chosen.is_none() && !f.layer.trim().is_empty() => {
                    let [r, g, b] = f.color;
                    cmd.layer_in(doc, &f.layer, Some(Color::rgb(r, g, b)))
                }
                _ => fixed,
            };
            cmd.add(layer, Entity::Point(point));
        }
        cmd
    }

    /// The layer called `name` (added by this command if the project has none),
    /// or the current layer when `name` is `None` or blank.
    pub fn layer_named(&mut self, doc: &Document, name: Option<&str>) -> LayerRef {
        match name.map(str::trim).filter(|n| !n.is_empty()) {
            Some(name) => self.layer_in(doc, name, None),
            None => LayerRef::Existing(doc.current_layer()),
        }
    }

    /// The layer called `name`, added by this command in `color` (else the next
    /// palette colour, so new layers are told apart) if the project has none.
    pub fn layer_in(&mut self, doc: &Document, name: &str, color: Option<Color>) -> LayerRef {
        if let Some(l) = doc.layer_by_name(name) {
            return LayerRef::Existing(l.id);
        }
        match self
            .new_layers
            .iter()
            .position(|(n, _, _)| n.eq_ignore_ascii_case(name))
        {
            Some(i) => LayerRef::New(i),
            None => {
                let color =
                    color.unwrap_or_else(|| Layer::palette_color(doc.layers().len() + self.new_layers.len()));
                self.add_layer(name.trim().to_string(), color, true)
            }
        }
    }

    pub fn new(label: String) -> Self {
        Self {
            label,
            new_layers: Vec::new(),
            items: Vec::new(),
            field_data: Vec::new(),
            inner: None,
        }
    }

    /// Declares a layer to create; returns its reference.
    pub fn add_layer(&mut self, name: String, color: Color, visible: bool) -> LayerRef {
        self.new_layers.push((name, color, visible));
        LayerRef::New(self.new_layers.len() - 1)
    }

    pub fn add(&mut self, layer: LayerRef, entity: Entity) {
        self.items.push(Item::Insert(layer, entity));
    }

    /// Replaces an existing entity (placing it on `layer`).
    pub fn update(&mut self, id: EntityId, layer: LayerRef, entity: Entity) {
        self.items.push(Item::Update(id, layer, entity));
    }

    pub fn delete(&mut self, id: EntityId) {
        self.items.push(Item::Delete(id));
    }

    /// Replaces a raw data record in the same step.
    pub fn update_field_data(&mut self, id: RecordId, data: FieldData) {
        self.field_data.push((id, data));
    }

    /// Number of entities replaced.
    pub fn updated(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i, Item::Update(..)))
            .count()
    }

    /// Number of entities removed.
    pub fn deleted(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i, Item::Delete(..)))
            .count()
    }

    /// Number of entities added.
    pub fn len(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i, Item::Insert(..)))
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.field_data.is_empty()
    }
}

impl Command for ImportEntities {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn apply(&mut self, doc: &mut Document) {
        let inner = self.inner.get_or_insert_with(|| {
            let mut cs = ChangeSet::new(self.label.clone());
            let mut ids = Vec::new();
            for (name, color, visible) in &self.new_layers {
                let id = doc.alloc_layer_id();
                let mut layer = Layer::new(id, name.clone(), *color);
                layer.visible = *visible;
                cs = cs.update_layer(layer);
                ids.push(id);
            }
            let resolve = |layer: LayerRef| match layer {
                LayerRef::Existing(id) => id,
                LayerRef::New(i) => ids[i],
            };
            for item in std::mem::take(&mut self.items) {
                match item {
                    Item::Insert(layer, mut entity) => {
                        entity.set_layer(resolve(layer));
                        cs.push_insert_entity(entity);
                    }
                    Item::Update(id, layer, mut entity) => {
                        entity.set_layer(resolve(layer));
                        cs.push_update_entity(id, entity);
                    }
                    Item::Delete(id) => cs.push_delete_entity(id),
                }
            }
            for (id, data) in std::mem::take(&mut self.field_data) {
                cs = cs.update_record(id, data);
            }
            cs
        });
        inner.apply(doc);
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(inner) = &mut self.inner {
            inner.revert(doc);
        }
    }

    fn memory_size(&self) -> usize {
        self.inner.as_ref().map_or(0, Command::memory_size)
    }

    fn audit_detail(&self) -> Value {
        self.inner.as_ref().map_or(Value::Null, Command::audit_detail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use survey_model::{History, SurveyPoint};

    #[test]
    fn import_with_new_layer_undoes_as_one_step() {
        let mut doc = Document::new();
        let mut history = History::new("t");
        let mut cmd = ImportEntities::new("Import".into());
        let layer = cmd.add_layer("SURVEY".into(), Color::rgb(1, 2, 3), true);
        for i in 0..3 {
            cmd.add(
                layer,
                Entity::Point(SurveyPoint {
                    name: i.to_string(),
                    position: DVec3::splat(f64::from(i)),
                    height_known: true,
                    code: String::new(),
                    description: String::new(),
                    layer: LayerId(0),
                }),
            );
        }
        history.execute(Box::new(cmd), &mut doc);
        let survey = doc.layer_by_name("SURVEY").expect("layer created").id;
        assert_eq!(doc.entity_count(), 3);
        assert!(doc.entities().all(|(_, e)| e.layer() == survey));
        history.undo(&mut doc);
        assert_eq!(doc.entity_count(), 0);
        assert!(doc.layer_by_name("SURVEY").is_none());
        history.redo(&mut doc);
        assert_eq!(doc.layer_by_name("SURVEY").map(|l| l.id), Some(survey));
    }
}
