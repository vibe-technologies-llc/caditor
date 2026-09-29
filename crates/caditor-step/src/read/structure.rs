use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Plane, RigidTransform, Vector3};

use crate::read::{
    geometry::{Geometry, MAX_WORK, Work},
    graph::{Entity, Fields, Graph},
    units::{Units, context_units},
};

pub(crate) const MAX_INSTANCES: usize = 1000;
pub(crate) const MAX_DEPTH: usize = 32;
const OPERATOR_FIELDS: usize = 8;
const RIGID_SLACK: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq)]
struct Parent {
    representation: u64,
    transform: Option<RigidTransform>,
    occurrence: Option<String>,
}

struct Occurrence {
    child: u64,
    name: Option<String>,
}

pub(crate) struct Structure {
    item_representation: BTreeMap<u64, u64>,
    contexts: BTreeMap<u64, u64>,
    names: BTreeMap<u64, String>,
    identical: BTreeMap<u64, BTreeSet<u64>>,
    parents: BTreeMap<u64, Vec<Parent>>,
    units: BTreeMap<u64, Units>,
    walked: BTreeMap<u64, Walk>,
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
            walked: BTreeMap::new(),
            truncated: false,
        };
        for entity in graph.entities() {
            structure.representation(entity);
        }
        let definitions = definition_representations(graph);
        structure.names = product_names(graph, &definitions);
        let children = assembly_children(graph, &definitions);
        let occurrences = occurrences(graph);
        let context = Context {
            children: &children,
            definitions: &definitions,
            occurrences: &occurrences,
        };
        for entity in graph.entities() {
            structure.relationship(graph, entity, &context);
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

    fn relationship(&mut self, graph: &Graph<'_>, entity: Entity<'_>, context: &Context<'_>) {
        let Some(relation) = entity
            .find("REPRESENTATION_RELATIONSHIP")
            .or_else(|| entity.find("SHAPE_REPRESENTATION_RELATIONSHIP"))
        else {
            return;
        };
        let (Ok(first), Ok(second)) = (relation.reference(2), relation.reference(3)) else {
            return;
        };
        let transformation = entity
            .find("REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION")
            .and_then(|with| with.reference(0).ok());
        let Some(transformation) = transformation else {
            self.identical.entry(first).or_default().insert(second);
            self.identical.entry(second).or_default().insert(first);
            return;
        };
        let occurrence = context.occurrences.get(&entity.id);
        let owned_by = |representation: u64, definition: u64| {
            context
                .definitions
                .get(&definition)
                .is_some_and(|representations| {
                    representations
                        .iter()
                        .any(|owned| self.class(*owned).contains(&representation))
                })
        };
        let child_is_first = match occurrence {
            Some(occurrence) if owned_by(first, occurrence.child) => true,
            Some(occurrence) if owned_by(second, occurrence.child) => false,
            _ => !(context.children.contains(&second) && !context.children.contains(&first)),
        };
        let (child, parent) = if child_is_first {
            (first, second)
        } else {
            (second, first)
        };
        let transform = self.item_transform(graph, transformation, (first, second), child_is_first);
        self.parents.entry(child).or_default().push(Parent {
            representation: parent,
            transform,
            occurrence: occurrence.and_then(|occurrence| occurrence.name.clone()),
        });
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
        let geometry = Geometry::new(*graph, units, Work::new(MAX_WORK));
        let to = frame(&geometry, target).or_else(|| operator(&geometry, target));
        let transform = frame(&geometry, origin)
            .zip(to)
            .map(|(from, to)| from.inverse().then(&to));
        self.parents.entry(child).or_default().push(Parent {
            representation: parent,
            transform,
            occurrence: None,
        });
    }

    fn item_transform(
        &mut self,
        graph: &Graph<'_>,
        transformation: u64,
        (first, second): (u64, u64),
        child_is_first: bool,
    ) -> Option<RigidTransform> {
        let entity = graph.entity(transformation).ok()?;
        if entity.is("CARTESIAN_TRANSFORMATION_OPERATOR_3D") {
            let geometry = Geometry::new(*graph, self.units_of(graph, second), Work::new(MAX_WORK));
            let carried = operator(&geometry, transformation)?;
            return Some(if child_is_first {
                carried
            } else {
                carried.inverse()
            });
        }
        let fields = entity.find("ITEM_DEFINED_TRANSFORMATION")?;
        let mut item = |representation: u64, index: usize| {
            let geometry = Geometry::new(
                *graph,
                self.units_of(graph, representation),
                Work::new(MAX_WORK),
            );
            frame(&geometry, fields.reference(index).ok()?)
        };
        let in_first = item(first, 2)?;
        let in_second = item(second, 3)?;
        Some(if child_is_first {
            in_first.inverse().then(&in_second)
        } else {
            in_second.inverse().then(&in_first)
        })
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

    pub fn placements(&mut self, representation: u64) -> Placements {
        self.walk(representation, 0)
    }

    fn walk(&mut self, representation: u64, depth: usize) -> Placements {
        let class = self.class(representation);
        let key = class.first().copied().unwrap_or(representation);
        match self.walked.get(&key) {
            Some(Walk::Done(placements)) => return placements.clone(),
            Some(Walk::InProgress) => return Placements::unplaced(Unplaced::InsideItself),
            None => {}
        }
        let parents: Vec<Parent> = class
            .iter()
            .flat_map(|member| self.parents.get(member).into_iter().flatten())
            .filter(|parent| !class.contains(&parent.representation))
            .cloned()
            .collect();
        let placements = if parents.is_empty() {
            Placements::at_origin()
        } else if depth >= MAX_DEPTH {
            Placements::unplaced(Unplaced::TooDeep)
        } else {
            self.walked.insert(key, Walk::InProgress);
            let mut placed = Placements {
                transforms: Vec::new(),
                occurrences: Vec::new(),
                unplaced: None,
                left_out: false,
            };
            for parent in parents {
                let Some(transform) = parent.transform else {
                    placed.left_out = true;
                    continue;
                };
                let outer = self.walk(parent.representation, depth + 1);
                placed.left_out |= outer.left_out;
                if outer.transforms.is_empty() {
                    placed.unplaced = placed.unplaced.or(outer.unplaced);
                }
                for outer in outer.transforms {
                    if placed.transforms.len() >= MAX_INSTANCES {
                        self.truncated = true;
                        break;
                    }
                    placed.transforms.push(transform.then(&outer));
                    placed.occurrences.push(parent.occurrence.clone());
                }
            }
            if !placed.transforms.is_empty() {
                placed.unplaced = None;
            } else if placed.unplaced.is_none() && placed.left_out {
                placed.unplaced = Some(Unplaced::Unreadable);
            }
            placed
        };
        self.walked.insert(key, Walk::Done(placements.clone()));
        placements
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unplaced {
    InsideItself,
    TooDeep,
    Unreadable,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Placements {
    pub transforms: Vec<RigidTransform>,
    pub occurrences: Vec<Option<String>>,
    pub unplaced: Option<Unplaced>,
    pub left_out: bool,
}

impl Placements {
    pub fn at_origin() -> Self {
        Self {
            transforms: vec![RigidTransform::IDENTITY],
            occurrences: vec![None],
            unplaced: None,
            left_out: false,
        }
    }

    fn unplaced(reason: Unplaced) -> Self {
        Self {
            transforms: Vec::new(),
            occurrences: Vec::new(),
            unplaced: Some(reason),
            left_out: false,
        }
    }
}

struct Context<'a> {
    children: &'a BTreeSet<u64>,
    definitions: &'a BTreeMap<u64, Vec<u64>>,
    occurrences: &'a BTreeMap<u64, Occurrence>,
}

fn occurrences(graph: &Graph<'_>) -> BTreeMap<u64, Occurrence> {
    let mut found = BTreeMap::new();
    for entity in graph.entities() {
        if entity.kind() != "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION" {
            continue;
        }
        let Ok(fields) = entity.fields() else {
            continue;
        };
        let (Ok(relationship), Ok(shape)) = (fields.reference(0), fields.reference(1)) else {
            continue;
        };
        let usage = graph
            .entity(shape)
            .ok()
            .and_then(|shape| shape.fields().ok()?.reference(2).ok())
            .and_then(|usage| graph.entity(usage).ok());
        let Some(usage) = usage else {
            continue;
        };
        let Ok(usage_fields) = usage.fields() else {
            continue;
        };
        let Ok(child) = usage_fields.reference(4) else {
            continue;
        };
        let name = usage_fields.name(1).or_else(|| usage_fields.name(5));
        found.insert(relationship, Occurrence { child, name });
    }
    found
}

#[derive(Debug, Clone)]
enum Walk {
    InProgress,
    Done(Placements),
}

fn frame(geometry: &Geometry<'_>, id: u64) -> Option<RigidTransform> {
    let plane: Plane = geometry.placement(id).ok()?;
    RigidTransform::from_frame(&plane)
}

struct Operator {
    axis1: Option<u64>,
    axis2: Option<u64>,
    origin: u64,
    scale: Option<f64>,
    axis3: Option<u64>,
}

fn operator_fields(graph: &Graph<'_>, entity: Entity<'_>) -> Option<Operator> {
    let scale = |fields: &Fields<'_>, index: usize| fields.get(index).ok()?.real();
    if let (Some(base), Some(spatial)) = (
        entity.find("CARTESIAN_TRANSFORMATION_OPERATOR"),
        entity.find("CARTESIAN_TRANSFORMATION_OPERATOR_3D"),
    ) {
        return Some(Operator {
            axis1: base.optional_reference(0),
            axis2: base.optional_reference(1),
            origin: base.reference(2).ok()?,
            scale: scale(&base, 3),
            axis3: spatial.optional_reference(0),
        });
    }
    let fields = entity.find("CARTESIAN_TRANSFORMATION_OPERATOR_3D")?;
    let is_point = |index: usize| {
        fields
            .optional_reference(index)
            .and_then(|id| graph.entity(id).ok())
            .is_some_and(|point| point.kind() == "CARTESIAN_POINT")
    };
    let origin = (2..OPERATOR_FIELDS).find(|index| is_point(*index))?;
    Some(Operator {
        axis1: fields.optional_reference(origin - 2),
        axis2: fields.optional_reference(origin - 1),
        origin: fields.reference(origin).ok()?,
        scale: scale(&fields, origin + 1),
        axis3: fields.optional_reference(origin + 2),
    })
}

fn operator(geometry: &Geometry<'_>, id: u64) -> Option<RigidTransform> {
    let entity = geometry.graph.entity(id).ok()?;
    let fields = operator_fields(&geometry.graph, entity)?;
    if fields
        .scale
        .is_some_and(|scale| (scale - 1.0).abs() > RIGID_SLACK)
    {
        return None;
    }
    let direction = |id: Option<u64>| match id {
        Some(id) => geometry.direction(id).ok().map(Some),
        None => Some(None),
    };
    let z = direction(fields.axis3)?.unwrap_or(Vector3::Z);
    let x = direction(fields.axis1)?
        .unwrap_or(Vector3::X)
        .reject_from_normalized(z)
        .try_normalize()
        .or_else(|| Vector3::Y.reject_from_normalized(z).try_normalize())?;
    if let Some(y) = direction(fields.axis2)? {
        let y = y.reject_from_normalized(x).reject_from_normalized(z);
        if y.dot(z.cross(x)) <= 0.0 {
            return None;
        }
    }
    let origin = geometry.point(fields.origin).ok()?;
    RigidTransform::from_frame(&Plane::with_x_axis(origin, z, x)?)
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
                fields.name(1).or_else(|| fields.name(0))
            });
        if let Some(name) = name {
            for representation in representations {
                names.insert(*representation, name.clone());
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use super::*;
    use crate::part21::parse;

    const ORIGIN: &str = "#1=AXIS2_PLACEMENT_3D('',#2,#3,#4);#2=CARTESIAN_POINT('',(0.,0.,0.));\
                          #3=DIRECTION('',(0.,0.,1.));#4=DIRECTION('',(1.,0.,0.));\
                          #5=ITEM_DEFINED_TRANSFORMATION('','',#1,#1);";

    fn representation(id: u64) -> String {
        format!("#{id}=SHAPE_REPRESENTATION('',(#1),#9);")
    }

    fn placed(id: u64, child: u64, parent: u64) -> String {
        format!(
            "#{id}=(REPRESENTATION_RELATIONSHIP('','',#{child},#{parent})\
             REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#5)\
             SHAPE_REPRESENTATION_RELATIONSHIP());"
        )
    }

    fn file(data: &str) -> String {
        format!("ISO-10303-21;HEADER;ENDSEC;DATA;{ORIGIN}{data}ENDSEC;END-ISO-10303-21;")
    }

    #[test]
    fn a_layered_assembly_is_walked_once_per_part_and_capped_per_file() {
        let layers = 30_u64;
        let mut data = String::new();
        let mut relation = 10_000;
        for layer in 0..=layers {
            for side in 0..2 {
                data.push_str(&representation(100 + layer * 2 + side));
            }
        }
        for layer in 0..layers {
            for side in 0..2 {
                for parent in 0..2 {
                    relation += 1;
                    let child = 100 + layer * 2 + side;
                    write!(
                        data,
                        "{}",
                        placed(relation, child, 102 + layer * 2 + parent)
                    )
                    .unwrap();
                }
            }
        }
        let text = file(&data);
        let exchange = parse(&text).unwrap();
        let graph = Graph::new(&exchange);
        let mut structure = Structure::read(&graph);
        let started = std::time::Instant::now();
        let placements = structure.placements(100);
        assert!(started.elapsed().as_secs() < 5);
        assert_eq!(placements.transforms.len(), MAX_INSTANCES);
        assert!(structure.truncated);
    }

    #[test]
    fn a_part_placed_only_inside_itself_is_reported() {
        let data = [
            representation(100),
            representation(101),
            representation(102),
            placed(200, 100, 101),
            placed(201, 101, 100),
        ]
        .concat();
        let text = file(&data);
        let exchange = parse(&text).unwrap();
        let graph = Graph::new(&exchange);
        let mut structure = Structure::read(&graph);
        assert_eq!(
            structure.placements(100).unplaced,
            Some(Unplaced::InsideItself)
        );

        let data = [data, placed(202, 101, 102)].concat();
        let text = file(&data);
        let exchange = parse(&text).unwrap();
        let graph = Graph::new(&exchange);
        let mut structure = Structure::read(&graph);
        let placements = structure.placements(100);
        assert_eq!(placements.transforms, [RigidTransform::IDENTITY]);
        assert_eq!(placements.unplaced, None);
    }

    #[test]
    fn a_nested_subassembly_is_placed_by_its_occurrences_whichever_way_it_is_listed() {
        let data = "\
            #20=CARTESIAN_POINT('',(10.,0.,0.));#21=AXIS2_PLACEMENT_3D('',#20,#3,#4);\
            #22=CARTESIAN_POINT('',(0.,5.,0.));#23=AXIS2_PLACEMENT_3D('',#22,#3,#4);\
            #30=ITEM_DEFINED_TRANSFORMATION('','',#1,#21);\
            #31=ITEM_DEFINED_TRANSFORMATION('','',#23,#1);\
            #100=SHAPE_REPRESENTATION('',(#1),#9);\
            #101=SHAPE_REPRESENTATION('',(#1),#9);\
            #102=SHAPE_REPRESENTATION('',(#1),#9);\
            #400=PRODUCT_DEFINITION_SHAPE('','',#300);\
            #401=PRODUCT_DEFINITION_SHAPE('','',#301);\
            #402=PRODUCT_DEFINITION_SHAPE('','',#302);\
            #410=SHAPE_DEFINITION_REPRESENTATION(#400,#100);\
            #411=SHAPE_DEFINITION_REPRESENTATION(#401,#101);\
            #412=SHAPE_DEFINITION_REPRESENTATION(#402,#102);\
            #500=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','Frame:1','',#300,#301,$);\
            #501=NEXT_ASSEMBLY_USAGE_OCCURRENCE('2','Bolt:1','',#301,#302,$);\
            #510=PRODUCT_DEFINITION_SHAPE('','',#500);\
            #511=PRODUCT_DEFINITION_SHAPE('','',#501);\
            #600=(REPRESENTATION_RELATIONSHIP('','',#101,#100)\
            REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#30)\
            SHAPE_REPRESENTATION_RELATIONSHIP());\
            #601=(REPRESENTATION_RELATIONSHIP('','',#101,#102)\
            REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#31)\
            SHAPE_REPRESENTATION_RELATIONSHIP());\
            #700=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#600,#510);\
            #701=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#601,#511);";
        let text = file(data);
        let exchange = parse(&text).unwrap();
        let graph = Graph::new(&exchange);
        let mut structure = Structure::read(&graph);
        let moved = |x: f64, y: f64| {
            RigidTransform::translation(caditor_geometry::Vector3::new(x, y, 0.0)).unwrap()
        };
        let frame = structure.placements(101);
        assert_eq!(frame.transforms, [moved(10.0, 0.0)]);
        assert_eq!(frame.occurrences, [Some("Frame:1".to_owned())]);
        let bolt = structure.placements(102);
        assert_eq!(bolt.transforms.len(), 1);
        let origin = bolt.transforms[0].apply_point(caditor_geometry::Point3::ZERO);
        assert!(
            origin.distance(caditor_geometry::Point3::new(10.0, 5.0, 0.0)) < 1e-12,
            "{origin}"
        );
        assert_eq!(bolt.occurrences, [Some("Bolt:1".to_owned())]);
        assert_eq!(
            structure.placements(100).transforms,
            [RigidTransform::IDENTITY]
        );
    }

    fn operated(operator: &str) -> Placements {
        let data = format!(
            "#20=CARTESIAN_POINT('',(5.,0.,0.));#21=DIRECTION('',(0.,1.,0.));\
             #22=DIRECTION('',(-1.,0.,0.));#30={operator};\
             #100=SHAPE_REPRESENTATION('',(#1),#9);\
             #101=SHAPE_REPRESENTATION('',(#1),#9);\
             #600=(REPRESENTATION_RELATIONSHIP('','',#100,#101)\
             REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#30)\
             SHAPE_REPRESENTATION_RELATIONSHIP());"
        );
        let text = file(&data);
        let exchange = parse(&text).unwrap();
        let graph = Graph::new(&exchange);
        Structure::read(&graph).placements(100)
    }

    #[test]
    fn a_part_placed_by_a_transformation_operator_is_turned_and_moved() {
        use caditor_geometry::Point3;

        let simple = operated("CARTESIAN_TRANSFORMATION_OPERATOR_3D('','',$,#21,#22,#20,1.,#3)");
        let complex = operated(
            "(CARTESIAN_TRANSFORMATION_OPERATOR(#21,$,#20,$)\
             CARTESIAN_TRANSFORMATION_OPERATOR_3D(#3)\
             FUNCTIONALLY_DEFINED_TRANSFORMATION('','')\
             GEOMETRIC_REPRESENTATION_ITEM()REPRESENTATION_ITEM(''))",
        );

        for placements in [simple, complex] {
            assert_eq!(placements.unplaced, None);
            assert!(!placements.left_out);
            let [transform] = placements.transforms.as_slice() else {
                panic!("expected one placement, found {placements:?}");
            };
            let moved = transform.apply_point(Point3::X);
            assert!(
                moved.distance(Point3::new(5.0, 1.0, 0.0)) < 1e-12,
                "{moved}"
            );
        }
    }

    #[test]
    fn a_part_placed_by_a_scaling_or_mirroring_operator_is_left_out_with_the_reason() {
        let scaled = operated("CARTESIAN_TRANSFORMATION_OPERATOR_3D('','',$,#21,$,#20,2.,#3)");
        let mirrored = operated("CARTESIAN_TRANSFORMATION_OPERATOR_3D('','',$,#21,#4,#20,1.,#3)");

        for placements in [scaled, mirrored] {
            assert!(placements.transforms.is_empty());
            assert_eq!(placements.unplaced, Some(Unplaced::Unreadable));
        }
    }
}
