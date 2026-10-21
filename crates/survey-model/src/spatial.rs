//! R-tree spatial index over entity bounding boxes.

use std::collections::HashMap;

use rstar::{AABB, RTree, RTreeObject, SelectionFunction};
use survey_core::Aabb3;

use crate::ids::EntityId;

#[derive(Clone, Debug, PartialEq)]
struct IndexItem {
    id: EntityId,
    envelope: AABB<[f64; 3]>,
}

impl RTreeObject for IndexItem {
    type Envelope = AABB<[f64; 3]>;

    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}

fn to_envelope(b: &Aabb3) -> AABB<[f64; 3]> {
    AABB::from_corners(b.min.to_array(), b.max.to_array())
}

fn from_envelope(e: &AABB<[f64; 3]>) -> Aabb3 {
    Aabb3::new(e.lower().into(), e.upper().into())
}

/// Spatial index kept in sync with the document by [`crate::Document`].
#[derive(Default)]
pub struct SpatialIndex {
    tree: RTree<IndexItem>,
    envelopes: HashMap<EntityId, AABB<[f64; 3]>>,
}

impl SpatialIndex {
    pub fn bulk_load(items: impl IntoIterator<Item = (EntityId, Aabb3)>) -> Self {
        let items: Vec<IndexItem> = items
            .into_iter()
            .filter(|(_, b)| !b.is_empty())
            .map(|(id, b)| IndexItem {
                id,
                envelope: to_envelope(&b),
            })
            .collect();
        let envelopes = items.iter().map(|i| (i.id, i.envelope)).collect();
        Self {
            tree: RTree::bulk_load(items),
            envelopes,
        }
    }

    pub fn insert(&mut self, id: EntityId, bounds: Aabb3) {
        self.remove(id);
        if bounds.is_empty() {
            return;
        }
        let envelope = to_envelope(&bounds);
        self.envelopes.insert(id, envelope);
        self.tree.insert(IndexItem { id, envelope });
    }

    pub fn remove(&mut self, id: EntityId) {
        if let Some(envelope) = self.envelopes.remove(&id) {
            self.tree.remove(&IndexItem { id, envelope });
        }
    }

    /// Ids whose bounds pass `region`. `region` is also used to prune tree nodes,
    /// so it must return `true` for any box that might contain a match.
    pub fn query(&self, region: &dyn Fn(&Aabb3) -> bool, out: &mut Vec<EntityId>) {
        struct Select<'a>(&'a dyn Fn(&Aabb3) -> bool);
        impl SelectionFunction<IndexItem> for Select<'_> {
            fn should_unpack_parent(&self, envelope: &AABB<[f64; 3]>) -> bool {
                (self.0)(&from_envelope(envelope))
            }
            fn should_unpack_leaf(&self, leaf: &IndexItem) -> bool {
                (self.0)(&from_envelope(&leaf.envelope))
            }
        }
        out.extend(
            self.tree
                .locate_with_selection_function(Select(region))
                .map(|i| i.id),
        );
    }

    /// Bounds of everything indexed (empty if nothing).
    pub fn bounds(&self) -> Aabb3 {
        if self.tree.size() == 0 {
            Aabb3::EMPTY
        } else {
            from_envelope(&self.tree.root().envelope())
        }
    }

    pub fn len(&self) -> usize {
        self.tree.size()
    }

    pub fn is_empty(&self) -> bool {
        self.tree.size() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    #[test]
    fn insert_query_remove() {
        let mut idx = SpatialIndex::default();
        for i in 0..100 {
            let p = DVec3::new(500_000.0 + f64::from(i), 7_000_000.0, 0.0);
            idx.insert(EntityId(i64::from(i)), Aabb3::from_point(p));
        }
        let region = Aabb3::new(
            DVec3::new(500_010.0, 6_999_999.0, -1.0),
            DVec3::new(500_012.0, 7_000_001.0, 1.0),
        );
        let mut out = Vec::new();
        idx.query(&|b| b.intersects(&region), &mut out);
        out.sort();
        assert_eq!(out, vec![EntityId(10), EntityId(11), EntityId(12)]);

        idx.remove(EntityId(11));
        out.clear();
        idx.query(&|b| b.intersects(&region), &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(idx.len(), 99);
        assert_eq!(idx.bounds().min.x, 500_000.0);
    }
}
