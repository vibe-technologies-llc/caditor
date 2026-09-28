use std::collections::BTreeSet;

use caditor_document::{Document, Evaluation, FeatureId, FeatureResult, SketchRegion};
use caditor_geometry::{Plane, Vector3};
use caditor_kernel::{EdgeName, RegionKey};
use caditor_sketch::{ConstraintId, EntityId};

use crate::{
    blend_tools,
    bodies::{self, FaceKey},
    editing::Context,
};

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

    pub fn rgb(self) -> [u8; 3] {
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
        match self {
            Self::X => "X axis",
            Self::Y => "Y axis",
            Self::Z => "Z axis",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrincipalPlane {
    Xy,
    Xz,
    Yz,
}

impl PrincipalPlane {
    pub const ALL: [Self; 3] = [Self::Xy, Self::Xz, Self::Yz];

    pub fn plane(self) -> Plane {
        match self {
            Self::Xy => Plane::XY,
            Self::Xz => Plane::XZ,
            Self::Yz => Plane::YZ,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Xy => "XY plane",
            Self::Xz => "XZ plane",
            Self::Yz => "YZ plane",
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
    Face {
        body: FeatureId,
        face: FaceKey,
    },
    Edge {
        body: FeatureId,
        edge: EdgeName,
    },
    Region {
        feature: FeatureId,
        region: RegionKey,
    },
    BlendEdge {
        feature: FeatureId,
        edge: EdgeName,
    },
}

pub fn swept_regions<'a>(
    document: &Document,
    evaluation: &'a Evaluation,
    feature: FeatureId,
) -> Option<(FeatureId, &'a [SketchRegion])> {
    let sketch = document.feature(feature)?.kind.solid()?.sketch();
    let regions = evaluation
        .feature(sketch)?
        .result
        .as_deref()
        .and_then(FeatureResult::sketch)?
        .regions()?
        .as_ref()
        .ok()?;
    Some((sketch, regions.as_slice()))
}

fn body_name(document: &Document, body: FeatureId) -> &str {
    document
        .feature(body)
        .map_or("A deleted body", |feature| feature.name.as_str())
}

impl Pickable {
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
                match evaluation.body(body) {
                    Some(solid) => {
                        format!("{name} › {}", bodies::describe_face(document, solid, face))
                    }
                    None => format!("{name} › Face"),
                }
            }
            Self::Edge { body, edge } => {
                let name = body_name(document, body);
                match evaluation.body(body) {
                    Some(solid) => {
                        format!("{name} › {}", bodies::describe_edge(document, solid, edge))
                    }
                    None => format!("{name} › Edge"),
                }
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
                let described = blend_tools::input_solid(evaluation, feature).map_or_else(
                    || "Edge".to_owned(),
                    |solid| bodies::describe_edge(document, solid, edge),
                );
                format!("{described}: click to add to {owner} or leave it out")
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
        match self {
            Self::SketchEntity { feature, entity } => {
                let in_context = editing.is_none_or(|edited| edited == feature);
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
                in_context && exists
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
            Self::Origin | Self::Axis(_) | Self::Plane(_) => editing.is_none(),
            Self::Face { body, face } => {
                editing.is_none()
                    && evaluation
                        .body(body)
                        .is_some_and(|solid| bodies::find_face(solid, face).is_some())
            }
            Self::Edge { body, edge } => {
                editing.is_none()
                    && evaluation
                        .body(body)
                        .is_some_and(|solid| bodies::find_edge(solid, edge).is_some())
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
                    && blend_tools::input_solid(evaluation, feature)
                        .is_some_and(|solid| bodies::find_edge(solid, edge).is_some())
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    items: BTreeSet<Pickable>,
}

impl Selection {
    pub fn contains(&self, pickable: Pickable) -> bool {
        self.items.contains(&pickable)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = Pickable> + '_ {
        self.items.iter().copied()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn replace_with(&mut self, pickable: Pickable) {
        self.items.clear();
        self.items.insert(pickable);
    }

    pub fn toggle(&mut self, pickable: Pickable) {
        if !self.items.remove(&pickable) {
            self.items.insert(pickable);
        }
    }

    pub fn retain_available(
        &mut self,
        document: &Document,
        evaluation: &Evaluation,
        context: Context,
    ) {
        self.items
            .retain(|pickable| pickable.is_available(document, evaluation, context));
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
}
