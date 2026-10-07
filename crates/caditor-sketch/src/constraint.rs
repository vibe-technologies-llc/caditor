use caditor_expression::{Dimension, EvalError, Expression};
use caditor_geometry::Point2;

use crate::id::EntityId;

pub const MAX_LENGTH: f64 = 1e6;

#[derive(Debug, Clone, PartialEq)]
pub enum Constraint {
    Coincident(EntityId, EntityId),
    Horizontal(EntityId),
    Vertical(EntityId),
    HorizontalPoints(EntityId, EntityId),
    VerticalPoints(EntityId, EntityId),
    Parallel(EntityId, EntityId),
    Perpendicular(EntityId, EntityId),
    Tangent(EntityId, EntityId),
    Equal(EntityId, EntityId),
    Midpoint {
        point: EntityId,
        curve: EntityId,
    },
    Concentric(EntityId, EntityId),
    Collinear(EntityId, EntityId),
    Symmetric {
        first: EntityId,
        second: EntityId,
        about: EntityId,
    },
    Fix {
        point: EntityId,
        at: Point2,
    },
    Distance {
        from: EntityId,
        to: EntityId,
        value: Expression,
    },
    HorizontalDistance {
        from: EntityId,
        to: EntityId,
        value: Expression,
    },
    VerticalDistance {
        from: EntityId,
        to: EntityId,
        value: Expression,
    },
    Angle {
        from: EntityId,
        to: EntityId,
        reversed: bool,
        value: Expression,
    },
    Radius {
        entity: EntityId,
        value: Expression,
    },
    Diameter {
        entity: EntityId,
        value: Expression,
    },
}

impl Constraint {
    pub fn heap_size(&self) -> usize {
        self.dimension().map_or(0, Expression::heap_size)
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Coincident(..) => "Coincident",
            Self::Horizontal(_) | Self::HorizontalPoints(..) => "Horizontal",
            Self::Vertical(_) | Self::VerticalPoints(..) => "Vertical",
            Self::Parallel(..) => "Parallel",
            Self::Perpendicular(..) => "Perpendicular",
            Self::Tangent(..) => "Tangent",
            Self::Equal(..) => "Equal",
            Self::Midpoint { .. } => "Midpoint",
            Self::Concentric(..) => "Concentric",
            Self::Collinear(..) => "Collinear",
            Self::Symmetric { .. } => "Symmetric",
            Self::Fix { .. } => "Fix",
            Self::Distance { .. } => "Distance",
            Self::HorizontalDistance { .. } => "Horizontal distance",
            Self::VerticalDistance { .. } => "Vertical distance",
            Self::Angle { .. } => "Angle",
            Self::Radius { .. } => "Radius",
            Self::Diameter { .. } => "Diameter",
        }
    }

    pub fn entities(&self) -> Vec<EntityId> {
        match *self {
            Self::Horizontal(entity)
            | Self::Vertical(entity)
            | Self::Fix { point: entity, .. }
            | Self::Radius { entity, .. }
            | Self::Diameter { entity, .. } => vec![entity],
            Self::Coincident(a, b)
            | Self::HorizontalPoints(a, b)
            | Self::VerticalPoints(a, b)
            | Self::Parallel(a, b)
            | Self::Perpendicular(a, b)
            | Self::Tangent(a, b)
            | Self::Equal(a, b)
            | Self::Midpoint { point: a, curve: b }
            | Self::Concentric(a, b)
            | Self::Collinear(a, b)
            | Self::Distance { from: a, to: b, .. }
            | Self::HorizontalDistance { from: a, to: b, .. }
            | Self::VerticalDistance { from: a, to: b, .. }
            | Self::Angle { from: a, to: b, .. } => vec![a, b],
            Self::Symmetric {
                first,
                second,
                about,
            } => vec![first, second, about],
        }
    }

    pub fn dimension(&self) -> Option<&Expression> {
        match self {
            Self::Distance { value, .. }
            | Self::HorizontalDistance { value, .. }
            | Self::VerticalDistance { value, .. }
            | Self::Angle { value, .. }
            | Self::Radius { value, .. }
            | Self::Diameter { value, .. } => Some(value),
            Self::Coincident(..)
            | Self::Horizontal(_)
            | Self::Vertical(_)
            | Self::HorizontalPoints(..)
            | Self::VerticalPoints(..)
            | Self::Parallel(..)
            | Self::Perpendicular(..)
            | Self::Tangent(..)
            | Self::Equal(..)
            | Self::Midpoint { .. }
            | Self::Concentric(..)
            | Self::Collinear(..)
            | Self::Symmetric { .. }
            | Self::Fix { .. } => None,
        }
    }

    pub fn dimension_kind(&self) -> Option<Dimension> {
        match self {
            Self::Angle { .. } => Some(Dimension::ANGLE),
            _ if self.dimension().is_some() => Some(Dimension::LENGTH),
            _ => None,
        }
    }

    pub fn check_dimension_value(&self, value: f64) -> Result<(), DimensionError> {
        match self {
            _ if !value.is_finite() => Err(DimensionError::NotFinite),
            Self::Distance { .. }
            | Self::HorizontalDistance { .. }
            | Self::VerticalDistance { .. }
                if value < 0.0 =>
            {
                Err(DimensionError::Negative)
            }
            Self::Radius { .. } if value <= 0.0 => Err(DimensionError::NotPositive),
            Self::Diameter { .. } if value <= 0.0 => Err(DimensionError::DiameterNotPositive),
            Self::Angle { .. } => Ok(()),
            _ if self.dimension().is_some() && value > MAX_LENGTH => Err(DimensionError::TooLong),
            _ => Ok(()),
        }
    }

    pub(crate) fn dimension_mut(&mut self) -> Option<&mut Expression> {
        match self {
            Self::Distance { value, .. }
            | Self::HorizontalDistance { value, .. }
            | Self::VerticalDistance { value, .. }
            | Self::Angle { value, .. }
            | Self::Radius { value, .. }
            | Self::Diameter { value, .. } => Some(value),
            Self::Coincident(..)
            | Self::Horizontal(_)
            | Self::Vertical(_)
            | Self::HorizontalPoints(..)
            | Self::VerticalPoints(..)
            | Self::Parallel(..)
            | Self::Perpendicular(..)
            | Self::Tangent(..)
            | Self::Equal(..)
            | Self::Midpoint { .. }
            | Self::Concentric(..)
            | Self::Collinear(..)
            | Self::Symmetric { .. }
            | Self::Fix { .. } => None,
        }
    }

    pub(crate) fn references(&self, id: EntityId) -> bool {
        self.entities().contains(&id)
    }

    pub(crate) fn with_entity_replaced(&self, from: EntityId, to: EntityId) -> Self {
        let swap = |entity: EntityId| if entity == from { to } else { entity };
        let mut replaced = self.clone();
        match &mut replaced {
            Self::Horizontal(entity)
            | Self::Vertical(entity)
            | Self::Fix { point: entity, .. }
            | Self::Radius { entity, .. }
            | Self::Diameter { entity, .. } => *entity = swap(*entity),
            Self::Coincident(a, b)
            | Self::HorizontalPoints(a, b)
            | Self::VerticalPoints(a, b)
            | Self::Parallel(a, b)
            | Self::Perpendicular(a, b)
            | Self::Tangent(a, b)
            | Self::Equal(a, b)
            | Self::Midpoint { point: a, curve: b }
            | Self::Concentric(a, b)
            | Self::Collinear(a, b)
            | Self::Distance { from: a, to: b, .. }
            | Self::HorizontalDistance { from: a, to: b, .. }
            | Self::VerticalDistance { from: a, to: b, .. }
            | Self::Angle { from: a, to: b, .. } => {
                *a = swap(*a);
                *b = swap(*b);
            }
            Self::Symmetric {
                first,
                second,
                about,
            } => {
                *first = swap(*first);
                *second = swap(*second);
                *about = swap(*about);
            }
        }
        replaced
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
    #[error("a diameter must be greater than zero")]
    DiameterNotPositive,
    #[error("the value is too large to use")]
    NotFinite,
    #[error("a length cannot be more than {} m", MAX_LENGTH / 1_000.0)]
    TooLong,
}
