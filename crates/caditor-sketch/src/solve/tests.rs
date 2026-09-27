use std::cell::Cell;

use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2};

use crate::{
    Constraint, ConstraintId, Entity, EntityId, EntityState, Redundancy, Sketch, SketchError,
    Solved,
};

const EXACT: f64 = 1e-9;

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn solve(sketch: &Sketch) -> Result<Solved, SketchError> {
    sketch.solve(&no_parameters, &|| false)
}

fn mm(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

fn degrees(value: f64) -> Expression {
    Expression::Measure(value, Unit::Degree)
}

fn ends(sketch: &Sketch, line: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(line) {
        Some(Entity::Line { start, end }) => (*start, *end),
        other => panic!("expected a line, found {other:?}"),
    }
}

fn center(sketch: &Sketch, curve: EntityId) -> EntityId {
    match sketch.entity(curve) {
        Some(Entity::Circle { center, .. } | Entity::Arc { center, .. }) => *center,
        other => panic!("expected a circle or an arc, found {other:?}"),
    }
}

fn add(sketch: &mut Sketch, constraint: Constraint) -> ConstraintId {
    sketch.add_constraint(constraint).unwrap()
}

fn at(solved: &Solved, point: EntityId) -> Point2 {
    solved.geometry.point(point).unwrap()
}

fn assert_near(actual: Point2, expected: Point2) {
    assert!(
        actual.distance(expected) < EXACT,
        "{actual} is not {expected}"
    );
}

fn assert_finite(solved: &Solved) {
    for (id, entity) in solved.geometry.entities() {
        match entity {
            Entity::Point(position) => assert!(position.is_finite(), "Point {id} is {position}"),
            Entity::Circle { radius, .. } => assert!(radius.is_finite() && *radius > 0.0),
            Entity::Line { .. } | Entity::Arc { .. } | Entity::Spline { .. } => {}
        }
    }
}

struct Rectangle {
    sketch: Sketch,
    corners: [EntityId; 4],
    lines: [EntityId; 4],
}

fn rectangle(corners: [Point2; 4]) -> Rectangle {
    let mut sketch = Sketch::new(Plane::XY);
    let [a, b, c, d] = corners;
    let lines = [
        sketch.add_line(a, b),
        sketch.add_line(b, c),
        sketch.add_line(c, d),
        sketch.add_line(d, a),
    ];
    let ends: Vec<(EntityId, EntityId)> = lines.iter().map(|line| ends(&sketch, *line)).collect();
    for index in 0..4 {
        add(
            &mut sketch,
            Constraint::Coincident(ends[index].1, ends[(index + 1) % 4].0),
        );
    }
    let [bottom, right, top, left] = lines;
    add(&mut sketch, Constraint::Horizontal(bottom));
    add(&mut sketch, Constraint::Vertical(right));
    add(&mut sketch, Constraint::Horizontal(top));
    add(&mut sketch, Constraint::Vertical(left));
    add(
        &mut sketch,
        Constraint::Distance {
            from: ends[0].0,
            to: ends[0].1,
            value: mm(40.0),
        },
    );
    add(
        &mut sketch,
        Constraint::Distance {
            from: ends[1].0,
            to: ends[1].1,
            value: mm(20.0),
        },
    );
    add(
        &mut sketch,
        Constraint::Coincident(ends[0].0, EntityId::ORIGIN),
    );
    Rectangle {
        sketch,
        corners: [ends[0].0, ends[1].0, ends[2].0, ends[3].0],
        lines,
    }
}

#[test]
fn a_rectangle_solves_exactly_and_is_fully_constrained() {
    let Rectangle {
        sketch,
        corners,
        lines,
    } = rectangle([
        Point2::new(0.5, -0.3),
        Point2::new(38.0, 1.0),
        Point2::new(41.0, 19.0),
        Point2::new(1.0, 21.0),
    ]);

    let solved = solve(&sketch).unwrap();

    let expected = [
        Point2::ZERO,
        Point2::new(40.0, 0.0),
        Point2::new(40.0, 20.0),
        Point2::new(0.0, 20.0),
    ];
    for (corner, expected) in corners.iter().zip(expected) {
        assert_near(at(&solved, *corner), expected);
    }
    let solution = &solved.solution;
    assert_eq!(solution.degrees_of_freedom(), 0);
    assert!(solution.is_fully_constrained());
    assert!(solution.redundancies().is_empty());
    for (entity, _) in sketch.entities() {
        assert_eq!(
            solution.entity_state(entity),
            Some(EntityState::FullyConstrained)
        );
    }
    assert_eq!(
        solution.entity_state(EntityId::ORIGIN),
        Some(EntityState::FullyConstrained)
    );
    assert_eq!(solved.geometry.entities().len(), sketch.entities().len());
    assert_eq!(
        solved.geometry.constraints().len(),
        sketch.constraints().len()
    );
    assert!(
        lines
            .iter()
            .all(|line| solved.geometry.entity(*line).is_some())
    );
}

#[test]
fn geometry_that_already_satisfies_its_constraints_does_not_move() {
    let exact = rectangle([
        Point2::ZERO,
        Point2::new(40.0, 0.0),
        Point2::new(40.0, 20.0),
        Point2::new(0.0, 20.0),
    ]);
    assert_eq!(solve(&exact.sketch).unwrap().geometry, exact.sketch);

    let mut loose = Sketch::new(Plane::XY);
    let line = loose.add_line(Point2::new(3.25, 7.5), Point2::new(9.125, 7.5));
    add(&mut loose, Constraint::Horizontal(line));
    loose.add_circle(Point2::new(-4.0, 1.0), 2.5);
    let solved = solve(&loose).unwrap();
    assert_eq!(solved.geometry, loose);
    assert_eq!(solved.solution.degrees_of_freedom(), 3 + 3);
}

#[test]
fn degrees_of_freedom_count_what_is_left_to_fix() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 1.0));
    let solved = solve(&sketch).unwrap();
    assert_eq!(solved.solution.degrees_of_freedom(), 4);
    assert_eq!(
        solved.solution.entity_state(line),
        Some(EntityState::UnderConstrained)
    );
    add(&mut sketch, Constraint::Horizontal(line));
    assert_eq!(solve(&sketch).unwrap().solution.degrees_of_freedom(), 3);

    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(3.0, 4.0), 5.0);
    add(
        &mut sketch,
        Constraint::Radius {
            entity: circle,
            value: mm(6.0),
        },
    );
    let solved = solve(&sketch).unwrap();
    assert_eq!(solved.solution.degrees_of_freedom(), 2);
    assert_eq!(
        solved.solution.entity_state(circle),
        Some(EntityState::UnderConstrained)
    );
    let middle = center(&sketch, circle);
    add(
        &mut sketch,
        Constraint::Coincident(middle, EntityId::ORIGIN),
    );
    let solved = solve(&sketch).unwrap();
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
    assert_eq!(
        solved.solution.entity_state(circle),
        Some(EntityState::FullyConstrained)
    );
    assert_eq!(solved.geometry.circle(circle), Some((Point2::ZERO, 6.0)));

    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_arc(Point2::ZERO, Point2::new(5.0, 0.0), Point2::new(0.0, 4.0));
    let solved = solve(&sketch).unwrap();
    assert_eq!(solved.solution.degrees_of_freedom(), 5);
}

#[test]
fn a_fixed_point_on_a_free_line_is_fully_constrained_while_the_line_is_not() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(1.0, 1.0), Point2::new(10.0, 2.0));
    let (start, end) = ends(&sketch, line);
    add(&mut sketch, Constraint::Coincident(start, EntityId::ORIGIN));
    let solved = solve(&sketch).unwrap();
    assert_eq!(solved.solution.degrees_of_freedom(), 2);
    assert_eq!(
        solved.solution.entity_state(start),
        Some(EntityState::FullyConstrained)
    );
    assert_eq!(
        solved.solution.entity_state(end),
        Some(EntityState::UnderConstrained)
    );
    assert_eq!(
        solved.solution.entity_state(line),
        Some(EntityState::UnderConstrained)
    );
    assert_near(at(&solved, start), Point2::ZERO);
    assert_near(at(&solved, end), Point2::new(10.0, 2.0));
}

#[test]
fn changing_a_distance_moves_the_geometry_as_little_as_possible() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let (start, end) = ends(&sketch, line);
    add(&mut sketch, Constraint::Horizontal(line));
    let distance = add(
        &mut sketch,
        Constraint::Distance {
            from: start,
            to: end,
            value: mm(40.0),
        },
    );
    let solved = solve(&sketch).unwrap();
    assert_eq!(solved.geometry, sketch);

    sketch.set_dimension(distance, mm(80.0)).unwrap();
    let solved = solve(&sketch).unwrap();
    assert_near(at(&solved, start), Point2::new(-20.0, 0.0));
    assert_near(at(&solved, end), Point2::new(60.0, 0.0));
    assert_eq!(solved.solution.dimension(distance), Some(80.0));

    add(&mut sketch, Constraint::Coincident(start, EntityId::ORIGIN));
    let solved = solve(&sketch).unwrap();
    assert_near(at(&solved, start), Point2::ZERO);
    assert_near(at(&solved, end), Point2::new(80.0, 0.0));
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
}

#[test]
fn a_line_becomes_tangent_to_a_circle_and_an_arc_on_its_own_side() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::ZERO, 5.0);
    let middle = center(&sketch, circle);
    add(
        &mut sketch,
        Constraint::Coincident(middle, EntityId::ORIGIN),
    );
    add(
        &mut sketch,
        Constraint::Radius {
            entity: circle,
            value: mm(5.0),
        },
    );
    let above = sketch.add_line(Point2::new(-10.0, 5.5), Point2::new(10.0, 5.2));
    add(&mut sketch, Constraint::Horizontal(above));
    add(&mut sketch, Constraint::Tangent(above, circle));
    let below = sketch.add_line(Point2::new(-10.0, -4.0), Point2::new(10.0, -4.5));
    add(
        &mut sketch,
        Constraint::Parallel(below, EntityId::HORIZONTAL_AXIS),
    );
    add(&mut sketch, Constraint::Tangent(circle, below));

    let solved = solve(&sketch).unwrap();
    let (start, end) = ends(&sketch, above);
    assert!((at(&solved, start).y - 5.0).abs() < EXACT);
    assert!((at(&solved, end).y - 5.0).abs() < EXACT);
    let (start, end) = ends(&sketch, below);
    assert!((at(&solved, start).y + 5.0).abs() < EXACT);
    assert!((at(&solved, end).y + 5.0).abs() < EXACT);

    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_arc(Point2::ZERO, Point2::new(4.0, 0.0), Point2::new(0.0, 4.5));
    let side = sketch.add_line(Point2::new(4.6, -3.0), Point2::new(4.2, 3.0));
    add(&mut sketch, Constraint::Vertical(side));
    add(&mut sketch, Constraint::Tangent(side, arc));
    let solved = solve(&sketch).unwrap();
    let (middle, radius) = solved.geometry.circle(arc).unwrap();
    let arc_end = match sketch.entity(arc) {
        Some(Entity::Arc { end, .. }) => *end,
        _ => panic!("expected an arc"),
    };
    assert!((at(&solved, arc_end).distance(middle) - radius).abs() < EXACT);
    let (start, end) = ends(&sketch, side);
    assert!((at(&solved, start).x - middle.x - radius).abs() < EXACT);
    assert!((at(&solved, end).x - middle.x - radius).abs() < EXACT);
}

#[test]
fn circles_touch_from_outside_or_inside_as_drawn() {
    let mut sketch = Sketch::new(Plane::XY);
    let big = sketch.add_circle(Point2::ZERO, 10.0);
    let outside = sketch.add_circle(Point2::new(14.0, 1.0), 3.0);
    let inside = sketch.add_circle(Point2::new(5.0, 0.0), 4.0);
    add(&mut sketch, Constraint::Tangent(big, outside));
    add(&mut sketch, Constraint::Tangent(inside, big));
    let solved = solve(&sketch).unwrap();
    let circle = |id| solved.geometry.circle(id).unwrap();
    let ((big_center, big_radius), (outer_center, outer_radius)) = (circle(big), circle(outside));
    assert!((big_center.distance(outer_center) - (big_radius + outer_radius)).abs() < EXACT);
    let (inner_center, inner_radius) = circle(inside);
    assert!((big_center.distance(inner_center) - (big_radius - inner_radius)).abs() < EXACT);
}

#[test]
fn an_angle_turns_the_second_line_counter_clockwise_from_the_first() {
    let mut sketch = Sketch::new(Plane::XY);
    let base = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    let (base_start, _) = ends(&sketch, base);
    add(&mut sketch, Constraint::Horizontal(base));
    add(
        &mut sketch,
        Constraint::Coincident(base_start, EntityId::ORIGIN),
    );
    let slope = sketch.add_line(Point2::new(1.0, 1.0), Point2::new(9.0, 3.0));
    let angle = add(
        &mut sketch,
        Constraint::Angle {
            from: base,
            to: slope,
            value: degrees(30.0),
        },
    );

    let solved = solve(&sketch).unwrap();
    let direction = solved.geometry.line_direction(slope).unwrap();
    assert!((direction.y.atan2(direction.x).to_degrees() - 30.0).abs() < 1e-7);
    assert_eq!(solved.solution.dimension(angle), Some(30.0));

    sketch.set_dimension(angle, degrees(-100.0)).unwrap();
    let solved = solve(&sketch).unwrap();
    let direction = solved.geometry.line_direction(slope).unwrap();
    assert!((direction.y.atan2(direction.x).to_degrees() + 100.0).abs() < 1e-7);
}

#[test]
fn equal_lines_and_radii_match() {
    let mut sketch = Sketch::new(Plane::XY);
    let short = sketch.add_line(Point2::ZERO, Point2::new(3.0, 0.0));
    let long = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(8.0, 6.0));
    let small = sketch.add_circle(Point2::new(20.0, 0.0), 1.0);
    let arc = sketch.add_arc(
        Point2::new(30.0, 0.0),
        Point2::new(33.0, 0.0),
        Point2::new(30.0, 3.0),
    );
    add(&mut sketch, Constraint::Equal(short, long));
    add(&mut sketch, Constraint::Equal(arc, small));
    let solved = solve(&sketch).unwrap();
    let length = |line| solved.geometry.line_direction(line).unwrap().length();
    assert!((length(short) - length(long)).abs() < EXACT);
    let radius = |curve| solved.geometry.circle(curve).unwrap().1;
    assert!((radius(small) - radius(arc)).abs() < EXACT);
}

#[test]
fn distances_to_lines_and_points_on_curves_hold() {
    let mut sketch = Sketch::new(Plane::XY);
    let point = sketch.add_point(Point2::new(2.0, 3.0));
    add(
        &mut sketch,
        Constraint::Distance {
            from: EntityId::HORIZONTAL_AXIS,
            to: point,
            value: mm(7.0),
        },
    );
    add(
        &mut sketch,
        Constraint::Coincident(EntityId::VERTICAL_AXIS, point),
    );
    let circle = sketch.add_circle(Point2::new(20.0, 0.0), 4.0);
    let on_circle = sketch.add_point(Point2::new(25.0, 1.0));
    add(&mut sketch, Constraint::Coincident(on_circle, circle));
    let solved = solve(&sketch).unwrap();
    assert_near(at(&solved, point), Point2::new(0.0, 7.0));
    let (middle, radius) = solved.geometry.circle(circle).unwrap();
    assert!((at(&solved, on_circle).distance(middle) - radius).abs() < EXACT);
}

#[test]
fn contradicting_directions_and_length_are_named_as_one_conflict() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    let (start, end) = ends(&sketch, line);
    let horizontal = add(&mut sketch, Constraint::Horizontal(line));
    let vertical = add(&mut sketch, Constraint::Vertical(line));
    let distance = add(
        &mut sketch,
        Constraint::Distance {
            from: start,
            to: end,
            value: mm(10.0),
        },
    );
    let unrelated = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(3.0, 9.0));
    add(&mut sketch, Constraint::Horizontal(unrelated));

    assert_eq!(
        solve(&sketch),
        Err(SketchError::Conflict {
            constraints: vec![horizontal, vertical, distance]
        })
    );
}

#[test]
fn two_different_distances_between_the_same_points_conflict() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let (start, end) = ends(&sketch, line);
    add(&mut sketch, Constraint::Horizontal(line));
    add(&mut sketch, Constraint::Coincident(start, EntityId::ORIGIN));
    let first = add(
        &mut sketch,
        Constraint::Distance {
            from: start,
            to: end,
            value: mm(40.0),
        },
    );
    let second = add(
        &mut sketch,
        Constraint::Distance {
            from: end,
            to: start,
            value: mm(80.0),
        },
    );
    assert_eq!(
        solve(&sketch),
        Err(SketchError::Conflict {
            constraints: vec![first, second]
        })
    );
}

#[test]
fn repeated_constraints_are_redundant_with_what_they_repeat() {
    let mut sketch = Sketch::new(Plane::XY);
    let first_line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    let second_line = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(10.0, 10.0));
    let (_, corner) = ends(&sketch, first_line);
    let (other_corner, _) = ends(&sketch, second_line);
    let original = add(&mut sketch, Constraint::Coincident(corner, other_corner));
    let horizontal = add(&mut sketch, Constraint::Horizontal(first_line));
    let repeated = add(&mut sketch, Constraint::Coincident(other_corner, corner));
    let parallel = add(
        &mut sketch,
        Constraint::Parallel(EntityId::HORIZONTAL_AXIS, first_line),
    );

    let solved = solve(&sketch).unwrap();
    assert_eq!(
        solved.solution.redundancies(),
        [
            Redundancy {
                constraint: repeated,
                duplicates: vec![original],
            },
            Redundancy {
                constraint: parallel,
                duplicates: vec![horizontal],
            },
        ]
    );
    assert_eq!(solved.solution.redundancy(original), None);
    assert_eq!(solved.solution.degrees_of_freedom(), 8 - 2 - 1);
}

#[test]
fn degenerate_geometry_never_produces_nan() {
    let mut sketch = Sketch::new(Plane::XY);
    let collapsed = sketch.add_line(Point2::new(1.0, 1.0), Point2::new(1.0, 1.0));
    let tilted = sketch.add_line(Point2::ZERO, Point2::new(3.0, 4.0));
    add(&mut sketch, Constraint::Parallel(collapsed, tilted));
    add(&mut sketch, Constraint::Perpendicular(tilted, collapsed));
    add(
        &mut sketch,
        Constraint::Angle {
            from: collapsed,
            to: EntityId::HORIZONTAL_AXIS,
            value: degrees(45.0),
        },
    );
    let outer = sketch.add_circle(Point2::new(20.0, 20.0), 5.0);
    let inner = sketch.add_circle(Point2::new(20.0, 20.0), 3.0);
    add(&mut sketch, Constraint::Tangent(outer, inner));
    let arc = sketch.add_arc(
        Point2::new(-5.0, -5.0),
        Point2::new(-5.0, -5.0),
        Point2::new(-5.0, -5.0),
    );
    add(&mut sketch, Constraint::Tangent(arc, tilted));
    let (a, b) = ends(&sketch, tilted);
    add(
        &mut sketch,
        Constraint::Distance {
            from: a,
            to: b,
            value: mm(0.0),
        },
    );
    let point = sketch.add_point(Point2::new(-5.0, -5.0));
    add(&mut sketch, Constraint::Coincident(point, arc));
    add(
        &mut sketch,
        Constraint::Distance {
            from: point,
            to: collapsed,
            value: mm(0.0),
        },
    );

    match solve(&sketch) {
        Ok(solved) => assert_finite(&solved),
        Err(error) => assert!(
            matches!(
                error,
                SketchError::Conflict { .. } | SketchError::Unsolvable
            ),
            "{error:?}"
        ),
    }

    let mut concentric = Sketch::new(Plane::XY);
    let outer = concentric.add_circle(Point2::ZERO, 5.0);
    let inner = concentric.add_circle(Point2::ZERO, 3.0);
    let centers = (center(&concentric, outer), center(&concentric, inner));
    add(
        &mut concentric,
        Constraint::Coincident(centers.0, centers.1),
    );
    add(&mut concentric, Constraint::Tangent(outer, inner));
    let solved = solve(&concentric).unwrap();
    assert_finite(&solved);
    let radius = |curve| solved.geometry.circle(curve).unwrap().1;
    assert!((radius(outer) - radius(inner)).abs() < EXACT);

    let mut zero = Sketch::new(Plane::XY);
    let line = zero.add_line(Point2::ZERO, Point2::new(4.0, 0.0));
    let (start, end) = ends(&zero, line);
    add(&mut zero, Constraint::Horizontal(line));
    add(
        &mut zero,
        Constraint::Distance {
            from: start,
            to: end,
            value: mm(0.0),
        },
    );
    let solved = solve(&zero).unwrap();
    assert_finite(&solved);
    assert_near(at(&solved, start), at(&solved, end));
    assert_eq!(solved.solution.degrees_of_freedom(), 2);
}

#[test]
fn a_cancelled_solve_reports_that_it_was_cancelled() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 3.0));
    add(&mut sketch, Constraint::Horizontal(line));
    let checks = Cell::new(0);
    let cancel_on_second_check = || {
        checks.set(checks.get() + 1);
        checks.get() > 1
    };
    assert_eq!(
        sketch.solve(&no_parameters, &cancel_on_second_check),
        Err(SketchError::Cancelled)
    );
    assert_eq!(
        sketch.solve(&no_parameters, &|| true),
        Err(SketchError::Cancelled)
    );
}

#[test]
fn dimension_errors_come_before_solving() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::ZERO, 1.0);
    let radius = add(
        &mut sketch,
        Constraint::Radius {
            entity: circle,
            value: Expression::Parameter(ParameterId::from_raw(3)),
        },
    );
    let error = sketch
        .solve(&|_: ParameterId| Ok(Quantity::length(-2.0)), &|| false)
        .unwrap_err();
    assert!(matches!(error, SketchError::Dimension { constraint, .. } if constraint == radius));
}
