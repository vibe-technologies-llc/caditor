use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};

use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2, Vector2};

use crate::{
    Constraint, ConstraintId, EllipseGeometry, Entity, EntityId, EntityState, Sketch, SketchError,
    Solved,
};

const EXACT: f64 = 1e-8;

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn solve(sketch: &Sketch) -> Result<Solved, SketchError> {
    sketch.solve(&no_parameters, &|| false)
}

fn mm(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

fn add(sketch: &mut Sketch, constraint: Constraint) -> ConstraintId {
    sketch.add_constraint(constraint).unwrap()
}

fn fix(sketch: &mut Sketch, point: EntityId) {
    let at = sketch.point(point).unwrap();
    add(sketch, Constraint::Fix { point, at });
}

fn axis_points(sketch: &Sketch, ellipse: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(ellipse) {
        Some(
            Entity::Ellipse { center, major, .. } | Entity::EllipticalArc { center, major, .. },
        ) => (*center, *major),
        other => panic!("expected an ellipse, found {other:?}"),
    }
}

fn arc_ends(sketch: &Sketch, arc: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(arc) {
        Some(Entity::EllipticalArc { start, end, .. }) => (*start, *end),
        other => panic!("expected an elliptical arc, found {other:?}"),
    }
}

fn level(ellipse: &EllipseGeometry, point: Point2) -> f64 {
    let axis = ellipse.axis();
    let offset = point - ellipse.center;
    let x = offset.dot(axis) / ellipse.major_radius();
    let y = offset.dot(axis.perp()) / ellipse.minor_radius;
    x * x + y * y - 1.0
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < EXACT,
        "{actual} is not {expected}"
    );
}

fn pinned_ellipse(sketch: &mut Sketch) -> EntityId {
    let ellipse = sketch.add_ellipse(Point2::ZERO, Point2::new(10.0, 0.0), 4.0);
    let (center, major) = axis_points(sketch, ellipse);
    fix(sketch, center);
    fix(sketch, major);
    add(
        sketch,
        Constraint::MinorRadius {
            ellipse,
            value: mm(4.0),
        },
    );
    ellipse
}

#[test]
fn a_free_ellipse_has_five_degrees_of_freedom_and_an_elliptical_arc_seven() {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_ellipse(Point2::new(1.0, 2.0), Point2::new(9.0, 3.0), 3.0);
    assert_eq!(solve(&sketch).unwrap().solution.degrees_of_freedom(), 5);

    let mut sketch = Sketch::new(Plane::XY);
    let start = Point2::new(10.0, 0.0);
    let end = Point2::new(0.0, 4.0);
    let arc = sketch.add_elliptical_arc(Point2::ZERO, Point2::new(10.0, 0.0), 4.0, start, end);
    let solved = solve(&sketch).unwrap();
    assert_eq!(solved.solution.degrees_of_freedom(), 7);
    let shape = solved.geometry.ellipse(arc).unwrap();
    assert!(!shape.is_full());
    assert_close(shape.sweep, std::f64::consts::FRAC_PI_2);
}

#[test]
fn centre_axis_and_both_radii_hold_an_ellipse_fully() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::new(0.5, -0.5), Point2::new(7.0, 2.0), 2.5);
    let (center, major) = axis_points(&sketch, ellipse);
    add(
        &mut sketch,
        Constraint::Coincident(center, EntityId::ORIGIN),
    );
    add(&mut sketch, Constraint::Horizontal(ellipse));
    add(
        &mut sketch,
        Constraint::MajorRadius {
            ellipse,
            value: mm(10.0),
        },
    );
    add(
        &mut sketch,
        Constraint::MinorRadius {
            ellipse,
            value: mm(4.0),
        },
    );

    let solved = solve(&sketch).unwrap();
    let shape = solved.geometry.ellipse(ellipse).unwrap();

    assert!(solved.solution.is_fully_constrained());
    assert_eq!(
        solved.solution.entity_state(ellipse),
        Some(EntityState::FullyConstrained)
    );
    assert_close(shape.major_radius(), 10.0);
    assert_close(shape.minor_radius, 4.0);
    assert_close(solved.geometry.point(major).unwrap().y, 0.0);
    assert_close(solved.geometry.point(center).unwrap().length(), 0.0);
    assert_close(
        solved
            .geometry
            .measured(&Constraint::MajorRadius {
                ellipse,
                value: mm(0.0),
            })
            .unwrap(),
        10.0,
    );
}

#[test]
fn a_vertical_ellipse_turns_its_major_axis_upright() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::ZERO, Point2::new(3.0, 8.0), 2.0);
    add(&mut sketch, Constraint::Vertical(ellipse));

    let solved = solve(&sketch).unwrap();
    let shape = solved.geometry.ellipse(ellipse).unwrap();

    assert_close(shape.axis().x, 0.0);
    assert_eq!(solved.solution.degrees_of_freedom(), 4);
}

#[test]
fn a_point_held_on_an_ellipse_lands_on_it_and_keeps_one_freedom() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = pinned_ellipse(&mut sketch);
    let point = sketch.add_point(Point2::new(6.0, 5.0));
    add(&mut sketch, Constraint::Coincident(point, ellipse));

    let solved = solve(&sketch).unwrap();
    let shape = solved.geometry.ellipse(ellipse).unwrap();

    assert!(level(&shape, solved.geometry.point(point).unwrap()).abs() < EXACT);
    assert_eq!(solved.solution.degrees_of_freedom(), 1);
}

#[test]
fn an_elliptical_arc_keeps_its_ends_on_its_ellipse_when_the_minor_radius_changes() {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_elliptical_arc(
        Point2::ZERO,
        Point2::new(10.0, 0.0),
        4.0,
        Point2::new(10.0 * FRAC_PI_4.cos(), 4.0 * FRAC_PI_4.sin()),
        Point2::new(-10.0, 0.0),
    );
    let (center, major) = axis_points(&sketch, arc);
    fix(&mut sketch, center);
    fix(&mut sketch, major);
    add(
        &mut sketch,
        Constraint::MinorRadius {
            ellipse: arc,
            value: mm(6.0),
        },
    );

    let solved = solve(&sketch).unwrap();
    let shape = solved.geometry.ellipse(arc).unwrap();
    let (start, end) = arc_ends(&sketch, arc);

    assert_close(shape.minor_radius, 6.0);
    for point in [start, end] {
        assert!(level(&shape, solved.geometry.point(point).unwrap()).abs() < EXACT);
    }
    assert_eq!(solved.solution.degrees_of_freedom(), 2);
}

#[test]
fn a_line_tangent_to_an_ellipse_touches_it_on_its_side() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = pinned_ellipse(&mut sketch);
    let line = sketch.add_line(Point2::new(-3.0, 6.0), Point2::new(5.0, 7.0));
    add(&mut sketch, Constraint::Horizontal(line));
    add(&mut sketch, Constraint::Tangent(line, ellipse));

    let solved = solve(&sketch).unwrap();
    let (start, _) = solved.geometry.line_endpoints(line).unwrap();

    assert_close(start.y, 4.0);

    let mut slanted = Sketch::new(Plane::XY);
    let ellipse = pinned_ellipse(&mut slanted);
    let line = slanted.add_line(Point2::new(-12.0, -2.0), Point2::new(2.0, -9.0));
    add(&mut slanted, Constraint::Tangent(ellipse, line));

    let solved = solve(&slanted).unwrap();
    let shape = solved.geometry.ellipse(ellipse).unwrap();
    let (start, end) = solved.geometry.line_endpoints(line).unwrap();
    let direction = (end - start).normalize();
    let touching = (0..3600)
        .map(|step| shape.point_at(step as f64 / 3600.0 * std::f64::consts::TAU))
        .map(|point| direction.perp_dot(point - start))
        .fold(f64::INFINITY, f64::min);
    assert!(touching.abs() < 1e-4, "{touching}");
}

#[test]
fn a_line_ending_on_an_ellipse_and_tangent_runs_along_it_there() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = pinned_ellipse(&mut sketch);
    let line = sketch.add_line(Point2::new(6.0, 3.3), Point2::new(14.0, 1.0));
    let Some(&Entity::Line { start, .. }) = sketch.entity(line) else {
        panic!("expected a line");
    };
    add(&mut sketch, Constraint::Coincident(start, ellipse));
    add(&mut sketch, Constraint::Tangent(line, ellipse));

    let solved = solve(&sketch).unwrap();
    let shape = solved.geometry.ellipse(ellipse).unwrap();
    let (from, to) = solved.geometry.line_endpoints(line).unwrap();
    let parameter = shape.parameter_of(from);
    let along = shape.tangent_at(parameter).normalize();

    assert!(level(&shape, from).abs() < EXACT);
    assert!(along.perp_dot((to - from).normalize()).abs() < 1e-7);
    assert!(solved.solution.redundancies().is_empty());
    assert_eq!(solved.solution.degrees_of_freedom(), 2);
}

#[test]
fn concentric_puts_a_circle_on_an_ellipse_centre() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = pinned_ellipse(&mut sketch);
    let circle = sketch.add_circle(Point2::new(1.0, 1.0), 2.0);
    add(&mut sketch, Constraint::Concentric(circle, ellipse));

    let solved = solve(&sketch).unwrap();
    let (center, _) = solved.geometry.circle(circle).unwrap();

    assert!(center.distance(Point2::ZERO) < EXACT);
}

#[test]
fn ellipse_constraints_refuse_what_does_not_fit() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::ZERO, Point2::new(10.0, 0.0), 4.0);
    let circle = sketch.add_circle(Point2::new(20.0, 0.0), 2.0);
    let other = sketch.add_ellipse(Point2::new(30.0, 0.0), Point2::new(35.0, 0.0), 2.0);
    let (center, major) = axis_points(&sketch, ellipse);

    for refused in [
        Constraint::Radius {
            entity: ellipse,
            value: mm(3.0),
        },
        Constraint::MajorRadius {
            ellipse: circle,
            value: mm(3.0),
        },
        Constraint::Tangent(ellipse, ellipse),
        Constraint::Tangent(other, center),
        Constraint::Distance {
            from: center,
            to: ellipse,
            value: mm(1.0),
        },
        Constraint::Equal(ellipse, circle),
        Constraint::Coincident(center, ellipse),
        Constraint::Midpoint {
            point: major,
            curve: ellipse,
        },
        Constraint::Midpoint {
            point: center,
            curve: ellipse,
        },
    ] {
        assert!(sketch.check_constraint(&refused).is_err(), "{refused:?}");
    }
    assert_eq!(
        sketch.add_constraint(Constraint::MinorRadius {
            ellipse,
            value: mm(0.0),
        }),
        Err(SketchError::DimensionValue {
            reason: crate::DimensionError::NotPositive
        })
    );

    let distance = add(
        &mut sketch,
        Constraint::Distance {
            from: major,
            to: center,
            value: mm(10.0),
        },
    );
    let level = add(&mut sketch, Constraint::HorizontalPoints(center, major));
    assert_eq!(
        sketch.restating(&Constraint::MajorRadius {
            ellipse,
            value: mm(10.0),
        }),
        Some(distance)
    );
    assert_eq!(
        sketch.restating(&Constraint::Horizontal(ellipse)),
        Some(level)
    );
}

#[test]
fn an_ellipse_needs_a_positive_minor_radius_and_distinct_points() {
    let mut sketch = Sketch::new(Plane::XY);
    let center = sketch.add_point(Point2::ZERO);
    let major = sketch.add_point(Point2::new(5.0, 0.0));
    let insert =
        |sketch: &mut Sketch, entity: Entity| sketch.insert_entity(EntityId::from_raw(20), entity);

    assert_eq!(
        insert(
            &mut sketch,
            Entity::Ellipse {
                center,
                major,
                minor_radius: f64::NAN,
            }
        ),
        Err(SketchError::InvalidMinorRadius)
    );
    assert!(matches!(
        insert(
            &mut sketch,
            Entity::Ellipse {
                center,
                major: center,
                minor_radius: 2.0,
            }
        ),
        Err(SketchError::SameEntity { .. })
    ));
    insert(
        &mut sketch,
        Entity::Ellipse {
            center,
            major,
            minor_radius: 2.0,
        },
    )
    .unwrap();
    let shape = sketch.ellipse(EntityId::from_raw(20)).unwrap();
    assert!(shape.is_full());
    assert_close(
        shape
            .closest_point(Point2::new(0.0, 7.0))
            .distance(Point2::new(0.0, 2.0)),
        0.0,
    );
    assert_eq!(Vector2::X, shape.axis());
}

#[test]
fn equal_ellipses_share_both_radii() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = pinned_ellipse(&mut sketch);
    let other = sketch.add_ellipse(Point2::new(30.0, 5.0), Point2::new(36.0, 9.0), 1.5);
    add(&mut sketch, Constraint::Equal(other, ellipse));

    let solved = solve(&sketch).unwrap();
    let shape = solved.geometry.ellipse(other).unwrap();

    assert_close(shape.major_radius(), 10.0);
    assert_close(shape.minor_radius, 4.0);
    assert_eq!(solved.solution.degrees_of_freedom(), 3);
}

#[test]
fn equal_ellipses_sharing_their_axis_hold_only_the_minor_radius() {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_elliptical_arc(
        Point2::ZERO,
        Point2::new(10.0, 0.0),
        4.0,
        Point2::new(10.0 * FRAC_PI_4.cos(), 4.0 * FRAC_PI_4.sin()),
        Point2::new(0.0, 4.0),
    );
    let (center, major) = axis_points(&sketch, first);
    let start = sketch.add_point(Point2::new(-10.0, 0.0));
    let end = sketch.add_point(Point2::new(0.0, -3.0));
    let second = EntityId::from_raw(sketch.next_id());
    sketch
        .insert_entity(
            second,
            Entity::EllipticalArc {
                center,
                major,
                minor_radius: 3.0,
                start,
                end,
            },
        )
        .unwrap();
    add(&mut sketch, Constraint::Equal(first, second));

    let solved = solve(&sketch).unwrap();

    assert_close(
        solved.geometry.ellipse(second).unwrap().minor_radius,
        solved.geometry.ellipse(first).unwrap().minor_radius,
    );
    assert!(solved.solution.redundancies().is_empty());
    assert_eq!(solved.solution.degrees_of_freedom(), 9);
}

#[test]
fn the_middle_of_an_elliptical_arc_halves_its_sweep() {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_elliptical_arc(
        Point2::ZERO,
        Point2::new(10.0, 0.0),
        4.0,
        Point2::new(10.0, 0.0),
        Point2::new(-10.0 * 0.6, -4.0 * 0.8),
    );
    let (center, major) = axis_points(&sketch, arc);
    let (start, end) = arc_ends(&sketch, arc);
    for point in [center, major, start, end] {
        fix(&mut sketch, point);
    }
    let middle = sketch.add_point(Point2::new(1.0, 1.0));
    add(
        &mut sketch,
        Constraint::Midpoint {
            point: middle,
            curve: arc,
        },
    );

    let solved = solve(&sketch).unwrap();
    let shape = solved.geometry.ellipse(arc).unwrap();
    let expected = shape.point_at(shape.start + shape.sweep / 2.0);

    assert!(solved.geometry.point(middle).unwrap().distance(expected) < EXACT);
    assert!(expected.y > 0.0);
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
}

#[test]
fn an_arc_sharing_a_point_with_an_ellipse_and_tangent_runs_along_it_there() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = pinned_ellipse(&mut sketch);
    let arc = sketch.add_arc(
        Point2::new(13.0, 4.0),
        Point2::new(8.5, 2.5),
        Point2::new(17.0, 7.0),
    );
    let Some(&Entity::Arc { start, .. }) = sketch.entity(arc) else {
        panic!("expected an arc");
    };
    add(&mut sketch, Constraint::Coincident(start, ellipse));
    let free = solve(&sketch).unwrap().solution.degrees_of_freedom();
    add(&mut sketch, Constraint::Tangent(arc, ellipse));

    let solved = solve(&sketch).unwrap();
    let shape = solved.geometry.ellipse(ellipse).unwrap();
    let round = solved.geometry.arc(arc).unwrap();
    let joint = solved.geometry.point(start).unwrap();
    let along = shape.tangent_at(shape.parameter_of(joint)).normalize();

    assert!(level(&shape, joint).abs() < EXACT);
    assert!(along.dot((joint - round.center).normalize()).abs() < 1e-7);
    assert!(solved.solution.redundancies().is_empty());
    assert_eq!(solved.solution.degrees_of_freedom(), free - 1);
}

fn gap_to(shape: &EllipseGeometry, point: Point2) -> f64 {
    let full = EllipseGeometry::full(shape.center, shape.center + shape.major, shape.minor_radius);
    full.closest_point(point).distance(point)
}

#[test]
fn a_circle_apart_from_an_ellipse_touches_it_at_a_parameter_along_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = pinned_ellipse(&mut sketch);
    let circle = sketch.add_circle(Point2::new(16.0, 3.0), 2.0);
    let Some(&Entity::Circle { center, .. }) = sketch.entity(circle) else {
        panic!("expected a circle");
    };
    fix(&mut sketch, center);
    add(&mut sketch, Constraint::Tangent(circle, ellipse));

    let solved = solve(&sketch).unwrap();
    let shape = solved.geometry.ellipse(ellipse).unwrap();
    let (middle, radius) = solved.geometry.circle(circle).unwrap();

    assert_close(radius, gap_to(&shape, middle));
    assert!(solved.solution.redundancies().is_empty());
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
}

#[test]
fn a_circle_inside_an_ellipse_touches_it_from_within() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = pinned_ellipse(&mut sketch);
    let circle = sketch.add_circle(Point2::new(3.0, 0.5), 1.0);
    let Some(&Entity::Circle { center, .. }) = sketch.entity(circle) else {
        panic!("expected a circle");
    };
    fix(&mut sketch, center);
    add(&mut sketch, Constraint::Tangent(ellipse, circle));

    let solved = solve(&sketch).unwrap();
    let shape = solved.geometry.ellipse(ellipse).unwrap();
    let (middle, radius) = solved.geometry.circle(circle).unwrap();

    assert_close(radius, gap_to(&shape, middle));
    assert!(level(&shape, middle) < 0.0);
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
}

#[test]
fn a_point_keeps_its_distance_from_an_ellipse_square_to_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = pinned_ellipse(&mut sketch);
    let point = sketch.add_point(Point2::new(12.0, 5.0));
    let distance = add(
        &mut sketch,
        Constraint::Distance {
            from: point,
            to: ellipse,
            value: mm(2.0),
        },
    );

    let solved = solve(&sketch).unwrap();
    let shape = solved.geometry.ellipse(ellipse).unwrap();
    let at = solved.geometry.point(point).unwrap();

    assert_close(gap_to(&shape, at), 2.0);
    assert!(level(&shape, at) > 0.0);
    assert_close(
        solved
            .geometry
            .measured(solved.geometry.constraint(distance).unwrap())
            .unwrap(),
        2.0,
    );
    assert_eq!(solved.solution.degrees_of_freedom(), 1);
}

#[test]
fn a_line_keeps_its_gap_from_an_ellipse_on_the_side_it_was_drawn() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = pinned_ellipse(&mut sketch);
    let line = sketch.add_line(Point2::new(-5.0, -6.0), Point2::new(5.0, -5.0));
    add(&mut sketch, Constraint::Horizontal(line));
    let distance = add(
        &mut sketch,
        Constraint::Distance {
            from: ellipse,
            to: line,
            value: mm(3.0),
        },
    );

    let solved = solve(&sketch).unwrap();
    let (start, end) = solved.geometry.line_endpoints(line).unwrap();

    assert_close(start.y, -7.0);
    assert_close(end.y, -7.0);
    assert_close(
        solved
            .geometry
            .measured(solved.geometry.constraint(distance).unwrap())
            .unwrap(),
        3.0,
    );
}

#[test]
fn a_circle_keeps_its_gap_from_an_ellipse() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = pinned_ellipse(&mut sketch);
    let circle = sketch.add_circle(Point2::new(20.0, 0.0), 3.0);
    let Some(&Entity::Circle { center, .. }) = sketch.entity(circle) else {
        panic!("expected a circle");
    };
    fix(&mut sketch, center);
    add(
        &mut sketch,
        Constraint::Distance {
            from: circle,
            to: ellipse,
            value: mm(4.0),
        },
    );

    let solved = solve(&sketch).unwrap();

    assert_close(solved.geometry.circle(circle).unwrap().1, 6.0);
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
}

#[test]
fn a_point_on_a_slanted_ellipse_and_its_minor_axis_stays_at_the_minor_axis_end() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::new(5.0, 5.0), Point2::new(13.0, 11.0), 4.0);
    let (center, major) = axis_points(&sketch, ellipse);
    fix(&mut sketch, center);
    fix(&mut sketch, major);
    let point = sketch.add_point(Point2::new(1.0, 9.0));
    add(&mut sketch, Constraint::Coincident(point, ellipse));
    add(&mut sketch, Constraint::OnMinorAxis { point, ellipse });
    add(
        &mut sketch,
        Constraint::MinorRadius {
            ellipse,
            value: mm(3.0),
        },
    );

    let solved = solve(&sketch).unwrap();
    let shape = solved.geometry.ellipse(ellipse).unwrap();
    let at = solved.geometry.point(point).unwrap();

    assert!(at.distance(shape.point_at(FRAC_PI_2)) < EXACT);
    assert!(solved.solution.redundancies().is_empty());
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
    assert!(
        sketch
            .check_constraint(&Constraint::OnMinorAxis {
                point: center,
                ellipse,
            })
            .is_err()
    );
}

fn upper_ellipse(sketch: &mut Sketch, minor_radius: f64) -> EntityId {
    let ellipse = sketch.add_ellipse(Point2::new(0.0, 12.0), Point2::new(8.0, 12.0), minor_radius);
    let (center, major) = axis_points(sketch, ellipse);
    fix(sketch, center);
    fix(sketch, major);
    ellipse
}

fn minor_radius(sketch: &Sketch, ellipse: EntityId) -> f64 {
    sketch.ellipse(ellipse).unwrap().minor_radius
}

fn tangents_parallel(sketch: &Sketch, first: EntityId, second: EntityId) -> bool {
    let (on_first, on_second) = sketch.ellipse_gap(first, second).unwrap();
    let direction = |curve: EntityId, at: Point2| {
        let shape = sketch.ellipse(curve).unwrap();
        shape.tangent_at(shape.parameter_of(at)).normalize()
    };
    on_first.distance(on_second) < EXACT
        && direction(first, on_first)
            .perp_dot(direction(second, on_second))
            .abs()
            < 1e-6
}

#[test]
fn two_ellipses_sharing_no_point_touch_at_a_parameter_on_each() {
    let mut sketch = Sketch::new(Plane::XY);
    let lower = pinned_ellipse(&mut sketch);
    let upper = upper_ellipse(&mut sketch, 6.0);
    add(&mut sketch, Constraint::Tangent(upper, lower));

    let solved = solve(&sketch).unwrap();

    assert_close(minor_radius(&solved.geometry, upper), 8.0);
    assert!(tangents_parallel(&solved.geometry, lower, upper));
    assert!(solved.solution.redundancies().is_empty());
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
}

#[test]
fn two_ellipses_keep_their_gap_where_they_come_closest() {
    let mut sketch = Sketch::new(Plane::XY);
    let lower = pinned_ellipse(&mut sketch);
    let upper = upper_ellipse(&mut sketch, 6.0);
    let distance = add(
        &mut sketch,
        Constraint::Distance {
            from: lower,
            to: upper,
            value: mm(1.5),
        },
    );

    let solved = solve(&sketch).unwrap();

    assert_close(minor_radius(&solved.geometry, upper), 6.5);
    assert_close(
        solved
            .geometry
            .measured(solved.geometry.constraint(distance).unwrap())
            .unwrap(),
        1.5,
    );
    assert_close(
        sketch
            .measured(sketch.constraint(distance).unwrap())
            .unwrap(),
        2.0,
    );
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
}

fn arch_over_ellipse(sketch: &mut Sketch, middle_height: f64) -> (EntityId, EntityId) {
    let spline = sketch.add_spline(&[
        Point2::new(-6.0, 9.0),
        Point2::new(0.0, middle_height),
        Point2::new(6.0, 9.0),
    ]);
    let Some(Entity::Spline { points, .. }) = sketch.entity(spline).cloned() else {
        panic!("expected a spline");
    };
    fix(sketch, points[0]);
    fix(sketch, points[2]);
    add(
        sketch,
        Constraint::VerticalPoints(points[1], EntityId::ORIGIN),
    );
    (spline, points[1])
}

#[test]
fn a_spline_sharing_no_end_with_an_ellipse_touches_it_at_a_parameter_on_each() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = pinned_ellipse(&mut sketch);
    let (spline, middle) = arch_over_ellipse(&mut sketch, 2.0);
    add(&mut sketch, Constraint::Tangent(ellipse, spline));

    let solved = solve(&sketch).unwrap();

    assert_close(solved.geometry.point(middle).unwrap().y, -1.0);
    assert!(solved.solution.redundancies().is_empty());
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
}

#[test]
fn a_spline_keeps_its_gap_from_an_ellipse_where_it_bulges_toward_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = pinned_ellipse(&mut sketch);
    let (spline, middle) = arch_over_ellipse(&mut sketch, 2.0);
    let distance = add(
        &mut sketch,
        Constraint::Distance {
            from: spline,
            to: ellipse,
            value: mm(2.0),
        },
    );

    let solved = solve(&sketch).unwrap();

    assert_close(solved.geometry.point(middle).unwrap().y, 3.0);
    assert_close(
        solved
            .geometry
            .measured(solved.geometry.constraint(distance).unwrap())
            .unwrap(),
        2.0,
    );
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
}

fn quarter_arc(sketch: &mut Sketch) -> (EntityId, EntityId) {
    let arc = sketch.add_elliptical_arc(
        Point2::ZERO,
        Point2::new(10.0, 0.0),
        4.0,
        Point2::new(10.0, 0.0),
        Point2::new(0.0, 4.0),
    );
    let (center, major) = axis_points(sketch, arc);
    let (start, end) = arc_ends(sketch, arc);
    for point in [center, major, start, end] {
        fix(sketch, point);
    }
    (arc, end)
}

#[test]
fn a_spline_ending_on_an_elliptical_arc_and_tangent_leaves_along_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let (arc, top) = quarter_arc(&mut sketch);
    let spline = sketch.add_spline(&[
        Point2::new(0.0, 4.0),
        Point2::new(-5.0, 6.0),
        Point2::new(-10.0, 0.0),
    ]);
    let Some(Entity::Spline { points, .. }) = sketch.entity(spline).cloned() else {
        panic!("expected a spline");
    };
    let below = sketch.add_point(Point2::new(-5.0, 0.0));
    fix(&mut sketch, below);
    fix(&mut sketch, points[2]);
    add(&mut sketch, Constraint::Coincident(points[0], top));
    add(&mut sketch, Constraint::VerticalPoints(points[1], below));
    add(&mut sketch, Constraint::Tangent(spline, arc));

    let solved = solve(&sketch).unwrap();

    assert_close(solved.geometry.point(points[1]).unwrap().y, 4.0);
    assert!(solved.solution.redundancies().is_empty());
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
}

#[test]
fn ellipses_sharing_a_point_and_tangent_run_along_each_other_there() {
    let mut sketch = Sketch::new(Plane::XY);
    let (arc, top) = quarter_arc(&mut sketch);
    let upper = sketch.add_ellipse(Point2::new(2.0, 12.0), Point2::new(10.0, 12.0), 8.3);
    let (center, major) = axis_points(&sketch, upper);
    let anchor = sketch.add_point(Point2::new(0.0, 12.0));
    fix(&mut sketch, anchor);
    add(&mut sketch, Constraint::HorizontalPoints(center, anchor));
    add(&mut sketch, Constraint::HorizontalPoints(center, major));
    add(
        &mut sketch,
        Constraint::Distance {
            from: center,
            to: major,
            value: mm(8.0),
        },
    );
    add(&mut sketch, Constraint::Coincident(top, upper));
    add(&mut sketch, Constraint::Tangent(arc, upper));

    let solved = solve(&sketch).unwrap();

    assert_close(solved.geometry.point(center).unwrap().x, 0.0);
    assert_close(minor_radius(&solved.geometry, upper), 8.0);
    assert!(solved.solution.redundancies().is_empty());
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
}

#[test]
fn an_angle_from_an_elliptical_arc_is_taken_from_its_tangent_where_a_line_leaves_it() {
    for (at_top, degrees) in [(true, 30.0), (false, 120.0)] {
        let mut sketch = Sketch::new(Plane::XY);
        let (arc, top) = quarter_arc(&mut sketch);
        let (start, _) = arc_ends(&sketch, arc);
        let (joint, from) = if at_top {
            (top, Point2::new(0.0, 4.0))
        } else {
            (start, Point2::new(10.0, 0.0))
        };
        let line = sketch.add_line(from, from + Vector2::new(3.0, 5.0));
        let Some(&Entity::Line {
            start: line_start,
            end,
        }) = sketch.entity(line)
        else {
            panic!("expected a line");
        };
        add(&mut sketch, Constraint::Coincident(line_start, joint));
        add(
            &mut sketch,
            Constraint::Distance {
                from: line_start,
                to: end,
                value: mm(5.0),
            },
        );
        let angle = add(
            &mut sketch,
            Constraint::Angle {
                from: arc,
                to: line,
                reversed: false,
                value: Expression::Measure(degrees, Unit::Degree),
            },
        );

        let solved = solve(&sketch).unwrap();
        let leaving = if at_top {
            Vector2::new(1.0, 0.0)
        } else {
            Vector2::new(0.0, 1.0)
        };
        let (begin, finish) = solved.geometry.line_endpoints(line).unwrap();
        let direction = (finish - begin).normalize();

        assert_close(
            leaving
                .perp_dot(direction)
                .atan2(leaving.dot(direction))
                .to_degrees(),
            degrees,
        );
        assert_close(
            solved
                .geometry
                .measured(solved.geometry.constraint(angle).unwrap())
                .unwrap(),
            degrees,
        );
        assert!(solved.solution.redundancies().is_empty());
        assert_eq!(solved.solution.degrees_of_freedom(), 0);
    }
}
