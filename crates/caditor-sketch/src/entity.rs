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
        points: Vec<EntityId>,
        kind: SplineKind,
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
            Self::Spline { points, .. } => size_of_val(points.as_slice()),
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
            Self::Spline { kind, .. } => kind.name(),
            Self::Ellipse { .. } => "Ellipse",
            Self::EllipticalArc { .. } => "Elliptical arc",
        }
    }

    pub(crate) fn references(&self, id: EntityId) -> bool {
        self.points().contains(&id)
    }

    pub fn same_structure(&self, other: &Entity) -> bool {
        self.kind_name() == other.kind_name()
            && self.points() == other.points()
            && match (self.spline_kind(), other.spline_kind()) {
                (Some(kind), Some(other)) => kind.same_form(other),
                (kind, other) => kind.is_none() && other.is_none(),
            }
    }

    pub fn points(&self) -> Vec<EntityId> {
        match self {
            Self::Point(_) => Vec::new(),
            Self::Line { start, end } => vec![*start, *end],
            Self::Circle { center, .. } => vec![*center],
            Self::Arc { center, start, end } => vec![*center, *start, *end],
            Self::Spline { points, .. } => points.clone(),
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
            Self::Spline { points, kind } => Self::Spline {
                points: points.iter().map(|point| map(*point)).collect(),
                kind: *kind,
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

    pub fn spline_kind(&self) -> Option<SplineKind> {
        match self {
            Self::Spline { kind, .. } => Some(*kind),
            Self::Point(_)
            | Self::Line { .. }
            | Self::Circle { .. }
            | Self::Arc { .. }
            | Self::Ellipse { .. }
            | Self::EllipticalArc { .. } => None,
        }
    }

    pub fn spline_ends(&self) -> Option<(EntityId, EntityId)> {
        match self {
            Self::Spline { points, kind } if !kind.is_closed() => {
                Some((*points.first()?, *points.last()?))
            }
            Self::Point(_)
            | Self::Line { .. }
            | Self::Circle { .. }
            | Self::Arc { .. }
            | Self::Spline { .. }
            | Self::Ellipse { .. }
            | Self::EllipticalArc { .. } => None,
        }
    }

    pub fn spline(points: Vec<EntityId>) -> Self {
        Self::Spline {
            points,
            kind: SplineKind::OPEN,
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

pub const MIN_RHO: f64 = 0.01;
pub const MAX_RHO: f64 = 0.99;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SplineKind {
    Control { closed: bool },
    Fit { closed: bool },
    Conic { rho: f64 },
}

impl SplineKind {
    pub const OPEN: Self = Self::Control { closed: false };

    pub fn is_closed(self) -> bool {
        match self {
            Self::Control { closed } | Self::Fit { closed } => closed,
            Self::Conic { .. } => false,
        }
    }

    pub fn same_form(self, other: Self) -> bool {
        match (self, other) {
            (Self::Conic { .. }, Self::Conic { .. }) => true,
            (kind, other) => kind == other,
        }
    }

    pub fn passes_its_points(self) -> bool {
        matches!(self, Self::Fit { .. })
    }

    pub fn fewest_points(self) -> usize {
        match self {
            Self::Control { closed: false } | Self::Fit { closed: false } => 2,
            Self::Control { closed: true } | Self::Fit { closed: true } | Self::Conic { .. } => 3,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Control { closed: false } => "Spline",
            Self::Control { closed: true } => "Closed spline",
            Self::Fit { closed: false } => "Fit-point spline",
            Self::Fit { closed: true } => "Closed fit-point spline",
            Self::Conic { .. } => "Conic",
        }
    }

    pub fn rho_is_valid(rho: f64) -> bool {
        (MIN_RHO..=MAX_RHO).contains(&rho)
    }
}
