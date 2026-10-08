use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};

pub use caditor_document::PrincipalPlane;
use caditor_document::{
    Document, Evaluation, FeatureId, FeatureResult, PrincipalAxis, PrincipalGeometry, SketchRegion,
};
use caditor_geometry::Vector3;
use caditor_kernel::{EdgeName, RegionKey};
use caditor_sketch::{ConstraintId, EntityId};

use crate::{
    bodies::{self, FaceKey, VertexKey},
    datum_tools,
    editing::Context,
    variants::all_variants,
    visibility,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SelectionFilter {
    #[default]
    Everything,
    Faces,
    Edges,
    Vertices,
    SketchGeometry,
}

all_variants!(SelectionFilter: Everything, Faces, Edges, Vertices, SketchGeometry);

impl SelectionFilter {
    pub fn allows(self, pickable: Pickable) -> bool {
        match self {
            Self::Everything => true,
            Self::Faces => matches!(pickable, Pickable::Face { .. }),
            Self::Edges => matches!(pickable, Pickable::Edge { .. }),
            Self::Vertices => matches!(pickable, Pickable::Vertex { .. }),
            Self::SketchGeometry => matches!(
                pickable,
                Pickable::SketchEntity { .. }
                    | Pickable::SketchConstraint { .. }
                    | Pickable::SketchRegion { .. }
                    | Pickable::Region { .. }
            ),
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Everything => "select.everything",
            Self::Faces => "select.faces",
            Self::Edges => "select.edges",
            Self::Vertices => "select.vertices",
            Self::SketchGeometry => "select.sketch_geometry",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Everything => "Select anything",
            Self::Faces => "Select faces only",
            Self::Edges => "Select edges only",
            Self::Vertices => "Select vertices only",
            Self::SketchGeometry => "Select sketch geometry only",
        }
    }

    pub fn status(self) -> &'static str {
        match self {
            Self::Everything => "Selecting anything",
            Self::Faces => "Selecting faces only",
            Self::Edges => "Selecting edges only",
            Self::Vertices => "Selecting vertices only",
            Self::SketchGeometry => "Selecting sketch geometry only",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Self; 3] = [Self::X, Self::Y, Self::Z];

    pub fn direction(self) -> Vector3 {
        match self {
            Self::X => Vector3::X,
            Self::Y => Vector3::Y,
            Self::Z => Vector3::Z,
        }
    }

    pub const fn rgb(self) -> [u8; 3] {
        match self {
            Self::X => [226, 84, 84],
            Self::Y => [112, 196, 88],
            Self::Z => [84, 144, 238],
        }
    }

    pub fn letter(self) -> &'static str {
        match self {
            Self::X => "X",
            Self::Y => "Y",
            Self::Z => "Z",
        }
    }

    pub fn name(self) -> &'static str {
        self.principal().name()
    }

    pub fn principal(self) -> PrincipalAxis {
        match self {
            Self::X => PrincipalAxis::X,
            Self::Y => PrincipalAxis::Y,
            Self::Z => PrincipalAxis::Z,
        }
    }

    pub fn of(axis: PrincipalAxis) -> Self {
        match axis {
            PrincipalAxis::X => Self::X,
            PrincipalAxis::Y => Self::Y,
            PrincipalAxis::Z => Self::Z,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Pickable {
    Origin,
    Axis(Axis),
    Plane(PrincipalPlane),
    SketchEntity {
        feature: FeatureId,
        entity: EntityId,
    },
    SketchConstraint {
        feature: FeatureId,
        constraint: ConstraintId,
    },
    SketchRegion {
        feature: FeatureId,
        region: RegionKey,
    },
    Face {
        body: FeatureId,
        face: FaceKey,
    },
    Edge {
        body: FeatureId,
        edge: EdgeName,
    },
    Vertex {
        body: FeatureId,
        vertex: VertexKey,
    },
    Region {
        feature: FeatureId,
        region: RegionKey,
    },
    BlendEdge {
        feature: FeatureId,
        edge: EdgeName,
    },
    ShellFace {
        feature: FeatureId,
        face: FaceKey,
    },
    Datum(FeatureId),
}

pub fn sketch_regions(evaluation: &Evaluation, sketch: FeatureId) -> Option<&[SketchRegion]> {
    evaluation
        .feature(sketch)?
        .result
        .as_deref()
        .and_then(FeatureResult::sketch)?
        .regions()?
        .as_ref()
        .ok()
        .map(Vec::as_slice)
}

pub fn swept_regions<'a>(
    document: &Document,
    evaluation: &'a Evaluation,
    feature: FeatureId,
) -> Option<(FeatureId, &'a [SketchRegion])> {
    let sketch = document.feature(feature)?.kind.solid()?.sketch();
    Some((sketch, sketch_regions(evaluation, sketch)?))
}

fn body_name(document: &Document, body: FeatureId) -> &str {
    document
        .feature(body)
        .map_or("A deleted body", |feature| feature.name.as_str())
}

impl Pickable {
    pub fn body(self) -> Option<FeatureId> {
        match self {
            Self::Face { body, .. } | Self::Edge { body, .. } | Self::Vertex { body, .. } => {
                Some(body)
            }
            _ => None,
        }
    }

    pub fn describe(self, document: &Document, evaluation: &Evaluation) -> String {
        match self {
            Self::Origin => "Origin".to_owned(),
            Self::Axis(axis) => axis.name().to_owned(),
            Self::Plane(plane) => plane.name().to_owned(),
            Self::SketchEntity { feature, entity } => {
                let Some(owner) = document.feature(feature) else {
                    return format!("Missing entity {entity}");
                };
                let Some(sketch) = owner.kind.sketch() else {
                    return owner.name.clone();
                };
                format!("{} › {}", owner.name, sketch.entity_label(entity))
            }
            Self::SketchConstraint {
                feature,
                constraint,
            } => {
                let Some(owner) = document.feature(feature) else {
                    return format!("Missing constraint {constraint}");
                };
                let Some(sketch) = owner.kind.sketch() else {
                    return owner.name.clone();
                };
                format!(
                    "{} › {}",
                    owner.name,
                    sketch.describe_constraint(constraint)
                )
            }
            Self::Face { body, face } => {
                let name = body_name(document, body);
                match bodies::shown(evaluation, body) {
                    Some(solid) => {
                        format!("{name} › {}", bodies::describe_face(document, solid, face))
                    }
                    None => format!("{name} › Face"),
                }
            }
            Self::Edge { body, edge } => {
                let name = body_name(document, body);
                match bodies::shown(evaluation, body) {
                    Some(solid) => {
                        format!("{name} › {}", bodies::describe_edge(document, solid, edge))
                    }
                    None => format!("{name} › Edge"),
                }
            }
            Self::Vertex { body, vertex } => {
                let name = body_name(document, body);
                match bodies::shown(evaluation, body) {
                    Some(solid) => {
                        format!(
                            "{name} › {}",
                            bodies::describe_vertex(document, solid, vertex)
                        )
                    }
                    None => format!("{name} › Vertex"),
                }
            }
            Self::SketchRegion { feature, .. } => {
                let sketch = document
                    .feature(feature)
                    .map_or("the sketch", |sketch| sketch.name.as_str());
                format!("{sketch} › Region")
            }
            Self::Region { feature, .. } => {
                let sketch = document
                    .feature(feature)
                    .and_then(|owner| owner.kind.solid())
                    .and_then(|solid| document.feature(solid.sketch()))
                    .map_or("the sketch", |sketch| sketch.name.as_str());
                format!("Region of {sketch}: click to choose or leave out")
            }
            Self::BlendEdge { feature, edge } => {
                let owner = document
                    .feature(feature)
                    .map_or("the feature", |owner| owner.name.as_str());
                let described = bodies::input(evaluation, feature).map_or_else(
                    || "Edge".to_owned(),
                    |solid| bodies::describe_edge(document, solid, edge),
                );
                format!("{described}: click to add to {owner} or leave it out")
            }
            Self::Datum(feature) => document
                .feature(feature)
                .map_or_else(|| "A deleted datum".to_owned(), |datum| datum.name.clone()),
            Self::ShellFace { feature, face } => {
                let owner = document
                    .feature(feature)
                    .map_or("the feature", |owner| owner.name.as_str());
                let described = bodies::input(evaluation, feature).map_or_else(
                    || "Face".to_owned(),
                    |solid| bodies::describe_face(document, solid, face),
                );
                format!("{described}: click to open it in {owner} or close it again")
            }
        }
    }

    pub fn constrained_entities(self, document: &Document) -> Vec<Self> {
        let Self::SketchConstraint {
            feature,
            constraint,
        } = self
        else {
            return Vec::new();
        };
        document
            .feature(feature)
            .and_then(|owner| owner.kind.sketch())
            .and_then(|sketch| sketch.constraint(constraint))
            .map(|constraint| {
                constraint
                    .entities()
                    .into_iter()
                    .map(|entity| Self::SketchEntity { feature, entity })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn is_available(
        self,
        document: &Document,
        evaluation: &Evaluation,
        context: Context,
    ) -> bool {
        let editing = context.sketch;
        let outside_sketch = editing.is_none() || context.projecting;
        match self {
            Self::SketchEntity { feature, entity } => {
                let in_context = editing.is_none_or(|edited| edited == feature)
                    || (context.projecting && !entity.is_reference());
                let exists = document
                    .feature(feature)
                    .and_then(|owner| owner.kind.sketch())
                    .is_some_and(|sketch| {
                        if entity.is_reference() {
                            editing == Some(feature)
                        } else {
                            sketch.entity(entity).is_some()
                        }
                    });
                let shown = editing == Some(feature) || visibility::is_shown(document, feature);
                in_context && exists && shown
            }
            Self::SketchConstraint {
                feature,
                constraint,
            } => {
                editing == Some(feature)
                    && document
                        .feature(feature)
                        .and_then(|owner| owner.kind.sketch())
                        .is_some_and(|sketch| sketch.constraint(constraint).is_some())
            }
            Self::Plane(plane) => {
                editing.is_none()
                    && (context.choosing_plane
                        || visibility::is_principal_shown(
                            document,
                            PrincipalGeometry::Plane(plane),
                        ))
            }
            Self::Origin | Self::Axis(_) => {
                editing.is_none()
                    && visibility::principal(self)
                        .is_some_and(|geometry| visibility::is_principal_shown(document, geometry))
            }
            Self::Face { body, face } => {
                outside_sketch
                    && visibility::is_shown(document, body)
                    && bodies::shown(evaluation, body)
                        .is_some_and(|solid| bodies::find_face(solid, face).is_some())
            }
            Self::Edge { body, edge } => {
                outside_sketch
                    && visibility::is_shown(document, body)
                    && bodies::shown(evaluation, body)
                        .is_some_and(|solid| bodies::find_edge(solid, edge).is_some())
            }
            Self::Vertex { body, vertex } => {
                outside_sketch
                    && visibility::is_shown(document, body)
                    && bodies::shown(evaluation, body)
                        .is_some_and(|solid| bodies::find_vertex(solid, vertex).is_some())
            }
            Self::SketchRegion { feature, region } => {
                editing == Some(feature)
                    && context.selecting
                    && sketch_regions(evaluation, feature).is_some_and(|regions| {
                        regions
                            .iter()
                            .any(|candidate| candidate.region.key() == region)
                    })
            }
            Self::Region { feature, region } => {
                context.solid == Some(feature)
                    && swept_regions(document, evaluation, feature).is_some_and(|(_, regions)| {
                        regions
                            .iter()
                            .any(|candidate| candidate.region.key() == region)
                    })
            }
            Self::BlendEdge { feature, edge } => {
                context.solid == Some(feature)
                    && bodies::input(evaluation, feature)
                        .is_some_and(|solid| bodies::find_edge(solid, edge).is_some())
            }
            Self::Datum(feature) => {
                editing.is_none()
                    && document
                        .feature(feature)
                        .is_some_and(|datum| datum.kind.datum().is_some())
                    && (context.solid == Some(feature) || visibility::is_shown(document, feature))
                    && datum_tools::result(evaluation, feature).is_some()
            }
            Self::ShellFace { feature, face } => {
                context.solid == Some(feature)
                    && bodies::input(evaluation, feature)
                        .is_some_and(|solid| bodies::find_face(solid, face).is_some())
            }
        }
    }
}

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Default)]
pub struct Selection {
    items: BTreeMap<Pickable, u64>,
    next_pick: u64,
    generation: u64,
}

impl PartialEq for Selection {
    fn eq(&self, other: &Self) -> bool {
        self.items.keys().eq(other.items.keys())
    }
}

impl Eq for Selection {}

impl Selection {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    fn changed(&mut self) {
        self.generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
    }

    fn insert(&mut self, pickable: Pickable) {
        let pick = self.next_pick;
        self.next_pick += 1;
        self.items.entry(pickable).or_insert(pick);
    }

    pub fn contains(&self, pickable: Pickable) -> bool {
        self.items.contains_key(&pickable)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = Pickable> + '_ {
        self.items.keys().copied()
    }

    pub fn in_pick_order(&self) -> Vec<Pickable> {
        let mut picked: Vec<(u64, Pickable)> = self
            .items
            .iter()
            .map(|(pickable, pick)| (*pick, *pickable))
            .collect();
        picked.sort_unstable();
        picked.into_iter().map(|(_, pickable)| pickable).collect()
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.changed();
    }

    pub fn replace_with(&mut self, pickable: Pickable) {
        self.items.clear();
        self.insert(pickable);
        self.changed();
    }

    pub fn replace_with_all(&mut self, pickables: impl IntoIterator<Item = Pickable>) {
        self.items.clear();
        for pickable in pickables {
            self.insert(pickable);
        }
        self.changed();
    }

    pub fn extend(&mut self, pickables: impl IntoIterator<Item = Pickable>) {
        for pickable in pickables {
            self.insert(pickable);
        }
        self.changed();
    }

    pub fn toggle(&mut self, pickable: Pickable) {
        if self.items.remove(&pickable).is_none() {
            self.insert(pickable);
        }
        self.changed();
    }

    pub fn retain_available(
        &mut self,
        document: &Document,
        evaluation: &Evaluation,
        context: Context,
    ) {
        let before = self.items.len();
        self.items
            .retain(|pickable, _| pickable.is_available(document, evaluation, context));
        if self.items.len() != before {
            self.changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_adds_then_removes_and_replace_keeps_only_one() {
        let mut selection = Selection::default();
        selection.toggle(Pickable::Origin);
        selection.toggle(Pickable::Axis(Axis::X));
        assert_eq!(selection.iter().count(), 2);

        selection.toggle(Pickable::Origin);
        assert!(!selection.contains(Pickable::Origin));

        selection.replace_with(Pickable::Plane(PrincipalPlane::Xy));
        assert_eq!(
            selection.iter().collect::<Vec<_>>(),
            vec![Pickable::Plane(PrincipalPlane::Xy)]
        );
    }

    #[test]
    fn the_pick_order_is_kept_beside_the_sorted_items() {
        let mut selection = Selection::default();
        selection.toggle(Pickable::Plane(PrincipalPlane::Yz));
        selection.toggle(Pickable::Origin);
        selection.extend([Pickable::Axis(Axis::Z), Pickable::Plane(PrincipalPlane::Yz)]);

        assert_eq!(
            selection.in_pick_order(),
            vec![
                Pickable::Plane(PrincipalPlane::Yz),
                Pickable::Origin,
                Pickable::Axis(Axis::Z)
            ]
        );
        assert_eq!(selection.iter().next(), Some(Pickable::Origin));

        selection.toggle(Pickable::Plane(PrincipalPlane::Yz));
        selection.toggle(Pickable::Plane(PrincipalPlane::Yz));

        assert_eq!(
            selection.in_pick_order().last(),
            Some(&Pickable::Plane(PrincipalPlane::Yz))
        );
    }
}
