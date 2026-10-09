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
    Ellipse {
        center: EntityId,
        major: EntityId,
        minor_radius: f64,
    },
    EllipticalArc {
        center: EntityId,
        major: EntityId,
        minor_radius: f64,
        start: EntityId,
        end: EntityId,
    },
}

impl Entity {
    pub fn heap_size(&self) -> usize {
        match self {
            Self::Spline { control_points } => size_of_val(control_points.as_slice()),
            Self::Point(_)
            | Self::Line { .. }
            | Self::Circle { .. }
            | Self::Arc { .. }
            | Self::Ellipse { .. }
            | Self::EllipticalArc { .. } => 0,
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Point(_) => "Point",
            Self::Line { .. } => "Line",
            Self::Circle { .. } => "Circle",
            Self::Arc { .. } => "Arc",
            Self::Spline { .. } => "Spline",
            Self::Ellipse { .. } => "Ellipse",
            Self::EllipticalArc { .. } => "Elliptical arc",
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
            Self::Ellipse { center, major, .. } => vec![*center, *major],
            Self::EllipticalArc {
                center,
                major,
                start,
                end,
                ..
            } => vec![*center, *major, *start, *end],
        }
    }

    pub(crate) fn with_points_mapped(&self, map: impl Fn(EntityId) -> EntityId) -> Self {
        match self {
            Self::Point(position) => Self::Point(*position),
            Self::Line { start, end } => Self::Line {
                start: map(*start),
                end: map(*end),
            },
            Self::Circle { center, radius } => Self::Circle {
                center: map(*center),
                radius: *radius,
            },
            Self::Arc { center, start, end } => Self::Arc {
                center: map(*center),
                start: map(*start),
                end: map(*end),
            },
            Self::Spline { control_points } => Self::Spline {
                control_points: control_points.iter().map(|point| map(*point)).collect(),
            },
            Self::Ellipse {
                center,
                major,
                minor_radius,
            } => Self::Ellipse {
                center: map(*center),
                major: map(*major),
                minor_radius: *minor_radius,
            },
            Self::EllipticalArc {
                center,
                major,
                minor_radius,
                start,
                end,
            } => Self::EllipticalArc {
                center: map(*center),
                major: map(*major),
                minor_radius: *minor_radius,
                start: map(*start),
                end: map(*end),
            },
        }
    }

    pub(crate) fn role(&self) -> Role {
        match self {
            Self::Point(_) => Role::Point,
            Self::Line { .. } => Role::Line,
            Self::Circle { .. } | Self::Arc { .. } => Role::Circular,
            Self::Spline { .. } => Role::Spline,
            Self::Ellipse { .. } | Self::EllipticalArc { .. } => Role::Elliptic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Point,
    Line,
    Circular,
    Spline,
    Elliptic,
}
