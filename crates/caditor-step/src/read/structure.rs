use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Plane, RigidTransform};

use crate::read::{
    geometry::Geometry,
    graph::{Entity, Graph},
    units::{Units, context_units},
};

pub(crate) const MAX_INSTANCES: usize = 1000;
const MAX_DEPTH: usize = 32;

pub(crate) struct Structure {
    item_representation: BTreeMap<u64, u64>,
    contexts: BTreeMap<u64, u64>,
    names: BTreeMap<u64, String>,
    identical: BTreeMap<u64, BTreeSet<u64>>,
    parents: BTreeMap<u64, Vec<(u64, RigidTransform)>>,
    units: BTreeMap<u64, Units>,
    pub truncated: bool,
}

impl Structure {
    pub fn read(graph: &Graph<'_>) -> Self {
        let mut structure = Self {
            item_representation: BTreeMap::new(),
            contexts: BTreeMap::new(),
            names: BTreeMap::new(),
            identical: BTreeMap::new(),
            parents: BTreeMap::new(),
            units: BTreeMap::new(),
            truncated: false,
        };
        for entity in graph.entities() {
            structure.representation(entity);
        }
        let definitions = definition_representations(graph);
        structure.names = product_names(graph, &definitions);
        let children = assembly_children(graph, &definitions);
        for entity in graph.entities() {
            structure.relationship(graph, entity, &children);
            structure.mapped_items(graph, entity);
        }
        structure
    }

    fn representation(&mut self, entity: Entity<'_>) {
        let Ok(fields) = entity.fields() else {
            return;
        };
        let kind = entity.kind();
        if !kind.ends_with("REPRESENTATION") || kind.contains("DEFINITION") {
            return;
        }
        let (Ok(items), Ok(context)) = (fields.references(1), fields.reference(2)) else {
            return;
        };
        self.contexts.insert(entity.id, context);
        for item in items {
            self.item_representation.entry(item).or_insert(entity.id);
        }
    }

    pub fn units_of(&mut self, graph: &Graph<'_>, representation: u64) -> Units {
        if let Some(units) = self.units.get(&representation) {
            return *units;
        }
        let units = self
            .contexts
            .get(&representation)
            .map(|context| context_units(graph, *context))
            .unwrap_or_default();
        self.units.insert(representation, units);
        units
    }

    pub fn representation_of(&self, item: u64) -> Option<u64> {
        self.item_representation.get(&item).copied()
    }

    pub fn name_of(&self, representation: u64) -> Option<&str> {
        self.class(representation)
            .into_iter()
            .find_map(|member| self.names.get(&member))
            .map(String::as_str)
    }

    fn relationship(&mut self, graph: &Graph<'_>, entity: Entity<'_>, children: &BTreeSet<u64>) {
        let Ok(relation) = entity
            .record("REPRESENTATION_RELATIONSHIP")
            .or_else(|_| entity.record("SHAPE_REPRESENTATION_RELATIONSHIP"))
        else {
            return;
        };
        let (Ok(first), Ok(second)) = (relation.reference(2), relation.reference(3)) else {
            return;
        };
        let transformation = entity
            .record("REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION")
            .ok()
            .and_then(|with| with.reference(0).ok());
        let Some(transformation) = transformation else {
            self.identical.entry(first).or_default().insert(second);
            self.identical.entry(second).or_default().insert(first);
            return;
        };
        let (child, parent) = if children.contains(&second) && !children.contains(&first) {
            (second, first)
        } else {
            (first, second)
        };
        let Some(transform) = self.item_transform(graph, transformation, parent) else {
            return;
        };
        self.parents
            .entry(child)
            .or_default()
            .push((parent, transform));
    }

    fn mapped_items(&mut self, graph: &Graph<'_>, entity: Entity<'_>) {
        if entity.kind() != "MAPPED_ITEM" {
            return;
        }
        let Some(parent) = self.representation_of(entity.id) else {
            return;
        };
        let Ok(fields) = entity.fields() else {
            return;
        };
        let (Ok(source), Ok(target)) = (fields.reference(1), fields.reference(2)) else {
            return;
        };
        let Ok(map) = graph
            .entity(source)
            .and_then(|map| map.record("REPRESENTATION_MAP"))
        else {
            return;
        };
        let (Ok(origin), Ok(child)) = (map.reference(0), map.reference(1)) else {
            return;
        };
        let units = self.units_of(graph, parent);
        let geometry = Geometry {
            graph: *graph,
            units,
        };
        let (Some(from), Some(to)) = (frame(&geometry, origin), frame(&geometry, target)) else {
            return;
        };
        self.parents
            .entry(child)
            .or_default()
            .push((parent, from.inverse().then(&to)));
    }

    fn item_transform(
        &mut self,
        graph: &Graph<'_>,
        transformation: u64,
        parent: u64,
    ) -> Option<RigidTransform> {
        let fields = graph
            .entity(transformation)
            .ok()?
            .record("ITEM_DEFINED_TRANSFORMATION")
            .ok()?;
        let units = self.units_of(graph, parent);
        let geometry = Geometry {
            graph: *graph,
            units,
        };
        let from = frame(&geometry, fields.reference(2).ok()?)?;
        let to = frame(&geometry, fields.reference(3).ok()?)?;
        Some(from.inverse().then(&to))
    }

    fn class(&self, representation: u64) -> BTreeSet<u64> {
        let mut class = BTreeSet::from([representation]);
        let mut pending = vec![representation];
        while let Some(current) = pending.pop() {
            for other in self.identical.get(&current).into_iter().flatten() {
                if class.insert(*other) {
                    pending.push(*other);
                }
            }
        }
        class
    }

    pub fn placements(&mut self, representation: u64) -> Vec<RigidTransform> {
        let mut path = Vec::new();
        let mut count = 0;
        let placements = self.walk(representation, &mut path, &mut count);
        if count > MAX_INSTANCES {
            self.truncated = true;
        }
        placements
    }

    fn walk(
        &self,
        representation: u64,
        path: &mut Vec<u64>,
        count: &mut usize,
    ) -> Vec<RigidTransform> {
        let class = self.class(representation);
        let parents: Vec<(u64, RigidTransform)> = class
            .iter()
            .flat_map(|member| self.parents.get(member).into_iter().flatten())
            .filter(|(parent, _)| !class.contains(parent))
            .copied()
            .collect();
        if parents.is_empty() {
            *count += 1;
            return vec![RigidTransform::IDENTITY];
        }
        if path.len() >= MAX_DEPTH {
            return Vec::new();
        }
        let mut placements = Vec::new();
        for (parent, transform) in parents {
            if path.contains(&parent) || *count > MAX_INSTANCES {
                continue;
            }
            path.push(parent);
            for outer in self.walk(parent, path, count) {
                placements.push(transform.then(&outer));
            }
            path.pop();
        }
        placements.truncate(MAX_INSTANCES);
        placements
    }
}

fn frame(geometry: &Geometry<'_>, id: u64) -> Option<RigidTransform> {
    let plane: Plane = geometry.placement(id).ok()?;
    RigidTransform::from_frame(&plane)
}

fn definition_representations(graph: &Graph<'_>) -> BTreeMap<u64, Vec<u64>> {
    let mut by_definition: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for entity in graph.entities() {
        if entity.kind() != "SHAPE_DEFINITION_REPRESENTATION" {
            continue;
        }
        let Ok(fields) = entity.fields() else {
            continue;
        };
        let (Ok(shape), Ok(representation)) = (fields.reference(0), fields.reference(1)) else {
            continue;
        };
        let definition = graph
            .entity(shape)
            .ok()
            .and_then(|shape| shape.fields().ok())
            .and_then(|fields| fields.reference(2).ok());
        if let Some(definition) = definition {
            by_definition
                .entry(definition)
                .or_default()
                .push(representation);
        }
    }
    by_definition
}

fn assembly_children(graph: &Graph<'_>, definitions: &BTreeMap<u64, Vec<u64>>) -> BTreeSet<u64> {
    graph
        .entities()
        .filter(|entity| entity.kind() == "NEXT_ASSEMBLY_USAGE_OCCURRENCE")
        .filter_map(|entity| entity.fields().ok()?.reference(4).ok())
        .flat_map(|child| definitions.get(&child).cloned().unwrap_or_default())
        .collect()
}

fn product_names(
    graph: &Graph<'_>,
    definitions: &BTreeMap<u64, Vec<u64>>,
) -> BTreeMap<u64, String> {
    let mut names = BTreeMap::new();
    for (definition, representations) in definitions {
        let name = graph
            .entity(*definition)
            .ok()
            .and_then(|definition| definition.fields().ok()?.reference(2).ok())
            .and_then(|formation| {
                graph
                    .entity(formation)
                    .ok()?
                    .fields()
                    .ok()?
                    .reference(2)
                    .ok()
            })
            .and_then(|product| {
                let fields = graph.entity(product).ok()?.fields().ok()?;
                let name = fields.text(1).trim();
                let name = if name.is_empty() {
                    fields.text(0).trim()
                } else {
                    name
                };
                (!name.is_empty()).then(|| name.to_owned())
            });
        if let Some(name) = name {
            for representation in representations {
                names.insert(*representation, name.clone());
            }
        }
    }
    names
}
