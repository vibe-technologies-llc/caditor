use caditor_document::{FeatureId, Transaction};
use caditor_geometry::Point2;
use caditor_sketch::{Caps, Chain, EntityId, Faceting, OffsetError, Outline, Side, Sketch};

use crate::{
    drawing::Preview,
    editing::Tool,
    feature_tree::count,
    model::Model,
    modifying::{Hint, Outcome, Prompt, Value, ValueField, length_text},
    shape_modes::OffsetMode,
    snap::{Pointer, Screen},
    trimming::{self, capitalized},
    units::LengthUnit,
};

pub const PROMPT: &str = "Click on the side and at the distance the offset should run, or type it";
pub const BOTH_PROMPT: &str =
    "Click at the distance the offset should run on each side, or type it";
pub const CHOOSE_PROMPT: &str = "Click a line, arc or circle to take its chain, or an ellipse, or \
                                 select the curves to offset first";
pub const FREE_SPLINE: &str = "as a spline that will not follow it";
pub const TRANSACTION: &str = "Offset curves";
pub const BOTH_TRANSACTION: &str = "Offset curves to both sides";
const KEYS: &str = "Type a distance, negative for the other side   Esc: back to Select";
const BOTH_KEYS: &str = "Type the distance on each side   Esc: back to Select";
const CHOOSE_KEYS: &str = "Esc: back to Select";
pub const FIELD: ValueField = ValueField {
    label: "Offset by",
    placeholder: "distance",
    action: "offset",
};

#[derive(Debug, Clone, Default)]
pub struct Offsetting {
    mode: OffsetMode,
    chain: Option<Result<Chain, OffsetError>>,
    pointer: Option<Point2>,
    under: Option<EntityId>,
    typed: Option<f64>,
}

impl Offsetting {
    pub fn sync(&mut self, sketch: &Sketch, selected: &[EntityId], mode: OffsetMode) {
        self.mode = mode;
        self.chain = (!selected.is_empty()).then(|| sketch.offset_chain(selected));
    }

    pub fn hover(&mut self, sketch: &Sketch, screen: &impl Screen, pointer: Option<Pointer>) {
        self.pointer = pointer.map(|pointer| pointer.sketch);
        self.under = match self.chain {
            Some(Ok(_)) => None,
            Some(Err(_)) | None => pointer
                .and_then(|pointer| trimming::curve_under(sketch, screen, pointer))
                .map(|(curve, _)| curve),
        };
    }

    pub fn leave(&mut self) {
        self.pointer = None;
        self.under = None;
    }

    pub fn show_typed(&mut self, typed: Option<f64>) {
        self.typed = typed;
    }

    pub fn can_pull(&self) -> bool {
        matches!(self.chain, Some(Ok(_)))
    }

    fn chain(&self) -> Option<&Chain> {
        self.chain.as_ref()?.as_ref().ok()
    }

    fn side(&self, chain: &Chain) -> Side {
        self.pointer
            .map_or(chain.default_side(), |pointer| chain.side_of(pointer))
    }

    fn placement(&self) -> Option<(Side, f64)> {
        let chain = self.chain()?;
        let side = self.side(chain);
        match self.typed {
            Some(value) if value < 0.0 => Some((side.other(), -value)),
            Some(value) => Some((side, value)),
            None => Some((side, chain.distance_to(self.pointer?))),
        }
    }

    fn outlines(&self) -> Option<Result<Vec<Outline>, OffsetError>> {
        let chain = self.chain()?;
        let (side, distance) = self.placement()?;
        Some(match self.mode.caps() {
            Some(caps) => chain.both_sides(distance, caps),
            None => chain.outline(side, distance).map(|outline| vec![outline]),
        })
    }

    pub fn preview(&self, faceting: Faceting) -> Preview {
        let mut preview = Preview::default();
        if let Some(Ok(outlines)) = self.outlines() {
            preview.curves = outlines
                .iter()
                .flat_map(|outline| outline.faceted(faceting))
                .collect();
        }
        preview
    }

    pub fn highlighted_entities(&self) -> Vec<EntityId> {
        self.under.into_iter().collect()
    }

    pub fn label(&self, sketch: &Sketch, unit: LengthUnit) -> Option<String> {
        if let Some(curve) = self.under {
            return Some(format!(
                "Take the chain of {} to offset",
                sketch.entity_label(curve)
            ));
        }
        let chain = match self.chain.as_ref()? {
            Ok(chain) => chain,
            Err(error) => return Some(capitalized(&error.to_string())),
        };
        let subject = subject(sketch, chain);
        let way = self.way(chain);
        match (self.placement(), self.outlines()) {
            (Some((_, distance)), Some(Ok(_))) if !chain.follows() => Some(format!(
                "Offset {subject} by {}{way} {FREE_SPLINE}",
                length_text(unit, distance)
            )),
            (Some((_, distance)), Some(Ok(_))) => Some(format!(
                "Offset {subject} by {}{way}",
                length_text(unit, distance)
            )),
            (_, Some(Err(error))) => Some(capitalized(&error.to_string())),
            _ => Some(format!("Offset {subject}")),
        }
    }

    fn way(&self, chain: &Chain) -> &'static str {
        match self.mode.caps() {
            None => "",
            Some(_) if chain.is_closed() => " inside and outside",
            Some(Caps::Round) => " to both sides with round ends",
            Some(Caps::Flat) => " to both sides with flat ends",
        }
    }

    pub fn prompt(&self) -> Prompt {
        if self.chain().is_some() && self.mode.caps().is_some() {
            Prompt {
                text: BOTH_PROMPT,
                hint: Hint::Keys(BOTH_KEYS),
            }
        } else if self.chain().is_some() {
            Prompt {
                text: PROMPT,
                hint: Hint::Keys(KEYS),
            }
        } else {
            Prompt {
                text: CHOOSE_PROMPT,
                hint: Hint::Keys(CHOOSE_KEYS),
            }
        }
    }

    pub fn click(&mut self, model: &Model, feature: FeatureId, sketch: &Sketch) -> Outcome {
        if let Some(curve) = self.under {
            let chain = sketch.offset_chain_through(curve);
            return match sketch.offset_chain(&chain) {
                Ok(_) => Outcome::Select(chain),
                Err(error) => Outcome::Refused(trimming::refusal(Tool::Offset, &error.to_string())),
            };
        }
        match &self.chain {
            Some(Ok(chain)) => {
                let Some((side, distance)) = self.placement() else {
                    return Outcome::Nothing;
                };
                let Some(value) = Value::pointed(model, distance) else {
                    return Outcome::Nothing;
                };
                commit(model, feature, chain, self.mode, side, value).into()
            }
            Some(Err(error)) => {
                Outcome::Refused(trimming::refusal(Tool::Offset, &error.to_string()))
            }
            None => Outcome::Nothing,
        }
    }

    pub fn enter_value(
        &mut self,
        model: &Model,
        feature: FeatureId,
        value: Value,
    ) -> Result<Outcome, String> {
        let chain = match &self.chain {
            Some(Ok(chain)) => chain,
            Some(Err(error)) => return Err(trimming::refusal(Tool::Offset, &error.to_string())),
            None => {
                return Err(trimming::refusal(
                    Tool::Offset,
                    &OffsetError::NothingSelected.to_string(),
                ));
            }
        };
        let side = self.side(chain);
        let (side, value) = if value.millimetres < 0.0 {
            (side.other(), value.negated())
        } else {
            (side, value)
        };
        commit(model, feature, chain, self.mode, side, value).map(Outcome::Apply)
    }
}

fn subject(sketch: &Sketch, chain: &Chain) -> String {
    match chain.curves().as_slice() {
        [only] => sketch.entity_label(*only),
        curves => count(curves.len(), "curve", "curves"),
    }
}

fn commit(
    model: &Model,
    feature: FeatureId,
    chain: &Chain,
    mode: OffsetMode,
    side: Side,
    value: Value,
) -> Result<Transaction, String> {
    let curves = chain.curves();
    let label = match mode.caps() {
        Some(_) => BOTH_TRANSACTION,
        None => TRANSACTION,
    };
    trimming::reshaped(model, feature, label.to_owned(), |sketch| {
        match mode.caps() {
            Some(caps) => {
                sketch.offset_both_sides(&curves, value.millimetres, value.expression, caps)
            }
            None => sketch.offset(&curves, side, value.millimetres, value.expression),
        }
        .map(|_| ())
        .map_err(|error| trimming::refusal(Tool::Offset, &error.to_string()))
    })
}
