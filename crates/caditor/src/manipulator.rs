use caditor_document::{Document, Edit, FeatureId, FeatureKind, ParameterOwner, Transaction};
use caditor_expression::{Dimension, EvalError, Expression, ParameterId};
use caditor_geometry::{Ray, Vector2};
use caditor_render::{Batch, View};
use thiserror::Error;

use crate::{
    feature_fields::{Rule, RuleError},
    feature_tree,
    field::{self, CheckError, Expected, ExpressionError},
    handle_snap::{Guide, Snap},
    hole_handles::{self, HoleDrag, HoleHandles},
    image_handles::{self, ImageDrag, ImageHandles},
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

#[derive(Debug, Clone, PartialEq, Error)]
pub enum TypingError {
    #[error("Type two values split by a comma, such as 10, 20")]
    TwoValues,
    #[error(transparent)]
    Expression(#[from] ExpressionError),
    #[error("{}", field::sentence(&.0.to_string()))]
    Unevaluable(EvalError),
    #[error(transparent)]
    Rule(#[from] RuleError),
    #[error("{label} cannot be set here")]
    CannotSetHere { label: String },
    #[error(transparent)]
    Rejected(#[from] CheckError),
}

const DRAGGED: &str = "handle-dragged-feature";

pub fn publish_dragged(ctx: &egui::Context, feature: Option<FeatureId>) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::unique(DRAGGED), feature));
}

pub fn dragged(ctx: &egui::Context) -> Option<FeatureId> {
    ctx.data(|data| data.get_temp::<Option<FeatureId>>(egui::Id::unique(DRAGGED)))
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

#[derive(Debug, Clone, PartialEq)]
pub enum Manipulator {
    Move(MoveHandles),
    Reach(ReachHandles, Option<ValueHandles>),
    Place(PlaceHandles, Option<ValueHandles>),
    Holes(HoleHandles, Option<ValueHandles>),
    Value(ValueHandles),
    Revolve(TurnHandles, Option<ValueHandles>),
    Image(ImageHandles),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Primary<'a> {
    Move(&'a MoveHandles),
    Reach(&'a ReachHandles),
    Place(&'a PlaceHandles),
    Holes(&'a HoleHandles),
    Revolve(&'a TurnHandles),
    Image(&'a ImageHandles),
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
        if let Some(handles) = ImageHandles::of(model, feature, view, pixels_per_point) {
            return Some(Self::Image(handles));
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
        if let Some(handles) = HoleHandles::of(model, feature, view, pixels_per_point) {
            let values = clear_of(&handles.segments());
            return Some(Self::Holes(handles, values));
        }
        clear_of(&[]).map(Self::Value)
    }

    fn parts(&self) -> (Option<Primary<'_>>, Option<&ValueHandles>) {
        match self {
            Self::Move(handles) => (Some(Primary::Move(handles)), None),
            Self::Reach(handles, values) => (Some(Primary::Reach(handles)), values.as_ref()),
            Self::Place(handles, values) => (Some(Primary::Place(handles)), values.as_ref()),
            Self::Holes(handles, values) => (Some(Primary::Holes(handles)), values.as_ref()),
            Self::Value(values) => (None, Some(values)),
            Self::Revolve(handles, values) => (Some(Primary::Revolve(handles)), values.as_ref()),
            Self::Image(handles) => (Some(Primary::Image(handles)), None),
        }
    }

    pub fn feature(&self) -> FeatureId {
        match self {
            Self::Move(handles) => handles.feature,
            Self::Reach(handles, _) => handles.feature,
            Self::Place(handles, _) => handles.feature,
            Self::Holes(handles, _) => handles.feature,
            Self::Value(handles) => handles.feature,
            Self::Revolve(handles, _) => handles.feature,
            Self::Image(handles) => handles.feature,
        }
    }

    #[cfg(test)]
    pub fn step(&self) -> f64 {
        match self.parts() {
            (Some(Primary::Move(handles)), _) => handles.step(),
            (Some(Primary::Reach(handles)), _) => handles.step(),
            (Some(Primary::Place(handles)), _) => handles.step(),
            (Some(Primary::Holes(handles)), _) => handles.step(),
            (Some(Primary::Revolve(handles)), _) => handles.step(),
            (Some(Primary::Image(handles)), _) => handles.step(),
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
                Primary::Holes(handles) => handles.hit(view, cursor, pixels_per_point),
                Primary::Revolve(handles) => handles.hit(view, cursor, pixels_per_point),
                Primary::Image(handles) => handles.hit(view, cursor, pixels_per_point),
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
            Primary::Holes(handles) => handles.grip(handle, along),
            Primary::Revolve(handles) => handles.grip_at(handle, along),
            Primary::Image(handles) => handles.grip(handle, along),
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
            Primary::Move(_) | Primary::Place(_) | Primary::Holes(_) | Primary::Image(_) => None,
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
            Primary::Holes(handles) => handles.driven(model, handle),
            Primary::Revolve(handles) => handles.driven(model, handle),
            Primary::Image(handles) => handles.driven(model, handle),
        }
    }

    pub fn words(&self, model: &Model, handle: Handle) -> String {
        self.driven(model, handle).unwrap_or_else(|| {
            let (primary, values) = self.parts();
            match (handle, primary, values) {
                (Handle::Value(measured), _, Some(values)) => values.words(model, measured),
                (_, Some(Primary::Place(handles)), _) => handles.words(handle),
                (_, Some(Primary::Holes(handles)), _) => handles.words(handle),
                _ => handle.words(),
            }
        })
    }

    pub fn typing(&self, model: &Model, handle: Handle) -> Option<Typing> {
        if self.driven(model, handle).is_some() {
            return None;
        }
        let (primary, values) = self.parts();
        let feature = self.feature();
        let (label, dimension, rules) = match (handle, primary) {
            (Handle::Value(measured), _) => {
                let typing = values?.typing(model, measured)?;
                let dimension = measured.amount().dimension();
                (typing.caption, dimension, vec![typing.rule])
            }
            (Handle::Reach(reach), Some(Primary::Reach(_))) => {
                let caption = reach.field_caption().to_owned();
                (caption, Dimension::LENGTH, vec![reach.field_rule()])
            }
            (Handle::Revolve(end), Some(Primary::Revolve(_))) => {
                let rule = turn_handles::field_rule(model, feature, end);
                (end.field_caption().to_owned(), Dimension::ANGLE, vec![rule])
            }
            (Handle::Place(grip), Some(Primary::Place(handles))) => {
                let (label, count) = handles.typing(grip)?;
                (label, Dimension::LENGTH, vec![Rule::Any; count])
            }
            (Handle::Hole(grip), Some(Primary::Holes(handles))) => {
                let (label, rules) = handles.typing(grip)?;
                (label, Dimension::LENGTH, rules)
            }
            (Handle::Image(grip), Some(Primary::Image(handles))) => handles.typing(grip),
            (_, Some(Primary::Move(_))) => {
                let (label, dimension, count) = move_manipulator::typing(handle)?;
                (label, dimension, vec![Rule::Any; count])
            }
            _ => return None,
        };
        Some(Typing {
            feature,
            handle,
            label,
            dimension,
            rules,
        })
    }

    pub fn drawn(&self, highlighted: Option<Handle>) -> Drawn {
        Drawn {
            manipulator: self.clone(),
            highlighted,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Typing {
    pub feature: FeatureId,
    pub handle: Handle,
    pub label: String,
    dimension: Dimension,
    rules: Vec<Rule>,
}

impl Typing {
    pub fn placeholder(&self) -> &'static str {
        match (self.rules.len(), self.dimension) {
            (2, _) => "x, y",
            (_, Dimension::ANGLE) => "an angle",
            (_, Dimension::NONE) => "a number",
            _ => "a length",
        }
    }

    pub fn parse(&self, model: &Model, text: &str) -> Result<Vec<Expression>, TypingError> {
        let parts = if self.rules.len() == 1 {
            vec![text]
        } else {
            typed_point::coordinates(text)
        };
        if parts.len() != self.rules.len() {
            return Err(TypingError::TwoValues);
        }
        let parameters = model.parameters();
        parts
            .into_iter()
            .zip(&self.rules)
            .map(|(part, rule)| {
                let parsed = field::parse_expression(
                    model.document(),
                    parameters,
                    part.trim(),
                    Expected {
                        dimension: Some(self.dimension),
                        non_negative: false,
                    },
                    model.units(),
                )?;
                let value = parameters
                    .evaluate_expression(&parsed)
                    .map_err(TypingError::Unevaluable)?;
                rule.check(value.value)?;
                Ok(parsed)
            })
            .collect()
    }

    pub fn transaction(
        &self,
        model: &Model,
        values: Vec<Expression>,
    ) -> Result<Transaction, TypingError> {
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
            Handle::Hole(grip) => hole_handles::typed(model, feature, grip, &values),
            Handle::Image(grip) => image_handles::typed(model, feature, grip, &values),
            Handle::Along(_) | Handle::Across(_) | Handle::Turn(_) | Handle::TurnAbout => {
                move_manipulator::typed(model, feature, self.handle, &values)
            }
        };
        let transaction = built.ok_or_else(|| TypingError::CannotSetHere {
            label: self.label.clone(),
        })?;
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
            Some(Primary::Holes(handles)) => handles.add_to(batch, palette, highlighted),
            Some(Primary::Revolve(handles)) => handles.add_to(batch, palette, highlighted),
            Some(Primary::Image(handles)) => handles.add_to(batch, palette, highlighted),
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
    Hole(HoleDrag),
    Value(Box<ValueDrag>),
    Revolve(TurnDrag),
    Image(ImageDrag),
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
            (Some(Primary::Holes(handles)), Handle::Hole(grip)) => {
                Drag::Hole(HoleDrag::begin(handles, grip, ray)?)
            }
            (Some(Primary::Revolve(handles)), Handle::Revolve(end)) => {
                Drag::Revolve(TurnDrag::begin(model, handles, end, ray)?)
            }
            (Some(Primary::Image(handles)), Handle::Image(grip)) => {
                Drag::Image(ImageDrag::begin(model, handles, grip, ray)?)
            }
            _ => return None,
        };
        Some(Self {
            feature: manipulator.feature(),
            handle,
            drag,
        })
    }

    pub fn guide(&self) -> Guide {
        match &self.drag {
            Drag::Move(drag) => drag.guide(),
            Drag::Reach(drag) => drag.guide(),
            Drag::Value(drag) => drag.guide(),
            Drag::Place(_) | Drag::Hole(_) | Drag::Revolve(_) | Drag::Image(_) => Guide::Points,
        }
    }

    pub fn follow(&mut self, ray: Ray, free: bool, snap: Option<Snap>) -> bool {
        match &mut self.drag {
            Drag::Move(drag) => drag.follow(ray, free, snap),
            Drag::Reach(drag) => drag.follow(ray, free, snap),
            Drag::Place(drag) => drag.follow(ray, free, snap),
            Drag::Hole(drag) => drag.follow(ray, free, snap),
            Drag::Value(drag) => drag.follow(ray, free, snap),
            Drag::Revolve(drag) => drag.follow(ray, free, snap),
            Drag::Image(drag) => drag.follow(ray, free, snap),
        }
    }

    pub fn has_moved(&self) -> bool {
        match &self.drag {
            Drag::Move(drag) => drag.has_moved(),
            Drag::Reach(drag) => drag.has_moved(),
            Drag::Place(drag) => drag.has_moved(),
            Drag::Hole(drag) => drag.has_moved(),
            Drag::Value(drag) => drag.has_moved(),
            Drag::Revolve(drag) => drag.has_moved(),
            Drag::Image(drag) => drag.has_moved(),
        }
    }

    pub fn transaction(&self, model: &Model) -> Option<Transaction> {
        match &self.drag {
            Drag::Move(drag) => drag.transaction(model),
            Drag::Reach(drag) => drag.transaction(model, self.feature),
            Drag::Place(drag) => drag.transaction(model, self.feature),
            Drag::Hole(drag) => drag.transaction(model, self.feature),
            Drag::Value(drag) => drag.transaction(model, self.feature),
            Drag::Revolve(drag) => drag.transaction(model, self.feature),
            Drag::Image(drag) => drag.transaction(model, self.feature),
        }
    }

    pub fn readout(&self, units: Units) -> String {
        match &self.drag {
            Drag::Move(drag) => drag.readout(units),
            Drag::Reach(drag) => drag.readout(units),
            Drag::Place(drag) => drag.readout(units),
            Drag::Hole(drag) => drag.readout(units),
            Drag::Value(drag) => drag.readout(units),
            Drag::Revolve(drag) => drag.readout(units),
            Drag::Image(drag) => drag.readout(units),
        }
    }
}
