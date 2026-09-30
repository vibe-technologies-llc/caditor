use caditor_document::{FeatureId, Transaction};
use caditor_geometry::Point2;
use caditor_sketch::{Corner, Entity, EntityId, Faceting, FilletError, Rounding, Sketch};

use crate::{
    drawing::Preview,
    editing::Tool,
    model::Model,
    modifying::{Hint, Outcome, Prompt, Value, ValueField, length_text},
    snap::{self, Pointer, Screen},
    trimming::{self, capitalized},
    units::LengthUnit,
};

pub const CORNER_PROMPT: &str =
    "Click the corner to round, where two lines or arcs meet, or drag from it";
pub const RADIUS_PROMPT: &str = "Click where the fillet should pass, or type its radius";
pub const TRANSACTION: &str = "Fillet corner";
const RADIUS_KEYS: &str = "Type the radius   Enter: round it here   Esc: choose another corner";
const NO_CORNER_HIGHLIGHTED: &str =
    "Highlight a corner first, with Highlight the next item in the view";
const NOTHING_TO_ROUND: &str = "The sketch has no corner where two lines or arcs meet";
const CHOOSE_FIRST: &str = "Choose the corner to round first";
pub const FIELD: ValueField = ValueField {
    label: "Fillet radius",
    placeholder: "radius",
    action: "round the corner",
};

#[derive(Debug, Clone, Default)]
pub struct Filleting {
    chosen: Option<Corner>,
    hover: Option<Result<Corner, FilletError>>,
    highlight: Option<Corner>,
    pointer: Option<Point2>,
    typed: Option<f64>,
    rounding: Option<Result<Rounding, FilletError>>,
}

impl Filleting {
    pub fn starting(sketch: &Sketch, selected: &[EntityId]) -> Self {
        let chosen = match selected {
            [point] => sketch.corner_at(*point).ok(),
            [first, second] => sketch.corner_between(*first, *second).ok(),
            _ => None,
        };
        Self {
            chosen,
            ..Self::default()
        }
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
        self.rounding = self.target().and_then(|corner| {
            let radius = self.typed.or_else(|| {
                self.chosen
                    .and_then(|chosen| sketch.radius_through(&chosen, self.pointer?))
            })?;
            Some(sketch.rounding(&corner, radius))
        });
    }

    pub fn leave(&mut self) {
        self.pointer = None;
        self.hover = None;
        if self.typed.is_none() {
            self.rounding = None;
        }
    }

    pub fn show_typed(&mut self, typed: Option<f64>) {
        self.typed = typed;
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
        if let Some(Ok(rounding)) = &self.rounding {
            preview.curves.push(rounding.faceted(faceting));
            preview.points = rounding.touches.to_vec();
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
        let subject = format!("Round the corner of {first} and {second}");
        Some(match &self.rounding {
            Some(Ok(rounding)) => format!(
                "{subject} with a radius of {}",
                length_text(unit, rounding.radius)
            ),
            Some(Err(error)) => capitalized(&error.to_string()),
            None => subject,
        })
    }

    pub fn prompt(&self) -> Prompt {
        if self.chosen.is_some() {
            Prompt {
                text: RADIUS_PROMPT,
                hint: Hint::Keys(RADIUS_KEYS),
            }
        } else {
            Prompt {
                text: CORNER_PROMPT,
                hint: Hint::Targets,
            }
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
                Outcome::Refused(trimming::refusal(Tool::Fillet, &error.to_string()))
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
        let (Some(corner), Some(Ok(rounding))) = (self.chosen, &self.rounding) else {
            return match &self.rounding {
                Some(Err(error)) => {
                    Outcome::Refused(trimming::refusal(Tool::Fillet, &error.to_string()))
                }
                _ => Outcome::Nothing,
            };
        };
        let Some(value) = Value::pointed(model, rounding.radius) else {
            return Outcome::Nothing;
        };
        self.round(model, feature, corner, value).into()
    }

    pub fn enter_value(
        &mut self,
        model: &Model,
        feature: FeatureId,
        value: Value,
    ) -> Result<Outcome, String> {
        let corner = self
            .target()
            .ok_or_else(|| trimming::refusal(Tool::Fillet, CHOOSE_FIRST))?;
        self.round(model, feature, corner, value)
            .map(Outcome::Apply)
    }

    fn round(
        &mut self,
        model: &Model,
        feature: FeatureId,
        corner: Corner,
        value: Value,
    ) -> Result<Transaction, String> {
        let rounded = trimming::reshaped(model, feature, TRANSACTION.to_owned(), |sketch| {
            sketch
                .fillet(&corner, value.millimetres, value.expression)
                .map(|_| ())
                .map_err(|error| trimming::refusal(Tool::Fillet, &error.to_string()))
        })?;
        self.chosen = None;
        self.highlight = None;
        self.rounding = None;
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
            Entity::Line { start, end } | Entity::Arc { start, end, .. } => vec![*start, *end],
            Entity::Spline { control_points } => control_points
                .first()
                .into_iter()
                .chain(control_points.last())
                .copied()
                .collect(),
            Entity::Point(_) | Entity::Circle { .. } => Vec::new(),
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
