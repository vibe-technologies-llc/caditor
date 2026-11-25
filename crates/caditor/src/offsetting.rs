use std::collections::BTreeMap;

use caditor_document::{FeatureId, Transaction};
use caditor_geometry::Point2;
use caditor_sketch::{Caps, Chain, Entity, EntityId, Faceting, OffsetError, Side, Sketch};
use thiserror::Error;

use crate::{
    drawing::Preview,
    editing::Tool,
    feature_tree::count,
    model::{Model, Waker},
    modifying::{
        HighlightNeeded, Hint, Outcome, Prompt, StepError, Value, ValueError, ValueField,
        length_text,
    },
    offset_preview::{OffsetPreview, OffsetRequest, Offsets},
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
pub const WORKING: &str = "Working out the offset of";
pub const BOTH_TRANSACTION: &str = "Offset curves to both sides";
const KEYS: &str = "Type a distance, negative for the other side   Esc: choose another curve";
const BOTH_KEYS: &str = "Type the distance on each side   Esc: choose another curve";
const CHOOSE_KEYS: &str = "Esc: back to Select";
pub const FIELD: ValueField = ValueField {
    label: "Offset by",
    placeholder: "distance",
    action: "offset",
};

#[derive(Clone, Debug, PartialEq, Error)]
pub enum OffsettingError {
    #[error(transparent)]
    SketchGone(#[from] SketchGone),
    #[error("{}: {source}.", Tool::Offset.label())]
    Refused { source: OffsetError },
    #[error(transparent)]
    Value(#[from] ValueError),
}

#[derive(Debug, Default)]
pub struct Offsetting {
    mode: OffsetMode,
    chains: Option<Result<Vec<Chain>, OffsetError>>,
    pointer: Option<Point2>,
    under: Option<EntityId>,
    highlight: Option<EntityId>,
    typed: Option<f64>,
    last: Option<String>,
    request: Option<OffsetRequest>,
    preview: OffsetPreview,
}

impl Offsetting {
    pub fn new(last: Option<String>) -> Self {
        Self {
            last,
            ..Self::default()
        }
    }

    pub fn last(&self) -> Option<&str> {
        self.last.as_deref()
    }

    pub fn sync(&mut self, sketch: &Sketch, selected: &[EntityId], mode: OffsetMode) {
        self.mode = mode;
        self.chains = (!selected.is_empty()).then(|| sketch.offset_chains(selected));
        self.highlight = self
            .highlight
            .filter(|curve| is_offsettable(sketch, *curve));
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

    pub fn refresh(&mut self, faceting: Faceting, wake: impl FnOnce() -> Waker) {
        self.request = self
            .chains()
            .filter(|_| self.pointer.is_some() || self.typed.is_some())
            .map(|chains| OffsetRequest {
                chains: chains.to_vec(),
                caps: self.mode.caps(),
                pointer: self.pointer,
                typed: self.typed,
                faceting,
            });
        self.preview.refresh(self.request.as_ref(), wake);
    }

    #[cfg(test)]
    pub fn preview_worker(&mut self) -> &mut OffsetPreview {
        &mut self.preview
    }

    fn reused(&self, text: &str) -> String {
        match self.last.as_deref() {
            Some(last) if text.trim().is_empty() => last.to_owned(),
            _ => text.trim().to_owned(),
        }
    }

    pub fn show_text(&mut self, model: &Model, text: Option<&str>) {
        self.typed = text
            .map(|text| self.reused(text))
            .and_then(|text| Value::typed(model, &text).ok())
            .map(|value| value.millimetres);
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

    fn placement(&self) -> Option<Placement> {
        Placement::of(self.chains()?, self.pointer, self.typed)
    }

    fn previewed(&self) -> Option<&Result<Offsets, OffsetError>> {
        self.preview.shown_for(self.request.as_ref())
    }

    pub fn preview(&self) -> Preview {
        let mut preview = Preview::default();

        if let Some(Ok(offsets)) = self.previewed() {
            preview.curves.clone_from(&offsets.curves);
        }
        preview
    }

    pub fn highlighted_entities(&self) -> Vec<EntityId> {
        self.highlight.or(self.under).into_iter().collect()
    }

    pub fn label(&self, sketch: &Sketch, unit: Units) -> Option<String> {
        if let Some(curve) = self.highlight.or(self.under) {
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
        match self.previewed() {
            Some(Ok(offsets)) if free => Some(format!(
                "Offset {subject} by {}{way} {}",
                length_text(unit, offsets.distance),
                free_spline(chains)
            )),
            Some(Ok(offsets)) => Some(format!(
                "Offset {subject} by {}{way}",
                length_text(unit, offsets.distance)
            )),
            Some(Err(error)) => Some(capitalized(&error.to_string())),
            None if self.request.is_some() && self.preview.is_working() => {
                Some(format!("{WORKING} {subject}…"))
            }
            None => Some(format!("Offset {subject}")),
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
                hint: match self.last.as_deref() {
                    Some(last) => Hint::Text(format!(
                        "Type the distance on each side   Enter: offset by {last}, as last time   \
                         Esc: choose another curve"
                    )),
                    None => Hint::Keys(BOTH_KEYS),
                },
            }
        } else if self.chains().is_some() {
            Prompt {
                text: PROMPT,
                hint: match self.last.as_deref() {
                    Some(last) => Hint::Text(format!(
                        "Type a distance, negative for the other side   Enter: offset by {last}, \
                         as last time   Esc: choose another curve"
                    )),
                    None => Hint::Keys(KEYS),
                },
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
            return select_chain(sketch, curve);
        }
        match &self.chains {
            Some(Ok(chains)) => {
                let Some(placement) = self.placement() else {
                    return Outcome::Nothing;
                };
                let Some(value) = Value::pointed(model, placement.distance) else {
                    return Outcome::Nothing;
                };
                let text = length_text(model.units(), value.millimetres);

                let result = commit(model, feature, chains, self.mode, &placement.sides, value);

                if result.is_ok() {
                    self.last = Some(text);
                }
                result.into()
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

    pub fn finish(&mut self, model: &Model, feature: FeatureId, sketch: &Sketch) -> Outcome {
        if let Some(curve) = self.highlight.take() {
            return select_chain(sketch, curve);
        }
        if self.can_pull()
            && let Some(last) = self.last.clone()
        {
            return self
                .enter_text(model, feature, &last)
                .unwrap_or_else(|error| Outcome::Refused(error.into()));
        }
        self.click(model, feature, sketch)
    }

    pub fn enter_text(
        &mut self,
        model: &Model,
        feature: FeatureId,
        text: &str,
    ) -> Result<Outcome, OffsettingError> {
        let text = self.reused(text);
        let value = Value::typed(model, &text)?;
        let outcome = self.enter_value(model, feature, value)?;

        self.last = Some(text);
        Ok(outcome)
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
            .map(|chain| (chain.curves(), side_toward(chain, self.pointer, flipped)))
            .collect();
        commit(model, feature, chains, self.mode, &sides, value).map(Outcome::Apply)
    }

    pub fn steps_targets(&self) -> bool {
        !self.can_pull()
    }

    pub fn steppable(&self, sketch: &Sketch) -> Result<(), StepError> {
        if sketch.entities().any(|(id, _)| is_offsettable(sketch, id)) {
            Ok(())
        } else {
            Err(StepError::NothingToPick)
        }
    }

    pub fn step(&mut self, sketch: &Sketch, step: isize) {
        let curves: Vec<EntityId> = sketch
            .entities()
            .filter(|(id, _)| is_offsettable(sketch, *id))
            .map(|(id, _)| id)
            .collect();
        let total = curves.len() as isize;

        if total == 0 {
            return;
        }

        let current = self
            .highlight
            .or(self.under)
            .and_then(|aim| curves.iter().position(|curve| *curve == aim));
        let next = match current {
            Some(index) => (index as isize + step).rem_euclid(total),
            None if step < 0 => total - 1,
            None => 0,
        };

        self.highlight = usize::try_from(next)
            .ok()
            .and_then(|next| curves.get(next).copied());
    }

    pub fn highlight_needed(&self) -> Result<(), HighlightNeeded> {
        match self.highlight {
            Some(_) => Ok(()),
            None => Err(HighlightNeeded::Curve),
        }
    }

    pub fn activate(&mut self, sketch: &Sketch) -> Outcome {
        match self.highlight.take() {
            Some(curve) => select_chain(sketch, curve),
            None => Outcome::Nothing,
        }
    }

    pub fn clear_highlight(&mut self) {
        self.highlight = None;
    }

    pub fn can_back_out(&self) -> bool {
        self.highlight.is_some()
    }

    pub fn back_out(&mut self) {
        self.highlight = None;
    }
}

fn is_offsettable(sketch: &Sketch, id: EntityId) -> bool {
    !id.is_reference() && !matches!(sketch.entity(id), None | Some(Entity::Point(_)))
}

fn select_chain(sketch: &Sketch, curve: EntityId) -> Outcome {
    let chain = sketch.offset_chain_through(curve);

    match sketch.offset_chains(&chain) {
        Ok(_) => Outcome::Select(chain),
        Err(source) => Outcome::Refused(OffsettingError::Refused { source }.into()),
    }
}

type Sides = BTreeMap<Vec<EntityId>, Side>;

fn side_toward(chain: &Chain, pointer: Option<Point2>, flipped: bool) -> Side {
    let side = pointer.map_or(chain.default_side(), |pointer| chain.side_of(pointer));
    if flipped { side.other() } else { side }
}

pub struct Placement {
    sides: Sides,
    pub distance: f64,
}

impl Placement {
    pub fn of(chains: &[Chain], pointer: Option<Point2>, typed: Option<f64>) -> Option<Self> {
        let (flipped, distance) = match typed {
            Some(value) => (value < 0.0, value.abs()),
            None => {
                let pointer = pointer?;
                let nearest = chains
                    .iter()
                    .map(|chain| chain.distance_to(pointer))
                    .fold(f64::INFINITY, f64::min);
                (false, nearest)
            }
        };
        Some(Self {
            sides: chains
                .iter()
                .map(|chain| (chain.curves(), side_toward(chain, pointer, flipped)))
                .collect(),
            distance,
        })
    }

    pub fn side_of(&self, chain: &Chain) -> Side {
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
