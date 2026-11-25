use std::collections::BTreeSet;

use caditor_geometry::Aabb;

use crate::{
    boolean::{
        BooleanError, BooleanOperation, Planned,
        apart::{APART, enclosure, settled},
        combine_planned, face_box,
    },
    box_tree::BoxTree,
    naming::EdgeName,
    topology::{ShellId, Solid, doubly_curved},
};

struct Shells {
    ids: Vec<ShellId>,
    boxes: Vec<Aabb>,
}

fn shell_boxes(solid: &Solid) -> Option<Shells> {
    let mut ids = Vec::new();
    let mut boxes = Vec::new();

    for (id, shell) in solid.shells() {
        let mut bounds: Option<Aabb> = None;

        for face_id in shell.faces() {
            let face = solid.face(*face_id)?;
            let bulge = doubly_curved(face.surface())
                .then(|| face_box(solid, *face_id))
                .flatten();
            let edges = face
                .loops()
                .iter()
                .filter_map(|loop_id| solid.face_loop(*loop_id))
                .flat_map(|face_loop| face_loop.coedges().iter())
                .filter_map(|coedge| solid.coedge(*coedge))
                .filter_map(|coedge| solid.edge(coedge.edge()))
                .map(|edge| edge.curve().bounding_box(edge.interval()));

            bounds = edges.chain(bulge).chain(bounds).reduce(Aabb::union);
        }
        ids.push(id);
        boxes.push(bounds?);
    }
    Some(Shells { ids, boxes })
}

fn root(parents: &mut [usize], mut item: usize) -> usize {
    while let Some(parent) = parents.get(item).copied()
        && parent != item
    {
        let grandparent = parents.get(parent).copied().unwrap_or(parent);

        if let Some(slot) = parents.get_mut(item) {
            *slot = grandparent;
        }
        item = grandparent;
    }
    item
}

fn reached_shells(shells: &Shells, tool: &Aabb) -> BTreeSet<ShellId> {
    let tree = BoxTree::new(shells.boxes.iter().copied());
    let mut parents: Vec<usize> = (0..shells.boxes.len()).collect();

    for (index, bounds) in shells.boxes.iter().enumerate() {
        for other in tree.overlapping(bounds, APART) {
            let (first, second) = (root(&mut parents, index), root(&mut parents, other));

            if let Some(slot) = parents.get_mut(first.max(second)) {
                *slot = first.min(second);
            }
        }
    }

    let reached_groups: BTreeSet<usize> = tree
        .overlapping(tool, APART)
        .into_iter()
        .map(|index| root(&mut parents, index))
        .collect();
    (0..shells.ids.len())
        .filter(|index| reached_groups.contains(&root(&mut parents, *index)))
        .filter_map(|index| shells.ids.get(index).copied())
        .collect()
}

fn names(solid: &Solid) -> BTreeSet<EdgeName> {
    solid.edges().map(|(_, edge)| edge.name()).collect()
}

pub(super) fn combine_reached(
    first: &Solid,
    second: &Solid,
    operation: BooleanOperation,
) -> Option<Result<Solid, BooleanError>> {
    first.shells().nth(1)?;

    let tool = enclosure(second)?;
    let shells = shell_boxes(first)?;

    let reached = reached_shells(&shells, &tool);

    if reached.is_empty() || reached.len() == shells.ids.len() {
        return None;
    }

    let unreached: BTreeSet<ShellId> = shells
        .ids
        .iter()
        .copied()
        .filter(|id| !reached.contains(id))
        .collect();
    let left_alone = first.keeping_shells(&unreached)?;

    if !settled(&[&left_alone]) {
        return None;
    }

    let combined = combine_planned(&first.keeping_shells(&reached)?, second, operation);
    match (combined, operation) {
        (Err(BooleanError::Cancelled(interrupted)), _) => {
            Some(Err(BooleanError::Cancelled(interrupted)))
        }
        (Err(BooleanError::Empty), BooleanOperation::Intersection) => {
            Some(Err(BooleanError::Empty))
        }
        (Err(BooleanError::Empty), _) => Some(Ok(left_alone)),
        (Err(_), _) => None,
        (Ok(Planned { solid, .. }), BooleanOperation::Intersection) => Some(Ok(solid)),
        (
            Ok(Planned {
                solid,
                names: planned,
            }),
            _,
        ) => {
            if !names(&left_alone).is_disjoint(&planned) {
                return None;
            }
            solid.beside(&left_alone).map(Ok)
        }
    }
}
