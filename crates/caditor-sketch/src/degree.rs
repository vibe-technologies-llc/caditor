use crate::{
    constraint::Constraint,
    entity::{Entity, MAX_DEGREE, MIN_DEGREE, SplineKind},
    id::EntityId,
    sketch::{Sketch, SketchError},
};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DegreeError {
    #[error("{label} is not a spline")]
    NotASpline { entity: EntityId, label: String },
    #[error(
        "{label} passes through its points, so it has no degree to choose; only splines drawn from \
         control points have"
    )]
    FitSpline { entity: EntityId, label: String },
    #[error("{label} is a conic, which is always of degree two")]
    Conic { entity: EntityId, label: String },
    #[error("{label} is closed, and a closed spline is always of degree three")]
    ClosedSpline { entity: EntityId, label: String },
    #[error("{label} follows the geometry it was projected from, so it cannot be changed")]
    Projected { entity: EntityId, label: String },
    #[error("a spline's degree is a whole number from {MIN_DEGREE} to {MAX_DEGREE}")]
    OutOfRange { degree: usize },
    #[error("{label} has {points} control points, so its degree can be at most {most}")]
    TooFewPoints {
        entity: EntityId,
        label: String,
        points: usize,
        most: usize,
    },
    #[error("{label} is already of degree {degree}")]
    Unchanged {
        entity: EntityId,
        label: String,
        degree: usize,
    },
    #[error(transparent)]
    Edit(SketchError),
}

impl Sketch {
    pub fn spline_degree(&self, curve: EntityId) -> Option<usize> {
        let Some(Entity::Spline { points, kind }) = self.entity(curve) else {
            return None;
        };
        kind.degree_of(points.len())
    }

    pub fn check_spline_degree(&self, curve: EntityId, degree: usize) -> Result<(), DegreeError> {
        self.degree_kind(curve, degree).map(|_| ())
    }

    pub fn set_spline_degree(&mut self, curve: EntityId, degree: usize) -> Result<(), DegreeError> {
        let kind = self.degree_kind(curve, degree)?;
        let Some(Entity::Spline { points, .. }) = self.entity(curve).cloned() else {
            return Err(DegreeError::NotASpline {
                entity: curve,
                label: self.entity_label(curve),
            });
        };
        let mut working = self.clone();
        working
            .restructure(curve, Entity::Spline { points, kind }, &[], |constraint| {
                degree > MIN_DEGREE || !matches!(constraint, Constraint::Curvature(..))
            })
            .map_err(DegreeError::Edit)?;
        *self = working;
        Ok(())
    }

    fn degree_kind(&self, curve: EntityId, degree: usize) -> Result<SplineKind, DegreeError> {
        let label = || self.entity_label(curve);
        let Some(Entity::Spline { points, kind }) = self.entity(curve) else {
            return Err(DegreeError::NotASpline {
                entity: curve,
                label: label(),
            });
        };
        if self.is_projected(curve) {
            return Err(DegreeError::Projected {
                entity: curve,
                label: label(),
            });
        }
        match kind {
            SplineKind::Fit { .. } => {
                return Err(DegreeError::FitSpline {
                    entity: curve,
                    label: label(),
                });
            }
            SplineKind::Conic { .. } => {
                return Err(DegreeError::Conic {
                    entity: curve,
                    label: label(),
                });
            }
            SplineKind::Control { closed: true } | SplineKind::Knotted { closed: true, .. } => {
                return Err(DegreeError::ClosedSpline {
                    entity: curve,
                    label: label(),
                });
            }
            SplineKind::Control { closed: false }
            | SplineKind::Piece { .. }
            | SplineKind::Knotted { closed: false, .. }
            | SplineKind::Graded { .. } => {}
        }
        if !SplineKind::degree_is_valid(degree) {
            return Err(DegreeError::OutOfRange { degree });
        }
        let most = points.len().saturating_sub(1).min(MAX_DEGREE);
        if degree > most {
            return Err(DegreeError::TooFewPoints {
                entity: curve,
                label: label(),
                points: points.len(),
                most,
            });
        }
        if kind.degree_of(points.len()) == Some(degree) {
            return Err(DegreeError::Unchanged {
                entity: curve,
                label: label(),
                degree,
            });
        }
        SplineKind::graded(degree, points.len()).ok_or(DegreeError::OutOfRange { degree })
    }
}

#[cfg(test)]
mod tests;
