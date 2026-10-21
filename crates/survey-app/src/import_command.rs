//! Point import as a single undoable, audited command — including the optional
//! creation of a new layer for the imported points.

use serde_json::Value;
use survey_model::{ChangeSet, Color, Command, Document, Entity, Layer, LayerId, SurveyPoint};

pub enum TargetLayer {
    Existing(LayerId),
    New { name: String, color: Color },
}

pub struct ImportPoints {
    label: String,
    target: TargetLayer,
    points: Vec<SurveyPoint>,
    /// Built on first apply (when ids are allocated) and replayed on redo.
    inner: Option<ChangeSet>,
}

impl ImportPoints {
    pub fn new(label: String, target: TargetLayer, points: Vec<SurveyPoint>) -> Self {
        Self {
            label,
            target,
            points,
            inner: None,
        }
    }
}

impl Command for ImportPoints {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn apply(&mut self, doc: &mut Document) {
        let inner = self.inner.get_or_insert_with(|| {
            let mut cs = ChangeSet::new(self.label.clone());
            let layer = match &self.target {
                TargetLayer::Existing(id) => *id,
                TargetLayer::New { name, color } => {
                    let id = doc.alloc_layer_id();
                    cs = cs.update_layer(Layer::new(id, name.clone(), *color));
                    id
                }
            };
            for mut p in std::mem::take(&mut self.points) {
                p.layer = layer;
                cs.push_insert_entity(Entity::Point(p));
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
