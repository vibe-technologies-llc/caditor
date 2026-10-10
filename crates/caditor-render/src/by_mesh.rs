use std::sync::Arc;

use ahash::AHashMap;

use crate::mesh::ShadedMesh;

pub(crate) trait OfMesh {
    fn mesh(&self) -> &Arc<ShadedMesh>;
}

impl OfMesh for Arc<ShadedMesh> {
    fn mesh(&self) -> &Arc<ShadedMesh> {
        self
    }
}

pub(crate) fn mesh_key(mesh: &Arc<ShadedMesh>) -> usize {
    Arc::as_ptr(mesh).addr()
}

pub(crate) struct ByMesh<T> {
    slots: Vec<Option<T>>,
    cursor: usize,
    index: AHashMap<usize, usize>,
    indexed: bool,
}

impl<T> Default for ByMesh<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            cursor: 0,
            index: AHashMap::new(),
            indexed: false,
        }
    }
}

impl<T: OfMesh> ByMesh<T> {
    pub(crate) fn refill(&mut self, items: &mut Vec<T>) {
        self.slots.clear();
        self.slots.extend(items.drain(..).map(Some));
        self.cursor = 0;
        self.indexed = false;
    }

    pub(crate) fn take(&mut self, mesh: &Arc<ShadedMesh>) -> Option<T> {
        let at_cursor = self
            .slots
            .get(self.cursor)
            .and_then(Option::as_ref)
            .is_some_and(|item| Arc::ptr_eq(item.mesh(), mesh));
        let found = if at_cursor {
            self.cursor
        } else {
            self.build_index();
            *self.index.get(&mesh_key(mesh))?
        };
        let taken = self.slots.get_mut(found)?.take()?;
        self.cursor = found + 1;
        Some(taken)
    }

    fn build_index(&mut self) {
        if self.indexed {
            return;
        }
        self.index.clear();
        for (place, slot) in self.slots.iter().enumerate() {
            if let Some(item) = slot {
                self.index.insert(mesh_key(item.mesh()), place);
            }
        }
        self.indexed = true;
    }

    pub(crate) fn rest(&mut self) -> impl Iterator<Item = T> + '_ {
        self.slots.drain(..).flatten()
    }

    pub(crate) fn clear(&mut self) {
        self.slots.clear();
    }
}

#[derive(Debug, Default)]
pub(crate) struct DrawnOrder {
    keys: Vec<usize>,
    changed: bool,
}

impl DrawnOrder {
    pub(crate) fn note<T: OfMesh>(&mut self, drawn: &[T], rewritten: bool) {
        let same = self.keys.len() == drawn.len()
            && self
                .keys
                .iter()
                .zip(drawn)
                .all(|(key, item)| *key == mesh_key(item.mesh()));
        self.changed = rewritten || !same;
        if !same {
            self.keys.clear();
            self.keys
                .extend(drawn.iter().map(|item| mesh_key(item.mesh())));
        }
    }

    pub(crate) fn changed(&self) -> bool {
        self.changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meshes(count: usize) -> Vec<Arc<ShadedMesh>> {
        (0..count).map(|_| Arc::new(ShadedMesh::new([]))).collect()
    }

    #[test]
    fn items_are_taken_by_their_mesh_in_any_order_and_only_once() {
        let all = meshes(4);
        let mut by_mesh = ByMesh::default();
        by_mesh.refill(&mut all.clone());

        let taken = [3, 0, 1, 1].map(|index| by_mesh.take(&all[index]).is_some());
        let rest: Vec<Arc<ShadedMesh>> = by_mesh.rest().collect();

        assert_eq!(taken, [true, true, true, false]);
        assert_eq!(rest.len(), 1);
        assert!(Arc::ptr_eq(&rest[0], &all[2]));
        assert!(by_mesh.take(&meshes(1)[0]).is_none());
    }

    #[test]
    fn the_drawn_order_changes_with_a_rewrite_a_new_mesh_a_dropped_one_or_a_new_order() {
        let all = meshes(3);
        let mut order = DrawnOrder::default();

        order.note(&all, false);
        let first = order.changed();
        order.note(&all, false);
        let again = order.changed();
        order.note(&all, true);
        let rewritten = order.changed();
        order.note(&all[..2], false);
        let dropped = order.changed();
        order.note(&[Arc::clone(&all[1]), Arc::clone(&all[0])], false);
        let swapped = order.changed();
        order.note(&[Arc::clone(&all[1]), Arc::clone(&all[0])], false);
        let settled = order.changed();

        assert!(first);
        assert!(!again);
        assert!(rewritten);
        assert!(dropped);
        assert!(swapped);
        assert!(!settled);
    }
}
