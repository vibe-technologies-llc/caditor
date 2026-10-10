use caditor_geometry::{Point2, Vector2};

use crate::{constraint::Constraint, entity::Role, id::EntityId, sketch::Sketch};

const LEVEL_TOLERANCE: f64 = 1e-9;

impl Constraint {
    pub fn measuring_axis(&self) -> Option<Vector2> {
        match self {
            Self::HorizontalDistance { .. } => Some(Vector2::X),
            Self::VerticalDistance { .. } => Some(Vector2::Y),
            _ => None,
        }
    }
}

impl Sketch {
    pub fn axis_offset_ends(&self, constraint: &Constraint) -> Option<(Point2, Point2, Vector2)> {
        let along = constraint.measuring_axis()?;
        let [from, to] = constraint.entities()[..] else {
            return None;
        };
        let (start, end) = match (self.role(from)?, self.role(to)?) {
            (Role::Line, _) => {
                let anchor = self.offset_anchor(to)?;
                (self.line_crossing(from, anchor, along)?, anchor)
            }
            (_, Role::Line) => {
                let anchor = self.offset_anchor(from)?;
                (anchor, self.line_crossing(to, anchor, along)?)
            }
            _ => (self.offset_anchor(from)?, self.offset_anchor(to)?),
        };
        Some((start, end, along))
    }

    pub fn runs_along(&self, line: EntityId, along: Vector2) -> bool {
        self.line_direction(line)
            .and_then(Vector2::try_normalize)
            .is_none_or(|direction| along.perp_dot(direction).abs() <= LEVEL_TOLERANCE)
    }

    fn offset_anchor(&self, id: EntityId) -> Option<Point2> {
        match self.role(id)? {
            Role::Point => self.point(id),
            Role::Circular => self.circle(id).map(|(center, _)| center),
            Role::Line | Role::Spline | Role::Elliptic => None,
        }
    }

    fn line_crossing(&self, line: EntityId, through: Point2, along: Vector2) -> Option<Point2> {
        if self.runs_along(line, along) {
            return None;
        }
        let direction = self.line_direction(line)?;
        let anchor = self
            .line_endpoints(line)
            .map_or(Point2::ZERO, |(start, _)| start);
        let reach = direction.perp_dot(anchor - through) / direction.perp_dot(along);
        Some(through + along * reach)
    }
}
