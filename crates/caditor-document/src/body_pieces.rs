use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

use caditor_expression::ParameterId;
use caditor_kernel::{FaceOrigin, Solid, TessellationError};
use caditor_sketch::EntityId;

use crate::{
    body_appearance::BodyAppearance,
    document::{Document, Feature, FeatureId},
    recompute::{Failure, FeatureError, FixTarget},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Pieces {
    #[default]
    OneBody,
    BodyEach,
}

impl Pieces {
    pub const ALL: [Self; 2] = [Self::OneBody, Self::BodyEach];

    pub fn words(self) -> &'static str {
        match self {
            Self::OneBody => "One body",
            Self::BodyEach => "A body each",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PieceKey(u32);

impl PieceKey {
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    pub fn of_entity(entity: EntityId) -> Option<Self> {
        u32::try_from(entity.raw()).ok().map(Self)
    }

    pub fn entity(self) -> EntityId {
        EntityId::from_raw(u64::from(self.0))
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PieceLooks {
    looks: BTreeMap<PieceKey, BodyAppearance>,
    hidden: BTreeSet<PieceKey>,
}

impl PieceLooks {
    #[must_use]
    pub fn with_appearance(mut self, key: PieceKey, appearance: BodyAppearance) -> Self {
        self.looks.insert(key, appearance);
        self
    }

    #[must_use]
    pub fn with_hidden(mut self, key: PieceKey) -> Self {
        self.hidden.insert(key);
        self
    }

    pub fn appearance(&self, key: PieceKey) -> Option<&BodyAppearance> {
        self.looks.get(&key)
    }

    pub fn is_hidden(&self, key: PieceKey) -> bool {
        self.hidden.contains(&key)
    }

    pub fn appearances(&self) -> impl Iterator<Item = (PieceKey, &BodyAppearance)> {
        self.looks
            .iter()
            .map(|(key, appearance)| (*key, appearance))
    }

    pub fn hidden(&self) -> impl Iterator<Item = PieceKey> + '_ {
        self.hidden.iter().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.looks.is_empty() && self.hidden.is_empty()
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.looks
            .values()
            .flat_map(BodyAppearance::parameters)
            .collect()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.looks
            .values()
            .any(|appearance| appearance.uses_parameter(parameter))
    }

    pub fn heap_size(&self) -> usize {
        self.looks
            .values()
            .map(|appearance| size_of::<(PieceKey, BodyAppearance)>() + appearance.heap_size())
            .sum::<usize>()
            + self.hidden.len() * size_of::<PieceKey>()
    }

    #[must_use]
    pub(crate) fn settled(self, inherited: &BodyAppearance) -> Self {
        Self {
            looks: self
                .looks
                .into_iter()
                .map(|(key, appearance)| (key, appearance.settled()))
                .filter(|(_, appearance)| appearance != inherited)
                .collect(),
            hidden: self.hidden,
        }
    }

    pub(crate) fn appearances_mut(&mut self) -> impl Iterator<Item = &mut BodyAppearance> {
        self.looks.values_mut()
    }

    pub(crate) fn set_appearance(
        &mut self,
        key: PieceKey,
        appearance: BodyAppearance,
        inherited: &BodyAppearance,
    ) -> BodyAppearance {
        let previous = self.looks.remove(&key);
        if appearance != *inherited {
            self.looks.insert(key, appearance);
        }
        previous.unwrap_or_else(|| inherited.clone())
    }

    pub(crate) fn set_hidden(&mut self, key: PieceKey, hidden: bool) -> bool {
        if hidden {
            !self.hidden.insert(key)
        } else {
            self.hidden.remove(&key)
        }
    }
}

impl Document {
    pub fn body_maker(&self, body: FeatureId) -> Option<&Feature> {
        self.feature(body.maker())
    }

    pub fn body_hidden(&self, body: FeatureId) -> bool {
        let Some(maker) = self.body_maker(body) else {
            return false;
        };
        maker.hidden
            || body
                .piece_key()
                .is_some_and(|key| maker.piece_looks.is_hidden(key))
    }

    pub fn body_label(&self, body: FeatureId) -> Option<String> {
        if body.is_piece() {
            self.body_name(body).map(Cow::into_owned)
        } else {
            self.feature(body).map(|feature| feature.name.clone())
        }
    }

    pub fn piece_curve(&self, body: FeatureId) -> Option<String> {
        let key = body.piece_key()?;
        let sketch = self.body_maker(body)?.kind.solid()?.sketch();
        let drawing = self.feature(sketch)?.kind.sketch()?;
        drawing
            .entity(key.entity())
            .is_some()
            .then(|| drawing.entity_label(key.entity()))
    }
}

pub(crate) fn separated(
    feature: &Feature,
    solid: &Solid,
) -> Result<Vec<(FeatureId, Solid)>, Failure> {
    let lumps = solid
        .lumps()
        .map_err(|error| apart_failure(feature, &error))?;
    let drawn: Vec<BTreeSet<u64>> = lumps
        .iter()
        .map(|lump| drawing_entities(lump, feature.id()))
        .collect();
    let mut pieces = Vec::with_capacity(lumps.len());
    for (index, lump) in lumps.into_iter().enumerate() {
        let entity = own_curve(index, &drawn).ok_or_else(|| untold(feature))?;
        let body = PieceKey::of_entity(EntityId::from_raw(entity))
            .and_then(|key| feature.id().piece(key))
            .ok_or_else(|| too_large(feature))?;
        pieces.push((body, lump));
    }
    pieces.sort_by_key(|(body, _)| *body);
    Ok(pieces)
}

fn drawing_entities(lump: &Solid, feature: FeatureId) -> BTreeSet<u64> {
    lump.faces()
        .filter_map(|(_, face)| match face.origin()? {
            FaceOrigin::Side {
                feature: maker,
                entity,
            } if maker == feature.raw() => Some(entity),
            _ => None,
        })
        .collect()
}

fn own_curve(index: usize, drawn: &[BTreeSet<u64>]) -> Option<u64> {
    let entities = drawn.get(index)?;
    let others = || {
        drawn
            .iter()
            .enumerate()
            .filter(move |(other, _)| *other != index)
            .map(|(_, entities)| entities)
    };
    let lowest = *entities.first()?;
    if others().all(|other| other.first() != Some(&lowest)) {
        return Some(lowest);
    }
    entities
        .iter()
        .copied()
        .find(|entity| others().all(|other| !other.contains(entity)))
}

fn untold(feature: &Feature) -> Failure {
    Failure::from(FeatureError {
        reason: format!(
            "Two separate pieces of {} are drawn by the same curves, so they cannot be told apart \
             as bodies.",
            feature.name
        ),
        remedy: "Draw each piece with a curve of its own, or keep the pieces one body.".to_owned(),
        fix: Some(FixTarget::Feature(feature.id())),
        constraints: Vec::new(),
        place: None,
    })
}

fn too_large(feature: &Feature) -> Failure {
    Failure::from(FeatureError {
        reason: format!(
            "The pieces of {} cannot each become a body in a model this large.",
            feature.name
        ),
        remedy: "Keep the pieces one body.".to_owned(),
        fix: Some(FixTarget::Feature(feature.id())),
        constraints: Vec::new(),
        place: None,
    })
}

fn apart_failure(feature: &Feature, error: &TessellationError) -> Failure {
    if matches!(error, TessellationError::Cancelled(_)) {
        return Failure::Cancelled;
    }
    Failure::from(FeatureError {
        reason: format!(
            "The separate pieces of {} could not be taken apart.",
            feature.name
        ),
        remedy: "Keep the pieces one body, or change the sketch slightly.".to_owned(),
        fix: Some(FixTarget::Feature(feature.id())),
        constraints: Vec::new(),
        place: None,
    })
}
