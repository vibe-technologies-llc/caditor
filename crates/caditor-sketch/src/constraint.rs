use caditor_expression::{Dimension, EvalError, Expression};
use caditor_geometry::Point2;

use crate::{
    entity::{MAX_RHO, MIN_RHO, SplineKind},
    id::EntityId,
};

pub const MAX_LENGTH: f64 = 1e6;

const FULL_TURN_DEGREES: f64 = 360.0;

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
    Curvature(EntityId, EntityId),
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
    AxisDiameter {
        point: EntityId,
        axis: EntityId,
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
    ArcLength {
        arc: EntityId,
        value: Expression,
    },
    Sweep {
        arc: EntityId,
        value: Expression,
    },
    MajorRadius {
        ellipse: EntityId,
        value: Expression,
    },
    MinorRadius {
        ellipse: EntityId,
        value: Expression,
    },
    Rho {
        conic: EntityId,
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
            Self::Curvature(..) => "Curvature",
            Self::Equal(..) => "Equal",
            Self::Midpoint { .. } => "Midpoint",
            Self::Concentric(..) => "Concentric",
            Self::Collinear(..) => "Collinear",
            Self::Symmetric { .. } => "Symmetric",
            Self::Fix { .. } => "Fix",
            Self::Distance { .. } => "Distance",
            Self::HorizontalDistance { .. } => "Horizontal distance",
            Self::VerticalDistance { .. } => "Vertical distance",
            Self::AxisDiameter { .. } => "Diameter across",
            Self::Angle { .. } => "Angle",
            Self::Radius { .. } => "Radius",
            Self::Diameter { .. } => "Diameter",
            Self::ArcLength { .. } => "Arc length",
            Self::Sweep { .. } => "Sweep",
            Self::MajorRadius { .. } => "Major radius",
            Self::MinorRadius { .. } => "Minor radius",
            Self::Rho { .. } => "Rho",
        }
    }

    pub fn entities(&self) -> Vec<EntityId> {
        match *self {
            Self::Horizontal(entity)
            | Self::Vertical(entity)
            | Self::Fix { point: entity, .. }
            | Self::Radius { entity, .. }
            | Self::Diameter { entity, .. }
            | Self::ArcLength { arc: entity, .. }
            | Self::Sweep { arc: entity, .. }
            | Self::MajorRadius {
                ellipse: entity, ..
            }
            | Self::MinorRadius {
                ellipse: entity, ..
            }
            | Self::Rho { conic: entity, .. } => vec![entity],
            Self::Coincident(a, b)
            | Self::HorizontalPoints(a, b)
            | Self::VerticalPoints(a, b)
            | Self::Parallel(a, b)
            | Self::Perpendicular(a, b)
            | Self::Tangent(a, b)
            | Self::Curvature(a, b)
            | Self::Equal(a, b)
            | Self::Midpoint { point: a, curve: b }
            | Self::Concentric(a, b)
            | Self::Collinear(a, b)
            | Self::Distance { from: a, to: b, .. }
            | Self::HorizontalDistance { from: a, to: b, .. }
            | Self::VerticalDistance { from: a, to: b, .. }
            | Self::AxisDiameter {
                point: a, axis: b, ..
            }
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
            | Self::AxisDiameter { value, .. }
            | Self::Angle { value, .. }
            | Self::Radius { value, .. }
            | Self::Diameter { value, .. }
            | Self::ArcLength { value, .. }
            | Self::Sweep { value, .. }
            | Self::MajorRadius { value, .. }
            | Self::MinorRadius { value, .. }
            | Self::Rho { value, .. } => Some(value),
            Self::Coincident(..)
            | Self::Horizontal(_)
            | Self::Vertical(_)
            | Self::HorizontalPoints(..)
            | Self::VerticalPoints(..)
            | Self::Parallel(..)
            | Self::Perpendicular(..)
            | Self::Tangent(..)
            | Self::Curvature(..)
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
            Self::Angle { .. } | Self::Sweep { .. } => Some(Dimension::ANGLE),
            Self::Rho { .. } => Some(Dimension::NONE),
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
            Self::Radius { .. } | Self::MajorRadius { .. } | Self::MinorRadius { .. }
                if value <= 0.0 =>
            {
                Err(DimensionError::NotPositive)
            }
            Self::Diameter { .. } if value <= 0.0 => Err(DimensionError::DiameterNotPositive),
            Self::AxisDiameter { .. } if value < 0.0 => Err(DimensionError::Negative),
            Self::ArcLength { .. } if value <= 0.0 => Err(DimensionError::ArcLengthNotPositive),
            Self::Sweep { .. } if value <= 0.0 || value >= FULL_TURN_DEGREES => {
                Err(DimensionError::SweepOutsideTurn)
            }
            Self::Rho { .. } if !SplineKind::rho_is_valid(value) => {
                Err(DimensionError::RhoOutOfRange)
            }
            Self::Angle { .. } | Self::Sweep { .. } | Self::Rho { .. } => Ok(()),
            _ if self.dimension().is_some() && value > MAX_LENGTH => Err(DimensionError::TooLong),
            _ => Ok(()),
        }
    }

    pub(crate) fn dimension_mut(&mut self) -> Option<&mut Expression> {
        match self {
            Self::Distance { value, .. }
            | Self::HorizontalDistance { value, .. }
            | Self::VerticalDistance { value, .. }
            | Self::AxisDiameter { value, .. }
            | Self::Angle { value, .. }
            | Self::Radius { value, .. }
            | Self::Diameter { value, .. }
            | Self::ArcLength { value, .. }
            | Self::Sweep { value, .. }
            | Self::MajorRadius { value, .. }
            | Self::MinorRadius { value, .. }
            | Self::Rho { value, .. } => Some(value),
            Self::Coincident(..)
            | Self::Horizontal(_)
            | Self::Vertical(_)
            | Self::HorizontalPoints(..)
            | Self::VerticalPoints(..)
            | Self::Parallel(..)
            | Self::Perpendicular(..)
            | Self::Tangent(..)
            | Self::Curvature(..)
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
        self.with_entities_mapped(|entity| if entity == from { to } else { entity })
    }

    pub(crate) fn with_entities_mapped(&self, swap: impl Fn(EntityId) -> EntityId) -> Self {
        let mut replaced = self.clone();
        match &mut replaced {
            Self::Horizontal(entity)
            | Self::Vertical(entity)
            | Self::Fix { point: entity, .. }
            | Self::Radius { entity, .. }
            | Self::Diameter { entity, .. }
            | Self::ArcLength { arc: entity, .. }
            | Self::Sweep { arc: entity, .. }
            | Self::MajorRadius {
                ellipse: entity, ..
            }
            | Self::MinorRadius {
                ellipse: entity, ..
            }
            | Self::Rho { conic: entity, .. } => *entity = swap(*entity),
            Self::Coincident(a, b)
            | Self::HorizontalPoints(a, b)
            | Self::VerticalPoints(a, b)
            | Self::Parallel(a, b)
            | Self::Perpendicular(a, b)
            | Self::Tangent(a, b)
            | Self::Curvature(a, b)
            | Self::Equal(a, b)
            | Self::Midpoint { point: a, curve: b }
            | Self::Concentric(a, b)
            | Self::Collinear(a, b)
            | Self::Distance { from: a, to: b, .. }
            | Self::HorizontalDistance { from: a, to: b, .. }
            | Self::VerticalDistance { from: a, to: b, .. }
            | Self::AxisDiameter {
                point: a, axis: b, ..
            }
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
    #[error("an arc length must be greater than zero")]
    ArcLengthNotPositive,
    #[error("a sweep must be more than 0° and less than 360°")]
    SweepOutsideTurn,
    #[error("the value is too large to use")]
    NotFinite,
    #[error("a length cannot be more than {} m", MAX_LENGTH / 1_000.0)]
    TooLong,
    #[error("a conic's rho must lie between {MIN_RHO} and {MAX_RHO}")]
    RhoOutOfRange,
}
