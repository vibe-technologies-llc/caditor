use caditor_expression::{Dimension, EvalError, Expression};

use crate::id::EntityId;

#[derive(Debug, Clone, PartialEq)]
pub enum Constraint {
    Coincident(EntityId, EntityId),
    Horizontal(EntityId),
    Vertical(EntityId),
    Parallel(EntityId, EntityId),
    Perpendicular(EntityId, EntityId),
    Tangent(EntityId, EntityId),
    Equal(EntityId, EntityId),
    Distance {
        from: EntityId,
        to: EntityId,
        value: Expression,
    },
    Angle {
        from: EntityId,
        to: EntityId,
        value: Expression,
    },
    Radius {
        entity: EntityId,
        value: Expression,
    },
}

impl Constraint {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Coincident(..) => "Coincident",
            Self::Horizontal(_) => "Horizontal",
            Self::Vertical(_) => "Vertical",
            Self::Parallel(..) => "Parallel",
            Self::Perpendicular(..) => "Perpendicular",
            Self::Tangent(..) => "Tangent",
            Self::Equal(..) => "Equal",
            Self::Distance { .. } => "Distance",
            Self::Angle { .. } => "Angle",
            Self::Radius { .. } => "Radius",
        }
    }

    pub fn entities(&self) -> Vec<EntityId> {
        match *self {
            Self::Horizontal(entity) | Self::Vertical(entity) | Self::Radius { entity, .. } => {
                vec![entity]
            }
            Self::Coincident(a, b)
            | Self::Parallel(a, b)
            | Self::Perpendicular(a, b)
            | Self::Tangent(a, b)
            | Self::Equal(a, b)
            | Self::Distance { from: a, to: b, .. }
            | Self::Angle { from: a, to: b, .. } => vec![a, b],
        }
    }

    pub fn dimension(&self) -> Option<&Expression> {
        match self {
            Self::Distance { value, .. }
            | Self::Angle { value, .. }
            | Self::Radius { value, .. } => Some(value),
            Self::Coincident(..)
            | Self::Horizontal(_)
            | Self::Vertical(_)
            | Self::Parallel(..)
            | Self::Perpendicular(..)
            | Self::Tangent(..)
            | Self::Equal(..) => None,
        }
    }

    pub fn dimension_kind(&self) -> Option<Dimension> {
        match self {
            Self::Distance { .. } | Self::Radius { .. } => Some(Dimension::LENGTH),
            Self::Angle { .. } => Some(Dimension::ANGLE),
            Self::Coincident(..)
            | Self::Horizontal(_)
            | Self::Vertical(_)
            | Self::Parallel(..)
            | Self::Perpendicular(..)
            | Self::Tangent(..)
            | Self::Equal(..) => None,
        }
    }

    pub fn check_dimension_value(&self, value: f64) -> Result<(), DimensionError> {
        match self {
            Self::Distance { .. } if value < 0.0 => Err(DimensionError::Negative),
            Self::Radius { .. } if value <= 0.0 => Err(DimensionError::NotPositive),
            _ if !value.is_finite() => Err(DimensionError::NotFinite),
            _ => Ok(()),
        }
    }

    pub(crate) fn dimension_mut(&mut self) -> Option<&mut Expression> {
        match self {
            Self::Distance { value, .. }
            | Self::Angle { value, .. }
            | Self::Radius { value, .. } => Some(value),
            Self::Coincident(..)
            | Self::Horizontal(_)
            | Self::Vertical(_)
            | Self::Parallel(..)
            | Self::Perpendicular(..)
            | Self::Tangent(..)
            | Self::Equal(..) => None,
        }
    }

    pub(crate) fn references(&self, id: EntityId) -> bool {
        self.entities().contains(&id)
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DimensionError {
    #[error(transparent)]
    Evaluation(#[from] EvalError),
    #[error("a distance cannot be negative")]
    Negative,
    #[error("a radius must be greater than zero")]
    NotPositive,
    #[error("the value is too large to use")]
    NotFinite,
}
