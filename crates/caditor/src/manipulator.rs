use caditor_document::{FeatureId, Transaction};
use caditor_geometry::{Ray, Vector2};
use caditor_render::{Batch, View};

use crate::{
    model::Model,
    move_manipulator::{Handle, MoveDrag, MoveHandles},
    reach_handles::{ReachDrag, ReachHandles},
    scene_palette::ScenePalette,
    units::Units,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Manipulator {
    Move(MoveHandles),
    Reach(ReachHandles),
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
    }

    pub fn feature(&self) -> FeatureId {
        match self {
            Self::Move(handles) => handles.feature,
            Self::Reach(handles) => handles.feature,
        }
    }

    #[cfg(test)]
    pub fn step(&self) -> f64 {
        match self {
            Self::Move(handles) => handles.step(),
            Self::Reach(handles) => handles.step(),
        }
    }

    pub fn hit(&self, view: &View, cursor: Vector2, pixels_per_point: f64) -> Option<Handle> {
        match self {
            Self::Move(handles) => handles.hit(view, cursor, pixels_per_point),
            Self::Reach(handles) => handles.hit(view, cursor, pixels_per_point),
        }
    }

    #[cfg(test)]
    pub fn grip(&self, handle: Handle, along: f64) -> Option<caditor_geometry::Point3> {
        match self {
            Self::Move(handles) => handles.grip(handle, along),
            Self::Reach(handles) => handles.grip(handle, along),
        }
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
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Drag {
    Move(MoveDrag),
    Reach(ReachDrag),
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
        let drag = match (manipulator, handle) {
            (Manipulator::Reach(handles), Handle::Reach(reach)) => {
                Drag::Reach(ReachDrag::begin(model, handles, reach, ray)?)
            }
            (Manipulator::Move(handles), _) => {
                Drag::Move(MoveDrag::begin(model, handles, handle, ray)?)
            }
            (Manipulator::Reach(_), _) => return None,
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
        }
    }

    pub fn has_moved(&self) -> bool {
        match &self.drag {
            Drag::Move(drag) => drag.has_moved(),
            Drag::Reach(drag) => drag.has_moved(),
        }
    }

    pub fn transaction(&self, model: &Model) -> Option<Transaction> {
        match &self.drag {
            Drag::Move(drag) => drag.transaction(model),
            Drag::Reach(drag) => drag.transaction(model, self.feature),
        }
    }

    pub fn readout(&self, units: Units) -> String {
        match &self.drag {
            Drag::Move(drag) => drag.readout(units),
            Drag::Reach(drag) => drag.readout(units),
        }
    }
}
