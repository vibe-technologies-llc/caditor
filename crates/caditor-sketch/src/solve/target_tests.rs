use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2, Vector2};

use crate::{
    Constraint, ConstraintId, EllipseEnd, Entity, EntityId, FitSpacing, PointTarget, Quadrant,
    Sketch, SketchError, Solved, SplineKind,
};

const EXACT: f64 = 1e-7;

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

fn ends(sketch: &Sketch, line: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(line) {
        Some(Entity::Line { start, end }) => (*start, *end),
        other => panic!("expected a line, found {other:?}"),
    }
}

fn pin(sketch: &mut Sketch, point: EntityId, at: Point2) -> ConstraintId {
    add(sketch, Constraint::Fix { point, at })
}

fn pin_line(sketch: &mut Sketch, line: EntityId, start: Point2, end: Point2) {
    let (first, second) = ends(sketch, line);
    pin(sketch, first, start);
    pin(sketch, second, end);
}

fn at(solved: &Solved, point: EntityId) -> Point2 {
    solved.geometry.point(point).unwrap()
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < EXACT,
        "{actual} is not {expected}"
    );
}

#[test]
fn a_point_level_with_a_line_middle_follows_the_line_and_keeps_its_other_freedom() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 4.0));
    let point = sketch.add_point(Point2::new(20.0, 2.0));
    add(
        &mut sketch,
        Constraint::HorizontalPoints(point, PointTarget::Middle(line)),
    );
    pin_line(&mut sketch, line, Point2::ZERO, Point2::new(10.0, 10.0));

    let solved = solve(&sketch).unwrap();

    assert_close(at(&solved, point).y, 5.0);
    assert_close(at(&solved, point).x, 20.0);
    assert_eq!(solved.solution.degrees_of_freedom(), 1);
}

#[test]
fn a_point_upright_with_a_crossing_of_a_line_and_a_circle_stays_on_the_crossing_it_was_drawn_at() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::ZERO, 5.0);
    let centre = sketch.center_of(circle).unwrap();
    let line = sketch.add_line(Point2::new(-10.0, 1.0), Point2::new(10.0, 1.0));
    let drawn = 24.0_f64.sqrt();
    let point = sketch.add_point(Point2::new(drawn, 20.0));
    let upright = add(
        &mut sketch,
        Constraint::VerticalPoints(point, PointTarget::Crossing(line, circle)),
    );
    pin(&mut sketch, centre, Point2::ZERO);
    add(
        &mut sketch,
        Constraint::Radius {
            entity: circle,
            value: mm(5.0),
        },
    );
    pin_line(
        &mut sketch,
        line,
        Point2::new(-10.0, 3.0),
        Point2::new(10.0, 3.0),
    );

    let solved = solve(&sketch).unwrap();

    assert_close(at(&solved, point).x, 4.0);
    assert_close(at(&solved, point).y, 20.0);
    assert_eq!(solved.solution.degrees_of_freedom(), 1);
    assert_eq!(solved.solution.redundancy(upright), None);
}

#[test]
fn a_point_level_with_where_a_line_crosses_a_spline_follows_the_line_along_the_spline() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&[
        Point2::ZERO,
        Point2::new(4.0, 6.0),
        Point2::new(8.0, 2.0),
        Point2::new(12.0, 8.0),
    ]);
    let line = sketch.add_line(Point2::new(5.0, -5.0), Point2::new(5.0, 15.0));
    let crossing = sketch.curve_crossings(spline, line);
    let [drawn] = crossing[..] else {
        panic!("one crossing is expected, found {crossing:?}");
    };
    let point = sketch.add_point(Point2::new(20.0, drawn.y));
    add(
        &mut sketch,
        Constraint::HorizontalPoints(point, PointTarget::Crossing(spline, line)),
    );
    if let Some(Entity::Spline { points, .. }) = sketch.entity(spline).cloned() {
        for control in points {
            let place = sketch.point(control).unwrap();
            pin(&mut sketch, control, place);
        }
    }
    pin_line(
        &mut sketch,
        line,
        Point2::new(7.0, -5.0),
        Point2::new(7.0, 15.0),
    );

    let solved = solve(&sketch).unwrap();
    let moved = solved.geometry.curve_crossings(spline, line);

    assert_eq!(moved.len(), 1);
    assert_close(at(&solved, point).y, moved[0].y);
    assert!((moved[0].y - drawn.y).abs() > 0.1);
    assert_eq!(solved.solution.degrees_of_freedom(), 1);
}

#[test]
fn a_point_at_the_centre_of_corners_and_a_point_level_with_it_follow_the_corners() {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        sketch.add_point(Point2::ZERO),
        sketch.add_point(Point2::new(9.0, 0.0)),
        sketch.add_point(Point2::new(3.0, 6.0)),
    ];
    let centre = sketch.add_point(Point2::new(4.0, 2.0));
    let level = sketch.add_point(Point2::new(20.0, 2.0));
    add(
        &mut sketch,
        Constraint::Centroid {
            point: centre,
            corners: corners.to_vec(),
        },
    );
    add(
        &mut sketch,
        Constraint::HorizontalPoints(level, PointTarget::Centroid(corners.to_vec())),
    );
    let moved = [
        Point2::new(1.0, 1.0),
        Point2::new(13.0, 1.0),
        Point2::new(4.0, 10.0),
    ];
    for (corner, to) in corners.into_iter().zip(moved) {
        pin(&mut sketch, corner, to);
    }

    let solved = solve(&sketch).unwrap();

    assert!(at(&solved, centre).distance(Point2::new(6.0, 4.0)) < EXACT);
    assert_close(at(&solved, level).y, 4.0);
    assert_eq!(solved.solution.degrees_of_freedom(), 1);
}

#[test]
fn a_centre_over_lines_and_arcs_takes_their_middles_and_follows_them() {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_arc(Point2::ZERO, Point2::new(10.0, 0.0), Point2::new(0.0, 10.0));
    let line = sketch.add_line(Point2::new(20.0, 0.0), Point2::new(30.0, 0.0));
    let lone = sketch.add_point(Point2::new(0.0, -6.0));
    let centre = sketch.add_point(Point2::new(10.0, 1.0));
    let level = sketch.add_point(Point2::new(40.0, 1.0));
    let members = vec![arc, line, lone];
    let Some(&Entity::Arc { center, start, end }) = sketch.entity(arc) else {
        panic!("an arc was added");
    };
    let shift = Vector2::new(1.0, 2.0);

    add(
        &mut sketch,
        Constraint::Centroid {
            point: centre,
            corners: members.clone(),
        },
    );
    add(
        &mut sketch,
        Constraint::HorizontalPoints(level, PointTarget::Centroid(members)),
    );
    pin(&mut sketch, center, Point2::ZERO + shift);
    pin(&mut sketch, start, Point2::new(10.0, 0.0) + shift);
    pin(&mut sketch, end, Point2::new(0.0, 10.0) + shift);
    pin_line(
        &mut sketch,
        line,
        Point2::new(20.0, 0.0),
        Point2::new(30.0, 0.0),
    );
    pin(&mut sketch, lone, Point2::new(0.0, -6.0));

    let solved = solve(&sketch).unwrap();
    let middle = Point2::ZERO + shift + Vector2::from_angle(std::f64::consts::FRAC_PI_4) * 10.0;
    let expected = (middle + Point2::new(25.0, 0.0) + Point2::new(0.0, -6.0)) / 3.0;

    assert!(at(&solved, centre).distance(expected) < EXACT);
    assert_close(at(&solved, level).y, expected.y);
    assert!(
        solved
            .geometry
            .target_places(&PointTarget::Centroid(vec![arc, line, lone]))
            .first()
            .is_some_and(|place| place.distance(expected) < EXACT)
    );
    assert_eq!(solved.solution.degrees_of_freedom(), 1);
}

#[test]
fn a_point_level_with_a_quadrant_follows_the_circle_as_it_moves_and_grows() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::ZERO, 5.0);
    let centre = sketch.center_of(circle).unwrap();
    let arc = sketch.add_arc(
        Point2::new(30.0, 0.0),
        Point2::new(34.0, 0.0),
        Point2::new(26.0, 0.0),
    );
    let arc_centre = sketch.center_of(arc).unwrap();
    let top = sketch.add_point(Point2::new(15.0, 5.0));
    let left = sketch.add_point(Point2::new(26.0, -20.0));
    add(
        &mut sketch,
        Constraint::HorizontalPoints(
            top,
            PointTarget::Quadrant {
                curve: circle,
                side: Quadrant::Top,
            },
        ),
    );
    add(
        &mut sketch,
        Constraint::VerticalPoints(
            left,
            PointTarget::Quadrant {
                curve: arc,
                side: Quadrant::Left,
            },
        ),
    );
    pin(&mut sketch, centre, Point2::new(2.0, 3.0));
    add(
        &mut sketch,
        Constraint::Radius {
            entity: circle,
            value: mm(7.0),
        },
    );
    pin(&mut sketch, arc_centre, Point2::new(31.0, 0.0));
    add(
        &mut sketch,
        Constraint::Radius {
            entity: arc,
            value: mm(6.0),
        },
    );

    let solved = solve(&sketch).unwrap();

    assert_close(at(&solved, top).y, 10.0);
    assert_close(at(&solved, left).x, 25.0);
}

#[test]
fn a_point_upright_with_a_minor_axis_end_follows_a_turning_ellipse() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::ZERO, Point2::new(8.0, 0.0), 3.0);
    let (centre, major) = match sketch.entity(ellipse) {
        Some(Entity::Ellipse { center, major, .. }) => (*center, *major),
        other => panic!("expected an ellipse, found {other:?}"),
    };
    let point = sketch.add_point(Point2::new(0.0, 20.0));
    let far = sketch.add_point(Point2::new(20.0, 0.0));
    add(
        &mut sketch,
        Constraint::VerticalPoints(
            point,
            PointTarget::EllipseEnd {
                ellipse,
                end: EllipseEnd::Minor,
            },
        ),
    );
    add(
        &mut sketch,
        Constraint::HorizontalPoints(
            far,
            PointTarget::EllipseEnd {
                ellipse,
                end: EllipseEnd::FarMajor,
            },
        ),
    );
    pin(&mut sketch, centre, Point2::ZERO);
    pin(&mut sketch, major, Point2::new(6.0, 8.0));
    add(
        &mut sketch,
        Constraint::MinorRadius {
            ellipse,
            value: mm(5.0),
        },
    );

    let solved = solve(&sketch).unwrap();

    assert_close(at(&solved, point).x, -4.0);
    assert_close(at(&solved, far).y, -8.0);
    assert_eq!(solved.solution.degrees_of_freedom(), 2);
}

#[test]
fn a_point_on_a_slanted_track_and_on_a_perpendicular_bisector_follows_the_line_it_runs_by() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 10.0));
    let anchor = sketch.add_point(Point2::new(0.0, 10.0));
    let along = sketch.add_point(Point2::new(5.0, 15.0));
    let bisected = sketch.add_point(Point2::new(0.0, 10.0));
    add(
        &mut sketch,
        Constraint::OnDirection {
            point: along,
            through: PointTarget::Point(anchor),
            line,
            square: false,
        },
    );
    add(
        &mut sketch,
        Constraint::OnDirection {
            point: bisected,
            through: PointTarget::Middle(line),
            line,
            square: true,
        },
    );
    pin(&mut sketch, anchor, Point2::new(0.0, 10.0));
    pin_line(&mut sketch, line, Point2::ZERO, Point2::new(10.0, 5.0));

    let solved = solve(&sketch).unwrap();
    let direction = Vector2::new(10.0, 5.0).normalize();
    let middle = Point2::new(5.0, 2.5);

    assert_close(
        direction.perp_dot(at(&solved, along) - Point2::new(0.0, 10.0)),
        0.0,
    );
    assert_close(direction.dot(at(&solved, bisected) - middle), 0.0);
    assert_eq!(solved.solution.degrees_of_freedom(), 2);
}

#[test]
fn a_point_level_with_the_middle_of_an_arc_follows_the_arc() {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_arc(Point2::ZERO, Point2::new(5.0, 0.0), Point2::new(-5.0, 0.0));
    let centre = sketch.center_of(arc).unwrap();
    let point = sketch.add_point(Point2::new(20.0, 5.0));
    add(
        &mut sketch,
        Constraint::HorizontalPoints(point, PointTarget::Middle(arc)),
    );
    pin(&mut sketch, centre, Point2::new(0.0, 2.0));
    add(
        &mut sketch,
        Constraint::Radius {
            entity: arc,
            value: mm(8.0),
        },
    );
    let (start, end) = ends_of_arc(&sketch, arc);
    add(
        &mut sketch,
        Constraint::HorizontalPoints(start, centre.into()),
    );
    add(
        &mut sketch,
        Constraint::HorizontalPoints(end, centre.into()),
    );

    let solved = solve(&sketch).unwrap();

    assert_close(at(&solved, point).y, 10.0);
}

fn ends_of_arc(sketch: &Sketch, arc: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(arc) {
        Some(Entity::Arc { start, end, .. }) => (*start, *end),
        other => panic!("expected an arc, found {other:?}"),
    }
}

#[test]
fn a_point_held_level_with_a_middle_and_fixed_elsewhere_names_both_as_the_conflict() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 4.0));
    let point = sketch.add_point(Point2::new(20.0, 2.0));
    let level = add(
        &mut sketch,
        Constraint::HorizontalPoints(point, PointTarget::Middle(line)),
    );
    pin_line(&mut sketch, line, Point2::ZERO, Point2::new(10.0, 4.0));
    let fixed = pin(&mut sketch, point, Point2::new(20.0, 7.0));

    match solve(&sketch) {
        Err(SketchError::Conflict { constraints }) => {
            assert!(constraints.contains(&level), "{constraints:?}");
            assert!(constraints.contains(&fixed), "{constraints:?}");
        }
        other => panic!("a conflict is expected, found {other:?}"),
    }
}

#[test]
fn a_track_already_held_by_a_fix_is_redundant() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 4.0));
    let point = sketch.add_point(Point2::new(20.0, 2.0));
    pin_line(&mut sketch, line, Point2::ZERO, Point2::new(10.0, 4.0));
    pin(&mut sketch, point, Point2::new(20.0, 2.0));
    let level = add(
        &mut sketch,
        Constraint::HorizontalPoints(point, PointTarget::Middle(line)),
    );

    let solved = solve(&sketch).unwrap();

    assert!(solved.solution.redundancy(level).is_some());
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
}

#[test]
fn targets_that_do_not_fit_are_refused_in_words() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 4.0));
    let circle = sketch.add_circle(Point2::new(3.0, 3.0), 2.0);
    let point = sketch.add_point(Point2::new(20.0, 2.0));
    let (start, _) = ends(&sketch, line);

    assert!(matches!(
        sketch.add_constraint(Constraint::HorizontalPoints(
            point,
            PointTarget::Middle(circle)
        )),
        Err(SketchError::WrongKind { .. })
    ));
    assert!(matches!(
        sketch.add_constraint(Constraint::VerticalPoints(
            point,
            PointTarget::Crossing(line, line)
        )),
        Err(SketchError::SameEntity { .. })
    ));
    assert_eq!(
        sketch.add_constraint(Constraint::Centroid {
            point,
            corners: vec![start],
        }),
        Err(SketchError::TooFewCorners)
    );
    assert!(matches!(
        sketch.add_constraint(Constraint::Centroid {
            point,
            corners: vec![start, circle],
        }),
        Err(SketchError::WrongKind { .. })
    ));
    assert!(matches!(
        sketch.add_constraint(Constraint::OnDirection {
            point,
            through: PointTarget::Point(point),
            line,
            square: false,
        }),
        Err(SketchError::SameEntity { .. })
    ));
}

#[test]
fn a_track_names_its_target_in_plain_words_and_restates_only_its_own_kind() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 4.0));
    let circle = sketch.add_circle(Point2::new(3.0, 3.0), 2.0);
    let point = sketch.add_point(Point2::new(20.0, 2.0));
    let line_label = sketch.entity_label(line);
    let circle_label = sketch.entity_label(circle);
    let point_label = sketch.entity_label(point);
    let crossing = add(
        &mut sketch,
        Constraint::HorizontalPoints(point, PointTarget::Crossing(line, circle)),
    );
    let bisector = Constraint::OnDirection {
        point,
        through: PointTarget::Middle(line),
        line,
        square: true,
    };

    assert_eq!(
        sketch.describe_constraint(crossing),
        format!("Horizontal {point_label} and the crossing of {line_label} and {circle_label}")
    );
    assert_eq!(
        sketch.describe(&bisector),
        format!("Perpendicular bisector of {line_label} at {point_label}")
    );
    assert_eq!(
        sketch.restating(&Constraint::HorizontalPoints(
            point,
            PointTarget::Crossing(circle, line)
        )),
        Some(crossing)
    );
    assert_eq!(
        sketch.restating(&Constraint::VerticalPoints(
            point,
            PointTarget::Crossing(line, circle)
        )),
        None
    );
}

#[test]
fn a_spline_ending_on_a_circle_crosses_it_at_that_end() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::ZERO, 5.0);
    let end = Point2::new(3.0, 4.0 + 1e-12);
    let spline = sketch.add_spline_of(
        &[end, Point2::new(8.0, 1.0), Point2::new(9.0, -6.0)],
        SplineKind::Fit {
            closed: false,
            spacing: FitSpacing::Centripetal,
        },
    );

    let places = sketch.target_places(&PointTarget::Crossing(circle, spline));

    assert!(
        places.iter().any(|place| place.distance(end) < EXACT),
        "{places:?} misses the end at {end}"
    );
}
