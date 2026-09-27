use std::f64::consts::TAU;

use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{Entity, EntityId, Sketch};

pub const POINT_TOLERANCE: f64 = 8.0;
pub const CURVE_TOLERANCE: f64 = 6.0;

pub trait Screen {
    fn to_screen(&self, point: Point2) -> Option<Vector2>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Pending(usize),
    Point(EntityId),
    Curve(EntityId),
}

impl Target {
    pub fn entity(self) -> Option<EntityId> {
        match self {
            Self::Pending(_) => None,
            Self::Point(entity) | Self::Curve(entity) => Some(entity),
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

pub fn resolve(
    sketch: &Sketch,
    screen: &impl Screen,
    pointer: Pointer,
    pending: &[(usize, Point2)],
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
    nearest(pending, POINT_TOLERANCE)
        .or_else(|| nearest(points(sketch), POINT_TOLERANCE))
        .or_else(|| nearest(curves(sketch, pointer.sketch), CURVE_TOLERANCE))
}

fn points(sketch: &Sketch) -> Vec<Snapped> {
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
            let offset = position - arc.center;
            let turned = (offset.y.atan2(offset.x) - arc.start_angle).rem_euclid(TAU);
            (turned <= arc.sweep).then_some(position)
        }
        Entity::Point(_) | Entity::Spline { .. } => None,
    }
}

fn closest_on_segment(start: Point2, end: Point2, at: Point2) -> Point2 {
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
        resolve(sketch, &Scaled(10.0), pointer_at(at), &[])
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

        let on_line = resolve_at(&sketch, Point2::new(20.0, 10.5)).unwrap();
        assert_eq!(on_line.target, Target::Curve(line));
        assert_eq!(on_line.position, Point2::new(20.0, 10.0));

        assert_eq!(resolve_at(&sketch, Point2::new(20.0, 10.7)), None);
        assert_eq!(resolve_at(&sketch, Point2::new(10.0, 10.9)), None);
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
        );
        assert_eq!(
            pending.map(|snapped| snapped.target),
            Some(Target::Pending(3))
        );
    }
}
