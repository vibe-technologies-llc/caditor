use std::collections::BTreeSet;

use caditor_geometry::{Point2, Vector2};

use crate::{
    entity::{Entity, Role},
    id::{EntityId, Reference},
    intersect::{Carrier, Shape, crossings, line_crossings},
    sketch::{Sketch, SketchError},
};

const CROSSING_TOLERANCE: f64 = 1e-9;
const TIED_CHOICE: f64 = 1e-9;
const FEWEST_CORNERS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Quadrant {
    Right,
    Top,
    Left,
    Bottom,
}

impl Quadrant {
    pub const ALL: [Self; 4] = [Self::Right, Self::Top, Self::Left, Self::Bottom];

    pub fn outward(self) -> Vector2 {
        match self {
            Self::Right => Vector2::X,
            Self::Top => Vector2::Y,
            Self::Left => -Vector2::X,
            Self::Bottom => -Vector2::Y,
        }
    }

    pub fn is_level(self) -> bool {
        matches!(self, Self::Right | Self::Left)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Top => "top",
            Self::Left => "left",
            Self::Bottom => "bottom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EllipseEnd {
    Major,
    FarMajor,
    Minor,
    FarMinor,
}

impl EllipseEnd {
    pub const ALL: [Self; 4] = [Self::Major, Self::FarMajor, Self::Minor, Self::FarMinor];

    pub fn turn(self) -> (f64, f64) {
        match self {
            Self::Major => (1.0, 0.0),
            Self::FarMajor => (-1.0, 0.0),
            Self::Minor => (0.0, 1.0),
            Self::FarMinor => (0.0, -1.0),
        }
    }

    pub fn nearest(sketch: &Sketch, ellipse: EntityId, to: Point2) -> Option<Self> {
        Self::ALL
            .into_iter()
            .filter_map(|end| Some((end, end_of(sketch, ellipse, end)?.distance(to))))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(end, _)| end)
    }

    fn words(self) -> &'static str {
        match self {
            Self::Major => "the end of the major axis",
            Self::FarMajor => "the far end of the major axis",
            Self::Minor => "the end of the minor axis",
            Self::FarMinor => "the far end of the minor axis",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PointTarget {
    Point(EntityId),
    Middle(EntityId),
    Crossing(EntityId, EntityId),
    Centroid(Vec<EntityId>),
    Quadrant { curve: EntityId, side: Quadrant },
    EllipseEnd { ellipse: EntityId, end: EllipseEnd },
}

impl From<EntityId> for PointTarget {
    fn from(point: EntityId) -> Self {
        Self::Point(point)
    }
}

impl PointTarget {
    pub fn entities(&self) -> Vec<EntityId> {
        match self {
            Self::Point(entity)
            | Self::Middle(entity)
            | Self::Quadrant { curve: entity, .. }
            | Self::EllipseEnd {
                ellipse: entity, ..
            } => vec![*entity],
            Self::Crossing(first, second) => vec![*first, *second],
            Self::Centroid(corners) => corners.clone(),
        }
    }

    pub fn point(&self) -> Option<EntityId> {
        match self {
            Self::Point(point) => Some(*point),
            Self::Middle(_)
            | Self::Crossing(..)
            | Self::Centroid(_)
            | Self::Quadrant { .. }
            | Self::EllipseEnd { .. } => None,
        }
    }

    pub fn follows_length(&self) -> bool {
        matches!(self, Self::Middle(_))
    }

    pub(crate) fn map_entities(&mut self, swap: &impl Fn(EntityId) -> EntityId) {
        match self {
            Self::Point(entity)
            | Self::Middle(entity)
            | Self::Quadrant { curve: entity, .. }
            | Self::EllipseEnd {
                ellipse: entity, ..
            } => *entity = swap(*entity),
            Self::Crossing(first, second) => {
                *first = swap(*first);
                *second = swap(*second);
            }
            Self::Centroid(corners) => {
                for corner in corners {
                    *corner = swap(*corner);
                }
            }
        }
    }
}

impl Sketch {
    pub fn target_places(&self, target: &PointTarget) -> Vec<Point2> {
        match target {
            PointTarget::Point(point) => self.point(*point).into_iter().collect(),
            PointTarget::Middle(curve) => self.middle_of(*curve).into_iter().collect(),
            PointTarget::Crossing(first, second) => crossing_places(self, *first, *second),
            PointTarget::Centroid(corners) => centroid_of(self, corners).into_iter().collect(),
            PointTarget::Quadrant { curve, side } => self
                .circle(*curve)
                .map(|(center, radius)| center + side.outward() * radius)
                .into_iter()
                .collect(),
            PointTarget::EllipseEnd { ellipse, end } => {
                end_of(self, *ellipse, *end).into_iter().collect()
            }
        }
    }

    pub fn target_position(
        &self,
        target: &PointTarget,
        held: Point2,
        across: Vector2,
    ) -> Option<Point2> {
        let places = self.target_places(target);
        let miss = |place: &Point2| across.dot(held - *place).abs();
        let least = places.iter().map(miss).fold(f64::INFINITY, f64::min);
        let slack = TIED_CHOICE * (1.0 + least + held.abs().max_element());
        places
            .into_iter()
            .filter(|place| miss(place) <= least + slack)
            .min_by(|a, b| a.distance(held).total_cmp(&b.distance(held)))
    }

    pub fn middle_of(&self, curve: EntityId) -> Option<Point2> {
        match self.entity(curve)? {
            Entity::Line { .. } => {
                let (start, end) = self.line_endpoints(curve)?;
                Some(start.midpoint(end))
            }
            Entity::Arc { .. } => {
                let arc = self.arc(curve)?;
                Some(arc.point_at(arc.start_angle + arc.sweep / 2.0))
            }
            Entity::EllipticalArc { .. } => {
                let ellipse = self.ellipse(curve)?;
                Some(ellipse.point_at(ellipse.start + ellipse.sweep / 2.0))
            }
            Entity::Point(_)
            | Entity::Circle { .. }
            | Entity::Spline { .. }
            | Entity::Ellipse { .. } => None,
        }
    }

    pub fn describe_target(&self, target: &PointTarget) -> String {
        let label = |entity: EntityId| self.entity_label(entity);
        match target {
            PointTarget::Point(point) => label(*point),
            PointTarget::Middle(curve) => format!("the midpoint of {}", label(*curve)),
            PointTarget::Crossing(first, second) => {
                format!("the crossing of {} and {}", label(*first), label(*second))
            }
            PointTarget::Centroid(corners) => format!(
                "the centre of {}",
                listed(corners.iter().map(|corner| label(*corner)).collect())
            ),
            PointTarget::Quadrant { curve, side } => {
                format!("the {} of {}", side.name(), label(*curve))
            }
            PointTarget::EllipseEnd { ellipse, end } => {
                format!("{} of {}", end.words(), label(*ellipse))
            }
        }
    }

    pub(crate) fn check_target(
        &self,
        target: &PointTarget,
        held: EntityId,
    ) -> Result<(), SketchError> {
        let entities = target.entities();
        if let Some(missing) = entities.iter().find(|entity| !self.contains(**entity)) {
            return Err(SketchError::MissingEntity(*missing));
        }
        let mut seen = BTreeSet::from([held]);
        if let Some(repeated) = entities.iter().find(|entity| !seen.insert(**entity)) {
            return Err(SketchError::SameEntity {
                entity: *repeated,
                label: self.entity_label(*repeated),
            });
        }
        let wrong = |entity: EntityId, needed: &'static str| SketchError::WrongKind {
            entity,
            found: self.entity_label(entity),
            needed,
        };
        match target {
            PointTarget::Point(point) => self.expect_role(*point, &[Role::Point], "a point"),
            PointTarget::Middle(curve) => {
                let needed = "a line, an arc or an elliptical arc";
                match self.entity(*curve) {
                    Some(
                        Entity::Line { .. } | Entity::Arc { .. } | Entity::EllipticalArc { .. },
                    ) => Ok(()),
                    _ => Err(wrong(*curve, needed)),
                }
            }
            PointTarget::Crossing(first, second) => {
                let needed = "a line, a circle, an arc, a spline or an ellipse";
                let curves = [Role::Line, Role::Circular, Role::Spline, Role::Elliptic];
                self.expect_role(*first, &curves, needed)?;
                self.expect_role(*second, &curves, needed)
            }
            PointTarget::Centroid(corners) => {
                if corners.len() < FEWEST_CORNERS {
                    return Err(SketchError::TooFewCorners);
                }
                corners
                    .iter()
                    .try_for_each(|corner| self.expect_role(*corner, &[Role::Point], "a point"))
            }
            PointTarget::Quadrant { curve, .. } => {
                self.expect_role(*curve, &[Role::Circular], "a circle or an arc")
            }
            PointTarget::EllipseEnd { ellipse, .. } => self.expect_role(
                *ellipse,
                &[Role::Elliptic],
                "an ellipse or an elliptical arc",
            ),
        }
    }

    fn expect_role(
        &self,
        entity: EntityId,
        roles: &[Role],
        needed: &'static str,
    ) -> Result<(), SketchError> {
        match self.role(entity) {
            Some(role) if roles.contains(&role) => Ok(()),
            Some(_) => Err(SketchError::WrongKind {
                entity,
                found: self.entity_label(entity),
                needed,
            }),
            None => Err(SketchError::MissingEntity(entity)),
        }
    }
}

pub(crate) fn listed(mut names: Vec<String>) -> String {
    let Some(last) = names.pop() else {
        return String::new();
    };
    if names.is_empty() {
        last
    } else {
        format!("{} and {last}", names.join(", "))
    }
}

fn end_of(sketch: &Sketch, ellipse: EntityId, end: EllipseEnd) -> Option<Point2> {
    let shape = sketch.ellipse(ellipse)?;
    let (along, across) = end.turn();
    Some(shape.center + shape.major * along + shape.axis().perp() * (shape.minor_radius * across))
}

fn centroid_of(sketch: &Sketch, corners: &[EntityId]) -> Option<Point2> {
    let mut sum = Vector2::ZERO;
    for corner in corners {
        sum += sketch.point(*corner)?;
    }
    (!corners.is_empty()).then(|| sum / corners.len() as f64)
}

fn carrier_of(sketch: &Sketch, curve: EntityId) -> Option<Carrier> {
    match curve.reference() {
        Some(Reference::HorizontalAxis) => {
            return Some(Carrier::Line {
                through: Point2::ZERO,
                direction: Vector2::X,
            });
        }
        Some(Reference::VerticalAxis) => {
            return Some(Carrier::Line {
                through: Point2::ZERO,
                direction: Vector2::Y,
            });
        }
        Some(Reference::Origin) => return None,
        None => {}
    }
    match sketch.entity(curve)? {
        Entity::Line { .. } => {
            let (start, end) = sketch.line_endpoints(curve)?;
            Some(Carrier::Line {
                through: start,
                direction: (end - start).try_normalize()?,
            })
        }
        Entity::Circle { .. } | Entity::Arc { .. } => {
            let (center, radius) = sketch.circle(curve)?;
            Some(Carrier::Circle { center, radius })
        }
        Entity::Point(_)
        | Entity::Spline { .. }
        | Entity::Ellipse { .. }
        | Entity::EllipticalArc { .. } => None,
    }
}

fn crossing_places(sketch: &Sketch, first: EntityId, second: EntityId) -> Vec<Point2> {
    let scale = [first, second]
        .into_iter()
        .filter_map(|curve| sketch.entity(curve))
        .flat_map(Entity::points)
        .filter_map(|point| sketch.point(point))
        .fold(1.0_f64, |most, point| most.max(point.abs().max_element()));
    let tolerance = CROSSING_TOLERANCE * scale;
    match (carrier_of(sketch, first), carrier_of(sketch, second)) {
        (Some(carrier), Some(Carrier::Line { through, direction }))
        | (Some(Carrier::Line { through, direction }), Some(carrier)) => {
            line_crossings(carrier, through, direction, tolerance)
        }
        (Some(carrier), Some(Carrier::Circle { center, radius })) => {
            crossings(carrier, &Shape::Circle { center, radius }, tolerance)
        }
        (None, _) => sketch.curve_crossings(first, second),
        (Some(_), None) => sketch.curve_crossings(second, first),
    }
}
