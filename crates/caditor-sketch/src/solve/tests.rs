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
            reversed: false,
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
            constraints: vec![horizontal, vertical]
        })
    );
    sketch.remove_constraint(vertical).unwrap();
    assert!(solve(&sketch).is_ok());
    let _ = distance;
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
            reversed: false,
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
                SketchError::Conflict { .. }
                    | SketchError::Unsolvable { .. }
                    | SketchError::NoLength { .. }
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
    let collapse = add(
        &mut zero,
        Constraint::Distance {
            from: start,
            to: end,
            value: mm(0.0),
        },
    );
    assert_eq!(
        solve(&zero),
        Err(SketchError::Conflict {
            constraints: vec![collapse]
        })
    );
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

#[test]
fn a_zero_distance_between_separate_points_joins_them_with_full_rank() {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_point(Point2::new(1.0, 2.0));
    let second = sketch.add_point(Point2::new(4.0, 6.0));
    add(
        &mut sketch,
        Constraint::Distance {
            from: first,
            to: second,
            value: mm(0.0),
        },
    );
    let solved = solve(&sketch).unwrap();
    assert_near(at(&solved, first), at(&solved, second));
    assert_eq!(solved.solution.degrees_of_freedom(), 2);
    assert!(solved.solution.redundancies().is_empty());
}

#[test]
fn horizontal_on_a_nearly_vertical_line_is_a_conflict_not_a_collapse() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(0.001, 10.0));
    let (start, _) = ends(&sketch, line);
    add(&mut sketch, Constraint::Coincident(start, EntityId::ORIGIN));
    add(&mut sketch, Constraint::Vertical(line));
    let horizontal = add(&mut sketch, Constraint::Horizontal(line));
    let Err(SketchError::Conflict { constraints }) = solve(&sketch) else {
        panic!("the line would collapse");
    };
    assert!(constraints.contains(&horizontal), "{constraints:?}");
}

#[test]
fn a_tangent_at_a_line_arc_joint_adds_one_degree_of_constraint() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(-10.0, 5.0), Point2::new(0.0, 5.0));
    let arc = sketch.add_arc(
        Point2::new(0.2, 0.1),
        Point2::new(0.0, 5.0),
        Point2::new(5.0, 0.0),
    );
    let (_, line_end) = ends(&sketch, line);
    let Some(Entity::Arc {
        start: arc_start, ..
    }) = sketch.entity(arc).cloned()
    else {
        panic!("expected an arc");
    };
    add(&mut sketch, Constraint::Coincident(line_end, arc_start));
    let before = solve(&sketch).unwrap().solution.degrees_of_freedom();
    let tangent = add(&mut sketch, Constraint::Tangent(line, arc));
    let solved = solve(&sketch).unwrap();
    assert_eq!(solved.solution.degrees_of_freedom(), before - 1);
    assert!(solved.solution.redundancy(tangent).is_none());
    let (start, end) = solved.geometry.line_endpoints(line).unwrap();
    let center = solved.geometry.arc(arc).unwrap().center;
    let direction = (end - start).normalize();
    assert!(direction.dot((center - end).normalize()).abs() < 1e-9);
}

#[test]
fn internal_tangency_follows_whichever_circle_becomes_larger() {
    let mut sketch = Sketch::new(Plane::XY);
    let outer = sketch.add_circle(Point2::ZERO, 5.0);
    let inner = sketch.add_circle(Point2::new(2.0, 0.0), 3.0);
    add(&mut sketch, Constraint::Tangent(outer, inner));
    let outer_center = center(&sketch, outer);
    add(
        &mut sketch,
        Constraint::Coincident(outer_center, EntityId::ORIGIN),
    );
    let outer_radius = add(
        &mut sketch,
        Constraint::Radius {
            entity: outer,
            value: mm(5.0),
        },
    );
    add(
        &mut sketch,
        Constraint::Radius {
            entity: inner,
            value: mm(3.0),
        },
    );
    assert!(solve(&sketch).is_ok());
    sketch.set_dimension(outer_radius, mm(1.0)).unwrap();
    let solved = solve(&sketch).unwrap();
    let (outer_middle, outer_size) = solved.geometry.circle(outer).unwrap();
    let (inner_middle, inner_size) = solved.geometry.circle(inner).unwrap();
    assert!((outer_middle.distance(inner_middle) - (inner_size - outer_size)).abs() < 1e-6);
}

#[test]
fn a_reversed_angle_holds_the_corner_between_a_chain_of_lines() {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    let second = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(6.0, 7.0));
    let (first_start, first_end) = ends(&sketch, first);
    let (second_start, _) = ends(&sketch, second);
    add(&mut sketch, Constraint::Coincident(first_end, second_start));
    add(
        &mut sketch,
        Constraint::Coincident(first_start, EntityId::ORIGIN),
    );
    add(&mut sketch, Constraint::Horizontal(first));
    add(
        &mut sketch,
        Constraint::Angle {
            from: first,
            to: second,
            reversed: true,
            value: degrees(60.0),
        },
    );
    let solved = solve(&sketch).unwrap();
    let (corner, far) = solved.geometry.line_endpoints(second).unwrap();
    let (start, _) = solved.geometry.line_endpoints(first).unwrap();
    let back = (start - corner).normalize();
    let along = (far - corner).normalize();
    assert!((back.angle_to(along).abs().to_degrees() - 60.0).abs() < 1e-6);
}

fn chain(count: usize) -> (Sketch, Vec<EntityId>) {
    let mut sketch = Sketch::new(Plane::XY);
    let mut lines = Vec::with_capacity(count);
    let mut previous_end = None;
    for index in 0..count {
        let x = index as f64;
        let line = sketch.add_line(
            Point2::new(x + 0.01, (index % 3) as f64 * 0.01),
            Point2::new(x + 1.02, 0.02),
        );
        let (start, end) = ends(&sketch, line);
        match previous_end {
            Some(previous) => {
                add(&mut sketch, Constraint::Coincident(previous, start));
            }
            None => {
                add(&mut sketch, Constraint::Coincident(start, EntityId::ORIGIN));
            }
        }
        add(&mut sketch, Constraint::Horizontal(line));
        add(
            &mut sketch,
            Constraint::Distance {
                from: start,
                to: end,
                value: mm(1.0),
            },
        );
        previous_end = Some(end);
        lines.push(line);
    }
    (sketch, lines)
}

#[test]
fn a_long_chain_solves_and_analyses_within_its_time_budget() {
    let (sketch, lines) = chain(400);
    let started = std::time::Instant::now();
    let solved = solve(&sketch).unwrap();
    assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
    assert!(solved.solution.is_fully_constrained());
    let (_, end) = solved.geometry.line_endpoints(lines[399]).unwrap();
    assert_near(end, Point2::new(400.0, 0.0));
    assert!(
        lines
            .iter()
            .all(|line| solved.solution.entity_state(*line) == Some(EntityState::FullyConstrained))
    );
}

#[test]
fn large_parts_report_freedoms_and_redundancies_like_small_ones() {
    let (mut sketch, lines) = chain(15);
    let horizontal = sketch
        .constraints()
        .find_map(|(id, constraint)| {
            (*constraint == Constraint::Horizontal(lines[7])).then_some(id)
        })
        .unwrap();
    sketch.remove_constraint(horizontal).unwrap();
    let solved = solve(&sketch).unwrap();
    assert_eq!(solved.solution.degrees_of_freedom(), 1);
    assert_eq!(
        solved.solution.entity_state(lines[3]),
        Some(EntityState::FullyConstrained)
    );
    assert_eq!(
        solved.solution.entity_state(lines[7]),
        Some(EntityState::UnderConstrained)
    );

    let (mut sketch, lines) = chain(15);
    let parallel = add(&mut sketch, Constraint::Parallel(lines[4], lines[5]));
    let solved = solve(&sketch).unwrap();
    assert!(solved.solution.is_fully_constrained());
    let redundancy = solved.solution.redundancy(parallel).unwrap();
    assert!(!redundancy.duplicates.is_empty());
}

#[test]
fn a_conflict_at_the_end_of_a_long_chain_is_found_quickly() {
    let (mut sketch, lines) = chain(120);
    let vertical = add(&mut sketch, Constraint::Vertical(lines[119]));
    let started = std::time::Instant::now();
    let Err(SketchError::Conflict { constraints }) = solve(&sketch) else {
        panic!("a line cannot be horizontal and vertical");
    };
    assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
    assert!(constraints.contains(&vertical));
    assert!(constraints.len() <= 3, "{constraints:?}");
}

fn drag(sketch: &Sketch, drags: &[crate::Drag]) -> Solved {
    sketch
        .solve_dragging(&no_parameters, &|| false, drags)
        .unwrap()
}

#[test]
fn a_dragged_point_goes_where_its_constraints_let_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let free = sketch.add_point(Point2::new(1.0, 1.0));
    let solved = drag(
        &sketch,
        &[crate::Drag::Point {
            point: free,
            to: Point2::new(4.0, -2.0),
        }],
    );
    assert_near(at(&solved, free), Point2::new(4.0, -2.0));

    let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    let (start, end) = ends(&sketch, line);
    add(&mut sketch, Constraint::Coincident(start, EntityId::ORIGIN));
    add(
        &mut sketch,
        Constraint::Distance {
            from: start,
            to: end,
            value: mm(10.0),
        },
    );
    let solved = drag(
        &sketch,
        &[crate::Drag::Point {
            point: end,
            to: Point2::new(3.0, 4.0),
        }],
    );
    assert_near(at(&solved, start), Point2::ZERO);
    assert_near(at(&solved, end), Point2::new(6.0, 8.0));
}

#[test]
fn a_dragged_radius_grows_the_circle_unless_a_dimension_holds_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let free = sketch.add_circle(Point2::new(2.0, 3.0), 4.0);
    let held = sketch.add_circle(Point2::new(20.0, 0.0), 4.0);
    let free_centre = center(&sketch, free);
    add(
        &mut sketch,
        Constraint::Coincident(free_centre, EntityId::ORIGIN),
    );
    add(
        &mut sketch,
        Constraint::Radius {
            entity: held,
            value: mm(4.0),
        },
    );

    let solved = drag(
        &sketch,
        &[
            crate::Drag::Radius {
                circle: free,
                to: 7.5,
            },
            crate::Drag::Radius {
                circle: held,
                to: 9.0,
            },
        ],
    );

    let (free_center, free_radius) = solved.geometry.circle(free).unwrap();
    let (held_center, held_radius) = solved.geometry.circle(held).unwrap();
    assert_near(free_center, Point2::ZERO);
    assert!((free_radius - 7.5).abs() < EXACT, "{free_radius}");
    assert!((held_radius - 4.0).abs() < EXACT, "{held_radius}");
    assert_near(held_center, Point2::new(20.0, 0.0));
}

#[test]
fn dragging_a_corner_of_a_free_rectangle_moves_the_rest_with_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        Point2::ZERO,
        Point2::new(4.0, 0.0),
        Point2::new(4.0, 2.0),
        Point2::new(0.0, 2.0),
    ];
    let lines: Vec<EntityId> = (0..4)
        .map(|index| sketch.add_line(corners[index], corners[(index + 1) % 4]))
        .collect();
    for index in 0..4 {
        let (_, end) = ends(&sketch, lines[index]);
        let (next_start, _) = ends(&sketch, lines[(index + 1) % 4]);
        add(&mut sketch, Constraint::Coincident(end, next_start));
    }
    add(&mut sketch, Constraint::Horizontal(lines[0]));
    add(&mut sketch, Constraint::Vertical(lines[1]));
    add(&mut sketch, Constraint::Horizontal(lines[2]));
    add(&mut sketch, Constraint::Vertical(lines[3]));
    let (first, second) = ends(&sketch, lines[0]);
    add(
        &mut sketch,
        Constraint::Distance {
            from: first,
            to: second,
            value: mm(4.0),
        },
    );
    let (side_start, side_end) = ends(&sketch, lines[1]);
    add(
        &mut sketch,
        Constraint::Distance {
            from: side_start,
            to: side_end,
            value: mm(2.0),
        },
    );
    let solved = drag(
        &sketch,
        &[crate::Drag::Point {
            point: first,
            to: Point2::new(3.0, 2.0),
        }],
    );
    assert_near(at(&solved, first), Point2::new(3.0, 2.0));
    let (_, far) = solved.geometry.line_endpoints(lines[1]).unwrap();
    assert_near(far, Point2::new(7.0, 4.0));
}

#[test]
fn a_start_that_needs_perturbing_keeps_the_tangency_it_was_drawn_with() {
    let mut sketch = Sketch::new(Plane::XY);
    let base = sketch.add_line(Point2::ZERO, Point2::new(4.0, 0.0));
    let (a, b) = ends(&sketch, base);
    add(&mut sketch, Constraint::Coincident(a, EntityId::ORIGIN));
    let c = sketch.add_point(Point2::new(2.0, 0.0));
    for (from, to, length) in [(a, b, 4.0), (b, c, 3.0), (a, c, 5.0)] {
        add(
            &mut sketch,
            Constraint::Distance {
                from,
                to,
                value: mm(length),
            },
        );
    }
    let first = sketch.add_circle(Point2::new(4.0, 0.0), 1.0);
    let second = sketch.add_circle(Point2::new(2.0, 0.0), 1.0);
    let first_center = center(&sketch, first);
    let second_center = center(&sketch, second);
    add(&mut sketch, Constraint::Coincident(first_center, b));
    add(&mut sketch, Constraint::Coincident(second_center, c));
    add(&mut sketch, Constraint::Tangent(first, second));

    let solved = solve(&sketch).unwrap();
    assert_finite(&solved);
    assert!((at(&solved, a).distance(at(&solved, b)) - 4.0).abs() < EXACT);
    assert!((at(&solved, b).distance(at(&solved, c)) - 3.0).abs() < EXACT);
    assert!((at(&solved, a).distance(at(&solved, c)) - 5.0).abs() < EXACT);
    let (_, first_radius) = solved.geometry.circle(first).unwrap();
    let (_, second_radius) = solved.geometry.circle(second).unwrap();
    assert!((first_radius + second_radius - 3.0).abs() < EXACT);
}

#[test]
fn an_edit_reanalyses_only_the_parts_it_touches() {
    let first = rectangle([
        Point2::new(0.0, 0.0),
        Point2::new(40.0, 0.0),
        Point2::new(40.0, 20.0),
        Point2::new(0.0, 20.0),
    ]);
    let mut sketch = first.sketch;
    let circle = sketch.add_circle(Point2::new(80.0, 10.0), 4.0);
    let radius = add(
        &mut sketch,
        Constraint::Radius {
            entity: circle,
            value: mm(4.0),
        },
    );
    let line = sketch.add_line(Point2::new(60.0, 30.0), Point2::new(70.0, 31.0));
    add(&mut sketch, Constraint::Horizontal(line));

    let fresh = solve(&sketch).unwrap();
    assert_eq!(fresh.memo.remembered(), 4);
    assert_eq!(fresh.memo.recalled(), 0);
    let mut settled = fresh.geometry.clone();
    let again = settled
        .solve_from(&no_parameters, &|| false, &[], Some(&fresh.memo))
        .unwrap();
    assert_eq!(again.memo.recalled(), 3);
    assert_eq!(again.solution, fresh.solution);
    assert_eq!(again.geometry, fresh.geometry);

    settled
        .set_dimension(radius, mm(6.0))
        .expect("the radius takes a new value");
    let edited = settled
        .solve_from(&no_parameters, &|| false, &[], Some(&again.memo))
        .unwrap();
    let from_scratch = solve(&settled).unwrap();
    assert_eq!(edited.memo.recalled(), 2);
    assert_eq!(edited.solution, from_scratch.solution);
    assert_eq!(edited.geometry, from_scratch.geometry);
    assert_eq!(edited.geometry.circle(circle).unwrap().1, 6.0);
}

#[test]
fn growing_the_outermost_part_keeps_every_other_part_remembered() {
    let Rectangle {
        sketch: mut settled,
        corners,
        ..
    } = rectangle([
        Point2::new(0.0, 0.0),
        Point2::new(40.0, 0.0),
        Point2::new(40.0, 20.0),
        Point2::new(0.0, 20.0),
    ]);
    let width = settled
        .constraints()
        .find_map(|(id, constraint)| match constraint {
            Constraint::Distance { value, .. } if *value == mm(40.0) => Some(id),
            _ => None,
        })
        .unwrap();
    let line = settled.add_line(Point2::new(5.0, 30.0), Point2::new(15.0, 31.0));
    let (start, end) = ends(&settled, line);
    add(&mut settled, Constraint::Horizontal(line));
    add(
        &mut settled,
        Constraint::Distance {
            from: start,
            to: end,
            value: mm(10.0),
        },
    );

    let fresh = solve(&settled).unwrap();
    settled = fresh.geometry.clone();
    let again = settled
        .solve_from(&no_parameters, &|| false, &[], Some(&fresh.memo))
        .unwrap();
    settled
        .set_dimension(width, mm(60.0))
        .expect("the width takes a new value");
    let edited = settled
        .solve_from(&no_parameters, &|| false, &[], Some(&again.memo))
        .unwrap();
    let from_scratch = solve(&settled).unwrap();
    let dragged_to = crate::Drag::Point {
        point: corners[2],
        to: Point2::new(90.0, 45.0),
    };
    let dragged = settled
        .solve_from(&no_parameters, &|| false, &[dragged_to], Some(&edited.memo))
        .unwrap();
    let dragged_from_scratch = settled
        .solve_dragging(&no_parameters, &|| false, &[dragged_to])
        .unwrap();

    assert_eq!(again.memo.recalled(), 2);
    assert_eq!(edited.memo.recalled(), 1);
    assert_eq!(edited.solution, from_scratch.solution);
    assert_eq!(edited.geometry, from_scratch.geometry);
    assert_near(at(&edited, corners[2]), Point2::new(60.0, 20.0));
    assert_near(at(&edited, end), Point2::new(15.0, 30.5));
    assert_eq!(dragged.memo.recalled(), 1);
    assert_eq!(dragged.solution, dragged_from_scratch.solution);
    assert_eq!(dragged.geometry, dragged_from_scratch.geometry);
}

#[test]
fn a_line_that_starts_with_no_length_is_named_instead_of_its_constraints() {
    let mut pinned = Sketch::new(Plane::XY);
    let line = pinned.add_line(Point2::ZERO, Point2::ZERO);
    let (start, _) = ends(&pinned, line);
    pinned
        .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
        .unwrap();
    let mut free = Sketch::new(Plane::XY);
    let free_line = free.add_line(Point2::new(1.0, 1.0), Point2::new(1.0, 1.0));
    free.add_constraint(Constraint::Horizontal(free_line))
        .unwrap();

    let pinned = solve(&pinned).err();
    let free = solve(&free);

    assert_eq!(
        pinned,
        Some(SketchError::NoLength {
            entity: line,
            label: "Line 2".to_owned()
        })
    );
    assert!(free.is_ok());
}

fn arc_ends(sketch: &Sketch, arc: EntityId) -> (EntityId, EntityId, EntityId) {
    match sketch.entity(arc) {
        Some(Entity::Arc { center, start, end }) => (*center, *start, *end),
        other => panic!("expected an arc, found {other:?}"),
    }
}

struct ArcsAndLines {
    sketch: Sketch,
    lines: [EntityId; 4],
    arcs: [EntityId; 2],
    circle: EntityId,
}

fn arcs_joined_by_tangent_lines() -> ArcsAndLines {
    let mut sketch = Sketch::new(Plane::XY);
    let first_line = sketch.add_line(Point2::new(0.2, 0.1), Point2::new(9.8, -0.2));
    let first_arc = sketch.add_arc(
        Point2::new(10.3, 4.6),
        Point2::new(10.1, 0.2),
        Point2::new(14.6, 5.3),
    );
    let second_line = sketch.add_line(Point2::new(14.8, 5.1), Point2::new(15.3, 11.6));
    let circle = sketch.add_circle(Point2::new(21.0, 5.5), 4.0);
    let third_line = sketch.add_line(Point2::new(15.1, 12.2), Point2::new(9.9, 14.6));
    let second_arc = sketch.add_arc(
        Point2::new(7.0, 10.9),
        Point2::new(9.7, 15.2),
        Point2::new(3.2, 13.0),
    );
    let fourth_line = sketch.add_line(Point2::new(3.0, 13.4), Point2::new(1.6, 10.8));

    let (first_start, first_end) = ends(&sketch, first_line);
    let (_, first_arc_start, first_arc_end) = arc_ends(&sketch, first_arc);
    let (second_start, second_end) = ends(&sketch, second_line);
    let (third_start, third_end) = ends(&sketch, third_line);
    let (_, second_arc_start, second_arc_end) = arc_ends(&sketch, second_arc);
    let (fourth_start, fourth_end) = ends(&sketch, fourth_line);
    let circle_center = center(&sketch, circle);

    for constraint in [
        Constraint::Coincident(first_start, EntityId::ORIGIN),
        Constraint::Horizontal(first_line),
        Constraint::Distance {
            from: first_start,
            to: first_end,
            value: mm(10.0),
        },
        Constraint::Coincident(first_end, first_arc_start),
        Constraint::Tangent(first_line, first_arc),
        Constraint::Radius {
            entity: first_arc,
            value: mm(5.0),
        },
        Constraint::Coincident(first_arc_end, second_start),
        Constraint::Tangent(first_arc, second_line),
        Constraint::Vertical(second_line),
        Constraint::Distance {
            from: second_start,
            to: second_end,
            value: mm(7.0),
        },
        Constraint::Equal(first_arc, circle),
        Constraint::Tangent(second_line, circle),
        Constraint::Distance {
            from: circle_center,
            to: first_line,
            value: mm(5.0),
        },
        Constraint::Coincident(second_end, third_start),
        Constraint::Angle {
            from: second_line,
            to: third_line,
            reversed: false,
            value: degrees(60.0),
        },
        Constraint::Distance {
            from: third_start,
            to: third_end,
            value: mm(6.0),
        },
        Constraint::Coincident(third_end, second_arc_start),
        Constraint::Tangent(third_line, second_arc),
        Constraint::Equal(second_arc, first_arc),
        Constraint::Coincident(second_arc_end, fourth_start),
        Constraint::Tangent(second_arc, fourth_line),
        Constraint::Angle {
            from: third_line,
            to: fourth_line,
            reversed: false,
            value: degrees(90.0),
        },
        Constraint::Distance {
            from: fourth_start,
            to: fourth_end,
            value: mm(3.0),
        },
    ] {
        add(&mut sketch, constraint);
    }
    ArcsAndLines {
        sketch,
        lines: [first_line, second_line, third_line, fourth_line],
        arcs: [first_arc, second_arc],
        circle,
    }
}

#[test]
fn arcs_follow_tangent_equal_and_angle_constraints_exactly() {
    let ArcsAndLines {
        sketch,
        lines,
        arcs,
        circle,
    } = arcs_joined_by_tangent_lines();
    let root_three = 3.0_f64.sqrt();
    let third_end = Point2::new(15.0 - 3.0 * root_three, 15.0);
    let second_center = third_end + 5.0 * Point2::new(-0.5, -root_three / 2.0);
    let second_arc_end = second_center + 5.0 * Point2::new(-root_three / 2.0, 0.5);
    let fourth_end = second_arc_end + 3.0 * Point2::new(-0.5, -root_three / 2.0);

    let solved = solve(&sketch).unwrap();

    assert!(solved.solution.is_fully_constrained());
    assert!(solved.solution.redundancies().is_empty());
    let (first_center, first_radius) = solved.geometry.circle(arcs[0]).unwrap();
    let (second_arc_center, second_radius) = solved.geometry.circle(arcs[1]).unwrap();
    let (circle_center, circle_radius) = solved.geometry.circle(circle).unwrap();
    assert_near(first_center, Point2::new(10.0, 5.0));
    assert!((first_radius - 5.0).abs() < EXACT);
    assert!((second_radius - 5.0).abs() < EXACT);
    assert!((circle_radius - 5.0).abs() < EXACT);
    assert_near(circle_center, Point2::new(20.0, 5.0));
    assert_near(second_arc_center, second_center);
    let (_, first_arc_end) = solved.geometry.line_endpoints(lines[1]).unwrap();
    assert_near(first_arc_end, Point2::new(15.0, 12.0));
    let (_, third) = solved.geometry.line_endpoints(lines[2]).unwrap();
    assert_near(third, third_end);
    let (fourth_start, fourth) = solved.geometry.line_endpoints(lines[3]).unwrap();
    assert_near(fourth_start, second_arc_end);
    assert_near(fourth, fourth_end);
    let second_arc = solved.geometry.arc(arcs[1]).unwrap();
    assert!((second_arc.sweep.to_degrees() - 90.0).abs() < 1e-6);
    let first_arc = solved.geometry.arc(arcs[0]).unwrap();
    assert!((first_arc.sweep.to_degrees() - 90.0).abs() < 1e-6);
}

#[test]
fn two_conflicts_in_one_large_part_are_named_one_at_a_time() {
    let (mut sketch, lines) = chain(20);
    let horizontal = |sketch: &Sketch, line: EntityId| {
        sketch
            .constraints()
            .find_map(|(id, constraint)| {
                (*constraint == Constraint::Horizontal(line)).then_some(id)
            })
            .unwrap()
    };
    let early_horizontal = horizontal(&sketch, lines[4]);
    let late_horizontal = horizontal(&sketch, lines[15]);
    let early_vertical = add(&mut sketch, Constraint::Vertical(lines[4]));
    let late_vertical = add(&mut sketch, Constraint::Vertical(lines[15]));

    let first = solve(&sketch);
    sketch.remove_constraint(late_vertical).unwrap();
    let second = solve(&sketch);
    sketch.remove_constraint(early_vertical).unwrap();
    let third = solve(&sketch);

    assert_eq!(
        first,
        Err(SketchError::Conflict {
            constraints: vec![late_horizontal, late_vertical]
        })
    );
    assert_eq!(
        second,
        Err(SketchError::Conflict {
            constraints: vec![early_horizontal, early_vertical]
        })
    );
    assert!(third.unwrap().solution.is_fully_constrained());
}

#[test]
fn dense_and_sparse_analyses_agree_on_both_sides_of_the_dense_limit() {
    use std::collections::BTreeSet;

    use crate::solve::{
        numeric::{DENSE_LIMIT, Elimination, STIFF, Solver, components},
        system::System,
    };

    let loosened = |count: usize| {
        let (mut sketch, lines) = chain(count);
        let horizontal = sketch
            .constraints()
            .find_map(|(id, constraint)| {
                (*constraint == Constraint::Horizontal(lines[count / 2])).then_some(id)
            })
            .unwrap();
        sketch.remove_constraint(horizontal).unwrap();
        sketch
    };
    let repeated = |count: usize| {
        let (mut sketch, lines) = chain(count);
        add(
            &mut sketch,
            Constraint::Parallel(lines[1], lines[count - 2]),
        );
        sketch
    };
    let sketches = [
        loosened(DENSE_LIMIT / 4),
        loosened(DENSE_LIMIT / 4 + 1),
        repeated(DENSE_LIMIT / 4),
        repeated(DENSE_LIMIT / 4 + 1),
        arcs_joined_by_tangent_lines().sketch,
    ];

    let mut sizes = BTreeSet::new();
    for sketch in sketches {
        let geometry = solve(&sketch).unwrap().geometry;
        let dimensions = geometry.evaluate(&no_parameters).unwrap();
        let system = System::build(&geometry, &dimensions).unwrap();
        let stiff = BTreeSet::new();
        let solver = Solver {
            system: &system,
            cancelled: &|| false,
            stiff: &stiff,
            stiffness: STIFF,
        };
        let every: Vec<usize> = (0..system.equations.len()).collect();
        for component in components(&system, &every, &system.values) {
            let dense = solver.analyze_by(Elimination::Dense, &component, &system.values);
            let sparse = solver.analyze_by(Elimination::Sparse, &component, &system.values);
            sizes.insert(component.variables.len());
            assert_eq!(dense, sparse, "{} variables", component.variables.len());
        }
    }

    assert!(sizes.contains(&DENSE_LIMIT));
    assert!(sizes.iter().any(|size| *size > DENSE_LIMIT));
}

fn chain_closed_out_of_reach(count: usize) -> (Sketch, Vec<EntityId>, ConstraintId) {
    let (mut sketch, lines) = chain(count);
    let (first, _) = ends(&sketch, lines[0]);
    let (_, last) = ends(&sketch, lines[count - 1]);
    let closing = add(
        &mut sketch,
        Constraint::Distance {
            from: first,
            to: last,
            value: mm(count as f64 + 5.0),
        },
    );
    (sketch, lines, closing)
}

#[test]
fn a_conflict_spanning_a_whole_chain_names_every_constraint_in_it() {
    let (sketch, _, closing) = chain_closed_out_of_reach(8);
    let spanning: Vec<ConstraintId> = sketch
        .constraints()
        .filter(|(_, constraint)| match constraint {
            Constraint::Distance { .. } => true,
            Constraint::Coincident(_, other) => *other != EntityId::ORIGIN,
            _ => false,
        })
        .map(|(id, _)| id)
        .collect();

    let result = solve(&sketch);

    assert_eq!(spanning.len(), 8 + 7 + 1);
    assert!(spanning.contains(&closing));
    assert_eq!(
        result,
        Err(SketchError::Conflict {
            constraints: spanning
        })
    );
}

#[test]
fn a_diagnosis_out_of_work_names_the_part_and_its_newest_constraint() {
    use std::collections::BTreeSet;

    use crate::solve::{
        DIAGNOSIS_WORK, diagnose_failure,
        numeric::{STIFF, Solver, components},
        system::System,
    };

    let (sketch, lines, closing) = chain_closed_out_of_reach(6);
    let dimensions = sketch.evaluate(&no_parameters).unwrap();
    let system = System::build(&sketch, &dimensions).unwrap();
    let stiff = BTreeSet::new();
    let solver = Solver {
        system: &system,
        cancelled: &|| false,
        stiff: &stiff,
        stiffness: STIFF,
    };
    let every: Vec<usize> = (0..system.equations.len()).collect();
    let mut values = system.values.clone();
    let failed = solver.solve(&every, &mut values).unwrap();

    let enough_once_stalled_attempts_stop = 10_000;
    let starved = diagnose_failure(&sketch, &solver, &failed, 100).unwrap();
    let funded =
        diagnose_failure(&sketch, &solver, &failed, enough_once_stalled_attempts_stop).unwrap();

    assert!(enough_once_stalled_attempts_stop < DIAGNOSIS_WORK);
    assert_eq!(components(&system, &every, &system.values).len(), 1);
    assert_eq!(failed.len(), 1);
    assert_eq!(
        starved,
        SketchError::Unsolvable {
            entities: lines,
            newest: Some(closing),
        }
    );
    let SketchError::Conflict { constraints } = funded else {
        panic!("with enough work the conflict is found, not {funded:?}");
    };
    assert_eq!(constraints.len(), 6 + 5 + 1);
    assert_eq!(constraints.last(), Some(&closing));
}
