use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

use caditor_expression::ParameterId;
use caditor_geometry::Point2;
use caditor_kernel::{FaceOrigin, LINEAR_RESOLUTION, Solid, TessellationError};
use caditor_sketch::{Entity, EntityId, Sketch};

use crate::{
    body_appearance::BodyAppearance,
    document::{Document, Feature, FeatureId, FeatureKind},
    failure::Cause,
    recompute::{Failure, FeatureError, FixTarget},
};

const JOINED_END_TOLERANCE: f64 = LINEAR_RESOLUTION;

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PieceStandIns {
    carriers: BTreeMap<EntityId, EntityId>,
}

impl PieceStandIns {
    #[must_use]
    pub fn with(mut self, deleted: EntityId, carrier: EntityId) -> Self {
        if deleted != carrier {
            self.carry(deleted, carrier);
        }
        self
    }

    pub fn carrier_of(&self, deleted: EntityId) -> Option<EntityId> {
        self.carriers.get(&deleted).copied()
    }

    pub fn standing_for(&self, curve: EntityId) -> EntityId {
        self.carriers
            .iter()
            .filter(|(_, carrier)| **carrier == curve)
            .map(|(deleted, _)| *deleted)
            .fold(curve, EntityId::min)
    }

    pub fn iter(&self) -> impl Iterator<Item = (EntityId, EntityId)> + '_ {
        self.carriers
            .iter()
            .map(|(deleted, carrier)| (*deleted, *carrier))
    }

    pub fn is_empty(&self) -> bool {
        self.carriers.is_empty()
    }

    pub fn heap_size(&self) -> usize {
        self.carriers.len() * 2 * size_of::<EntityId>()
    }

    #[must_use]
    pub fn after_removing(&self, sketch: &Sketch, removed: &BTreeSet<EntityId>) -> Self {
        let mut kept = self.clone();
        for curve in removed {
            let stood_for = kept.standing_for(*curve);
            kept.carriers
                .retain(|deleted, carrier| carrier != curve && deleted != curve);
            let carrier = joined_curves(sketch, *curve)
                .into_iter()
                .filter(|joined| !removed.contains(joined))
                .min_by_key(|joined| kept.standing_for(*joined));
            if let Some(carrier) = carrier
                && stood_for < kept.standing_for(carrier)
            {
                kept.carry(stood_for, carrier);
            }
        }
        kept
    }

    fn carry(&mut self, deleted: EntityId, carrier: EntityId) {
        self.carriers.retain(|_, held| *held != carrier);
        self.carriers.insert(deleted, carrier);
    }
}

fn joined_curves(sketch: &Sketch, curve: EntityId) -> BTreeSet<EntityId> {
    let ends_of = |id: EntityId| -> Vec<Point2> {
        sketch
            .entity(id)
            .map(Entity::ends)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|end| sketch.point(end))
            .collect()
    };
    let own = ends_of(curve);
    if own.is_empty() || sketch.is_construction(curve) {
        return BTreeSet::new();
    }
    sketch
        .entities()
        .map(|(id, _)| id)
        .filter(|id| *id != curve && !id.is_reference() && !sketch.is_construction(*id))
        .filter(|id| {
            ends_of(*id).iter().any(|end| {
                own.iter()
                    .any(|own| own.distance(*end) <= JOINED_END_TOLERANCE)
            })
        })
        .collect()
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

    pub fn image_source(&self, body: FeatureId) -> Option<FeatureId> {
        let key = body.piece_key()?;
        match &self.body_maker(body)?.kind {
            FeatureKind::Mirror(mirror) => mirror.image_source(key),
            _ => None,
        }
    }

    pub fn pattern_copy(&self, body: FeatureId) -> Option<String> {
        let key = body.piece_key()?;
        let FeatureKind::Pattern(pattern) = &self.body_maker(body)?.kind else {
            return None;
        };
        let sketch = pattern
            .kind
            .sketch()
            .and_then(|sketch| self.feature(sketch))
            .and_then(|sketch| sketch.kind.sketch());
        pattern
            .copy_instance(key, sketch)
            .map(|instance| pattern.instance_words(instance))
    }

    pub fn piece_curve(&self, body: FeatureId) -> Option<String> {
        let key = body.piece_key()?;
        let maker = self.body_maker(body)?;
        let sketch = maker.kind.solid()?.sketch();
        let drawing = self.feature(sketch)?.kind.sketch()?;
        [key.entity()]
            .into_iter()
            .chain(maker.piece_stand_ins.carrier_of(key.entity()))
            .find(|curve| drawing.entity(*curve).is_some())
            .map(|curve| drawing.entity_label(curve))
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
        .map(|lump| {
            drawing_entities(lump, feature.id())
                .into_iter()
                .map(|entity| {
                    feature
                        .piece_stand_ins
                        .standing_for(EntityId::from_raw(entity))
                        .raw()
                })
                .collect()
        })
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

fn pieces_failure(feature: &Feature, cause: PiecesCause) -> Failure {
    Failure::from(FeatureError::new(cause).fixing(FixTarget::Feature(feature.id())))
}

fn untold(feature: &Feature) -> Failure {
    pieces_failure(
        feature,
        PiecesCause::Untold {
            feature: feature.name.clone(),
        },
    )
}

fn too_large(feature: &Feature) -> Failure {
    pieces_failure(
        feature,
        PiecesCause::TooLarge {
            feature: feature.name.clone(),
        },
    )
}

fn apart_failure(feature: &Feature, error: &TessellationError) -> Failure {
    if matches!(error, TessellationError::Cancelled(_)) {
        return Failure::Cancelled;
    }
    pieces_failure(
        feature,
        PiecesCause::Apart {
            feature: feature.name.clone(),
        },
    )
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PiecesCause {
    Untold { feature: String },
    TooLarge { feature: String },
    Apart { feature: String },
}

impl PiecesCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::Untold { feature } => format!(
                "Two separate pieces of {feature} are drawn by the same curves, so they cannot be \
                 told apart as bodies."
            ),
            Self::TooLarge { feature } => {
                format!("The pieces of {feature} cannot each become a body in a model this large.")
            }
            Self::Apart { feature } => {
                format!("The separate pieces of {feature} could not be taken apart.")
            }
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::Untold { .. } => {
                "Draw each piece with a curve of its own, or keep the pieces one body.".to_owned()
            }
            Self::TooLarge { .. } => "Keep the pieces one body.".to_owned(),
            Self::Apart { .. } => {
                "Keep the pieces one body, or change the sketch slightly.".to_owned()
            }
        }
    }
}

impl From<PiecesCause> for Cause {
    fn from(cause: PiecesCause) -> Self {
        Self::Pieces(cause)
    }
}
