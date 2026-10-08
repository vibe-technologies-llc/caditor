use caditor_document::FeatureId;
use caditor_geometry::Point2;
use caditor_sketch::{
    Dimensioned, Entity, EntityId, Faceting, Sketch, TangentCircle, TangentError,
};

use crate::{
    drawing::Preview,
    editing::Tool,
    model::Model,
    modifying::{Hint, Outcome, Prompt, Value, ValueField, length_text},
    snap::{self, Pointer, Screen},
    trimming::{self, capitalized},
    units::LengthUnit,
};

pub const TRANSACTION: &str = "Draw tangent circle";
pub const PROMPT: &str = "Click the lines, circles or arcs the circle should touch: a third one finishes it, or choose two and type a radius";
pub const RADIUS_PROMPT: &str = "Click a third curve for the circle touching all three, or type the radius of one touching these two";
const PICK_KEYS: &str = "Esc: back to Select";
const RADIUS_KEYS: &str = "Type the radius   Enter: draw it   Esc: let go of the last curve";
const NOTHING_HIGHLIGHTED: &str =
    "Highlight a line, circle or arc first, with Highlight the next item in the view";
const TWO_FIRST: &str = "choose the two curves the circle should touch first";
const MOST_PICKS: usize = 3;
pub const FIELD: ValueField = ValueField {
    label: "Radius",
    placeholder: "radius",
    action: "draw the circle",
};

#[derive(Debug, Clone, Default)]
pub struct TangentCircling {
    picked: Vec<EntityId>,
    hover: Option<EntityId>,
    highlight: Option<EntityId>,
    pointer: Option<Point2>,
    radius: Option<Result<Dimensioned, String>>,
    circle: Option<(Vec<EntityId>, Result<TangentCircle, TangentError>)>,
}

impl TangentCircling {
    pub fn starting(sketch: &Sketch, selected: &[EntityId]) -> Self {
        let curves: Vec<EntityId> = selected
            .iter()
            .copied()
            .filter(|curve| is_touchable(sketch, *curve))
            .collect();
        Self {
            picked: if curves.len() < MOST_PICKS {
                curves
            } else {
                Vec::new()
            },
            ..Self::default()
        }
    }

    pub fn field(&self) -> ValueField {
        FIELD
    }

    pub fn sync(&mut self, sketch: &Sketch) {
        self.picked.retain(|curve| is_touchable(sketch, *curve));
        self.highlight = self.highlight.filter(|curve| is_touchable(sketch, *curve));
    }

    pub fn hover(&mut self, sketch: &Sketch, screen: &impl Screen, pointer: Option<Pointer>) {
        if let Some(pointer) = pointer {
            self.pointer = Some(pointer.sketch);
        }
        self.hover = pointer.and_then(|pointer| curve_under(sketch, screen, pointer));
        self.circle = self.candidate().map(|(curves, radius)| {
            let found = sketch.tangent_circle_near(&curves, radius, self.pointer);
            (curves, found)
        });
    }

    pub fn leave(&mut self) {
        self.hover = None;
        if self.picked.len() < 2 {
            self.circle = None;
        }
    }

    fn aim(&self) -> Option<EntityId> {
        self.highlight
            .or(self.hover)
            .filter(|curve| !self.picked.contains(curve))
    }

    fn candidate(&self) -> Option<(Vec<EntityId>, Option<f64>)> {
        if self.picked.len() != 2 {
            return None;
        }
        if let Some(Ok(radius)) = &self.radius {
            return Some((self.picked.clone(), Some(radius.value)));
        }
        let third = self.aim()?;
        let mut curves = self.picked.clone();
        curves.push(third);
        Some((curves, None))
    }

    pub fn show_text(&mut self, model: &Model, text: Option<&str>) {
        self.radius = text.filter(|text| !text.trim().is_empty()).map(|text| {
            Value::typed(model, text).map(|value| Dimensioned {
                expression: value.expression,
                value: value.millimetres,
            })
        });
    }

    pub fn preview(&self, faceting: Faceting) -> Preview {
        let mut preview = Preview::default();
        if let Some((_, Ok(circle))) = &self.circle {
            preview.curves.push(circle.faceted(faceting));
            preview.points = vec![circle.center];
        }
        preview
    }

    pub fn highlighted_entities(&self) -> Vec<EntityId> {
        self.picked
            .iter()
            .copied()
            .chain(self.aim())
            .filter(|curve| !curve.is_reference())
            .collect()
    }

    pub fn label(&self, sketch: &Sketch, unit: LengthUnit) -> Option<String> {
        let (curves, found) = self.circle.as_ref()?;
        Some(match found {
            Ok(circle) => format!(
                "Circle touching {} with a radius of {}",
                names(sketch, curves),
                length_text(unit, circle.radius)
            ),
            Err(error) => capitalized(&error.to_string()),
        })
    }

    pub fn prompt(&self) -> Prompt {
        if self.picked.len() == 2 {
            Prompt {
                text: RADIUS_PROMPT,
                hint: Hint::Keys(RADIUS_KEYS),
            }
        } else {
            Prompt {
                text: PROMPT,
                hint: if self.picked.is_empty() {
                    Hint::Keys(PICK_KEYS)
                } else {
                    Hint::Targets
                },
            }
        }
    }

    pub fn click(&mut self, model: &Model, feature: FeatureId) -> Outcome {
        match self.hover {
            Some(curve) => self.pick(model, feature, curve),
            None => Outcome::Nothing,
        }
    }

    pub fn activate(&mut self, model: &Model, feature: FeatureId) -> Outcome {
        match self.highlight.take() {
            Some(curve) => self.pick(model, feature, curve),
            None => Outcome::Nothing,
        }
    }

    fn pick(&mut self, model: &Model, feature: FeatureId, curve: EntityId) -> Outcome {
        if let Some(index) = self.picked.iter().position(|picked| *picked == curve) {
            self.picked.remove(index);
            self.circle = None;
            return Outcome::Nothing;
        }
        if self.picked.len() < 2 {
            self.picked.push(curve);
            return Outcome::Nothing;
        }
        let mut curves = self.picked.clone();
        curves.push(curve);
        self.draw(model, feature, curves, None).into()
    }

    pub fn enter_text(
        &mut self,
        model: &Model,
        feature: FeatureId,
        text: &str,
    ) -> Result<Outcome, String> {
        let value = Value::typed(model, text)?;
        if self.picked.len() != 2 {
            return Err(trimming::refusal(Tool::TangentCircle, TWO_FIRST));
        }
        let radius = Dimensioned {
            expression: value.expression,
            value: value.millimetres,
        };
        let curves = self.picked.clone();
        self.draw(model, feature, curves, Some(radius))
            .map(Outcome::Apply)
    }

    fn draw(
        &mut self,
        model: &Model,
        feature: FeatureId,
        curves: Vec<EntityId>,
        radius: Option<Dimensioned>,
    ) -> Result<caditor_document::Transaction, String> {
        let near = self.pointer;
        let drawn = trimming::reshaped(model, feature, TRANSACTION.to_owned(), |sketch| {
            sketch
                .tangent_circle(&curves, radius.as_ref(), near)
                .map(|_| ())
                .map_err(|error| trimming::refusal(Tool::TangentCircle, &error.to_string()))
        })?;
        self.picked.clear();
        self.highlight = None;
        self.circle = None;
        Ok(drawn)
    }

    pub fn steppable(&self) -> Result<(), &'static str> {
        Ok(())
    }

    pub fn step(&mut self, sketch: &Sketch, step: isize) {
        let curves = curves(sketch);
        let count = curves.len() as isize;
        if count == 0 {
            return;
        }
        let current = self
            .highlight
            .and_then(|aim| curves.iter().position(|curve| *curve == aim));
        let next = match current {
            Some(index) => (index as isize + step).rem_euclid(count),
            None if step < 0 => count - 1,
            None => 0,
        };
        self.highlight = usize::try_from(next)
            .ok()
            .and_then(|next| curves.get(next).copied());
    }

    pub fn highlight_needed(&self) -> Result<(), &'static str> {
        match self.highlight {
            Some(_) => Ok(()),
            None => Err(NOTHING_HIGHLIGHTED),
        }
    }

    pub fn clear_highlight(&mut self) {
        self.highlight = None;
    }

    pub fn can_back_out(&self) -> bool {
        self.highlight.is_some() || !self.picked.is_empty()
    }

    pub fn back_out(&mut self) {
        if self.highlight.take().is_none() {
            self.picked.pop();
            self.circle = None;
        }
    }
}

fn names(sketch: &Sketch, curves: &[EntityId]) -> String {
    let labels: Vec<String> = curves
        .iter()
        .map(|curve| sketch.entity_label(*curve))
        .collect();
    match labels.as_slice() {
        [front @ .., last] if !front.is_empty() => format!("{} and {last}", front.join(", ")),
        [only] => only.clone(),
        _ => String::new(),
    }
}

fn is_touchable(sketch: &Sketch, curve: EntityId) -> bool {
    curve == EntityId::HORIZONTAL_AXIS
        || curve == EntityId::VERTICAL_AXIS
        || matches!(
            sketch.entity(curve),
            Some(Entity::Line { .. } | Entity::Circle { .. } | Entity::Arc { .. })
        )
}

fn curves(sketch: &Sketch) -> Vec<EntityId> {
    [EntityId::HORIZONTAL_AXIS, EntityId::VERTICAL_AXIS]
        .into_iter()
        .chain(
            sketch
                .entities()
                .map(|(id, _)| id)
                .filter(|id| is_touchable(sketch, *id)),
        )
        .collect()
}

fn curve_under(sketch: &Sketch, screen: &impl Screen, pointer: Pointer) -> Option<EntityId> {
    let at = pointer.sketch;
    let axes = [
        (EntityId::HORIZONTAL_AXIS, Point2::new(at.x, 0.0)),
        (EntityId::VERTICAL_AXIS, Point2::new(0.0, at.y)),
    ];
    let found = sketch
        .entities()
        .map(|(id, _)| id)
        .filter(|id| is_touchable(sketch, *id))
        .filter_map(|curve| Some((curve, sketch.closest_on_curve(curve, at)?)));
    found
        .chain(axes)
        .filter_map(|(curve, on)| {
            let offset = screen.to_screen(on)?.distance(pointer.screen);
            (offset <= snap::CURVE_TOLERANCE).then_some((offset, curve))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, curve)| curve)
}
