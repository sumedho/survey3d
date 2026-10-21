//! Imports as a single undoable, audited command — including creating any new
//! layers the imported entities need.

use serde_json::Value;
use survey_model::{ChangeSet, Color, Command, Document, Entity, Layer, LayerId};

/// Which layer an imported entity goes on.
#[derive(Clone, Copy, Debug)]
pub enum LayerRef {
    Existing(LayerId),
    /// Index into [`ImportEntities`]' new layers.
    New(usize),
}

pub struct ImportEntities {
    label: String,
    new_layers: Vec<(String, Color, bool)>,
    items: Vec<(LayerRef, Entity)>,
    /// Built on first apply (when ids are allocated) and replayed on redo.
    inner: Option<ChangeSet>,
}

impl ImportEntities {
    pub fn new(label: String) -> Self {
        Self {
            label,
            new_layers: Vec::new(),
            items: Vec::new(),
            inner: None,
        }
    }

    /// Declares a layer to create; returns its reference.
    pub fn add_layer(&mut self, name: String, color: Color, visible: bool) -> LayerRef {
        self.new_layers.push((name, color, visible));
        LayerRef::New(self.new_layers.len() - 1)
    }

    pub fn add(&mut self, layer: LayerRef, entity: Entity) {
        self.items.push((layer, entity));
    }

    pub fn len(&self) -> usize {
        self.items.len()
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
            for (layer, mut entity) in std::mem::take(&mut self.items) {
                entity.set_layer(match layer {
                    LayerRef::Existing(id) => id,
                    LayerRef::New(i) => ids[i],
                });
                cs.push_insert_entity(entity);
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
