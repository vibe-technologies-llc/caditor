use caditor_document::{Document, Edit, FeatureId, FeatureKind, ParameterOwner, Transaction};
use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Ray, Vector2};
use caditor_render::{Batch, View};

use crate::{
    feature_tree,
    hole_handles::{HoleDrag, HoleHandles},
    model::Model,
    move_manipulator::{Handle, MoveDrag, MoveHandles},
    reach_handles::{ReachDrag, ReachHandles},
    scene_palette::ScenePalette,
    units::Units,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Held {
    Typed,
    Named(ParameterId),
    Driven(String),
}

impl Held {
    pub fn of(
        document: &Document,
        feature: FeatureId,
        caption: &str,
        expression: &Expression,
    ) -> Self {
        let owner = ParameterOwner::Feature {
            feature,
            value: caption.to_owned(),
        };
        let named = document.owned_parameter(&owner, expression);
        let shown = named.map_or(expression, |parameter| &parameter.expression);
        let used = shown.parameters();
        if !used.is_empty() {
            let names: Vec<String> = used
                .into_iter()
                .map(|id| {
                    document.parameter(id).map_or_else(
                        || "a deleted parameter".to_owned(),
                        |used| used.name.clone(),
                    )
                })
                .collect();
            let names = feature_tree::in_words(&names);
            return Self::Driven(format!(
                "{caption} follows {names}; change {names} in Parameters"
            ));
        }
        named.map_or(Self::Typed, |parameter| Self::Named(parameter.id()))
    }

    pub fn driven(self) -> Option<String> {
        match self {
            Self::Driven(words) => Some(words),
            Self::Typed | Self::Named(_) => None,
        }
    }

    pub fn set(self, slot: &mut Expression, value: Expression, edits: &mut Vec<Edit>) {
        match self {
            Self::Typed => *slot = value,
            Self::Named(id) => edits.push(Edit::SetParameterExpression {
                id,
                expression: value,
            }),
            Self::Driven(_) => {}
        }
    }
}

pub fn keeping_names(
    document: &Document,
    feature: FeatureId,
    kind: FeatureKind,
    named: Vec<Edit>,
) -> Option<Transaction> {
    let owner = document.feature(feature)?;
    let changed = (owner.kind != kind).then_some(Edit::SetFeatureKind { id: feature, kind });
    Some(Transaction::new(
        format!("Edit {}", owner.name),
        changed.into_iter().chain(named).collect(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Manipulator {
    Move(MoveHandles),
    Reach(ReachHandles),
    Hole(HoleHandles),
}

impl Manipulator {
    pub fn of(
        model: &Model,
        open: Option<FeatureId>,
        view: &View,
        pixels_per_point: f64,
    ) -> Option<Self> {
        let feature = open?;
        MoveHandles::of(model, open, view, pixels_per_point)
            .map(Self::Move)
            .or_else(|| ReachHandles::of(model, feature, view, pixels_per_point).map(Self::Reach))
            .or_else(|| HoleHandles::of(model, feature, view, pixels_per_point).map(Self::Hole))
    }

    pub fn feature(&self) -> FeatureId {
        match self {
            Self::Move(handles) => handles.feature,
            Self::Reach(handles) => handles.feature,
            Self::Hole(handles) => handles.feature,
        }
    }

    #[cfg(test)]
    pub fn step(&self) -> f64 {
        match self {
            Self::Move(handles) => handles.step(),
            Self::Reach(handles) => handles.step(),
            Self::Hole(handles) => handles.step(),
        }
    }

    pub fn hit(&self, view: &View, cursor: Vector2, pixels_per_point: f64) -> Option<Handle> {
        match self {
            Self::Move(handles) => handles.hit(view, cursor, pixels_per_point),
            Self::Reach(handles) => handles.hit(view, cursor, pixels_per_point),
            Self::Hole(handles) => handles.hit(view, cursor, pixels_per_point),
        }
    }

    #[cfg(test)]
    pub fn grip(&self, handle: Handle, along: f64) -> Option<caditor_geometry::Point3> {
        match self {
            Self::Move(handles) => handles.grip(handle, along),
            Self::Reach(handles) => handles.grip(handle, along),
            Self::Hole(handles) => handles.grip(handle, along),
        }
    }

    pub fn driven(&self, model: &Model, handle: Handle) -> Option<String> {
        match self {
            Self::Move(handles) => handles.driven(model, handle),
            Self::Reach(handles) => handles.driven(model, handle),
            Self::Hole(_) => None,
        }
    }

    pub fn words(&self, model: &Model, handle: Handle) -> String {
        self.driven(model, handle).unwrap_or_else(|| handle.words())
    }

    pub fn drawn(self, highlighted: Option<Handle>) -> Drawn {
        Drawn {
            manipulator: self,
            highlighted,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Drawn {
    manipulator: Manipulator,
    highlighted: Option<Handle>,
}

impl Drawn {
    pub fn add_to(&self, batch: &mut Batch, palette: &ScenePalette) {
        match &self.manipulator {
            Manipulator::Move(handles) => handles.add_to(batch, palette, self.highlighted),
            Manipulator::Reach(handles) => handles.add_to(batch, self.highlighted),
            Manipulator::Hole(handles) => handles.add_to(batch, self.highlighted),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Drag {
    Move(MoveDrag),
    Reach(ReachDrag),
    Hole(HoleDrag),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Manipulating {
    pub feature: FeatureId,
    pub handle: Handle,
    drag: Drag,
}

impl Manipulating {
    pub fn begin(
        model: &Model,
        manipulator: &Manipulator,
        handle: Handle,
        ray: Ray,
    ) -> Option<Self> {
        if manipulator.driven(model, handle).is_some() {
            return None;
        }
        let drag = match (manipulator, handle) {
            (Manipulator::Reach(handles), Handle::Reach(reach)) => {
                Drag::Reach(ReachDrag::begin(model, handles, reach, ray)?)
            }
            (Manipulator::Move(handles), _) => {
                Drag::Move(MoveDrag::begin(model, handles, handle, ray)?)
            }
            (Manipulator::Hole(handles), Handle::Hole(grip)) => {
                Drag::Hole(HoleDrag::begin(model, handles, grip, ray)?)
            }
            (Manipulator::Reach(_) | Manipulator::Hole(_), _) => return None,
        };
        Some(Self {
            feature: manipulator.feature(),
            handle,
            drag,
        })
    }

    pub fn follow(&mut self, ray: Ray, free: bool) -> bool {
        match &mut self.drag {
            Drag::Move(drag) => drag.follow(ray, free),
            Drag::Reach(drag) => drag.follow(ray, free),
            Drag::Hole(drag) => drag.follow(ray, free),
        }
    }

    pub fn has_moved(&self) -> bool {
        match &self.drag {
            Drag::Move(drag) => drag.has_moved(),
            Drag::Reach(drag) => drag.has_moved(),
            Drag::Hole(drag) => drag.has_moved(),
        }
    }

    pub fn transaction(&self, model: &Model) -> Option<Transaction> {
        match &self.drag {
            Drag::Move(drag) => drag.transaction(model),
            Drag::Reach(drag) => drag.transaction(model, self.feature),
            Drag::Hole(drag) => drag.transaction(model, self.feature),
        }
    }

    pub fn readout(&self, units: Units) -> String {
        match &self.drag {
            Drag::Move(drag) => drag.readout(units),
            Drag::Reach(drag) => drag.readout(units),
            Drag::Hole(drag) => drag.readout(units),
        }
    }
}
