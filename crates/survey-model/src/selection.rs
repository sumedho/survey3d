use std::collections::BTreeSet;

use crate::ids::EntityId;

/// The set of selected entities, with a revision counter so views can cache.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    ids: BTreeSet<EntityId>,
    revision: u64,
}

impl Selection {
    pub fn ids(&self) -> &BTreeSet<EntityId> {
        &self.ids
    }

    pub fn contains(&self, id: EntityId) -> bool {
        self.ids.contains(&id)
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn clear(&mut self) {
        if !self.ids.is_empty() {
            self.ids.clear();
            self.revision += 1;
        }
    }

    pub fn set(&mut self, ids: impl IntoIterator<Item = EntityId>) {
        self.ids = ids.into_iter().collect();
        self.revision += 1;
    }

    pub fn toggle(&mut self, id: EntityId) {
        if !self.ids.remove(&id) {
            self.ids.insert(id);
        }
        self.revision += 1;
    }

    pub fn insert(&mut self, id: EntityId) {
        if self.ids.insert(id) {
            self.revision += 1;
        }
    }

    /// Drops ids that no longer satisfy `keep` (e.g. deleted entities).
    pub fn retain(&mut self, mut keep: impl FnMut(EntityId) -> bool) {
        let before = self.ids.len();
        self.ids.retain(|id| keep(*id));
        if self.ids.len() != before {
            self.revision += 1;
        }
    }
}
