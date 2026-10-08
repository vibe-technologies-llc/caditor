use crate::{
    constraint::Constraint,
    entity::Entity,
    id::EntityId,
    sketch::{Sketch, SketchError},
    split::SplitError,
    trim::TrimError,
};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BreakError {
    #[error("select the lines and arcs to break")]
    NothingSelected,
    #[error("none of the selected curves crosses another curve between its ends")]
    NothingToBreak,
    #[error("the curve to break no longer exists")]
    NoSuchCurve(EntityId),
    #[error("{label} is not a curve, so it cannot be broken")]
    NotACurve { entity: EntityId, label: String },
    #[error("{label} cannot be broken; only lines and arcs can")]
    NotLineOrArc { entity: EntityId, label: String },
    #[error("{label} has no length to break")]
    NoLength { entity: EntityId, label: String },
    #[error("{label} is built into the sketch, so it cannot be broken")]
    Reference { entity: EntityId, label: String },
    #[error("{label} follows the geometry it was projected from, so it cannot be broken")]
    Projected { entity: EntityId, label: String },
    #[error("{label} crosses no other curve between its ends, so there is nothing to break")]
    NoCrossing { entity: EntityId, label: String },
    #[error(transparent)]
    Split(SplitError),
    #[error(transparent)]
    Edit(SketchError),
}

impl BreakError {
    fn is_unbreakable(&self) -> bool {
        matches!(
            self,
            Self::NoSuchCurve(_)
                | Self::NotACurve { .. }
                | Self::NotLineOrArc { .. }
                | Self::NoLength { .. }
                | Self::Reference { .. }
                | Self::Projected { .. }
                | Self::NoCrossing { .. }
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Broken {
    pub curves: usize,
    pub pieces: Vec<EntityId>,
}

impl Sketch {
    pub fn break_curve(&mut self, curve: EntityId) -> Result<Vec<EntityId>, BreakError> {
        let mut working = self.clone();
        let pieces = working.break_at_crossings(curve)?;
        if pieces.is_empty() {
            return Err(BreakError::NoCrossing {
                entity: curve,
                label: self.entity_label(curve),
            });
        }
        *self = working;
        Ok(pieces)
    }

    pub fn break_curves(&mut self, curves: &[EntityId]) -> Result<Broken, BreakError> {
        let [only] = curves else {
            return self.break_several(curves);
        };
        let pieces = self.break_curve(*only)?;
        Ok(Broken { curves: 1, pieces })
    }

    fn break_several(&mut self, curves: &[EntityId]) -> Result<Broken, BreakError> {
        if curves.is_empty() {
            return Err(BreakError::NothingSelected);
        }
        let mut working = self.clone();
        let mut broken = Broken::default();
        for curve in curves {
            match working.break_at_crossings(*curve) {
                Ok(pieces) if pieces.is_empty() => {}
                Ok(pieces) => {
                    broken.curves += 1;
                    broken.pieces.extend(pieces);
                }
                Err(error) if error.is_unbreakable() => {}
                Err(error) => return Err(error),
            }
        }
        if broken.curves == 0 {
            return Err(BreakError::NothingToBreak);
        }
        *self = working;
        Ok(broken)
    }

    fn break_at_crossings(&mut self, curve: EntityId) -> Result<Vec<EntityId>, BreakError> {
        let label = || self.entity_label(curve);
        let cuts = match self.open_cuts(curve) {
            Ok(Some(cuts)) => cuts,
            Ok(None) => {
                return Err(BreakError::NotLineOrArc {
                    entity: curve,
                    label: label(),
                });
            }
            Err(error) => return Err(self.unbreakable(curve, error)),
        };
        let mut pieces = Vec::new();
        let mut remaining = curve;
        for cut in &cuts {
            let existing = cut
                .point
                .filter(|point| matches!(self.entity(*point), Some(Entity::Point(_))))
                .filter(|point| self.check_split(remaining, *point).is_ok());
            let point = match existing {
                Some(point) => point,
                None => {
                    let point = self.add_point(cut.position);
                    self.add_constraint(Constraint::Coincident(point, cut.cutter))
                        .map_err(BreakError::Edit)?;
                    point
                }
            };
            remaining = self.split_at(remaining, point).map_err(BreakError::Split)?;
            pieces.push(remaining);
        }
        Ok(pieces)
    }

    fn unbreakable(&self, curve: EntityId, error: TrimError) -> BreakError {
        let label = self.entity_label(curve);
        match error {
            TrimError::NoSuchCurve(entity) => BreakError::NoSuchCurve(entity),
            TrimError::NotACurve { entity, .. } => BreakError::NotACurve { entity, label },
            TrimError::Spline { entity, .. } => BreakError::NotLineOrArc { entity, label },
            TrimError::NoLength { entity, .. } => BreakError::NoLength { entity, label },
            TrimError::Reference { entity, .. } => BreakError::Reference { entity, label },
            TrimError::Projected { entity, .. } => BreakError::Projected { entity, label },
            TrimError::Edit(error) => BreakError::Edit(error),
        }
    }
}

#[cfg(test)]
mod tests;
