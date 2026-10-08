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

fn ends(sketch: &Sketch, line: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(line) {
        Some(Entity::Line { start, end }) => (*start, *end),
        other => panic!("expected a line, found {other:?}"),
    }
}

fn center(sketch: &Sketch, curve: EntityId) -> EntityId {
    sketch.center_of(curve).unwrap()
}

fn add(sketch: &mut Sketch, constraint: Constraint) -> ConstraintId {
    sketch.add_constraint(constraint).unwrap()
}

fn add_stored(sketch: &mut Sketch, constraint: Constraint) -> ConstraintId {
    let id = ConstraintId::from_raw(sketch.next_id());
    sketch.insert_constraint(id, constraint).unwrap();
    id
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

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < EXACT,
        "{actual} is not {expected}"
    );
}

fn fix(sketch: &mut Sketch, point: EntityId) -> ConstraintId {
    let at = sketch.point(point).unwrap();
    add(sketch, Constraint::Fix { point, at })
}

fn fix_line(sketch: &mut Sketch, line: EntityId) {
    let (start, end) = ends(sketch, line);
    fix(sketch, start);
    fix(sketch, end);
}

#[test]
fn a_fixed_point_stays_where_it_was_locked_and_is_fully_constrained() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(1.0, 2.0), Point2::new(9.0, 3.0));
    let (start, end) = ends(&sketch, line);
    add(
        &mut sketch,
        Constraint::Fix {
            point: start,
            at: Point2::new(-4.0, 6.5),
        },
    );
    add(&mut sketch, Constraint::Horizontal(line));

    let solved = solve(&sketch).unwrap();

    assert_near(at(&solved, start), Point2::new(-4.0, 6.5));
    assert_close(at(&solved, end).y, 6.5);
    assert_eq!(solved.solution.degrees_of_freedom(), 1);
    assert_eq!(
        solved.solution.entity_state(start),
        Some(EntityState::FullyConstrained)
    );
    assert_eq!(
        solved.solution.entity_state(line),
        Some(EntityState::UnderConstrained)
    );
}

#[test]
fn a_fix_away_from_where_a_coincidence_puts_the_point_is_a_conflict() {
    let mut sketch = Sketch::new(Plane::XY);
    let point = sketch.add_point(Point2::new(1.0, 1.0));
    let coincident = add(&mut sketch, Constraint::Coincident(point, EntityId::ORIGIN));
    let fixed = add(
        &mut sketch,
        Constraint::Fix {
            point,
            at: Point2::new(1.0, 1.0),
        },
    );

    assert_eq!(
        solve(&sketch),
        Err(SketchError::Conflict {
            constraints: vec![coincident, fixed]
        })
    );

    sketch.remove_constraint(fixed).unwrap();
    let again = add(
        &mut sketch,
        Constraint::Fix {
            point,
            at: Point2::ZERO,
        },
    );
    let solved = solve(&sketch).unwrap();
    assert_eq!(
        solved.solution.redundancies(),
        [Redundancy {
            constraint: again,
            duplicates: vec![coincident],
        }]
    );
}

#[test]
fn a_midpoint_holds_a_point_halfway_round_an_arc_on_the_arc_itself() {
    let mut sketch = Sketch::new(Plane::XY);
    let quarter = sketch.add_arc(
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 0.0),
        Point2::new(0.0, 10.0),
    );
    let most = sketch.add_arc(
        Point2::new(40.0, 0.0),
        Point2::new(50.0, 0.0),
        Point2::new(40.0, -10.0),
    );
    let beyond = sketch.add_point(Point2::new(-6.0, -5.0));
    let inside = sketch.add_point(Point2::new(44.0, -3.0));
    add(
        &mut sketch,
        Constraint::Midpoint {
            point: beyond,
            curve: quarter,
        },
    );
    add(
        &mut sketch,
        Constraint::Midpoint {
            point: inside,
            curve: most,
        },
    );
    for arc in [quarter, most] {
        let points = sketch.entity(arc).unwrap().points();
        for point in points {
            fix(&mut sketch, point);
        }
    }

    let solved = solve(&sketch).unwrap();

    let diagonal = 10.0 / 2f64.sqrt();
    assert_near(at(&solved, beyond), Point2::new(diagonal, diagonal));
    assert_near(at(&solved, inside), Point2::new(40.0 - diagonal, diagonal));
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
    assert_eq!(
        solved.solution.entity_state(beyond),
        Some(EntityState::FullyConstrained)
    );
}

#[test]
fn a_midpoint_on_a_circle_is_refused_as_it_has_no_middle() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(0.0, 0.0), 5.0);
    let point = sketch.add_point(Point2::new(5.0, 0.0));

    let refused = sketch.check_constraint(&Constraint::Midpoint {
        point,
        curve: circle,
    });

    assert!(refused.is_err());
}

#[test]
fn a_midpoint_holds_a_point_halfway_along_a_line() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(10.0, 2.0));
    let point = sketch.add_point(Point2::new(3.0, 4.0));
    add(&mut sketch, Constraint::Midpoint { point, curve: line });
    let loose = solve(&sketch).unwrap();
    assert_eq!(loose.solution.degrees_of_freedom(), 4);

    fix_line(&mut sketch, line);
    let solved = solve(&sketch).unwrap();

    assert_near(at(&solved, point), Point2::new(5.0, 1.0));
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
    assert_eq!(
        solved.solution.entity_state(point),
        Some(EntityState::FullyConstrained)
    );
}

#[test]
fn concentric_curves_share_a_centre_and_a_point_can_sit_on_one() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(1.0, 1.0), 5.0);
    let arc = sketch.add_arc(
        Point2::new(2.0, 0.5),
        Point2::new(5.0, 0.5),
        Point2::new(2.0, 3.5),
    );
    let lone = sketch.add_point(Point2::new(-3.0, 2.0));
    let middle = center(&sketch, circle);
    fix(&mut sketch, middle);
    add(&mut sketch, Constraint::Concentric(arc, circle));
    add(&mut sketch, Constraint::Concentric(lone, circle));

    let solved = solve(&sketch).unwrap();

    assert_near(at(&solved, center(&sketch, arc)), Point2::new(1.0, 1.0));
    assert_near(at(&solved, lone), Point2::new(1.0, 1.0));
    assert_eq!(solved.solution.degrees_of_freedom(), 1 + 3);
}

#[test]
fn collinear_lines_lie_on_one_line_and_repeat_a_parallel() {
    let mut sketch = Sketch::new(Plane::XY);
    let base = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(10.0, 5.0));
    let other = sketch.add_line(Point2::new(14.0, 8.0), Point2::new(20.0, 9.5));
    fix_line(&mut sketch, base);
    let collinear = add(&mut sketch, Constraint::Collinear(base, other));
    let parallel = add(&mut sketch, Constraint::Parallel(other, base));

    let solved = solve(&sketch).unwrap();

    let (start, end) = ends(&sketch, other);
    for point in [start, end] {
        let position = at(&solved, point);
        assert_close(position.y, position.x / 2.0);
    }
    assert_eq!(solved.solution.degrees_of_freedom(), 2);
    assert_eq!(
        solved.solution.redundancies(),
        [Redundancy {
            constraint: parallel,
            duplicates: vec![collinear],
        }]
    );

    let axis_line = sketch.add_line(Point2::new(2.0, 1.0), Point2::new(8.0, -1.0));
    add(
        &mut sketch,
        Constraint::Collinear(axis_line, EntityId::HORIZONTAL_AXIS),
    );
    let solved = solve(&sketch).unwrap();
    let (start, end) = ends(&sketch, axis_line);
    assert_close(at(&solved, start).y, 0.0);
    assert_close(at(&solved, end).y, 0.0);
}

#[test]
fn symmetric_points_mirror_about_a_line_or_a_point() {
    let mut sketch = Sketch::new(Plane::XY);
    let mirror = sketch.add_line(Point2::ZERO, Point2::new(10.0, 10.0));
    fix_line(&mut sketch, mirror);
    let first = sketch.add_point(Point2::new(6.0, 1.0));
    let second = sketch.add_point(Point2::new(1.5, 5.0));
    fix(&mut sketch, first);
    add(
        &mut sketch,
        Constraint::Symmetric {
            first,
            second,
            about: mirror,
        },
    );
    let centre = sketch.add_point(Point2::new(20.0, 0.0));
    let left = sketch.add_point(Point2::new(17.0, 1.0));
    let right = sketch.add_point(Point2::new(22.0, 0.5));
    fix(&mut sketch, centre);
    fix(&mut sketch, left);
    add(
        &mut sketch,
        Constraint::Symmetric {
            first: left,
            second: right,
            about: centre,
        },
    );

    let solved = solve(&sketch).unwrap();

    assert_near(at(&solved, second), Point2::new(1.0, 6.0));
    assert_near(at(&solved, right), Point2::new(23.0, -1.0));
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
}

#[test]
fn symmetry_about_an_axis_holds_from_a_perturbed_start() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(-4.0, 3.0), Point2::new(5.0, 2.5));
    let (start, end) = ends(&sketch, line);
    add(
        &mut sketch,
        Constraint::Symmetric {
            first: start,
            second: end,
            about: EntityId::VERTICAL_AXIS,
        },
    );
    let solved = solve(&sketch).unwrap();
    let (a, b) = (at(&solved, start), at(&solved, end));
    assert_close(a.x, -b.x);
    assert_close(a.y, b.y);
    assert_eq!(solved.solution.degrees_of_freedom(), 2);
}

#[test]
fn horizontal_and_vertical_distances_keep_their_drawn_side() {
    let mut sketch = Sketch::new(Plane::XY);
    let from = sketch.add_point(Point2::new(2.0, 3.0));
    let to = sketch.add_point(Point2::new(-5.0, 9.0));
    fix(&mut sketch, from);
    let across = add(
        &mut sketch,
        Constraint::HorizontalDistance {
            from,
            to,
            value: mm(12.0),
        },
    );
    add(
        &mut sketch,
        Constraint::VerticalDistance {
            from,
            to,
            value: mm(4.0),
        },
    );

    let solved = solve(&sketch).unwrap();

    assert_near(at(&solved, to), Point2::new(-10.0, 7.0));
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
    assert_eq!(solved.solution.dimension(across), Some(12.0));

    sketch.set_dimension(across, mm(0.0)).unwrap();
    let solved = solve(&sketch).unwrap();
    assert_near(at(&solved, to), Point2::new(2.0, 7.0));
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
}

#[test]
fn a_negative_distance_is_refused_when_added() {
    let mut sketch = Sketch::new(Plane::XY);
    let from = sketch.add_point(Point2::ZERO);
    let to = sketch.add_point(Point2::X);

    let horizontal = sketch.add_constraint(Constraint::HorizontalDistance {
        from,
        to,
        value: mm(-1.0),
    });
    let vertical = sketch.add_constraint(Constraint::VerticalDistance {
        from,
        to,
        value: Expression::Number(-1.0),
    });

    let expected = Err(SketchError::DimensionValue {
        reason: crate::DimensionError::Negative,
    });
    assert_eq!(horizontal, expected);
    assert_eq!(vertical, expected);
    assert_eq!(sketch.constraints().len(), 0);
    assert_eq!(
        horizontal.unwrap_err().to_string(),
        "a distance cannot be negative"
    );
}

#[test]
fn a_negative_horizontal_distance_is_refused_before_solving() {
    let mut sketch = Sketch::new(Plane::XY);
    let from = sketch.add_point(Point2::ZERO);
    let to = sketch.add_point(Point2::X);
    add_stored(
        &mut sketch,
        Constraint::HorizontalDistance {
            from,
            to,
            value: mm(-1.0),
        },
    );
    let error = solve(&sketch).unwrap_err();
    assert_eq!(error.to_string(), "a distance cannot be negative");
}

#[test]
fn points_made_horizontal_or_vertical_line_up() {
    let mut sketch = Sketch::new(Plane::XY);
    let anchor = sketch.add_point(Point2::new(1.0, 2.0));
    let level = sketch.add_point(Point2::new(8.0, 3.0));
    let plumb = sketch.add_point(Point2::new(2.0, -6.0));
    fix(&mut sketch, anchor);
    add(&mut sketch, Constraint::HorizontalPoints(anchor, level));
    add(&mut sketch, Constraint::VerticalPoints(plumb, anchor));

    let solved = solve(&sketch).unwrap();

    assert_close(at(&solved, level).y, 2.0);
    assert_close(at(&solved, plumb).x, 1.0);
    assert_eq!(solved.solution.degrees_of_freedom(), 2);

    let line = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(5.0, 10.5));
    let (start, end) = ends(&sketch, line);
    let horizontal = add(&mut sketch, Constraint::Horizontal(line));
    let repeated = add(&mut sketch, Constraint::HorizontalPoints(start, end));
    let solved = solve(&sketch).unwrap();
    assert_eq!(
        solved.solution.redundancies(),
        [Redundancy {
            constraint: repeated,
            duplicates: vec![horizontal],
        }]
    );
}

#[test]
fn a_diameter_sets_twice_the_radius() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(3.0, 4.0), 5.0);
    let arc = sketch.add_arc(Point2::ZERO, Point2::new(2.0, 0.0), Point2::new(0.0, 2.0));
    add(
        &mut sketch,
        Constraint::Diameter {
            entity: circle,
            value: mm(18.0),
        },
    );
    add(
        &mut sketch,
        Constraint::Diameter {
            entity: arc,
            value: mm(7.0),
        },
    );

    let solved = solve(&sketch).unwrap();

    assert_close(solved.geometry.circle(circle).unwrap().1, 9.0);
    assert_close(solved.geometry.circle(arc).unwrap().1, 3.5);
    assert_eq!(solved.solution.degrees_of_freedom(), 2 + 4);

    let refused = add_stored(
        &mut sketch,
        Constraint::Diameter {
            entity: circle,
            value: mm(0.0),
        },
    );
    assert_eq!(
        solve(&sketch).unwrap_err(),
        SketchError::Dimension {
            constraint: refused,
            reason: crate::DimensionError::DiameterNotPositive,
        }
    );
}

#[test]
fn a_point_keeps_its_side_of_a_circle_at_a_distance() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::ZERO, 10.0);
    let middle = center(&sketch, circle);
    fix(&mut sketch, middle);
    add(
        &mut sketch,
        Constraint::Radius {
            entity: circle,
            value: mm(10.0),
        },
    );
    let outside = sketch.add_point(Point2::new(13.0, 0.0));
    let inside = sketch.add_point(Point2::new(0.0, -7.0));
    add(
        &mut sketch,
        Constraint::HorizontalPoints(outside, EntityId::ORIGIN),
    );
    add(
        &mut sketch,
        Constraint::VerticalPoints(inside, EntityId::ORIGIN),
    );
    add(
        &mut sketch,
        Constraint::Distance {
            from: outside,
            to: circle,
            value: mm(5.0),
        },
    );
    add(
        &mut sketch,
        Constraint::Distance {
            from: circle,
            to: inside,
            value: mm(4.0),
        },
    );

    let solved = solve(&sketch).unwrap();

    assert_near(at(&solved, outside), Point2::new(15.0, 0.0));
    assert_near(at(&solved, inside), Point2::new(0.0, -6.0));
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
}

#[test]
fn spaced_lines_become_parallel_at_the_distance_on_their_side() {
    let mut sketch = Sketch::new(Plane::XY);
    let base = sketch.add_line(Point2::ZERO, Point2::new(20.0, 0.0));
    fix_line(&mut sketch, base);
    let other = sketch.add_line(Point2::new(2.0, -3.0), Point2::new(18.0, -4.5));
    let spacing = add(
        &mut sketch,
        Constraint::Distance {
            from: base,
            to: other,
            value: mm(6.0),
        },
    );

    let solved = solve(&sketch).unwrap();

    let (start, end) = ends(&sketch, other);
    assert_close(at(&solved, start).y, -6.0);
    assert_close(at(&solved, end).y, -6.0);
    assert_eq!(solved.solution.degrees_of_freedom(), 2);

    let parallel = add(&mut sketch, Constraint::Parallel(other, base));
    let solved = solve(&sketch).unwrap();
    assert_eq!(
        solved.solution.redundancies(),
        [Redundancy {
            constraint: parallel,
            duplicates: vec![spacing],
        }]
    );

    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(1.0, 2.0), Point2::new(9.0, 3.0));
    add(
        &mut sketch,
        Constraint::Distance {
            from: EntityId::VERTICAL_AXIS,
            to: line,
            value: mm(0.0),
        },
    );
    let solved = solve(&sketch).unwrap();
    let (start, end) = ends(&sketch, line);
    assert_close(at(&solved, start).x, 0.0);
    assert_close(at(&solved, end).x, 0.0);
}

#[test]
fn circles_keep_their_gap_outside_or_within_each_other() {
    let mut sketch = Sketch::new(Plane::XY);
    let fixed = sketch.add_circle(Point2::ZERO, 10.0);
    let middle = center(&sketch, fixed);
    fix(&mut sketch, middle);
    add(
        &mut sketch,
        Constraint::Radius {
            entity: fixed,
            value: mm(10.0),
        },
    );
    let outside = sketch.add_circle(Point2::new(17.0, 0.0), 3.0);
    let inside = sketch.add_circle(Point2::new(0.0, -5.0), 2.0);
    let outside_centre = center(&sketch, outside);
    let inside_centre = center(&sketch, inside);
    for (circle, radius) in [(outside, 3.0), (inside, 2.0)] {
        add(
            &mut sketch,
            Constraint::Radius {
                entity: circle,
                value: mm(radius),
            },
        );
    }
    add(
        &mut sketch,
        Constraint::HorizontalPoints(outside_centre, middle),
    );
    add(
        &mut sketch,
        Constraint::VerticalPoints(inside_centre, middle),
    );
    add(
        &mut sketch,
        Constraint::Distance {
            from: outside,
            to: fixed,
            value: mm(5.0),
        },
    );
    add(
        &mut sketch,
        Constraint::Distance {
            from: fixed,
            to: inside,
            value: mm(1.5),
        },
    );

    let solved = solve(&sketch).unwrap();

    assert_near(at(&solved, outside_centre), Point2::new(18.0, 0.0));
    assert_near(at(&solved, inside_centre), Point2::new(0.0, -6.5));
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
}

#[test]
fn a_circle_keeps_its_gap_from_a_line_on_its_side() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(4.0, -9.0), 2.0);
    let centre = center(&sketch, circle);
    add(
        &mut sketch,
        Constraint::Radius {
            entity: circle,
            value: mm(2.5),
        },
    );
    add(
        &mut sketch,
        Constraint::VerticalPoints(centre, EntityId::ORIGIN),
    );
    let gap = add(
        &mut sketch,
        Constraint::Distance {
            from: EntityId::HORIZONTAL_AXIS,
            to: circle,
            value: mm(3.0),
        },
    );

    let solved = solve(&sketch).unwrap();

    assert_near(at(&solved, centre), Point2::new(0.0, -5.5));
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
    assert_close(
        solved
            .geometry
            .measured(sketch.constraint(gap).unwrap())
            .unwrap(),
        3.0,
    );

    sketch.set_dimension(gap, mm(0.0)).unwrap();
    let touching = solve(&sketch).unwrap();
    assert_near(at(&touching, centre), Point2::new(0.0, -2.5));
}

#[test]
fn new_kinds_survive_degenerate_starts_without_nan() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(3.0, 3.0), Point2::new(3.0, 3.0 + 1e-13));
    let (start, _) = ends(&sketch, line);
    let point = sketch.add_point(Point2::new(3.0, 3.0));
    let other = sketch.add_point(Point2::new(3.0, 3.0));
    let circle = sketch.add_circle(Point2::new(3.0, 3.0), 1.0);
    add(&mut sketch, Constraint::Midpoint { point, curve: line });
    add(
        &mut sketch,
        Constraint::Symmetric {
            first: point,
            second: other,
            about: line,
        },
    );
    add(
        &mut sketch,
        Constraint::Distance {
            from: other,
            to: circle,
            value: mm(2.0),
        },
    );
    add(
        &mut sketch,
        Constraint::HorizontalDistance {
            from: start,
            to: other,
            value: mm(1.0),
        },
    );

    match solve(&sketch) {
        Ok(solved) => {
            for (_, entity) in solved.geometry.entities() {
                if let Entity::Point(position) = entity {
                    assert!(position.is_finite());
                }
            }
        }
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
}

fn arch(sketch: &mut Sketch) -> EntityId {
    let spline = sketch.add_spline(&[
        Point2::ZERO,
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 0.0),
    ]);
    let Some(Entity::Spline { control_points }) = sketch.entity(spline).cloned() else {
        panic!("expected a spline");
    };
    for point in control_points {
        fix(sketch, point);
    }
    spline
}

fn arch_height(x: f64) -> f64 {
    x * (20.0 - x) / 20.0
}

#[test]
fn a_point_on_a_spline_slides_along_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = arch(&mut sketch);
    let point = sketch.add_point(Point2::new(5.0, 9.0));
    let guide = sketch.add_point(Point2::new(5.0, -3.0));
    fix(&mut sketch, guide);
    add(&mut sketch, Constraint::VerticalPoints(point, guide));
    add(&mut sketch, Constraint::Coincident(point, spline));

    let solved = solve(&sketch).unwrap();

    assert_near(at(&solved, point), Point2::new(5.0, arch_height(5.0)));
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
    assert_eq!(
        solved.solution.entity_state(point),
        Some(EntityState::FullyConstrained)
    );

    let mut loose = Sketch::new(Plane::XY);
    let free = loose.add_spline(&[
        Point2::ZERO,
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 0.0),
    ]);
    let rider = loose.add_point(Point2::new(3.0, 1.0));
    add(&mut loose, Constraint::Coincident(free, rider));
    assert_eq!(solve(&loose).unwrap().solution.degrees_of_freedom(), 8 - 1);
}

#[test]
fn a_point_already_on_a_spline_does_not_move() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = arch(&mut sketch);
    let point = sketch.add_point(Point2::new(6.0, arch_height(6.0)));
    add(&mut sketch, Constraint::Coincident(point, spline));

    let solved = solve(&sketch).unwrap();

    assert_eq!(solved.geometry, sketch);
    assert_eq!(solved.solution.degrees_of_freedom(), 1);
}

#[test]
fn a_point_held_beyond_the_end_of_a_free_spline_pulls_its_end_along() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&[
        Point2::ZERO,
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 0.0),
    ]);
    let Some(Entity::Spline { control_points }) = sketch.entity(spline).cloned() else {
        panic!("expected a spline");
    };
    fix(&mut sketch, control_points[0]);
    let point = sketch.add_point(Point2::new(30.0, -2.0));
    fix(&mut sketch, point);
    add(&mut sketch, Constraint::Coincident(point, spline));

    let solved = solve(&sketch).unwrap();

    assert_near(at(&solved, control_points[0]), Point2::ZERO);
    assert_near(at(&solved, control_points[2]), Point2::new(30.0, -2.0));
}

#[test]
fn a_point_held_beyond_the_end_of_a_pinned_spline_conflicts_with_every_pin() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = arch(&mut sketch);
    let point = sketch.add_point(Point2::new(30.0, -2.0));
    fix(&mut sketch, point);
    add(&mut sketch, Constraint::Coincident(point, spline));
    let every: Vec<ConstraintId> = sketch.constraints().map(|(id, _)| id).collect();

    let result = solve(&sketch);

    assert_eq!(every.len(), 3 + 1 + 1);
    assert_eq!(result, Err(SketchError::Conflict { constraints: every }));
}

#[test]
fn a_line_and_a_circle_touch_a_spline_where_it_bulges() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = arch(&mut sketch);
    let line = sketch.add_line(Point2::new(4.0, 6.5), Point2::new(15.0, 6.0));
    add(&mut sketch, Constraint::Horizontal(line));
    add(&mut sketch, Constraint::Tangent(line, spline));
    let circle = sketch.add_circle(Point2::new(10.0, 14.0), 3.0);
    let middle = center(&sketch, circle);
    add(
        &mut sketch,
        Constraint::Fix {
            point: middle,
            at: Point2::new(10.0, 14.0),
        },
    );
    add(&mut sketch, Constraint::Tangent(spline, circle));

    let solved = solve(&sketch).unwrap();

    let (start, end) = ends(&sketch, line);
    assert_close(at(&solved, start).y, 5.0);
    assert_close(at(&solved, end).y, 5.0);
    assert_close(solved.geometry.circle(circle).unwrap().1, 9.0);
    assert_eq!(solved.solution.degrees_of_freedom(), 2);
    assert!(solved.solution.redundancies().is_empty());
}

#[test]
fn a_line_joined_to_a_spline_end_turns_along_its_first_leg() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = arch(&mut sketch);
    let line = sketch.add_line(Point2::new(-8.0, -5.0), Point2::new(0.5, 0.2));
    let (start, end) = ends(&sketch, line);
    let Some(Entity::Spline { control_points }) = sketch.entity(spline).cloned() else {
        panic!("expected a spline");
    };
    add(&mut sketch, Constraint::Coincident(end, control_points[0]));
    add(
        &mut sketch,
        Constraint::Distance {
            from: start,
            to: end,
            value: mm(10.0),
        },
    );
    add(&mut sketch, Constraint::Tangent(spline, line));

    let solved = solve(&sketch).unwrap();

    let expected = Point2::new(-10.0, -10.0) / 2.0_f64.sqrt();
    assert_near(at(&solved, start), expected);
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
    assert!(solved.solution.redundancies().is_empty());
}

#[test]
fn a_settled_spline_tangency_is_remembered_by_the_next_solve() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = arch(&mut sketch);
    let line = sketch.add_line(Point2::new(4.0, 6.5), Point2::new(15.0, 6.0));
    add(&mut sketch, Constraint::Horizontal(line));
    add(&mut sketch, Constraint::Tangent(line, spline));

    let fresh = solve(&sketch).unwrap();
    let settled = fresh.geometry.clone();
    let again = settled
        .solve_from(&no_parameters, &|| false, &[], Some(&fresh.memo))
        .unwrap();

    assert!(again.memo.recalled() > 0);
    assert_eq!(again.solution, fresh.solution);
    assert_eq!(again.geometry, fresh.geometry);
}

#[test]
fn a_spline_lying_along_the_axis_it_touches_solves_again_from_where_it_settled() {
    let mut sketch = Sketch::new(Plane::XY);
    let start = sketch.add_point(Point2::new(0.0, -4.0));
    let rest = [
        Point2::new(0.0, 8.0),
        Point2::new(0.0, 1.0),
        Point2::new(3.0, 2.0),
    ]
    .map(|position| sketch.add_point(position));
    let spline = EntityId::from_raw(sketch.next_id());
    let control_points = [start, start, rest[0], rest[1], rest[2]].to_vec();
    sketch
        .insert_entity(spline, Entity::Spline { control_points })
        .unwrap();
    add(
        &mut sketch,
        Constraint::Tangent(spline, EntityId::VERTICAL_AXIS),
    );

    let solved = solve(&sketch).unwrap();

    assert!(solve(&solved.geometry).is_ok());
}

#[test]
fn a_spline_touching_an_axis_at_a_cusp_solves_again_whenever_it_solves() {
    let mut sketch = Sketch::new(Plane::XY);
    let [first, turn, last] = [
        Point2::new(9.49998, -7.00002),
        Point2::new(-7.00002, -7.0),
        Point2::new(-3.5, -10.0),
    ]
    .map(|position| sketch.add_point(position));
    let spline = EntityId::from_raw(sketch.next_id());
    let control_points = [first, turn, turn, last, turn].to_vec();
    sketch
        .insert_entity(spline, Entity::Spline { control_points })
        .unwrap();
    add(
        &mut sketch,
        Constraint::Tangent(spline, EntityId::HORIZONTAL_AXIS),
    );

    let solved = solve(&sketch);

    if let Ok(solved) = solved {
        assert!(solve(&solved.geometry).is_ok());
    }
}
