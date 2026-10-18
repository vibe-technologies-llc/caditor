use std::collections::BTreeMap;

use caditor_document::{FeatureId, Transaction};
use caditor_geometry::Point2;
use caditor_sketch::{Caps, Chain, EntityId, Faceting, OffsetError, Outline, Side, Sketch};
use thiserror::Error;

use crate::{
    drawing::Preview,
    editing::Tool,
    feature_tree::count,
    model::Model,
    modifying::{Hint, Outcome, Prompt, Value, ValueField, length_text},
    shape_modes::OffsetMode,
    snap::{Pointer, Screen},
    trimming::{self, SketchGone, capitalized},
    units::Units,
};

pub const PROMPT: &str = "Click on the side and at the distance the offset should run, or type it";
pub const BOTH_PROMPT: &str =
    "Click at the distance the offset should run on each side, or type it";
pub const CHOOSE_PROMPT: &str = "Click a curve to take its chain, or select the curves to offset \
                                 first";
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

#[derive(Debug, Clone, PartialEq, Error)]
pub enum OffsettingError {
    #[error(transparent)]
    SketchGone(#[from] SketchGone),
    #[error("{}: {source}.", Tool::Offset.label())]
    Refused { source: OffsetError },
}

#[derive(Debug, Clone, Default)]
pub struct Offsetting {
    mode: OffsetMode,
    chains: Option<Result<Vec<Chain>, OffsetError>>,
    pointer: Option<Point2>,
    under: Option<EntityId>,
    typed: Option<f64>,
}

impl Offsetting {
    pub fn sync(&mut self, sketch: &Sketch, selected: &[EntityId], mode: OffsetMode) {
        self.mode = mode;
        self.chains = (!selected.is_empty()).then(|| sketch.offset_chains(selected));
    }

    pub fn hover(&mut self, sketch: &Sketch, screen: &impl Screen, pointer: Option<Pointer>) {
        self.pointer = pointer.map(|pointer| pointer.sketch);
        self.under = match self.chains {
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
        matches!(self.chains, Some(Ok(_)))
    }

    fn chains(&self) -> Option<&[Chain]> {
        self.chains.as_ref()?.as_ref().ok().map(Vec::as_slice)
    }

    fn side(&self, chain: &Chain, flipped: bool) -> Side {
        let side = self
            .pointer
            .map_or(chain.default_side(), |pointer| chain.side_of(pointer));
        if flipped { side.other() } else { side }
    }

    fn placement(&self) -> Option<Placement> {
        let chains = self.chains()?;
        let (flipped, distance) = match self.typed {
            Some(value) => (value < 0.0, value.abs()),
            None => {
                let pointer = self.pointer?;
                let nearest = chains
                    .iter()
                    .map(|chain| chain.distance_to(pointer))
                    .fold(f64::INFINITY, f64::min);
                (false, nearest)
            }
        };
        Some(Placement {
            sides: chains
                .iter()
                .map(|chain| (chain.curves(), self.side(chain, flipped)))
                .collect(),
            distance,
        })
    }

    fn outlines(&self) -> Option<Result<Vec<Outline>, OffsetError>> {
        let chains = self.chains()?;
        let placement = self.placement()?;
        let mut outlines = Vec::new();
        for chain in chains {
            let made = match self.mode.caps() {
                Some(caps) => chain.both_sides(placement.distance, caps),
                None => chain
                    .outline(placement.side_of(chain), placement.distance)
                    .map(|outline| vec![outline]),
            };
            match made {
                Ok(made) => outlines.extend(made),
                Err(error) => return Some(Err(error)),
            }
        }
        Some(Ok(outlines))
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

    pub fn label(&self, sketch: &Sketch, unit: Units) -> Option<String> {
        if let Some(curve) = self.under {
            return Some(format!(
                "Take the chain of {} to offset",
                sketch.entity_label(curve)
            ));
        }
        let chains = match self.chains.as_ref()? {
            Ok(chains) => chains,
            Err(error) => return Some(capitalized(&error.to_string())),
        };
        let subject = subject(sketch, chains);
        let way = self.way(chains);
        let free = chains.iter().any(|chain| !chain.follows());
        match (self.placement(), self.outlines()) {
            (Some(placement), Some(Ok(_))) if free => Some(format!(
                "Offset {subject} by {}{way} {}",
                length_text(unit, placement.distance),
                free_spline(chains)
            )),
            (Some(placement), Some(Ok(_))) => Some(format!(
                "Offset {subject} by {}{way}",
                length_text(unit, placement.distance)
            )),
            (_, Some(Err(error))) => Some(capitalized(&error.to_string())),
            _ => Some(format!("Offset {subject}")),
        }
    }

    fn way(&self, chains: &[Chain]) -> &'static str {
        let closed = chains.iter().all(Chain::is_closed);
        match self.mode.caps() {
            None => "",
            Some(_) if closed => " inside and outside",
            Some(Caps::Round) => " to both sides with round ends",
            Some(Caps::Flat) => " to both sides with flat ends",
        }
    }

    pub fn prompt(&self) -> Prompt {
        if self.chains().is_some() && self.mode.caps().is_some() {
            Prompt {
                text: BOTH_PROMPT,
                hint: Hint::Keys(BOTH_KEYS),
            }
        } else if self.chains().is_some() {
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
            return match sketch.offset_chains(&chain) {
                Ok(_) => Outcome::Select(chain),
                Err(source) => Outcome::Refused(OffsettingError::Refused { source }.into()),
            };
        }
        match &self.chains {
            Some(Ok(chains)) => {
                let Some(placement) = self.placement() else {
                    return Outcome::Nothing;
                };
                let Some(value) = Value::pointed(model, placement.distance) else {
                    return Outcome::Nothing;
                };
                commit(model, feature, chains, self.mode, &placement.sides, value).into()
            }
            Some(Err(source)) => Outcome::Refused(
                OffsettingError::Refused {
                    source: source.clone(),
                }
                .into(),
            ),
            None => Outcome::Nothing,
        }
    }

    pub fn enter_value(
        &mut self,
        model: &Model,
        feature: FeatureId,
        value: Value,
    ) -> Result<Outcome, OffsettingError> {
        let chains = match &self.chains {
            Some(Ok(chains)) => chains,
            Some(Err(source)) => {
                return Err(OffsettingError::Refused {
                    source: source.clone(),
                });
            }
            None => {
                return Err(OffsettingError::Refused {
                    source: OffsetError::NothingSelected,
                });
            }
        };
        let flipped = value.millimetres < 0.0;
        let value = if flipped { value.negated() } else { value };
        let sides: Sides = chains
            .iter()
            .map(|chain| (chain.curves(), self.side(chain, flipped)))
            .collect();
        commit(model, feature, chains, self.mode, &sides, value).map(Outcome::Apply)
    }
}

type Sides = BTreeMap<Vec<EntityId>, Side>;

struct Placement {
    sides: Sides,
    distance: f64,
}

impl Placement {
    fn side_of(&self, chain: &Chain) -> Side {
        self.sides
            .get(&chain.curves())
            .copied()
            .unwrap_or_else(|| chain.default_side())
    }
}

fn free_spline(chains: &[Chain]) -> &'static str {
    match chains {
        [_] => FREE_SPLINE,
        _ => "with the splines among them not following",
    }
}

fn subject(sketch: &Sketch, chains: &[Chain]) -> String {
    let curves: Vec<EntityId> = chains.iter().flat_map(Chain::curves).collect();
    match curves.as_slice() {
        [only] => sketch.entity_label(*only),
        curves => count(curves.len(), "curve", "curves"),
    }
}

fn commit(
    model: &Model,
    feature: FeatureId,
    chains: &[Chain],
    mode: OffsetMode,
    sides: &Sides,
    value: Value,
) -> Result<Transaction, OffsettingError> {
    let curves: Vec<EntityId> = chains.iter().flat_map(Chain::curves).collect();
    let label = match mode.caps() {
        Some(_) => BOTH_TRANSACTION,
        None => TRANSACTION,
    };
    trimming::reshape(model, feature, label.to_owned(), |sketch| {
        match mode.caps() {
            Some(caps) => {
                sketch.offset_both_sides(&curves, value.millimetres, value.expression, caps)
            }
            None => sketch.offset_each(&curves, value.millimetres, value.expression, |chain| {
                sides
                    .get(&chain.curves())
                    .copied()
                    .unwrap_or_else(|| chain.default_side())
            }),
        }
        .map(|_| ())
        .map_err(|source| OffsettingError::Refused { source })
    })
}
