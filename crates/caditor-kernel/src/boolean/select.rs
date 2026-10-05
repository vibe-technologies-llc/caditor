use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Aabb, Point3};

use crate::{
    boolean::{
        BooleanError, BooleanOperation, FaceKey, Input, Operand, TOLERANCE,
        faces::SplitFace,
        imprint::Arrangement,
        trace::{Fragment, deepest_points, interior_points},
    },
    interrupt,
    intersect::boxes_overlap,
    naming::{FaceName, FaceOrigin},
    sense::Sense,
    surface::Surface,
    topology::{BoundaryClass, PointClass, SolidClassifier},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Class {
    Inside,
    Outside,
    Coincident(Sense),
}

#[derive(Debug, Clone)]
pub(super) struct KeptFace {
    pub key: FaceKey,
    pub surface: Surface,
    pub sense: Sense,
    pub name: FaceName,
    pub origin: Option<FaceOrigin>,
    pub fragment: Fragment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Keep {
    No,
    AsIs,
    Reversed,
}

fn keep(operand: Operand, class: Class, operation: BooleanOperation) -> Keep {
    use BooleanOperation::{Difference, Intersection, Union};
    match (operand, class, operation) {
        (Operand::First, Class::Outside, Union | Difference)
        | (Operand::First, Class::Inside, Intersection)
        | (Operand::First, Class::Coincident(Sense::Same), Union | Intersection)
        | (Operand::First, Class::Coincident(Sense::Reversed), Difference)
        | (Operand::Second, Class::Outside, Union)
        | (Operand::Second, Class::Inside, Intersection) => Keep::AsIs,
        (Operand::Second, Class::Inside, Difference) => Keep::Reversed,
        _ => Keep::No,
    }
}

pub(super) fn classify(
    input: &Input,
    key: FaceKey,
    surface: &Surface,
    sense: Sense,
    fragment: &Fragment,
) -> Result<Class, BooleanError> {
    let classifier = input.classifier(key.operand.other());
    let mut solid_side = None;
    let mut coincident: Option<Class> = None;
    let mut mixed_at = None;
    let mut first_sample = None;
    for uv in interior_points(fragment, surface) {
        let Some(normal) = surface.normal(uv.x, uv.y) else {
            continue;
        };
        let point = surface.point_at(uv);
        first_sample.get_or_insert(point);
        let ambiguous = || BooleanError::Ambiguous(Box::default()).or_point(|| Some(point));
        let found = classifier.classify_boundary_point(point, normal * sense.sign());
        match found {
            BoundaryClass::Inside | BoundaryClass::Outside => {
                let class = if matches!(found, BoundaryClass::Inside) {
                    Class::Inside
                } else {
                    Class::Outside
                };
                if solid_side.is_some_and(|known| known != class) {
                    return Err(ambiguous().or_faces([key]));
                }
                solid_side = Some(class);
            }
            BoundaryClass::Coincident { sense, .. } => {
                let class = Class::Coincident(sense);
                if coincident.is_some_and(|known| known != class) {
                    mixed_at.get_or_insert(point);
                }
                coincident = Some(class);
            }
            BoundaryClass::Touching(_) | BoundaryClass::Undecided => {}
        }
    }
    let ambiguous = |point: Option<Point3>| {
        BooleanError::Ambiguous(Box::default())
            .or_faces([key])
            .or_point(|| point)
    };
    if let Some(point) = mixed_at {
        return Err(ambiguous(Some(point)));
    }
    solid_side
        .or(coincident)
        .or_else(|| classify_thin(classifier, surface, fragment))
        .ok_or_else(|| ambiguous(first_sample))
}

fn classify_thin(
    classifier: &SolidClassifier,
    surface: &Surface,
    fragment: &Fragment,
) -> Option<Class> {
    deepest_points(fragment, surface)
        .into_iter()
        .find_map(
            |uv| match classifier.side_of_touched_faces(surface.point_at(uv)) {
                PointClass::Inside => Some(Class::Inside),
                PointClass::Outside => Some(Class::Outside),
                PointClass::OnBoundary(_) | PointClass::Undecided => None,
            },
        )
}

struct Components {
    parents: BTreeMap<FaceKey, FaceKey>,
}

impl Components {
    fn root(&self, mut key: FaceKey) -> FaceKey {
        while let Some(parent) = self.parents.get(&key)
            && *parent != key
        {
            key = *parent;
        }
        key
    }

    fn join(&mut self, first: FaceKey, second: FaceKey) {
        let (first, second) = (self.root(first), self.root(second));
        if first != second {
            self.parents.insert(first.max(second), first.min(second));
        }
    }
}

fn untouched_components(
    input: &Input,
    arrangement: &Arrangement,
    split: &[SplitFace],
) -> Components {
    let untouched: BTreeSet<FaceKey> = split
        .iter()
        .filter(|face| face.untouched)
        .map(|face| face.key)
        .collect();
    let shared = arrangement.shared_pieces();
    let mut components = Components {
        parents: BTreeMap::new(),
    };
    for operand in Operand::BOTH {
        let solid = input.solid(operand);
        for (id, edge) in solid.edges() {
            let [piece] = arrangement.edge_pieces(operand, id) else {
                continue;
            };
            if shared.contains(piece) {
                continue;
            }
            let faces: Vec<FaceKey> = edge
                .coedges()
                .iter()
                .filter_map(|coedge| solid.coedge_face(*coedge))
                .map(|face| FaceKey { operand, face })
                .collect();
            if let [first, second] = faces.as_slice()
                && untouched.contains(first)
                && untouched.contains(second)
            {
                components.join(*first, *second);
            }
        }
    }
    components
}

fn extent(input: &Input, operand: Operand) -> Option<Aabb> {
    input
        .faces(operand)
        .iter()
        .map(|face| face.bounds)
        .reduce(Aabb::union)
}

fn apart(input: &Input, key: FaceKey, other: Option<&Aabb>) -> bool {
    let Some(other) = other else {
        return true;
    };
    input
        .bounds(key)
        .is_some_and(|bounds| !boxes_overlap(&bounds.bounds, other, TOLERANCE))
}

pub(super) fn select(
    input: &Input,
    arrangement: &Arrangement,
    split: Vec<SplitFace>,
    operation: BooleanOperation,
) -> Result<Vec<KeptFace>, BooleanError> {
    let components = untouched_components(input, arrangement, &split);
    let extents = Operand::BOTH.map(|operand| extent(input, operand));
    let mut shared_classes: BTreeMap<FaceKey, Class> = BTreeMap::new();
    let mut kept = Vec::new();
    for face in split {
        interrupt::check()?;
        let original = input
            .face(face.key)
            .ok_or_else(|| BooleanError::split().or_faces([face.key]))?;
        let root = components.root(face.key);
        let [first_extent, second_extent] = &extents;
        let other_extent = match face.key.operand {
            Operand::First => second_extent.as_ref(),
            Operand::Second => first_extent.as_ref(),
        };
        for fragment in face.fragments {
            let known = if face.untouched {
                shared_classes.get(&root).copied()
            } else {
                None
            };
            let class = match known {
                Some(class) => class,
                None if face.untouched && apart(input, face.key, other_extent) => Class::Outside,
                None => classify(
                    input,
                    face.key,
                    original.surface(),
                    original.sense(),
                    &fragment,
                )?,
            };
            if face.untouched && !matches!(class, Class::Coincident(_)) {
                shared_classes.insert(root, class);
            }
            let (sense, fragment) = match keep(face.key.operand, class, operation) {
                Keep::No => continue,
                Keep::AsIs => (original.sense(), fragment),
                Keep::Reversed => (original.sense().reversed(), fragment.reversed()),
            };
            kept.push(KeptFace {
                key: face.key,
                surface: original.surface().clone(),
                sense,
                name: original.name(),
                origin: original.origin(),
                fragment,
            });
        }
    }
    Ok(kept)
}
