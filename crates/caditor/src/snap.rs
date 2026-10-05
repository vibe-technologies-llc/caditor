use std::f64::consts::TAU;

use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{ArcGeometry, Entity, EntityId, Sketch};

pub const POINT_TOLERANCE: f64 = 8.0;
pub const CURVE_TOLERANCE: f64 = 6.0;
const ON_CIRCLE_TOLERANCE: f64 = 1e-9;
const PARALLEL_TOLERANCE: f64 = 1e-12;

pub trait Screen {
    fn to_screen(&self, point: Point2) -> Option<Vector2>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Pending(usize),
    Point(EntityId),
    Curve(EntityId),
    Midpoint(EntityId),
    Intersection(EntityId, EntityId),
}

impl Target {
    #[cfg(test)]
    pub fn min_ordered(self) -> Self {
        match self {
            Self::Intersection(a, b) => Self::Intersection(a.min(b), a.max(b)),
            other => other,
        }
    }

    pub fn second_entity(self) -> Option<EntityId> {
        match self {
            Self::Intersection(_, second) => Some(second),
            Self::Pending(_) | Self::Point(_) | Self::Curve(_) | Self::Midpoint(_) => None,
        }
    }

    pub fn entity(self) -> Option<EntityId> {
        match self {
            Self::Pending(_) => None,
            Self::Point(entity)
            | Self::Curve(entity)
            | Self::Midpoint(entity)
            | Self::Intersection(entity, _) => Some(entity),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapped {
    pub position: Point2,
    pub target: Target,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pointer {
    pub screen: Vector2,
    pub sketch: Point2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Accept {
    Anything,
    Points,
    OnCircle { center: Point2, radius: f64 },
}

pub fn resolve(
    sketch: &Sketch,
    screen: &impl Screen,
    pointer: Pointer,
    pending: &[(usize, Point2)],
    accept: Accept,
) -> Option<Snapped> {
    let pending = pending
        .iter()
        .map(|(index, position)| Snapped {
            position: *position,
            target: Target::Pending(*index),
        })
        .collect();
    let nearest = |candidates: Vec<Snapped>, tolerance: f64| {
        candidates
            .into_iter()
            .filter_map(|candidate| {
                let offset = screen
                    .to_screen(candidate.position)?
                    .distance(pointer.screen);
                (offset <= tolerance).then_some((offset, candidate))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, candidate)| candidate)
    };
    let points = match accept {
        Accept::Anything | Accept::Points => points(sketch),
        Accept::OnCircle { center, radius } => points(sketch)
            .into_iter()
            .filter(|candidate| on_circle(center, radius, candidate.position))
            .collect(),
    };
    nearest(pending, POINT_TOLERANCE)
        .or_else(|| nearest(points, POINT_TOLERANCE))
        .or_else(|| match accept {
            Accept::Anything => nearest(midpoints(sketch), POINT_TOLERANCE)
                .or_else(|| nearest(intersections(sketch, screen, pointer), POINT_TOLERANCE)),
            Accept::Points | Accept::OnCircle { .. } => None,
        })
        .or_else(|| match accept {
            Accept::Anything => nearest(curves(sketch, pointer.sketch), CURVE_TOLERANCE),
            Accept::Points => None,
            Accept::OnCircle { center, radius } => {
                nearest(crossings(sketch, center, radius), CURVE_TOLERANCE)
            }
        })
}

pub fn on_circle(center: Point2, radius: f64, point: Point2) -> bool {
    (point.distance(center) - radius).abs() <= ON_CIRCLE_TOLERANCE * radius.max(1.0)
}

pub fn points(sketch: &Sketch) -> Vec<Snapped> {
    let origin = Snapped {
        position: Point2::ZERO,
        target: Target::Point(EntityId::ORIGIN),
    };
    let drawn = sketch.entities().filter_map(|(id, entity)| match entity {
        Entity::Point(position) => Some(Snapped {
            position: *position,
            target: Target::Point(id),
        }),
        Entity::Line { .. }
        | Entity::Circle { .. }
        | Entity::Arc { .. }
        | Entity::Spline { .. } => None,
    });
    std::iter::once(origin).chain(drawn).collect()
}

const NEAR_CURVE_PIXELS: f64 = 4.0 * POINT_TOLERANCE;
const MAX_CROSSING_CURVES: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Geometry {
    Infinite(Point2, Vector2),
    Segment(Point2, Point2),
    Circle(Point2, f64),
    Arc(ArcGeometry),
}

fn geometry(sketch: &Sketch, id: EntityId) -> Option<Geometry> {
    if id == EntityId::HORIZONTAL_AXIS {
        return Some(Geometry::Infinite(Point2::ZERO, Vector2::X));
    }
    if id == EntityId::VERTICAL_AXIS {
        return Some(Geometry::Infinite(Point2::ZERO, Vector2::Y));
    }
    match sketch.entity(id)? {
        Entity::Line { .. } => sketch
            .line_endpoints(id)
            .map(|(start, end)| Geometry::Segment(start, end)),
        Entity::Circle { .. } => sketch
            .circle(id)
            .map(|(center, radius)| Geometry::Circle(center, radius)),
        Entity::Arc { .. } => sketch.arc(id).map(Geometry::Arc),
        Entity::Point(_) | Entity::Spline { .. } => None,
    }
}

fn straight(geometry: Geometry) -> Option<(Point2, Vector2, bool)> {
    match geometry {
        Geometry::Infinite(through, along) => Some((through, along, false)),
        Geometry::Segment(start, end) => Some((start, end - start, true)),
        Geometry::Circle(..) | Geometry::Arc(_) => None,
    }
}

fn round(geometry: Geometry) -> Option<(Point2, f64, Option<ArcGeometry>)> {
    match geometry {
        Geometry::Circle(center, radius) => Some((center, radius, None)),
        Geometry::Arc(arc) => Some((arc.center, arc.radius, Some(arc))),
        Geometry::Infinite(..) | Geometry::Segment(..) => None,
    }
}

fn hits(first: Geometry, second: Geometry) -> Vec<Point2> {
    let on_arc = |arc: Option<ArcGeometry>, position: &Point2| {
        arc.is_none_or(|arc| within_sweep(&arc, *position))
    };
    match (
        straight(first),
        straight(second),
        round(first),
        round(second),
    ) {
        (Some((through, along, bounded)), Some((origin, direction, other_bounded)), _, _) => {
            let Some((fraction, position)) = straight_crossing(through, along, origin, direction)
            else {
                return Vec::new();
            };
            let own = (position - through).dot(along) / along.length_squared();
            let inside = |bounded: bool, fraction: f64| !bounded || (0.0..=1.0).contains(&fraction);
            (inside(bounded, own) && inside(other_bounded, fraction))
                .then_some(position)
                .into_iter()
                .collect()
        }
        (Some((through, along, bounded)), None, None, Some((center, radius, arc)))
        | (None, Some((through, along, bounded)), Some((center, radius, arc)), None) => {
            line_crossings(through, along, center, radius)
                .into_iter()
                .filter(|(fraction, _)| !bounded || (0.0..=1.0).contains(fraction))
                .map(|(_, position)| position)
                .filter(|position| on_arc(arc, position))
                .collect()
        }
        (None, None, Some((center, radius, arc)), Some((other, other_radius, other_arc))) => {
            circle_crossings(center, radius, other, other_radius)
                .into_iter()
                .filter(|position| on_arc(arc, position) && on_arc(other_arc, position))
                .collect()
        }
        _ => Vec::new(),
    }
}

fn intersections(sketch: &Sketch, screen: &impl Screen, pointer: Pointer) -> Vec<Snapped> {
    let mut near: Vec<(f64, EntityId, Geometry)> = curves(sketch, pointer.sketch)
        .into_iter()
        .filter_map(|candidate| {
            let Target::Curve(id) = candidate.target else {
                return None;
            };
            let distance = screen
                .to_screen(candidate.position)?
                .distance(pointer.screen);
            (distance <= NEAR_CURVE_PIXELS).then_some((distance, id, geometry(sketch, id)?))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.truncate(MAX_CROSSING_CURVES);
    let mut found = Vec::new();
    for (index, (_, first, first_geometry)) in near.iter().enumerate() {
        for (_, second, second_geometry) in near.iter().skip(index + 1) {
            let (low, high) = if first <= second {
                (*first, *second)
            } else {
                (*second, *first)
            };
            found.extend(
                hits(*first_geometry, *second_geometry)
                    .into_iter()
                    .map(|position| Snapped {
                        position,
                        target: Target::Intersection(low, high),
                    }),
            );
        }
    }
    found
}

fn midpoints(sketch: &Sketch) -> Vec<Snapped> {
    sketch
        .entities()
        .filter_map(|(id, entity)| {
            let Entity::Line { .. } = entity else {
                return None;
            };
            let (start, end) = sketch.line_endpoints(id)?;
            Some(Snapped {
                position: start.midpoint(end),
                target: Target::Midpoint(id),
            })
        })
        .collect()
}

fn curves(sketch: &Sketch, at: Point2) -> Vec<Snapped> {
    let axes = [
        Snapped {
            position: Point2::new(at.x, 0.0),
            target: Target::Curve(EntityId::HORIZONTAL_AXIS),
        },
        Snapped {
            position: Point2::new(0.0, at.y),
            target: Target::Curve(EntityId::VERTICAL_AXIS),
        },
    ];
    let drawn = sketch.entities().filter_map(|(id, entity)| {
        Some(Snapped {
            position: closest_on(sketch, id, entity, at)?,
            target: Target::Curve(id),
        })
    });
    axes.into_iter().chain(drawn).collect()
}

fn closest_on(sketch: &Sketch, id: EntityId, entity: &Entity, at: Point2) -> Option<Point2> {
    match entity {
        Entity::Line { .. } => {
            let (start, end) = sketch.line_endpoints(id)?;
            Some(closest_on_segment(start, end, at))
        }
        Entity::Circle { .. } => {
            let (center, radius) = sketch.circle(id)?;
            closest_on_circle(center, radius, at)
        }
        Entity::Arc { .. } => {
            let arc = sketch.arc(id)?;
            let position = closest_on_circle(arc.center, arc.radius, at)?;
            within_sweep(&arc, position).then_some(position)
        }
        Entity::Point(_) | Entity::Spline { .. } => None,
    }
}

fn crossings(sketch: &Sketch, center: Point2, radius: f64) -> Vec<Snapped> {
    let axes = [
        (EntityId::HORIZONTAL_AXIS, Point2::ZERO, Point2::X),
        (EntityId::VERTICAL_AXIS, Point2::ZERO, Point2::Y),
    ]
    .into_iter()
    .flat_map(|(axis, through, along)| {
        line_crossings(through, along, center, radius)
            .into_iter()
            .map(move |(_, position)| Snapped {
                position,
                target: Target::Curve(axis),
            })
    });
    let drawn = sketch.entities().flat_map(|(id, entity)| {
        crossings_with(sketch, id, entity, center, radius)
            .into_iter()
            .map(move |position| Snapped {
                position,
                target: Target::Curve(id),
            })
    });
    axes.chain(drawn).collect()
}

fn crossings_with(
    sketch: &Sketch,
    id: EntityId,
    entity: &Entity,
    center: Point2,
    radius: f64,
) -> Vec<Point2> {
    match entity {
        Entity::Line { .. } => sketch
            .line_endpoints(id)
            .map(|(start, end)| {
                line_crossings(start, end - start, center, radius)
                    .into_iter()
                    .filter(|(along, _)| (0.0..=1.0).contains(along))
                    .map(|(_, position)| position)
                    .collect()
            })
            .unwrap_or_default(),
        Entity::Circle { .. } => sketch
            .circle(id)
            .map(|(other, other_radius)| circle_crossings(center, radius, other, other_radius))
            .unwrap_or_default(),
        Entity::Arc { .. } => sketch
            .arc(id)
            .map(|arc| {
                circle_crossings(center, radius, arc.center, arc.radius)
                    .into_iter()
                    .filter(|position| within_sweep(&arc, *position))
                    .collect()
            })
            .unwrap_or_default(),
        Entity::Point(_) | Entity::Spline { .. } => Vec::new(),
    }
}

fn line_crossings(
    through: Point2,
    along: Vector2,
    center: Point2,
    radius: f64,
) -> Vec<(f64, Point2)> {
    let length_squared = along.length_squared();
    if length_squared == 0.0 {
        return Vec::new();
    }
    let foot = (center - through).dot(along) / length_squared;
    let closest = through + along * foot;
    let half_chord_squared = radius * radius - closest.distance_squared(center);
    if half_chord_squared < 0.0 {
        return Vec::new();
    }
    let half = half_chord_squared.sqrt() / length_squared.sqrt();
    [foot - half, foot + half]
        .into_iter()
        .map(|parameter| (parameter, through + along * parameter))
        .collect()
}

fn circle_crossings(center: Point2, radius: f64, other: Point2, other_radius: f64) -> Vec<Point2> {
    let between = other - center;
    let distance = between.length();
    if distance == 0.0
        || distance > radius + other_radius
        || distance < (radius - other_radius).abs()
    {
        return Vec::new();
    }
    let along =
        (radius * radius - other_radius * other_radius + distance * distance) / (2.0 * distance);
    let half_chord = (radius * radius - along * along).max(0.0).sqrt();
    let direction = between / distance;
    let middle = center + direction * along;
    let across = direction.perp() * half_chord;
    vec![middle + across, middle - across]
}

pub fn crossing_along(
    sketch: &Sketch,
    curve: EntityId,
    through: Point2,
    along: Vector2,
    near: Point2,
) -> Option<Point2> {
    let crossings: Vec<Point2> = if curve == EntityId::HORIZONTAL_AXIS {
        straight_crossing(through, along, Point2::ZERO, Vector2::X)
            .map(|(_, position)| position)
            .into_iter()
            .collect()
    } else if curve == EntityId::VERTICAL_AXIS {
        straight_crossing(through, along, Point2::ZERO, Vector2::Y)
            .map(|(_, position)| position)
            .into_iter()
            .collect()
    } else {
        match sketch.entity(curve)? {
            Entity::Line { .. } => {
                let (start, end) = sketch.line_endpoints(curve)?;
                straight_crossing(through, along, start, end - start)
                    .filter(|(fraction, _)| (0.0..=1.0).contains(fraction))
                    .map(|(_, position)| position)
                    .into_iter()
                    .collect()
            }
            Entity::Circle { .. } => {
                let (center, radius) = sketch.circle(curve)?;
                line_crossings(through, along, center, radius)
                    .into_iter()
                    .map(|(_, position)| position)
                    .collect()
            }
            Entity::Arc { .. } => {
                let arc = sketch.arc(curve)?;
                line_crossings(through, along, arc.center, arc.radius)
                    .into_iter()
                    .map(|(_, position)| position)
                    .filter(|position| within_sweep(&arc, *position))
                    .collect()
            }
            Entity::Point(_) | Entity::Spline { .. } => Vec::new(),
        }
    };
    crossings.into_iter().min_by(|a, b| {
        a.distance_squared(near)
            .total_cmp(&b.distance_squared(near))
    })
}

fn straight_crossing(
    through: Point2,
    along: Vector2,
    origin: Point2,
    direction: Vector2,
) -> Option<(f64, Point2)> {
    let denominator = along.perp_dot(direction);
    if denominator.abs() <= PARALLEL_TOLERANCE * along.length() * direction.length() {
        return None;
    }
    let fraction = along.perp_dot(through - origin) / denominator;
    Some((fraction, origin + direction * fraction))
}

fn within_sweep(arc: &ArcGeometry, position: Point2) -> bool {
    let offset = position - arc.center;
    let turned = (offset.y.atan2(offset.x) - arc.start_angle).rem_euclid(TAU);
    turned <= arc.sweep
}

pub fn closest_on_segment(start: Point2, end: Point2, at: Point2) -> Point2 {
    let along = end - start;
    let length_squared = along.length_squared();
    if length_squared == 0.0 {
        return start;
    }
    let fraction = ((at - start).dot(along) / length_squared).clamp(0.0, 1.0);
    start + along * fraction
}

pub fn closest_on_circle(center: Point2, radius: f64, at: Point2) -> Option<Point2> {
    let direction = (at - center).try_normalize()?;
    Some(center + direction * radius)
}

#[cfg(test)]
pub mod tests {
    use caditor_geometry::Plane;

    use super::*;

    pub struct Scaled(pub f64);

    impl Screen for Scaled {
        fn to_screen(&self, point: Point2) -> Option<Vector2> {
            Some(Vector2::new(point.x, -point.y) * self.0)
        }
    }

    fn pointer_at(sketch: Point2) -> Pointer {
        Pointer {
            screen: Scaled(10.0).to_screen(sketch).unwrap(),
            sketch,
        }
    }

    fn resolve_at(sketch: &Sketch, at: Point2) -> Option<Snapped> {
        resolve(sketch, &Scaled(10.0), pointer_at(at), &[], Accept::Anything)
    }

    fn point_of(sketch: &Sketch, curve: EntityId, index: usize) -> EntityId {
        sketch.entity(curve).unwrap().points()[index]
    }

    #[test]
    fn points_win_over_curves_within_their_tolerance() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(30.0, 10.0));
        let start = point_of(&sketch, line, 0);

        let near_start = resolve_at(&sketch, Point2::new(10.7, 10.1)).unwrap();
        assert_eq!(near_start.target, Target::Point(start));
        assert_eq!(near_start.position, Point2::new(10.0, 10.0));

        let on_line = resolve_at(&sketch, Point2::new(16.0, 10.5)).unwrap();
        assert_eq!(on_line.target, Target::Curve(line));
        assert_eq!(on_line.position, Point2::new(16.0, 10.0));

        assert_eq!(resolve_at(&sketch, Point2::new(16.0, 10.7)), None);
        assert_eq!(resolve_at(&sketch, Point2::new(10.0, 10.9)), None);
    }

    #[test]
    fn the_middle_of_a_line_is_a_snap_target_ahead_of_the_line_but_not_of_points_or_circles() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(30.0, 10.0));

        let middle = resolve_at(&sketch, Point2::new(20.4, 10.6)).unwrap();
        assert_eq!(middle.target, Target::Midpoint(line));
        assert_eq!(middle.position, Point2::new(20.0, 10.0));
        assert_eq!(middle.target.entity(), Some(line));

        let only_points = resolve(
            &sketch,
            &Scaled(10.0),
            pointer_at(Point2::new(20.4, 10.6)),
            &[],
            Accept::Points,
        );
        assert_eq!(only_points, None);
        let on_circle = resolve(
            &sketch,
            &Scaled(10.0),
            pointer_at(Point2::new(20.4, 10.6)),
            &[],
            Accept::OnCircle {
                center: Point2::new(20.0, 10.0),
                radius: 50.0,
            },
        );
        assert_eq!(on_circle, None);
    }

    #[test]
    fn where_two_curves_cross_is_a_snap_target_ahead_of_either_curve() {
        let mut sketch = Sketch::new(Plane::XY);
        let across = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(40.0, 40.0));
        let level = sketch.add_line(Point2::new(5.0, 20.0), Point2::new(55.0, 20.0));
        let circle = sketch.add_circle(Point2::new(60.0, 20.0), 10.0);

        let crossing = resolve_at(&sketch, Point2::new(20.4, 20.6)).unwrap();
        assert_eq!(crossing.target, Target::Intersection(across, level));
        assert!(crossing.position.distance(Point2::new(20.0, 20.0)) < 1e-9);
        assert_eq!(crossing.target.second_entity(), Some(level));

        let on_rim = resolve_at(&sketch, Point2::new(50.3, 20.4)).unwrap();
        assert_eq!(
            on_rim.target,
            Target::Intersection(level, circle).min_ordered()
        );
        assert!(on_rim.position.distance(Point2::new(50.0, 20.0)) < 1e-9);
    }

    #[test]
    fn curves_that_do_not_reach_each_other_have_no_crossing_to_snap_to() {
        let mut sketch = Sketch::new(Plane::XY);
        sketch.add_line(Point2::new(10.0, 10.0), Point2::new(18.0, 18.0));
        let level = sketch.add_line(Point2::new(5.0, 20.0), Point2::new(45.0, 20.0));

        let near_the_gap = resolve_at(&sketch, Point2::new(20.0, 20.3)).unwrap();

        assert_eq!(near_the_gap.target, Target::Curve(level));
    }

    #[test]
    fn the_axes_cross_other_curves_where_they_are_snapped() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::new(10.0, -6.0), Point2::new(10.0, 14.0));

        let crossing = resolve_at(&sketch, Point2::new(10.3, 0.4)).unwrap();

        assert_eq!(
            crossing.target,
            Target::Intersection(
                EntityId::HORIZONTAL_AXIS.min(line),
                EntityId::HORIZONTAL_AXIS.max(line)
            )
        );
        assert!(crossing.position.distance(Point2::new(10.0, 0.0)) < 1e-9);
    }

    #[test]
    fn the_origin_and_the_axes_are_snap_targets() {
        let sketch = Sketch::new(Plane::XY);
        assert_eq!(
            resolve_at(&sketch, Point2::new(0.3, -0.4)).map(|snapped| snapped.target),
            Some(Target::Point(EntityId::ORIGIN))
        );
        let on_axis = resolve_at(&sketch, Point2::new(25.0, 0.4)).unwrap();
        assert_eq!(on_axis.target, Target::Curve(EntityId::HORIZONTAL_AXIS));
        assert_eq!(on_axis.position, Point2::new(25.0, 0.0));
        let on_axis = resolve_at(&sketch, Point2::new(-0.5, -12.0)).unwrap();
        assert_eq!(on_axis.target, Target::Curve(EntityId::VERTICAL_AXIS));
        assert_eq!(on_axis.position, Point2::new(0.0, -12.0));
    }

    #[test]
    fn curves_project_the_pointer_onto_themselves() {
        let mut sketch = Sketch::new(Plane::XY);
        let circle = sketch.add_circle(Point2::new(40.0, 40.0), 10.0);
        let arc = sketch.add_arc(
            Point2::new(-40.0, 40.0),
            Point2::new(-30.0, 40.0),
            Point2::new(-40.0, 50.0),
        );

        let on_circle = resolve_at(&sketch, Point2::new(40.0, 29.6)).unwrap();
        assert_eq!(on_circle.target, Target::Curve(circle));
        assert!(on_circle.position.distance(Point2::new(40.0, 30.0)) < 1e-12);

        let diagonal = Point2::new(-40.0, 40.0) + Vector2::splat(10.3 / 2f64.sqrt());
        let on_arc = resolve_at(&sketch, diagonal).unwrap();
        assert_eq!(on_arc.target, Target::Curve(arc));
        assert!((on_arc.position.distance(Point2::new(-40.0, 40.0)) - 10.0).abs() < 1e-12);

        assert_eq!(resolve_at(&sketch, Point2::new(-40.0, 29.7)), None);
    }

    #[test]
    fn pending_points_come_first_and_the_nearest_point_wins() {
        let mut sketch = Sketch::new(Plane::XY);
        let near = sketch.add_point(Point2::new(20.0, 0.3));
        sketch.add_point(Point2::new(20.6, 0.0));

        let snapped = resolve_at(&sketch, Point2::new(20.0, 0.2)).unwrap();
        assert_eq!(snapped.target, Target::Point(near));

        let pending = resolve(
            &sketch,
            &Scaled(10.0),
            pointer_at(Point2::new(20.0, 0.2)),
            &[(3, Point2::new(20.5, 0.5))],
            Accept::Anything,
        );
        assert_eq!(
            pending.map(|snapped| snapped.target),
            Some(Target::Pending(3))
        );
    }
    #[test]
    fn a_point_on_a_circle_snaps_only_to_its_points_and_crossings() {
        let mut sketch = Sketch::new(Plane::XY);
        let on = sketch.add_point(Point2::new(40.0, 50.0));
        sketch.add_point(Point2::new(47.5, 47.0));
        let line = sketch.add_line(Point2::new(46.0, 20.0), Point2::new(46.0, 60.0));
        let arc = sketch.add_arc(
            Point2::new(60.0, 40.0),
            Point2::new(75.0, 40.0),
            Point2::new(45.0, 40.0),
        );
        let circle = Accept::OnCircle {
            center: Point2::new(40.0, 40.0),
            radius: 10.0,
        };
        let at = |point| resolve(&sketch, &Scaled(10.0), pointer_at(point), &[], circle);
        let upper = Point2::new(46.875, 40.0 + (100.0f64 - 6.875 * 6.875).sqrt());
        let lower = Point2::new(upper.x, 80.0 - upper.y);

        let point = at(Point2::new(40.3, 50.2)).unwrap();
        let crossing = at(Point2::new(46.1, 48.3)).unwrap();
        let around = Accept::OnCircle {
            center: Point2::new(5.0, 3.0),
            radius: 5.0,
        };
        let axis_crossing = resolve(
            &sketch,
            &Scaled(10.0),
            pointer_at(Point2::new(9.2, 0.3)),
            &[],
            around,
        )
        .unwrap();

        assert_eq!(point.target, Target::Point(on));
        assert_eq!(at(Point2::new(47.5, 47.0)), None);
        assert_eq!(crossing.target, Target::Curve(line));
        assert!(crossing.position.distance(Point2::new(46.0, 48.0)) < 1e-12);
        assert_eq!(
            at(upper).map(|snapped| snapped.target),
            Some(Target::Curve(arc))
        );
        assert!(at(upper).unwrap().position.distance(upper) < 1e-9);
        assert_eq!(at(lower), None);
        assert_eq!(
            axis_crossing.target,
            Target::Curve(EntityId::HORIZONTAL_AXIS)
        );
        assert!(axis_crossing.position.distance(Point2::new(9.0, 0.0)) < 1e-12);
    }

    #[test]
    fn a_rim_snaps_to_points_but_not_to_curves() {
        let mut sketch = Sketch::new(Plane::XY);
        let point = sketch.add_point(Point2::new(30.0, 30.0));
        sketch.add_line(Point2::new(10.0, 10.0), Point2::new(10.0, 50.0));
        let rim = |at| resolve(&sketch, &Scaled(10.0), pointer_at(at), &[], Accept::Points);

        assert_eq!(
            rim(Point2::new(30.2, 30.1)).map(|snapped| snapped.target),
            Some(Target::Point(point))
        );
        assert_eq!(rim(Point2::new(10.2, 30.0)), None);
    }

    #[test]
    fn a_ray_crosses_a_curve_nearest_the_pointer() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(20.0, 10.0));
        let circle = sketch.add_circle(Point2::new(50.0, 0.0), 10.0);
        let arc = sketch.add_arc(
            Point2::new(-50.0, 0.0),
            Point2::new(-40.0, 0.0),
            Point2::new(-60.0, 0.0),
        );
        let upward = |x: f64, curve, near| {
            crossing_along(&sketch, curve, Point2::new(x, -30.0), Vector2::Y, near)
        };

        assert_eq!(
            upward(5.0, line, Point2::new(5.0, 12.0)),
            Some(Point2::new(5.0, 10.0))
        );
        assert_eq!(upward(25.0, line, Point2::new(25.0, 10.0)), None);
        assert_eq!(
            upward(50.0, circle, Point2::new(50.0, -8.0)),
            Some(Point2::new(50.0, -10.0))
        );
        assert_eq!(
            upward(50.0, circle, Point2::new(50.0, 8.0)),
            Some(Point2::new(50.0, 10.0))
        );
        assert_eq!(
            upward(-50.0, arc, Point2::new(-50.0, -8.0)),
            Some(Point2::new(-50.0, 10.0))
        );
        assert_eq!(
            upward(7.0, EntityId::HORIZONTAL_AXIS, Point2::ZERO),
            Some(Point2::new(7.0, 0.0))
        );
        assert_eq!(upward(7.0, EntityId::VERTICAL_AXIS, Point2::ZERO), None);
    }
}
