use caditor_document::{FeatureId, Transaction};
use caditor_expression::{Dimension, Expression, Quantity};
use caditor_sketch::{EntityId, Faceting, Sketch};

use crate::{
    blend_curving::BlendCurving,
    drawing::Preview,
    editing::{ActiveSketch, Tool},
    field::{self, Expected},
    filleting::{CornerCut, Filleting},
    gearing::{GearSettings, Gearing},
    mirroring::Mirroring,
    model::Model,
    offsetting::{self, Offsetting},
    patterning::{PatternKind, Patterning},
    shape_modes::{ShapeMode, ShapeModes},
    sketch_tools,
    snap::{Pointer, Screen},
    tangent_circling::TangentCircling,
    units::LengthUnit,
};

const LENGTH: Expected = Expected {
    dimension: Some(Dimension::LENGTH),
    non_negative: false,
};

#[derive(Debug)]
pub enum Outcome {
    Nothing,
    Apply(Transaction),
    Select(Vec<EntityId>),
    Refused(String),
}

impl From<Result<Transaction, String>> for Outcome {
    fn from(result: Result<Transaction, String>) -> Self {
        match result {
            Ok(transaction) => Self::Apply(transaction),
            Err(reason) => Self::Refused(reason),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hint {
    Targets,
    Keys(&'static str),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub text: &'static str,
    pub hint: Hint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueField {
    pub label: &'static str,
    pub placeholder: &'static str,
    pub action: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Value {
    pub expression: Expression,
    pub millimetres: f64,
}

impl Value {
    pub fn typed(model: &Model, text: &str) -> Result<Self, String> {
        let expression = field::parse_expression(
            model.document(),
            model.parameters(),
            text,
            LENGTH,
            model.length_unit(),
        )?;
        Self::of(model, expression)
    }

    pub fn pointed(model: &Model, millimetres: f64) -> Option<Self> {
        let value = Self::of(model, model.length_unit().measured(millimetres)).ok()?;
        (value.millimetres > 0.0).then_some(value)
    }

    fn of(model: &Model, expression: Expression) -> Result<Self, String> {
        let millimetres = expression
            .evaluate_as(Dimension::LENGTH, &|id| model.parameters().value(id))
            .map_err(|error| field::sentence(&error.to_string()))?;
        Ok(Self {
            expression,
            millimetres,
        })
    }

    pub fn negated(self) -> Self {
        let expression = match self.expression {
            Expression::Negate(inner) => *inner,
            other => Expression::Negate(Box::new(other)),
        };
        Self {
            expression,
            millimetres: -self.millimetres,
        }
    }
}

pub fn length_text(unit: LengthUnit, millimetres: f64) -> String {
    unit.show(Quantity::length(sketch_tools::rounded_for_display(
        millimetres,
    )))
}

#[derive(Debug, Clone, Default)]
enum State {
    #[default]
    Idle,
    Offset(Offsetting),
    Mirror(Mirroring),
    Pattern(Box<Patterning>),
    Fillet(Box<Filleting>),
    Tangent(Box<TangentCircling>),
    Blend(Box<BlendCurving>),
    Gear(Box<Gearing>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LastSizes {
    round: Option<String>,
    chamfer: Option<String>,
}

impl LastSizes {
    fn of(&self, cut: CornerCut) -> Option<String> {
        match cut {
            CornerCut::Round => self.round.clone(),
            CornerCut::Chamfer => self.chamfer.clone(),
        }
    }

    fn keep(&mut self, filleting: &Filleting) {
        let last = filleting.last().map(str::to_owned);
        match filleting.cut() {
            CornerCut::Round => self.round = last,
            CornerCut::Chamfer => self.chamfer = last,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Modifying {
    context: Option<(FeatureId, Tool)>,
    state: State,
    last: LastSizes,
    gear: GearSettings,
}

impl Modifying {
    pub fn gear_settings(&mut self) -> &mut GearSettings {
        &mut self.gear
    }

    pub fn is_active(&self) -> bool {
        self.context.is_some()
    }

    pub fn gathers(&self) -> bool {
        match &self.state {
            State::Mirror(mirroring) => mirroring.gathers(),
            State::Pattern(patterning) => patterning.gathers(),
            State::Offset(_)
            | State::Fillet(_)
            | State::Tangent(_)
            | State::Blend(_)
            | State::Gear(_)
            | State::Idle => false,
        }
    }

    pub fn lets_go_of_selection(&self) -> bool {
        match &self.state {
            State::Mirror(mirroring) => mirroring.gathers() && mirroring.gathered(),
            State::Pattern(patterning) => patterning.gathers() && patterning.gathered(),
            State::Offset(_)
            | State::Fillet(_)
            | State::Tangent(_)
            | State::Blend(_)
            | State::Gear(_)
            | State::Idle => false,
        }
    }

    pub fn last_value(&self) -> Option<&str> {
        match &self.state {
            State::Fillet(filleting) => filleting.last(),
            State::Offset(_)
            | State::Mirror(_)
            | State::Pattern(_)
            | State::Tangent(_)
            | State::Blend(_)
            | State::Gear(_)
            | State::Idle => None,
        }
    }

    pub fn sync(
        &mut self,
        active: Option<ActiveSketch>,
        modes: ShapeModes,
        sketch: Option<&Sketch>,
        selected: &[EntityId],
    ) {
        let context = active
            .filter(|active| active.tool.reshapes())
            .map(|active| (active.feature, active.tool));
        if context != self.context {
            if let State::Fillet(filleting) = &self.state {
                self.last.keep(filleting);
            }
            self.context = context;
            let round = self.last.of(CornerCut::Round);
            let chamfer = self.last.of(CornerCut::Chamfer);
            self.state = match (context, sketch) {
                (Some((_, Tool::Offset)), _) => State::Offset(Offsetting::default()),
                (Some((_, Tool::Mirror)), _) => State::Mirror(Mirroring::starting(selected)),
                (Some((_, Tool::RectangularPattern)), _) => State::Pattern(Box::new(
                    Patterning::starting(PatternKind::Rectangular, selected),
                )),
                (Some((_, Tool::CircularPattern)), _) => State::Pattern(Box::new(
                    Patterning::starting(PatternKind::Circular, selected),
                )),
                (Some((_, Tool::TangentCircle)), Some(sketch)) => {
                    State::Tangent(Box::new(TangentCircling::starting(sketch, selected)))
                }
                (Some((_, Tool::TangentCircle)), None) => State::Tangent(Box::default()),
                (Some((_, Tool::BlendCurve)), Some(sketch)) => State::Blend(Box::new(
                    BlendCurving::starting(sketch, selected, modes.blend()),
                )),
                (Some((_, Tool::BlendCurve)), None) => State::Blend(Box::default()),
                (Some((_, Tool::Gear)), Some(sketch)) => {
                    State::Gear(Box::new(Gearing::starting(sketch, selected)))
                }
                (Some((_, Tool::Gear)), None) => State::Gear(Box::default()),
                (Some((_, Tool::Fillet)), Some(sketch)) => State::Fillet(Box::new(
                    Filleting::starting(sketch, selected, CornerCut::Round, round),
                )),
                (Some((_, Tool::Chamfer)), Some(sketch)) => State::Fillet(Box::new(
                    Filleting::starting(sketch, selected, CornerCut::Chamfer, chamfer),
                )),
                (Some((_, Tool::Fillet)), None) => {
                    State::Fillet(Box::new(Filleting::new(CornerCut::Round, round)))
                }
                (Some((_, Tool::Chamfer)), None) => {
                    State::Fillet(Box::new(Filleting::new(CornerCut::Chamfer, chamfer)))
                }
                _ => State::Idle,
            };
        }
        let Some(sketch) = sketch else {
            return;
        };
        match &mut self.state {
            State::Offset(offsetting) => offsetting.sync(sketch, selected),
            State::Mirror(mirroring) => mirroring.sync(selected),
            State::Pattern(patterning) => patterning.sync(sketch, selected),
            State::Fillet(filleting) => filleting.sync(sketch),
            State::Tangent(tangent) => tangent.sync(sketch),
            State::Blend(blend) => blend.sync(sketch, modes.blend()),
            State::Gear(gearing) => gearing.sync(sketch),
            State::Idle => {}
        }
    }

    pub fn hover(
        &mut self,
        sketch: &Sketch,
        screen: &impl Screen,
        pointer: Option<Pointer>,
        faceting: Faceting,
    ) {
        match &mut self.state {
            State::Offset(offsetting) => offsetting.hover(sketch, screen, pointer),
            State::Mirror(mirroring) => mirroring.hover(sketch, screen, pointer, faceting),
            State::Pattern(patterning) => patterning.hover(sketch, screen, pointer, faceting),
            State::Fillet(filleting) => filleting.hover(sketch, screen, pointer),
            State::Tangent(tangent) => tangent.hover(sketch, screen, pointer),
            State::Blend(blend) => blend.hover(sketch, screen, pointer),
            State::Gear(gearing) => gearing.hover(sketch, screen, pointer, self.gear.gear()),
            State::Idle => {}
        }
    }

    pub fn leave(&mut self) {
        match &mut self.state {
            State::Offset(offsetting) => offsetting.leave(),
            State::Mirror(mirroring) => mirroring.leave(),
            State::Pattern(patterning) => patterning.leave(),
            State::Fillet(filleting) => filleting.leave(),
            State::Tangent(tangent) => tangent.leave(),
            State::Blend(blend) => blend.leave(),
            State::Gear(gearing) => gearing.leave(),
            State::Idle => {}
        }
    }

    pub fn preview(&self, faceting: Faceting) -> Preview {
        match &self.state {
            State::Offset(offsetting) => offsetting.preview(faceting),
            State::Mirror(mirroring) => mirroring.preview(),
            State::Pattern(patterning) => patterning.preview(),
            State::Fillet(filleting) => filleting.preview(faceting),
            State::Tangent(tangent) => tangent.preview(faceting),
            State::Blend(blend) => blend.preview(faceting),
            State::Gear(gearing) => gearing.preview(faceting),
            State::Idle => Preview::default(),
        }
    }

    pub fn highlighted_entities(&self) -> Vec<EntityId> {
        match &self.state {
            State::Offset(offsetting) => offsetting.highlighted_entities(),
            State::Mirror(mirroring) => mirroring.highlighted_entities(),
            State::Pattern(patterning) => patterning.highlighted_entities(),
            State::Fillet(filleting) => filleting.highlighted_entities(),
            State::Tangent(tangent) => tangent.highlighted_entities(),
            State::Blend(blend) => blend.highlighted_entities(),
            State::Gear(gearing) => gearing.highlighted_entities(),
            State::Idle => Vec::new(),
        }
    }

    pub fn label(&self, sketch: &Sketch, unit: LengthUnit) -> Option<String> {
        match &self.state {
            State::Offset(offsetting) => offsetting.label(sketch, unit),
            State::Mirror(mirroring) => mirroring.label(sketch),
            State::Pattern(patterning) => patterning.label(sketch),
            State::Fillet(filleting) => filleting.label(sketch, unit),
            State::Tangent(tangent) => tangent.label(sketch, unit),
            State::Blend(blend) => blend.label(sketch),
            State::Gear(gearing) => gearing.label(sketch, unit, self.gear.gear()),
            State::Idle => None,
        }
    }

    pub fn prompt(&self) -> Option<Prompt> {
        match &self.state {
            State::Offset(offsetting) => Some(offsetting.prompt()),
            State::Mirror(mirroring) => Some(mirroring.prompt()),
            State::Pattern(patterning) => Some(patterning.prompt()),
            State::Fillet(filleting) => Some(filleting.prompt()),
            State::Tangent(tangent) => Some(tangent.prompt()),
            State::Blend(blend) => Some(blend.prompt()),
            State::Gear(gearing) => Some(gearing.prompt()),
            State::Idle => None,
        }
    }

    pub fn mode(&self, modes: ShapeModes) -> Option<ShapeMode> {
        self.context.and_then(|(_, tool)| modes.of(tool))
    }

    pub fn value_field(&self) -> Option<ValueField> {
        match &self.state {
            State::Offset(_) => Some(offsetting::FIELD),
            State::Fillet(filleting) => Some(filleting.field()),
            State::Pattern(patterning) => Some(patterning.field()),
            State::Tangent(tangent) => Some(tangent.field()),
            State::Mirror(_) | State::Blend(_) | State::Gear(_) | State::Idle => None,
        }
    }

    pub fn show_typed(&mut self, typed: Option<f64>) {
        match &mut self.state {
            State::Offset(offsetting) => offsetting.show_typed(typed),
            State::Fillet(_)
            | State::Mirror(_)
            | State::Pattern(_)
            | State::Tangent(_)
            | State::Blend(_)
            | State::Gear(_)
            | State::Idle => {}
        }
    }

    pub fn show_text(&mut self, model: &Model, text: Option<&str>) {
        match &mut self.state {
            State::Pattern(patterning) => patterning.show_text(model, text),
            State::Fillet(filleting) => filleting.show_text(model, text),
            State::Tangent(tangent) => tangent.show_text(model, text),
            State::Offset(_)
            | State::Mirror(_)
            | State::Blend(_)
            | State::Gear(_)
            | State::Idle => {
                let shown = text
                    .and_then(|text| Value::typed(model, text).ok())
                    .map(|value| value.millimetres);
                self.show_typed(shown);
            }
        }
    }

    pub fn enter_text(&mut self, model: &Model, text: &str) -> Result<Outcome, String> {
        match &mut self.state {
            State::Pattern(patterning) => match self.context {
                Some((feature, _)) => patterning.enter_text(model, feature, text),
                None => Ok(Outcome::Nothing),
            },
            State::Fillet(filleting) => match self.context {
                Some((feature, _)) => filleting.enter_text(model, feature, text),
                None => Ok(Outcome::Nothing),
            },
            State::Tangent(tangent) => match self.context {
                Some((feature, _)) => tangent.enter_text(model, feature, text),
                None => Ok(Outcome::Nothing),
            },
            State::Offset(_)
            | State::Mirror(_)
            | State::Blend(_)
            | State::Gear(_)
            | State::Idle => {
                Value::typed(model, text).and_then(|value| self.enter_value(model, value))
            }
        }
    }

    pub fn click(&mut self, model: &Model, sketch: &Sketch) -> Outcome {
        let Some((feature, _)) = self.context else {
            return Outcome::Nothing;
        };
        match &mut self.state {
            State::Offset(offsetting) => offsetting.click(model, feature, sketch),
            State::Mirror(mirroring) => mirroring.click(model, feature),
            State::Pattern(patterning) => patterning.click(),
            State::Fillet(filleting) => filleting.click(model, feature),
            State::Tangent(tangent) => tangent.click(model, feature),
            State::Blend(blend) => blend.click(model, feature),
            State::Gear(gearing) => gearing.click(model, feature, self.gear.gear()),
            State::Idle => Outcome::Nothing,
        }
    }

    pub fn begin_pull(&mut self) -> bool {
        match &mut self.state {
            State::Offset(offsetting) => offsetting.can_pull(),
            State::Fillet(filleting) => filleting.begin_pull(),
            State::Mirror(_)
            | State::Pattern(_)
            | State::Tangent(_)
            | State::Blend(_)
            | State::Gear(_)
            | State::Idle => false,
        }
    }

    pub fn enter_value(&mut self, model: &Model, value: Value) -> Result<Outcome, String> {
        let Some((feature, _)) = self.context else {
            return Ok(Outcome::Nothing);
        };
        match &mut self.state {
            State::Offset(offsetting) => offsetting.enter_value(model, feature, value),
            State::Fillet(_)
            | State::Mirror(_)
            | State::Pattern(_)
            | State::Tangent(_)
            | State::Blend(_)
            | State::Gear(_)
            | State::Idle => Ok(Outcome::Nothing),
        }
    }

    pub fn finish(&mut self, model: &Model, sketch: &Sketch) -> Outcome {
        let Some((feature, _)) = self.context else {
            return Outcome::Nothing;
        };
        match &mut self.state {
            State::Offset(offsetting) => offsetting.click(model, feature, sketch),
            State::Mirror(mirroring) => mirroring.finish(model, feature),
            State::Pattern(patterning) => patterning.finish(),
            State::Fillet(filleting) => filleting.activate(model, feature),
            State::Tangent(tangent) => tangent.activate(model, feature),
            State::Blend(blend) => blend.activate(model, feature),
            State::Gear(gearing) => gearing.finish(model, feature, self.gear.gear()),
            State::Idle => Outcome::Nothing,
        }
    }

    pub fn steps_targets(&self) -> bool {
        match &self.state {
            State::Pattern(patterning) => patterning.steps_targets(),
            State::Mirror(mirroring) => !mirroring.gathers(),
            State::Fillet(_) | State::Tangent(_) | State::Blend(_) | State::Gear(_) => true,
            State::Offset(_) | State::Idle => false,
        }
    }

    pub fn steppable(&self, sketch: &Sketch) -> Result<(), &'static str> {
        match &self.state {
            State::Mirror(mirroring) => mirroring.steppable(),
            State::Pattern(patterning) => patterning.steppable(),
            State::Fillet(filleting) => filleting.steppable(sketch),
            State::Tangent(tangent) => tangent.steppable(),
            State::Blend(blend) => blend.steppable(sketch),
            State::Gear(gearing) => gearing.steppable(),
            State::Offset(_) | State::Idle => Ok(()),
        }
    }

    pub fn step(&mut self, sketch: &Sketch, step: isize) {
        match &mut self.state {
            State::Mirror(mirroring) => mirroring.step(sketch, step),
            State::Pattern(patterning) => patterning.step(sketch, step),
            State::Fillet(filleting) => filleting.step(sketch, step),
            State::Tangent(tangent) => tangent.step(sketch, step),
            State::Blend(blend) => blend.step(sketch, step),
            State::Gear(gearing) => gearing.step(sketch, step),
            State::Offset(_) | State::Idle => {}
        }
    }

    pub fn highlight_needed(&self) -> Result<(), &'static str> {
        match &self.state {
            State::Mirror(mirroring) => mirroring.highlight_needed(),
            State::Pattern(patterning) => patterning.highlight_needed(),
            State::Fillet(filleting) => filleting.highlight_needed(),
            State::Tangent(tangent) => tangent.highlight_needed(),
            State::Blend(blend) => blend.highlight_needed(),
            State::Gear(gearing) => gearing.highlight_needed(),
            State::Offset(_) | State::Idle => Ok(()),
        }
    }

    pub fn activate(&mut self, model: &Model) -> Outcome {
        let Some((feature, _)) = self.context else {
            return Outcome::Nothing;
        };
        match &mut self.state {
            State::Mirror(mirroring) => mirroring.activate(model, feature),
            State::Pattern(patterning) => patterning.activate(),
            State::Fillet(filleting) => filleting.activate(model, feature),
            State::Tangent(tangent) => tangent.activate(model, feature),
            State::Blend(blend) => blend.activate(model, feature),
            State::Gear(gearing) => gearing.activate(model, feature, self.gear.gear()),
            State::Offset(_) | State::Idle => Outcome::Nothing,
        }
    }

    pub fn clear_highlight(&mut self) {
        match &mut self.state {
            State::Mirror(mirroring) => mirroring.clear_highlight(),
            State::Pattern(patterning) => patterning.clear_highlight(),
            State::Fillet(filleting) => filleting.clear_highlight(),
            State::Tangent(tangent) => tangent.clear_highlight(),
            State::Blend(blend) => blend.clear_highlight(),
            State::Gear(gearing) => gearing.clear_highlight(),
            State::Offset(_) | State::Idle => {}
        }
    }

    pub fn can_back_out(&self) -> bool {
        match &self.state {
            State::Mirror(mirroring) => mirroring.can_back_out(),
            State::Pattern(patterning) => patterning.can_back_out(),
            State::Fillet(filleting) => filleting.can_back_out(),
            State::Tangent(tangent) => tangent.can_back_out(),
            State::Blend(blend) => blend.can_back_out(),
            State::Gear(gearing) => gearing.can_back_out(),
            State::Offset(_) | State::Idle => false,
        }
    }

    pub fn back_out(&mut self) {
        match &mut self.state {
            State::Mirror(mirroring) => mirroring.back_out(),
            State::Pattern(patterning) => patterning.back_out(),
            State::Fillet(filleting) => filleting.back_out(),
            State::Tangent(tangent) => tangent.back_out(),
            State::Blend(blend) => blend.back_out(),
            State::Gear(gearing) => gearing.back_out(),
            State::Offset(_) | State::Idle => {}
        }
    }
}
