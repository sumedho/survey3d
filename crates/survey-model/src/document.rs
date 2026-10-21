//! The in-memory project document.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use survey_core::accuracy::Tolerances;
use survey_core::field_to_finish::FeatureLibrary;
use survey_core::traverse::TraverseRecord;
use survey_core::{Aabb3, Conventions};

use crate::coord_system::CoordinateSystem;
use crate::entity::{Entity, SurveyPoint};
use crate::ids::{EntityId, LayerId};
use crate::layer::{Color, Layer};
use crate::spatial::SpatialIndex;

/// Project-wide settings that are saved with the project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub name: String,
    pub description: String,
    pub coord_system: CoordinateSystem,
    pub conventions: Conventions,
    /// Feature codes for field-to-finish.
    #[serde(default)]
    pub feature_codes: FeatureLibrary,
    /// Accuracy limits for misclosure checks (none by default).
    #[serde(default)]
    pub tolerances: Tolerances,
    /// Traverses as entered, so they can be reopened and recomputed.
    #[serde(default)]
    pub traverses: Vec<TraverseRecord>,
}

impl Default for ProjectMeta {
    fn default() -> Self {
        Self {
            name: "Untitled".into(),
            description: String::new(),
            coord_system: CoordinateSystem::Unspecified,
            conventions: Conventions::default(),
            feature_codes: FeatureLibrary::default(),
            tolerances: Tolerances::default(),
            traverses: Vec::new(),
        }
    }
}

/// Records changed since the last save, used for incremental persistence.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DirtyState {
    pub entities: BTreeSet<EntityId>,
    pub layers: BTreeSet<LayerId>,
    pub meta: bool,
}

impl DirtyState {
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty() && self.layers.is_empty() && !self.meta
    }
}

/// The project document.
///
/// The mutation methods keep the spatial index, revision counter and dirty set
/// consistent. Application code should not call them directly; express edits as
/// [`crate::Command`]s so they are undoable and audited.
pub struct Document {
    meta: ProjectMeta,
    layers: Vec<Layer>,
    current_layer: LayerId,
    entities: BTreeMap<EntityId, Entity>,
    index: SpatialIndex,
    next_entity_id: i64,
    next_layer_id: i64,
    revision: u64,
    dirty: DirtyState,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    /// A new, empty project with a default layer.
    pub fn new() -> Self {
        let layer = Layer::new(LayerId(1), "0", Color::rgb(230, 230, 230));
        let mut doc = Self::from_parts(ProjectMeta::default(), vec![layer], Vec::new());
        // Everything in a new project is unsaved.
        doc.dirty.meta = true;
        doc.dirty.layers.insert(LayerId(1));
        doc
    }

    /// Builds a document from loaded records. The result has no dirty state.
    pub fn from_parts(meta: ProjectMeta, mut layers: Vec<Layer>, entities: Vec<(EntityId, Entity)>) -> Self {
        if layers.is_empty() {
            layers.push(Layer::new(LayerId(1), "0", Color::rgb(230, 230, 230)));
        }
        let index = SpatialIndex::bulk_load(entities.iter().map(|(id, e)| (*id, e.bounds())));
        let next_entity_id = entities.iter().map(|(id, _)| id.0).max().unwrap_or(0) + 1;
        let next_layer_id = layers.iter().map(|l| l.id.0).max().unwrap_or(0) + 1;
        let current_layer = layers[0].id;
        Self {
            meta,
            layers,
            current_layer,
            entities: entities.into_iter().collect(),
            index,
            next_entity_id,
            next_layer_id,
            revision: 1,
            dirty: DirtyState::default(),
        }
    }

    // ------------------------------------------------------------------ queries

    pub fn meta(&self) -> &ProjectMeta {
        &self.meta
    }

    pub fn conventions(&self) -> &Conventions {
        &self.meta.conventions
    }

    /// Increments on every change; views use it to know when to rebuild.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn contains(&self, id: EntityId) -> bool {
        self.entities.contains_key(&id)
    }

    pub fn entities(&self) -> impl Iterator<Item = (EntityId, &Entity)> {
        self.entities.iter().map(|(id, e)| (*id, e))
    }

    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn layer_by_name(&self, name: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.name.eq_ignore_ascii_case(name))
    }

    pub fn current_layer(&self) -> LayerId {
        self.current_layer
    }

    pub fn is_layer_visible(&self, id: LayerId) -> bool {
        self.layer(id).is_none_or(|l| l.visible)
    }

    /// An entity can be edited or snapped to when its layer is visible and unlocked.
    pub fn is_entity_editable(&self, id: EntityId) -> bool {
        self.entity(id)
            .and_then(|e| self.layer(e.layer()))
            .is_some_and(|l| l.visible && !l.locked)
    }

    /// Bounds of all entities.
    pub fn bounds(&self) -> Aabb3 {
        self.index.bounds()
    }

    /// Entity ids whose bounds pass `region` (see [`SpatialIndex::query`]).
    pub fn query(&self, region: &dyn Fn(&Aabb3) -> bool, out: &mut Vec<EntityId>) {
        self.index.query(region, out);
    }

    pub fn find_point_by_name(&self, name: &str) -> Option<(EntityId, &SurveyPoint)> {
        self.entities.iter().find_map(|(id, e)| match e {
            Entity::Point(p) if p.name == name => Some((*id, p)),
            _ => None,
        })
    }

    /// Next free numeric point name (one more than the largest numeric name in use).
    pub fn next_point_name(&self) -> String {
        self.point_names().next().unwrap_or_default()
    }

    /// Successive free numeric point names, for commands that add several points.
    pub fn point_names(&self) -> impl Iterator<Item = String> + use<> {
        let max = self
            .entities
            .values()
            .filter_map(|e| match e {
                Entity::Point(p) => p.name.parse::<u64>().ok(),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        (max + 1..).map(|n| n.to_string())
    }

    pub fn dirty(&self) -> &DirtyState {
        &self.dirty
    }

    pub fn is_dirty(&self) -> bool {
        !self.dirty.is_empty()
    }

    // ------------------------------------------------------------------ mutation

    pub fn alloc_entity_id(&mut self) -> EntityId {
        let id = EntityId(self.next_entity_id);
        self.next_entity_id += 1;
        id
    }

    pub fn alloc_layer_id(&mut self) -> LayerId {
        let id = LayerId(self.next_layer_id);
        self.next_layer_id += 1;
        id
    }

    /// Inserts, replaces (`Some`) or removes (`None`) an entity; returns the previous value.
    pub fn set_entity(&mut self, id: EntityId, entity: Option<Entity>) -> Option<Entity> {
        let previous = match entity {
            Some(e) => {
                self.index.insert(id, e.bounds());
                self.entities.insert(id, e)
            }
            None => {
                self.index.remove(id);
                self.entities.remove(&id)
            }
        };
        self.next_entity_id = self.next_entity_id.max(id.0 + 1);
        self.dirty.entities.insert(id);
        self.revision += 1;
        previous
    }

    /// Inserts, replaces (`Some`) or removes (`None`) a layer; returns the previous value.
    /// New layers are appended to the end of the layer list.
    pub fn set_layer(&mut self, id: LayerId, layer: Option<Layer>) -> Option<Layer> {
        let pos = self.layers.iter().position(|l| l.id == id);
        let previous = match (layer, pos) {
            (Some(l), Some(i)) => Some(std::mem::replace(&mut self.layers[i], l)),
            (Some(l), None) => {
                self.layers.push(l);
                None
            }
            (None, Some(i)) => Some(self.layers.remove(i)),
            (None, None) => None,
        };
        if self.layer(self.current_layer).is_none()
            && let Some(first) = self.layers.first()
        {
            self.current_layer = first.id;
        }
        self.next_layer_id = self.next_layer_id.max(id.0 + 1);
        self.dirty.layers.insert(id);
        self.revision += 1;
        previous
    }

    /// Replaces project settings; returns the previous value.
    pub fn set_meta(&mut self, meta: ProjectMeta) -> ProjectMeta {
        self.dirty.meta = true;
        self.revision += 1;
        std::mem::replace(&mut self.meta, meta)
    }

    /// View-state change (not undoable or audited): the layer new entities go on.
    pub fn set_current_layer(&mut self, id: LayerId) {
        if self.layer(id).is_some() {
            self.current_layer = id;
        }
    }

    /// Returns and clears the dirty set (called by the store after a successful save).
    pub fn take_dirty(&mut self) -> DirtyState {
        std::mem::take(&mut self.dirty)
    }

    /// Marks everything dirty, e.g. before "save as" to a brand-new file.
    pub fn mark_all_dirty(&mut self) {
        self.dirty.meta = true;
        self.dirty.layers = self.layers.iter().map(|l| l.id).collect();
        self.dirty.entities = self.entities.keys().copied().collect();
    }
}
