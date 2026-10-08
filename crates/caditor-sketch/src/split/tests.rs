use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2};

use crate::{Constraint, Entity, EntityId, Sketch, Solved, SplitError};

const EXACT: f64 = 1e-7;

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn solve(sketch: &Sketch) -> Solved {
    match sketch.solve(&no_parameters, &|| false) {
        Ok(solved) => solved,
        Err(error) => panic!("the sketch does not solve: {error:?}"),
    }
}

fn ends(sketch: &Sketch, curve: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(curve) {
        Some(Entity::Line { start, end } | Entity::Arc { start, end, .. }) => (*start, *end),
        other => panic!("expected a line or an arc, found {other:?}"),
    }
}

fn has(sketch: &Sketch, constraint: &Constraint) -> bool {
    sketch
        .constraints()
        .any(|(_, existing)| existing == constraint)
}

#[test]
fn a_line_splits_at_a_point_on_it_into_two_collinear_lines_meeting_there() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 10.0));
    let point = sketch.add_point(Point2::new(20.0, 5.0));
    sketch
        .add_constraint(Constraint::Coincident(point, line))
        .unwrap();
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    let piece = sketch.split_at(line, point).unwrap();
    let solved = solve(&sketch);

    assert_eq!(ends(&sketch, line).1, point);
    assert_eq!(ends(&sketch, piece).0, point);
    assert!(has(&sketch, &Constraint::Collinear(line, piece)));
    assert!(!has(&sketch, &Constraint::Coincident(point, line)));
    assert!(solved.solution.redundancies().is_empty());
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);
}

#[test]
fn a_level_line_split_at_its_midpoint_keeps_both_pieces_level_and_equal() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let point = sketch.add_point(Point2::new(20.0, 0.0));
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    sketch
        .add_constraint(Constraint::Midpoint { point, curve: line })
        .unwrap();
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    let piece = sketch.split_at(line, point).unwrap();
    let solved = solve(&sketch);

    assert!(has(&sketch, &Constraint::Horizontal(piece)));
    assert!(has(&sketch, &Constraint::Equal(line, piece)));
    assert!(solved.solution.redundancies().is_empty());
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);
}

#[test]
fn an_arc_splits_into_two_arcs_on_one_centre_and_a_far_tangent_follows_its_end() {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_arc(Point2::ZERO, Point2::new(10.0, 0.0), Point2::new(0.0, 10.0));
    let (_, arc_end) = ends(&sketch, arc);
    let line = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(-20.0, 10.0));
    let (line_start, _) = ends(&sketch, line);
    sketch
        .add_constraint(Constraint::Coincident(arc_end, line_start))
        .unwrap();
    sketch
        .add_constraint(Constraint::Tangent(arc, line))
        .unwrap();
    sketch
        .add_constraint(Constraint::Radius {
            entity: arc,
            value: Expression::Measure(10.0, Unit::Millimetre),
        })
        .unwrap();
    let half = std::f64::consts::FRAC_1_SQRT_2 * 10.0;
    let point = sketch.add_point(Point2::new(half, half));
    sketch
        .add_constraint(Constraint::Coincident(point, arc))
        .unwrap();

    let piece = sketch.split_at(arc, point).unwrap();
    let solved = solve(&sketch);
    let geometry = solved.geometry;

    let (Some(Entity::Arc { center: first, .. }), Some(Entity::Arc { center: second, .. })) =
        (sketch.entity(arc), sketch.entity(piece))
    else {
        panic!("both pieces are arcs");
    };
    assert_eq!(first, second);
    assert!(has(&sketch, &Constraint::Tangent(piece, line)));
    assert!(!has(&sketch, &Constraint::Tangent(arc, line)));
    assert!((geometry.arc(piece).unwrap().radius - 10.0).abs() < EXACT);
    assert!(solved.solution.redundancies().is_empty());
}

#[test]
fn a_point_off_the_curve_or_at_its_end_and_a_circle_are_refused() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let (start, _) = ends(&sketch, line);
    let off = sketch.add_point(Point2::new(20.0, 3.0));
    let beyond = sketch.add_point(Point2::new(50.0, 0.0));
    let circle = sketch.add_circle(Point2::new(0.0, 30.0), 5.0);
    let on_circle = sketch.add_point(Point2::new(5.0, 30.0));

    assert!(matches!(
        sketch.check_split(line, off),
        Err(SplitError::NotOnCurve { .. })
    ));
    assert!(matches!(
        sketch.check_split(line, beyond),
        Err(SplitError::NotOnCurve { .. })
    ));
    assert!(matches!(
        sketch.check_split(line, start),
        Err(SplitError::NotOnCurve { .. })
    ));
    assert!(matches!(
        sketch.check_split(circle, on_circle),
        Err(SplitError::NotLineOrArc { .. })
    ));
}
