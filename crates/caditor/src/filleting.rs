use caditor_document::{FeatureId, Transaction};
use caditor_expression::{Dimension, Expression, Unit};
use caditor_geometry::Point2;
use caditor_sketch::{
    Bevel, ChamferSize, Corner, Dimensioned, Entity, EntityId, Faceting, FilletError, Rounding,
    Sketch,
};

use crate::{
    drawing::Preview,
    editing::Tool,
    field::Expected,
    model::Model,
    modifying::{Hint, Outcome, Prompt, Value, ValueField, length_text},
    patterning,
    snap::{self, Pointer, Screen},
    trimming::{self, capitalized},
    typed_point,
    units::LengthUnit,
};

pub const CORNER_PROMPT: &str =
    "Click the corner to round, where two lines or arcs meet, or drag from it";
pub const RADIUS_PROMPT: &str = "Click where the fillet should pass, or type its radius";
pub const CHAMFER_CORNER_PROMPT: &str =
    "Click the corner to cut, where two lines or arcs meet, or drag from it";
pub const DISTANCE_PROMPT: &str = "Click where the chamfer should pass, or type how far from the corner it cuts: 5, or 5, 3 for a different distance on each curve, or 5 < 45 for a distance and an angle";
pub const TRANSACTION: &str = "Fillet corner";
pub const CHAMFER_TRANSACTION: &str = "Chamfer corner";
const RADIUS_KEYS: &str = "Type the radius   Enter: round it here   Esc: choose another corner";
const DISTANCE_KEYS: &str = "Type a distance, two distances, or a distance < angle   Enter: cut it here   Esc: choose another corner";
const NO_CORNER_HIGHLIGHTED: &str =
    "Highlight a corner first, with Highlight the next item in the view";
const NOTHING_TO_ROUND: &str = "The sketch has no corner where two lines or arcs meet";
const CHOOSE_FIRST: &str = "Choose the corner first";
pub const FIELD: ValueField = ValueField {
    label: "Fillet radius",
    placeholder: "radius",
    action: "round the corner",
};
pub const CHAMFER_FIELD: ValueField = ValueField {
    label: "Chamfer",
    placeholder: "5  or  5, 3  or  5 < 45",
    action: "cut the corner",
};
const CHAMFER_FORMS: &str = "Type a distance such as 5, two distances such as 5, 3, or a distance and an angle such as 5 < 45";
const LENGTH: Expected = Expected {
    dimension: Some(Dimension::LENGTH),
    non_negative: false,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CornerCut {
    #[default]
    Round,
    Chamfer,
}

impl CornerCut {
    fn tool(self) -> Tool {
        match self {
            Self::Round => Tool::Fillet,
            Self::Chamfer => Tool::Chamfer,
        }
    }

    fn field(self) -> ValueField {
        match self {
            Self::Round => FIELD,
            Self::Chamfer => CHAMFER_FIELD,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Cut {
    Round(Rounding),
    Bevel(Bevel),
}

impl Cut {
    fn size(self) -> f64 {
        match self {
            Self::Round(rounding) => rounding.radius,
            Self::Bevel(bevel) => bevel.distances[0],
        }
    }

    fn touches(self) -> [Point2; 2] {
        match self {
            Self::Round(rounding) => rounding.touches,
            Self::Bevel(bevel) => bevel.touches,
        }
    }

    fn faceted(self, faceting: Faceting) -> Vec<Point2> {
        match self {
            Self::Round(rounding) => rounding.faceted(faceting),
            Self::Bevel(bevel) => bevel.touches.to_vec(),
        }
    }
}

#[derive(Debug, Clone)]
enum Size {
    Radius(Value),
    Chamfer(ChamferSize),
}

#[derive(Debug, Clone, Default)]
pub struct Filleting {
    cut: CornerCut,
    chosen: Option<Corner>,
    hover: Option<Result<Corner, FilletError>>,
    highlight: Option<Corner>,
    pointer: Option<Point2>,
    typed: Option<f64>,
    typed_chamfer: Option<Result<ChamferSize, String>>,
    rounding: Option<Result<Cut, FilletError>>,
}

impl Filleting {
    pub fn new(cut: CornerCut) -> Self {
        Self {
            cut,
            ..Self::default()
        }
    }

    pub fn starting(sketch: &Sketch, selected: &[EntityId], cut: CornerCut) -> Self {
        let chosen = match selected {
            [point] => sketch.corner_at(*point).ok(),
            [first, second] => sketch.corner_between(*first, *second).ok(),
            _ => None,
        };
        Self {
            cut,
            chosen,
            ..Self::default()
        }
    }

    pub fn field(&self) -> ValueField {
        self.cut.field()
    }

    pub fn sync(&mut self, sketch: &Sketch) {
        let still = |corner: &Corner| {
            sketch
                .corner_at(corner.point)
                .is_ok_and(|now| now.curves == corner.curves)
        };
        self.chosen = self.chosen.filter(still);
        self.highlight = self.highlight.filter(still);
    }

    pub fn hover(&mut self, sketch: &Sketch, screen: &impl Screen, pointer: Option<Pointer>) {
        self.pointer = pointer.map(|pointer| pointer.sketch);
        self.hover = match self.chosen {
            Some(_) => None,
            None => pointer
                .and_then(|pointer| end_under(sketch, screen, pointer))
                .map(|point| sketch.corner_at(point)),
        };
        let cut = self.cut;
        self.rounding = self.target().and_then(|corner| match cut {
            CornerCut::Round => {
                let radius = self
                    .typed
                    .or_else(|| sketch.radius_through(&self.chosen?, self.pointer?))?;
                Some(sketch.rounding(&corner, radius).map(Cut::Round))
            }
            CornerCut::Chamfer => {
                let size = match &self.typed_chamfer {
                    Some(Ok(size)) => size.clone(),
                    Some(Err(_)) => return None,
                    None => {
                        let distance = sketch.distance_through(&self.chosen?, self.pointer?)?;
                        pointed(distance)
                    }
                };
                Some(sketch.bevel(&corner, &size).map(Cut::Bevel))
            }
        });
    }

    pub fn leave(&mut self) {
        self.pointer = None;
        self.hover = None;
        if self.typed.is_none() && self.typed_chamfer.is_none() {
            self.rounding = None;
        }
    }

    pub fn show_text(&mut self, model: &Model, text: Option<&str>) {
        match self.cut {
            CornerCut::Round => {
                self.typed = text
                    .and_then(|text| Value::typed(model, text).ok())
                    .map(|value| value.millimetres);
            }
            CornerCut::Chamfer => {
                self.typed_chamfer = text
                    .filter(|text| !text.trim().is_empty())
                    .map(|text| parse_chamfer(model, text));
            }
        }
    }

    pub fn enter_text(
        &mut self,
        model: &Model,
        feature: FeatureId,
        text: &str,
    ) -> Result<Outcome, String> {
        match self.cut {
            CornerCut::Round => {
                let value = Value::typed(model, text)?;
                self.enter_value(model, feature, value)
            }
            CornerCut::Chamfer => {
                let size = parse_chamfer(model, text)?;
                let corner = self
                    .target()
                    .ok_or_else(|| trimming::refusal(self.cut.tool(), CHOOSE_FIRST))?;
                self.round(model, feature, corner, Size::Chamfer(size))
                    .map(Outcome::Apply)
            }
        }
    }

    fn target(&self) -> Option<Corner> {
        self.chosen
            .or(self.highlight)
            .or_else(|| self.hover.as_ref()?.as_ref().ok().copied())
    }

    pub fn preview(&self, faceting: Faceting) -> Preview {
        let mut preview = Preview::default();
        if let Some(corner) = self.target() {
            preview.snap = Some(corner.position);
        }
        if let Some(Ok(cut)) = &self.rounding {
            preview.curves.push(cut.faceted(faceting));
            preview.points = cut.touches().to_vec();
        }
        preview
    }

    pub fn highlighted_entities(&self) -> Vec<EntityId> {
        self.target()
            .map(|corner| corner.curves.to_vec())
            .unwrap_or_default()
    }

    pub fn label(&self, sketch: &Sketch, unit: LengthUnit) -> Option<String> {
        let corner = match (self.target(), &self.hover) {
            (Some(corner), _) => corner,
            (None, Some(Err(error))) => return Some(capitalized(&error.to_string())),
            (None, _) => return None,
        };
        let [first, second] = corner.curves.map(|curve| sketch.entity_label(curve));
        let subject = match self.cut {
            CornerCut::Round => format!("Round the corner of {first} and {second}"),
            CornerCut::Chamfer => format!("Cut the corner of {first} and {second}"),
        };
        Some(match &self.rounding {
            Some(Ok(Cut::Round(rounding))) => format!(
                "{subject} with a radius of {}",
                length_text(unit, rounding.radius)
            ),
            Some(Ok(Cut::Bevel(bevel))) => format!("{subject} {}", bevel_words(unit, bevel)),
            Some(Err(error)) => capitalized(&error.to_string()),
            None => match &self.typed_chamfer {
                Some(Err(reason)) if self.cut == CornerCut::Chamfer => capitalized(reason),
                _ => subject,
            },
        })
    }

    pub fn prompt(&self) -> Prompt {
        match (self.chosen.is_some(), self.cut) {
            (true, CornerCut::Round) => Prompt {
                text: RADIUS_PROMPT,
                hint: Hint::Keys(RADIUS_KEYS),
            },
            (true, CornerCut::Chamfer) => Prompt {
                text: DISTANCE_PROMPT,
                hint: Hint::Keys(DISTANCE_KEYS),
            },
            (false, CornerCut::Round) => Prompt {
                text: CORNER_PROMPT,
                hint: Hint::Targets,
            },
            (false, CornerCut::Chamfer) => Prompt {
                text: CHAMFER_CORNER_PROMPT,
                hint: Hint::Targets,
            },
        }
    }

    pub fn click(&mut self, model: &Model, feature: FeatureId) -> Outcome {
        if self.chosen.is_some() {
            return self.round_at_pointer(model, feature);
        }
        match self.hover.clone() {
            Some(Ok(corner)) => {
                self.chosen = Some(corner);
                self.hover = None;
                Outcome::Nothing
            }
            Some(Err(error)) => {
                Outcome::Refused(trimming::refusal(self.cut.tool(), &error.to_string()))
            }
            None => Outcome::Nothing,
        }
    }

    pub fn begin_pull(&mut self) -> bool {
        if self.chosen.is_none()
            && let Some(Ok(corner)) = &self.hover
        {
            self.chosen = Some(*corner);
        }
        self.chosen.is_some()
    }

    pub fn activate(&mut self, model: &Model, feature: FeatureId) -> Outcome {
        if let Some(corner) = self.highlight.take() {
            self.chosen = Some(corner);
            return Outcome::Nothing;
        }
        if self.chosen.is_some() {
            return self.round_at_pointer(model, feature);
        }
        Outcome::Nothing
    }

    fn round_at_pointer(&mut self, model: &Model, feature: FeatureId) -> Outcome {
        let (Some(corner), Some(Ok(cut))) = (self.chosen, &self.rounding) else {
            return match &self.rounding {
                Some(Err(error)) => {
                    Outcome::Refused(trimming::refusal(self.cut.tool(), &error.to_string()))
                }
                _ => Outcome::Nothing,
            };
        };
        let Some(value) = Value::pointed(model, cut.size()) else {
            return Outcome::Nothing;
        };
        self.round(model, feature, corner, Size::Radius(value))
            .into()
    }

    pub fn enter_value(
        &mut self,
        model: &Model,
        feature: FeatureId,
        value: Value,
    ) -> Result<Outcome, String> {
        let corner = self
            .target()
            .ok_or_else(|| trimming::refusal(self.cut.tool(), CHOOSE_FIRST))?;
        self.round(model, feature, corner, Size::Radius(value))
            .map(Outcome::Apply)
    }

    fn round(
        &mut self,
        model: &Model,
        feature: FeatureId,
        corner: Corner,
        size: Size,
    ) -> Result<Transaction, String> {
        let cut = self.cut;
        let label = match cut {
            CornerCut::Round => TRANSACTION,
            CornerCut::Chamfer => CHAMFER_TRANSACTION,
        };
        let rounded = trimming::reshaped(model, feature, label.to_owned(), |sketch| {
            match (cut, &size) {
                (CornerCut::Round, Size::Radius(value)) => sketch
                    .fillet(&corner, value.millimetres, value.expression.clone())
                    .map(|_| ()),
                (CornerCut::Chamfer, Size::Radius(value)) => {
                    sketch.chamfer(&corner, &equal(value)).map(|_| ())
                }
                (_, Size::Chamfer(size)) => sketch.chamfer(&corner, size).map(|_| ()),
            }
            .map_err(|error| trimming::refusal(cut.tool(), &error.to_string()))
        })?;
        self.chosen = None;
        self.highlight = None;
        self.rounding = None;
        self.typed_chamfer = None;
        Ok(rounded)
    }

    pub fn steppable(&self, sketch: &Sketch) -> Result<(), &'static str> {
        let curves = sketch
            .entities()
            .filter(|(_, entity)| matches!(entity, Entity::Line { .. } | Entity::Arc { .. }))
            .take(2)
            .count();
        if curves == 2 {
            Ok(())
        } else {
            Err(NOTHING_TO_ROUND)
        }
    }

    pub fn step(&mut self, sketch: &Sketch, step: isize) {
        let corners = sketch.fillet_corners();
        let count = corners.len() as isize;
        if count == 0 {
            return;
        }
        let current = self.highlight.or(self.chosen).and_then(|aim| {
            corners
                .iter()
                .position(|corner| corner.curves == aim.curves && corner.position == aim.position)
        });
        let next = match current {
            Some(index) => (index as isize + step).rem_euclid(count),
            None if step < 0 => count - 1,
            None => 0,
        };
        self.highlight = usize::try_from(next)
            .ok()
            .and_then(|next| corners.get(next).copied());
    }

    pub fn highlight_needed(&self) -> Result<(), &'static str> {
        match self.highlight {
            Some(_) => Ok(()),
            None => Err(NO_CORNER_HIGHLIGHTED),
        }
    }

    pub fn clear_highlight(&mut self) {
        self.highlight = None;
    }

    pub fn can_back_out(&self) -> bool {
        self.highlight.is_some() || self.chosen.is_some()
    }

    pub fn back_out(&mut self) {
        if self.highlight.take().is_none() && self.chosen.take().is_some() {
            self.rounding = None;
        }
    }
}

fn end_under(sketch: &Sketch, screen: &impl Screen, pointer: Pointer) -> Option<EntityId> {
    sketch
        .entities()
        .flat_map(|(_, entity)| match entity {
            Entity::Line { start, end }
            | Entity::Arc { start, end, .. }
            | Entity::EllipticalArc { start, end, .. } => vec![*start, *end],
            Entity::Spline { control_points } => control_points
                .first()
                .into_iter()
                .chain(control_points.last())
                .copied()
                .collect(),
            Entity::Point(_) | Entity::Circle { .. } | Entity::Ellipse { .. } => Vec::new(),
        })
        .filter_map(|point| {
            let offset = screen
                .to_screen(sketch.point(point)?)?
                .distance(pointer.screen);
            (offset <= snap::POINT_TOLERANCE).then_some((offset, point))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, point)| point)
}

fn equal(value: &Value) -> ChamferSize {
    ChamferSize::Equal(Dimensioned {
        expression: value.expression.clone(),
        value: value.millimetres,
    })
}

fn pointed(millimetres: f64) -> ChamferSize {
    ChamferSize::Equal(Dimensioned {
        expression: Expression::Measure(millimetres, Unit::Millimetre),
        value: millimetres,
    })
}

fn bevel_words(unit: LengthUnit, bevel: &Bevel) -> String {
    let [first, second] = bevel.distances.map(|distance| length_text(unit, distance));
    if first == second {
        format!("{first} from it on each side")
    } else {
        format!("{first} from it on the first and {second} on the second")
    }
}

fn parse_chamfer(model: &Model, text: &str) -> Result<ChamferSize, String> {
    let distance = |term: &str| patterning::quantity(model, term, LENGTH, "distance");
    let terms = typed_point::coordinates(text);
    match terms.as_slice() {
        [term] => match typed_point::polar(term).as_slice() {
            [only] => Ok(ChamferSize::Equal(distance(only)?)),
            [only, angle] => Ok(ChamferSize::DistanceAndAngle {
                distance: distance(only)?,
                angle: patterning::degrees(model, angle)?,
            }),
            _ => Err(CHAMFER_FORMS.to_owned()),
        },
        [first, second] => {
            let plain = |term: &&str| typed_point::polar(term).len() == 1;
            if !(plain(first) && plain(second)) {
                return Err(CHAMFER_FORMS.to_owned());
            }
            Ok(ChamferSize::Distances {
                first: distance(first)?,
                second: distance(second)?,
            })
        }
        _ => Err(CHAMFER_FORMS.to_owned()),
    }
}
