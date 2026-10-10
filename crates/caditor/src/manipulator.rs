use caditor_document::{Document, Edit, FeatureId, FeatureKind, ParameterOwner, Transaction};
use caditor_expression::{Dimension, Expression, ParameterId};
use caditor_geometry::{Ray, Vector2};
use caditor_render::{Batch, View};

use crate::{
    feature_tree,
    field::{self, Expected},
    handle_snap::Snap,
    model::Model,
    move_manipulator::{self, Handle, HandleSegment, MoveDrag, MoveHandles},
    place_handles::{self, PlaceDrag, PlaceHandles},
    reach_handles::{self, ReachDrag, ReachHandles},
    scene_palette::ScenePalette,
    turn_handles::{self, TurnDrag, TurnHandles},
    typed_point,
    units::Units,
    value_handles::{self, ValueDrag, ValueHandles},
};

pub const TWO_VALUES: &str = "Type two values split by a comma, such as 10, 20";
const DRAGGED: &str = "handle-dragged-feature";

pub fn publish_dragged(ctx: &egui::Context, feature: Option<FeatureId>) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(DRAGGED), feature));
}

pub fn dragged(ctx: &egui::Context) -> Option<FeatureId> {
    ctx.data(|data| data.get_temp::<Option<FeatureId>>(egui::Id::new(DRAGGED)))
        .flatten()
}

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
    Reach(ReachHandles, Option<ValueHandles>),
    Place(PlaceHandles, Option<ValueHandles>),
    Value(ValueHandles),
    Revolve(TurnHandles, Option<ValueHandles>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Primary<'a> {
    Move(&'a MoveHandles),
    Reach(&'a ReachHandles),
    Place(&'a PlaceHandles),
    Revolve(&'a TurnHandles),
}

impl Manipulator {
    pub fn of(
        model: &Model,
        open: Option<FeatureId>,
        view: &View,
        pixels_per_point: f64,
    ) -> Option<Self> {
        let feature = open?;
        if let Some(handles) = MoveHandles::of(model, open, view, pixels_per_point) {
            return Some(Self::Move(handles));
        }
        let values = ValueHandles::of(model, feature, view, pixels_per_point);
        let clear_of = |taken: &[HandleSegment]| {
            values.and_then(|values| values.clear_of(view, pixels_per_point, taken))
        };
        if let Some(handles) = ReachHandles::of(model, feature, view, pixels_per_point) {
            return Some(Self::Reach(handles, clear_of(&handles.segments())));
        }
        if let Some(handles) = TurnHandles::of(model, feature, view, pixels_per_point) {
            return Some(Self::Revolve(handles, clear_of(&handles.segments())));
        }
        if let Some(handles) = PlaceHandles::of(model, feature, view, pixels_per_point) {
            return Some(Self::Place(handles, clear_of(&handles.segments())));
        }
        clear_of(&[]).map(Self::Value)
    }

    fn parts(&self) -> (Option<Primary<'_>>, Option<&ValueHandles>) {
        match self {
            Self::Move(handles) => (Some(Primary::Move(handles)), None),
            Self::Reach(handles, values) => (Some(Primary::Reach(handles)), values.as_ref()),
            Self::Place(handles, values) => (Some(Primary::Place(handles)), values.as_ref()),
            Self::Value(values) => (None, Some(values)),
            Self::Revolve(handles, values) => (Some(Primary::Revolve(handles)), values.as_ref()),
        }
    }

    pub fn feature(&self) -> FeatureId {
        match self {
            Self::Move(handles) => handles.feature,
            Self::Reach(handles, _) => handles.feature,
            Self::Place(handles, _) => handles.feature,
            Self::Value(handles) => handles.feature,
            Self::Revolve(handles, _) => handles.feature,
        }
    }

    #[cfg(test)]
    pub fn step(&self) -> f64 {
        match self.parts() {
            (Some(Primary::Move(handles)), _) => handles.step(),
            (Some(Primary::Reach(handles)), _) => handles.step(),
            (Some(Primary::Place(handles)), _) => handles.step(),
            (Some(Primary::Revolve(handles)), _) => handles.step(),
            (None, Some(values)) => values.step(),
            (None, None) => 1.0,
        }
    }

    pub fn hit(&self, view: &View, cursor: Vector2, pixels_per_point: f64) -> Option<Handle> {
        let (primary, values) = self.parts();
        values
            .and_then(|values| values.hit(view, cursor, pixels_per_point))
            .or_else(|| match primary? {
                Primary::Move(handles) => handles.hit(view, cursor, pixels_per_point),
                Primary::Reach(handles) => handles.hit(view, cursor, pixels_per_point),
                Primary::Place(handles) => handles.hit(view, cursor, pixels_per_point),
                Primary::Revolve(handles) => handles.hit(view, cursor, pixels_per_point),
            })
    }

    #[cfg(test)]
    pub fn grip(&self, handle: Handle, along: f64) -> Option<caditor_geometry::Point3> {
        let (primary, values) = self.parts();
        if let Handle::Value(_) = handle {
            return values?.grip(handle, along);
        }
        match primary? {
            Primary::Move(handles) => handles.grip(handle, along),
            Primary::Reach(handles) => handles.grip(handle, along),
            Primary::Place(handles) => handles.grip(handle, along),
            Primary::Revolve(handles) => handles.grip_at(handle, along),
        }
    }

    #[cfg(test)]
    pub fn foot(&self, handle: Handle) -> Option<caditor_geometry::Point3> {
        let (primary, values) = self.parts();
        if let Handle::Value(_) = handle {
            return values?.foot(handle);
        }
        match primary? {
            Primary::Reach(handles) => handles.foot(handle),
            Primary::Revolve(handles) => handles.foot(handle),
            Primary::Move(_) | Primary::Place(_) => None,
        }
    }

    pub fn driven(&self, model: &Model, handle: Handle) -> Option<String> {
        let (primary, values) = self.parts();
        if let Handle::Value(_) = handle {
            return values?.driven(model, handle);
        }
        match primary? {
            Primary::Move(handles) => handles.driven(model, handle),
            Primary::Reach(handles) => handles.driven(model, handle),
            Primary::Place(handles) => handles.driven(model, handle),
            Primary::Revolve(handles) => handles.driven(model, handle),
        }
    }

    pub fn words(&self, model: &Model, handle: Handle) -> String {
        self.driven(model, handle).unwrap_or_else(|| {
            let (primary, values) = self.parts();
            match (handle, primary, values) {
                (Handle::Value(measured), _, Some(values)) => values.words(model, measured),
                (_, Some(Primary::Place(handles)), _) => handles.words(handle),
                _ => handle.words(),
            }
        })
    }

    pub fn typing(&self, model: &Model, handle: Handle) -> Option<Typing> {
        if self.driven(model, handle).is_some() {
            return None;
        }
        let (primary, values) = self.parts();
        let (label, dimension, count) = match (handle, primary) {
            (Handle::Value(measured), _) => {
                let typing = values?.typing(model, measured)?;
                (typing.caption, measured.amount().dimension(), 1)
            }
            (Handle::Reach(reach), Some(Primary::Reach(_))) => {
                (reach.field_caption().to_owned(), Dimension::LENGTH, 1)
            }
            (Handle::Revolve(end), Some(Primary::Revolve(_))) => {
                (end.field_caption().to_owned(), Dimension::ANGLE, 1)
            }
            (Handle::Place(grip), Some(Primary::Place(handles))) => {
                let (label, count) = handles.typing(grip)?;
                (label, Dimension::LENGTH, count)
            }
            (_, Some(Primary::Move(_))) => move_manipulator::typing(handle)?,
            _ => return None,
        };
        Some(Typing {
            feature: self.feature(),
            handle,
            label,
            dimension,
            count,
        })
    }

    pub fn drawn(self, highlighted: Option<Handle>) -> Drawn {
        Drawn {
            manipulator: self,
            highlighted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Typing {
    pub feature: FeatureId,
    pub handle: Handle,
    pub label: String,
    dimension: Dimension,
    count: usize,
}

impl Typing {
    pub fn placeholder(&self) -> &'static str {
        match (self.count, self.dimension) {
            (2, _) => "x, y",
            (_, Dimension::ANGLE) => "an angle",
            (_, Dimension::NONE) => "a number",
            _ => "a length",
        }
    }

    pub fn parse(&self, model: &Model, text: &str) -> Result<Vec<Expression>, String> {
        let parts = if self.count == 1 {
            vec![text]
        } else {
            typed_point::coordinates(text)
        };
        if parts.len() != self.count {
            return Err(TWO_VALUES.to_owned());
        }
        parts
            .into_iter()
            .map(|part| {
                field::parse_expression(
                    model.document(),
                    model.parameters(),
                    part.trim(),
                    Expected {
                        dimension: Some(self.dimension),
                        non_negative: false,
                    },
                    model.units(),
                )
            })
            .collect()
    }

    pub fn transaction(
        &self,
        model: &Model,
        values: Vec<Expression>,
    ) -> Result<Transaction, String> {
        let feature = self.feature;
        let first = values.first().cloned();
        let built = match self.handle {
            Handle::Value(measured) => {
                first.and_then(|value| value_handles::written(model, feature, measured, value))
            }
            Handle::Reach(reach) => {
                first.and_then(|value| reach_handles::typed(model, feature, reach, value))
            }
            Handle::Revolve(end) => {
                first.and_then(|value| turn_handles::typed(model, feature, end, value))
            }
            Handle::Place(grip) => place_handles::typed(model, feature, grip, &values),
            Handle::Along(_) | Handle::Across(_) | Handle::Turn(_) | Handle::TurnAbout => {
                move_manipulator::typed(model, feature, self.handle, &values)
            }
        };
        let transaction = built.ok_or_else(|| format!("{} cannot be set here", self.label))?;
        field::checked(model.document(), transaction)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Drawn {
    manipulator: Manipulator,
    highlighted: Option<Handle>,
}

impl Drawn {
    pub fn add_to(&self, batch: &mut Batch, palette: &ScenePalette) {
        let highlighted = self.highlighted;
        let (primary, values) = self.manipulator.parts();
        match primary {
            Some(Primary::Move(handles)) => handles.add_to(batch, palette, highlighted),
            Some(Primary::Reach(handles)) => handles.add_to(batch, palette, highlighted),
            Some(Primary::Place(handles)) => handles.add_to(batch, palette, highlighted),
            Some(Primary::Revolve(handles)) => handles.add_to(batch, palette, highlighted),
            None => {}
        }
        if let Some(values) = values {
            values.add_to(batch, palette, highlighted);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Drag {
    Move(MoveDrag),
    Reach(ReachDrag),
    Place(PlaceDrag),
    Value(Box<ValueDrag>),
    Revolve(TurnDrag),
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
        let (primary, values) = manipulator.parts();
        let drag = match (primary, handle) {
            (_, Handle::Value(measured)) => {
                Drag::Value(Box::new(ValueDrag::begin(model, values?, measured, ray)?))
            }
            (Some(Primary::Reach(handles)), Handle::Reach(reach)) => {
                Drag::Reach(ReachDrag::begin(model, handles, reach, ray)?)
            }
            (Some(Primary::Move(handles)), _) => {
                Drag::Move(MoveDrag::begin(model, handles, handle, ray)?)
            }
            (Some(Primary::Place(handles)), Handle::Place(grip)) => {
                Drag::Place(PlaceDrag::begin(model, handles, grip, ray)?)
            }
            (Some(Primary::Revolve(handles)), Handle::Revolve(end)) => {
                Drag::Revolve(TurnDrag::begin(model, handles, end, ray)?)
            }
            _ => return None,
        };
        Some(Self {
            feature: manipulator.feature(),
            handle,
            drag,
        })
    }

    pub fn snaps(&self) -> bool {
        match &self.drag {
            Drag::Move(drag) => drag.snaps(),
            Drag::Reach(_) | Drag::Place(_) | Drag::Value(_) => true,
            Drag::Revolve(_) => false,
        }
    }

    pub fn follow(&mut self, ray: Ray, free: bool, snap: Option<Snap>) -> bool {
        match &mut self.drag {
            Drag::Move(drag) => drag.follow(ray, free, snap),
            Drag::Reach(drag) => drag.follow(ray, free, snap),
            Drag::Place(drag) => drag.follow(ray, free, snap),
            Drag::Value(drag) => drag.follow(ray, free, snap),
            Drag::Revolve(drag) => drag.follow(ray, free),
        }
    }

    pub fn has_moved(&self) -> bool {
        match &self.drag {
            Drag::Move(drag) => drag.has_moved(),
            Drag::Reach(drag) => drag.has_moved(),
            Drag::Place(drag) => drag.has_moved(),
            Drag::Value(drag) => drag.has_moved(),
            Drag::Revolve(drag) => drag.has_moved(),
        }
    }

    pub fn transaction(&self, model: &Model) -> Option<Transaction> {
        match &self.drag {
            Drag::Move(drag) => drag.transaction(model),
            Drag::Reach(drag) => drag.transaction(model, self.feature),
            Drag::Place(drag) => drag.transaction(model, self.feature),
            Drag::Value(drag) => drag.transaction(model, self.feature),
            Drag::Revolve(drag) => drag.transaction(model, self.feature),
        }
    }

    pub fn readout(&self, units: Units) -> String {
        match &self.drag {
            Drag::Move(drag) => drag.readout(units),
            Drag::Reach(drag) => drag.readout(units),
            Drag::Place(drag) => drag.readout(units),
            Drag::Value(drag) => drag.readout(units),
            Drag::Revolve(drag) => drag.readout(units),
        }
    }
}
