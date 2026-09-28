use crate::{
    boolean::{
        BooleanError, BooleanOperation, FaceKey, Input, Operand,
        faces::SplitFace,
        trace::{Fragment, interior_points},
    },
    interrupt,
    naming::{FaceName, FaceOrigin},
    sense::Sense,
    surface::Surface,
    topology::BoundaryClass,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
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

fn classify(
    input: &Input,
    key: FaceKey,
    surface: &Surface,
    sense: Sense,
    fragment: &Fragment,
) -> Result<Class, BooleanError> {
    let classifier = input.classifier(key.operand.other());
    let mut solid_side = None;
    let mut coincident = None;
    for uv in interior_points(fragment, surface) {
        let Some(normal) = surface.normal(uv.x, uv.y) else {
            continue;
        };
        let found = classifier.classify_boundary_point(surface.point_at(uv), normal * sense.sign());
        match found {
            BoundaryClass::Inside | BoundaryClass::Outside => {
                let class = if matches!(found, BoundaryClass::Inside) {
                    Class::Inside
                } else {
                    Class::Outside
                };
                if solid_side.is_some_and(|known| known != class) {
                    return Err(BooleanError::Ambiguous);
                }
                solid_side = Some(class);
            }
            BoundaryClass::Coincident { sense, .. } => {
                coincident = coincident.or(Some(Class::Coincident(sense)));
            }
            BoundaryClass::Touching(_) | BoundaryClass::Undecided => {}
        }
    }
    solid_side.or(coincident).ok_or(BooleanError::Ambiguous)
}

pub(super) fn select(
    input: &Input,
    split: Vec<SplitFace>,
    operation: BooleanOperation,
) -> Result<Vec<KeptFace>, BooleanError> {
    let mut kept = Vec::new();
    for face in split {
        interrupt::check()?;
        let original = input.face(face.key).ok_or(BooleanError::Split)?;
        for fragment in face.fragments {
            let class = classify(
                input,
                face.key,
                original.surface(),
                original.sense(),
                &fragment,
            )?;
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
