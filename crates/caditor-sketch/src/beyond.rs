use std::f64::consts::TAU;

use caditor_geometry::{Point2, Vector2};

use crate::{constraint::Constraint, entity::Entity, id::EntityId, sketch::Sketch};

const BEYOND_TOLERANCE: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PointBeyond {
    pub point: EntityId,
    pub curve: EntityId,
}

impl Sketch {
    pub fn points_beyond_curves(&self) -> Vec<PointBeyond> {
        let mut beyond: Vec<PointBeyond> = self
            .active_constraints()
            .filter_map(|(_, constraint)| match *constraint {
                Constraint::Coincident(a, b) => match (self.entity(a), self.entity(b)) {
                    (Some(Entity::Point(_)), Some(_)) => Some((a, b)),
                    (Some(_), Some(Entity::Point(_))) => Some((b, a)),
                    _ => None,
                },
                _ => None,
            })
            .filter(|(point, curve)| self.lies_beyond(*point, *curve))
            .map(|(point, curve)| PointBeyond { point, curve })
            .collect();
        beyond.sort_unstable();
        beyond.dedup();
        beyond
    }

    pub fn nearest_end(&self, beyond: PointBeyond) -> Option<Point2> {
        let at = self.point(beyond.point)?;
        let (start, end) = match self.entity(beyond.curve)? {
            Entity::Line { start, end } | Entity::Arc { start, end, .. } => (*start, *end),
            Entity::Point(_) | Entity::Circle { .. } | Entity::Spline { .. } => return None,
        };
        let (start, end) = (self.point(start)?, self.point(end)?);
        Some(if at.distance(start) <= at.distance(end) {
            start
        } else {
            end
        })
    }

    fn lies_beyond(&self, point: EntityId, curve: EntityId) -> bool {
        if curve.is_reference() || self.is_construction(curve) {
            return false;
        }
        let Some(at) = self.point(point) else {
            return false;
        };
        match self.entity(curve) {
            Some(Entity::Line { start, end }) if *start != point && *end != point => self
                .line_endpoints(curve)
                .is_some_and(|(start, end)| beyond_segment(at, start, end)),
            Some(Entity::Arc { start, end, .. }) if *start != point && *end != point => {
                self.arc(curve).is_some_and(|arc| {
                    let past_start =
                        (direction_angle(at - arc.center) - arc.start_angle).rem_euclid(TAU);
                    let outside = past_start - arc.sweep;
                    let back_to_start = TAU - past_start;
                    outside > 0.0 && outside.min(back_to_start) * arc.radius > BEYOND_TOLERANCE
                })
            }
            _ => false,
        }
    }
}

fn beyond_segment(at: Point2, start: Point2, end: Point2) -> bool {
    let along = end - start;
    let length = along.length();
    if length <= BEYOND_TOLERANCE {
        return false;
    }
    let reach = (at - start).dot(along) / length;
    reach < -BEYOND_TOLERANCE || reach > length + BEYOND_TOLERANCE
}

fn direction_angle(vector: Vector2) -> f64 {
    vector.y.atan2(vector.x)
}

#[cfg(test)]
mod tests {
    use caditor_geometry::{Plane, Point2};

    use super::*;

    #[test]
    fn a_point_on_a_line_past_its_end_is_beyond_it_and_one_between_its_ends_is_not() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        let inside = sketch.add_point(Point2::new(4.0, 0.0));
        let past = sketch.add_point(Point2::new(13.0, 0.0));
        let before = sketch.add_point(Point2::new(-2.0, 0.0));
        sketch
            .add_constraint(Constraint::Coincident(inside, line))
            .unwrap();
        sketch
            .add_constraint(Constraint::Coincident(line, past))
            .unwrap();
        sketch
            .add_constraint(Constraint::Coincident(before, line))
            .unwrap();

        let beyond = sketch.points_beyond_curves();

        assert_eq!(
            beyond,
            vec![
                PointBeyond {
                    point: past,
                    curve: line
                },
                PointBeyond {
                    point: before,
                    curve: line
                },
            ]
        );
        assert_eq!(sketch.nearest_end(beyond[0]), Some(Point2::new(10.0, 0.0)));
        assert_eq!(sketch.nearest_end(beyond[1]), Some(Point2::ZERO));
    }

    #[test]
    fn a_point_on_an_arc_outside_its_sweep_is_beyond_it() {
        let mut sketch = Sketch::new(Plane::XY);
        let arc = sketch.add_arc(Point2::ZERO, Point2::new(5.0, 0.0), Point2::new(0.0, 5.0));
        let inside = sketch.add_point(Point2::new(5.0, 5.0).normalize() * 5.0);
        let outside = sketch.add_point(Point2::new(-5.0, 0.0));
        sketch
            .add_constraint(Constraint::Coincident(inside, arc))
            .unwrap();
        sketch
            .add_constraint(Constraint::Coincident(outside, arc))
            .unwrap();

        assert_eq!(
            sketch.points_beyond_curves(),
            vec![PointBeyond {
                point: outside,
                curve: arc,
            }]
        );
    }

    #[test]
    fn construction_lines_axes_and_disabled_constraints_are_left_out() {
        let mut sketch = Sketch::new(Plane::XY);
        let centreline = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        sketch.set_construction(centreline, true).unwrap();
        let line = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(10.0, 5.0));
        let point = sketch.add_point(Point2::new(20.0, 0.0));
        let other = sketch.add_point(Point2::new(30.0, 5.0));
        sketch
            .add_constraint(Constraint::Coincident(point, centreline))
            .unwrap();
        sketch
            .add_constraint(Constraint::Coincident(point, EntityId::HORIZONTAL_AXIS))
            .unwrap();
        let disabled = sketch
            .add_constraint(Constraint::Coincident(other, line))
            .unwrap();
        sketch.set_active(disabled, false).unwrap();

        assert!(sketch.points_beyond_curves().is_empty());
    }
}
