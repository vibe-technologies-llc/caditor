use std::collections::{BTreeMap, BTreeSet};

use crate::{
    document::{Document, Feature, FeatureId, FeatureKind},
    edit::EditError,
    measurement::Readings,
    tally,
};

#[derive(Clone, Copy, Debug)]
pub(super) enum Tie {
    Use,
    Dependency,
}

impl Tie {
    fn uses(
        self,
        readings: &mut Readings,
        document: &Document,
        kind: &FeatureKind,
    ) -> BTreeSet<FeatureId> {
        match self {
            Self::Use => readings.features_read_by(document, kind),
            Self::Dependency => readings.dependencies_of(document, kind),
        }
    }

    fn holders(self, document: &Document, id: FeatureId) -> &[FeatureId] {
        match self {
            Self::Use => document.features.users_of(id),
            Self::Dependency => document.features.tied_to(id),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Above,
    Below,
}

#[derive(Clone, Copy, Debug)]
struct Gap {
    from: usize,
    at: usize,
}

impl Gap {
    fn side(self, position: usize) -> Side {
        let among_others = if position > self.from {
            position.saturating_sub(1)
        } else {
            position
        };

        if among_others < self.at {
            Side::Above
        } else {
            Side::Below
        }
    }
}

fn consuming_one_of(other: &Feature, bodies: &[FeatureId]) -> Option<FeatureId> {
    other
        .kind
        .consumed_bodies()
        .into_iter()
        .find(|body| bodies.contains(body))
}

impl Document {
    fn gap(&self, id: FeatureId, index: usize) -> Result<Gap, EditError> {
        let from = self.feature_index(id).ok_or(EditError::MissingFeature)?;
        let others = self.features.len().saturating_sub(1);

        Ok(Gap {
            from,
            at: index.min(others),
        })
    }

    fn first_on<T>(
        &self,
        gap: Gap,
        side: Side,
        moving: FeatureId,
        candidates: impl IntoIterator<Item = FeatureId>,
        mut holds: impl FnMut(&Feature) -> Option<T>,
    ) -> Option<(&Feature, T)> {
        let placed: BTreeMap<usize, &Feature> = candidates
            .into_iter()
            .filter(|candidate| *candidate != moving)
            .filter_map(|candidate| {
                tally::add(1);

                Some((self.feature_index(candidate)?, self.feature(candidate)?))
            })
            .filter(|(position, _)| gap.side(*position) == side)
            .collect();

        placed
            .into_values()
            .find_map(|feature| Some((feature, holds(feature)?)))
    }

    fn readers_of(&self, moving: &Feature) -> Vec<FeatureId> {
        let measured = moving
            .kind
            .measurement()
            .and_then(|measurement| measurement.parameter);

        match measured {
            Some(parameter) if !self.features.measures_nothing() => self
                .reading_users(parameter)
                .into_iter()
                .map(Feature::id)
                .collect(),
            Some(_) | None => Vec::new(),
        }
    }

    pub(super) fn check_passing(
        &self,
        id: FeatureId,
        index: usize,
        tie: Tie,
        readings: &mut Readings,
    ) -> Result<(), EditError> {
        let moving = self.feature(id).ok_or(EditError::MissingFeature)?;
        let gap = self.gap(id, index)?;
        let name = moving.name.clone();

        let uses = tie.uses(readings, self, &moving.kind);

        if let Some((needed, ())) = self.first_on(gap, Side::Below, id, uses, |_| Some(())) {
            return Err(EditError::AboveDependency {
                name,
                other: needed.name.clone(),
            });
        }

        let holders = tie
            .holders(self, id)
            .iter()
            .copied()
            .chain(self.readers_of(moving));

        if let Some((user, ())) = self.first_on(gap, Side::Above, id, holders, |other| {
            tie.uses(readings, self, &other.kind)
                .contains(&id)
                .then_some(())
        }) {
            return Err(EditError::BelowDependent {
                name,
                other: user.name.clone(),
            });
        }
        Ok(())
    }

    pub(super) fn check_move_order(
        &self,
        id: FeatureId,
        index: usize,
        readings: &mut Readings,
    ) -> Result<(), EditError> {
        self.check_passing(id, index, Tie::Dependency, readings)?;

        let moving = self.feature(id).ok_or(EditError::MissingFeature)?;
        let gap = self.gap(id, index)?;
        let name = moving.name.clone();
        let consumed = moving.kind.consumed_bodies();

        let used: Vec<FeatureId> = moving
            .kind
            .bodies_used()
            .into_iter()
            .filter(|body| !consumed.contains(body))
            .collect();
        let consumers = used
            .iter()
            .flat_map(|body| self.features.consuming(body.maker()).iter().copied());

        if let Some((consumer, body)) = self.first_on(gap, Side::Above, id, consumers, |other| {
            consuming_one_of(other, &used)
        }) {
            return Err(EditError::BelowConsumer {
                name,
                other: consumer.name.clone(),
                body: self.feature_name(body),
            });
        }

        let users = consumed
            .iter()
            .flat_map(|body| self.features.tied_to(body.maker()).iter().copied());

        if let Some((user, body)) = self.first_on(gap, Side::Below, id, users, |other| {
            other
                .kind
                .bodies_used()
                .into_iter()
                .find(|body| consumed.contains(body))
        }) {
            return Err(EditError::AboveConsumedUse {
                name,
                other: user.name.clone(),
                body: self.feature_name(body),
            });
        }
        Ok(())
    }
}
