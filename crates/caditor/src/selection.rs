use std::collections::BTreeSet;

use caditor_document::{Document, FeatureId, FeatureKind};
use caditor_geometry::{Plane, Vector3};
use caditor_sketch::{Entity, EntityId};

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
}

impl Pickable {
    pub fn describe(self, document: &Document) -> String {
        match self {
            Self::Origin => "Origin".to_owned(),
            Self::Axis(axis) => axis.name().to_owned(),
            Self::Plane(plane) => plane.name().to_owned(),
            Self::SketchEntity { feature, entity } => {
                let Some(owner) = document.feature(feature) else {
                    return format!("Missing entity {entity}");
                };
                let FeatureKind::Sketch(sketch) = &owner.kind;
                let kind = match sketch.entity(entity) {
                    Some(Entity::Point(_)) => "Point",
                    Some(Entity::Line { .. }) => "Line",
                    None => "Missing entity",
                };
                format!("{} › {kind} {entity}", owner.name)
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

    pub fn retain_existing(&mut self, document: &Document) {
        self.items.retain(|pickable| match *pickable {
            Pickable::SketchEntity { feature, entity } => {
                document
                    .feature(feature)
                    .is_some_and(|owner| match &owner.kind {
                        FeatureKind::Sketch(sketch) => sketch.entity(entity).is_some(),
                    })
            }
            Pickable::Origin | Pickable::Axis(_) | Pickable::Plane(_) => true,
        });
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
