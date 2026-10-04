use caditor_geometry::Point2;

use crate::id::EntityId;

#[derive(Debug, Clone, PartialEq)]
pub enum Entity {
    Point(Point2),
    Line {
        start: EntityId,
        end: EntityId,
    },
    Circle {
        center: EntityId,
        radius: f64,
    },
    Arc {
        center: EntityId,
        start: EntityId,
        end: EntityId,
    },
    Spline {
        control_points: Vec<EntityId>,
    },
}

impl Entity {
    pub fn heap_size(&self) -> usize {
        match self {
            Self::Spline { control_points } => size_of_val(control_points.as_slice()),
            Self::Point(_) | Self::Line { .. } | Self::Circle { .. } | Self::Arc { .. } => 0,
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Point(_) => "Point",
            Self::Line { .. } => "Line",
            Self::Circle { .. } => "Circle",
            Self::Arc { .. } => "Arc",
            Self::Spline { .. } => "Spline",
        }
    }

    pub(crate) fn references(&self, id: EntityId) -> bool {
        self.points().contains(&id)
    }

    pub fn same_structure(&self, other: &Entity) -> bool {
        self.kind_name() == other.kind_name() && self.points() == other.points()
    }

    pub fn points(&self) -> Vec<EntityId> {
        match self {
            Self::Point(_) => Vec::new(),
            Self::Line { start, end } => vec![*start, *end],
            Self::Circle { center, .. } => vec![*center],
            Self::Arc { center, start, end } => vec![*center, *start, *end],
            Self::Spline { control_points } => control_points.clone(),
        }
    }

    pub(crate) fn role(&self) -> Role {
        match self {
            Self::Point(_) => Role::Point,
            Self::Line { .. } => Role::Line,
            Self::Circle { .. } | Self::Arc { .. } => Role::Circular,
            Self::Spline { .. } => Role::Spline,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Point,
    Line,
    Circular,
    Spline,
}
