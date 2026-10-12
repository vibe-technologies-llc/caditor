use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2, Vector2};

use crate::{Constraint, ConstraintId, Entity, EntityId, Sketch, Solved, SplitError};

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

fn elliptic(sketch: &Sketch, curve: EntityId) -> (EntityId, EntityId, EntityId, EntityId) {
    match sketch.entity(curve) {
        Some(Entity::EllipticalArc {
            center,
            major,
            start,
            end,
            ..
        }) => (*center, *major, *start, *end),
        other => panic!("expected an elliptical arc, found {other:?}"),
    }
}

#[test]
fn an_elliptical_arc_splits_into_two_arcs_of_one_ellipse() {
    let mut sketch = Sketch::new(Plane::XY);
    let at = |angle: f64| Point2::new(10.0 * angle.cos(), 4.0 * angle.sin());
    let arc =
        sketch.add_elliptical_arc(Point2::ZERO, Point2::new(10.0, 0.0), 4.0, at(0.3), at(2.8));
    let point = sketch.add_point(at(1.2));
    sketch
        .add_constraint(Constraint::Coincident(point, arc))
        .unwrap();
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    let piece = sketch.split_at(arc, point).unwrap();
    let solved = solve(&sketch);
    let (center, major, _, end) = elliptic(&sketch, arc);
    let (piece_center, piece_major, piece_start, _) = elliptic(&sketch, piece);

    assert_eq!((center, major), (piece_center, piece_major));
    assert_eq!((end, piece_start), (point, point));
    assert!(!has(&sketch, &Constraint::Equal(arc, piece)));
    assert!(solved.solution.redundancies().is_empty());
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);
}

#[test]
fn an_elliptical_arc_split_on_its_major_axis_keeps_one_minor_radius_by_equal() {
    let mut sketch = Sketch::new(Plane::XY);
    let at = |angle: f64| Point2::new(10.0 * angle.cos(), 4.0 * angle.sin());
    let arc =
        sketch.add_elliptical_arc(Point2::ZERO, Point2::new(10.0, 0.0), 4.0, at(2.0), at(4.0));
    let point = sketch.add_point(Point2::new(-10.0, 0.0));
    sketch
        .add_constraint(Constraint::Coincident(point, arc))
        .unwrap();
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    let piece = sketch.split_at(arc, point).unwrap();
    let solved = solve(&sketch);

    assert!(has(&sketch, &Constraint::Equal(arc, piece)));
    assert!(solved.solution.redundancies().is_empty());
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);
}

const PLACED_LABEL: Vector2 = Vector2::new(3.0, -7.0);

fn arc_with_a_placed_radius_and_angle() -> (Sketch, EntityId, EntityId, EntityId, ConstraintId) {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_arc(Point2::ZERO, Point2::new(10.0, 0.0), Point2::new(0.0, 10.0));
    let (_, arc_end) = ends(&sketch, arc);
    let line = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(-20.0, 10.0));
    let (line_start, _) = ends(&sketch, line);
    sketch
        .add_constraint(Constraint::Coincident(arc_end, line_start))
        .unwrap();
    let radius = sketch
        .add_constraint(Constraint::Radius {
            entity: arc,
            value: Expression::Measure(10.0, Unit::Millimetre),
        })
        .unwrap();
    sketch.set_label_offset(radius, Some(PLACED_LABEL)).unwrap();
    let angle = sketch
        .add_constraint(Constraint::Angle {
            from: arc,
            to: line,
            reversed: false,
            value: Expression::Measure(180.0, Unit::Degree),
        })
        .unwrap();
    sketch.set_label_offset(angle, Some(PLACED_LABEL)).unwrap();
    let half = std::f64::consts::FRAC_1_SQRT_2 * 10.0;
    let point = sketch.add_point(Point2::new(half, half));
    sketch
        .add_constraint(Constraint::Coincident(point, arc))
        .unwrap();
    (sketch, arc, line, point, radius)
}

#[test]
fn a_dimension_kept_across_a_split_keeps_its_placed_label() {
    let (mut sketch, arc, _, point, radius) = arc_with_a_placed_radius_and_angle();

    sketch.split_at(arc, point).unwrap();

    assert!(matches!(
        sketch.constraint(radius),
        Some(Constraint::Radius { .. })
    ));
    assert_eq!(sketch.label_offset(radius), Some(PLACED_LABEL));
}

#[test]
fn a_dimension_moved_to_the_far_piece_of_a_split_keeps_its_placed_label() {
    let (mut sketch, arc, line, point, _) = arc_with_a_placed_radius_and_angle();

    let piece = sketch.split_at(arc, point).unwrap();

    let moved: Vec<Option<Vector2>> = sketch
        .constraints()
        .filter(|(_, constraint)| {
            matches!(constraint, Constraint::Angle { from, to, .. } if *from == piece && *to == line)
        })
        .map(|(id, _)| sketch.label_offset(id))
        .collect();
    assert_eq!(moved, vec![Some(PLACED_LABEL)]);
}

#[test]
fn pieces_remember_the_curve_they_were_split_from_until_removed() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let other = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(40.0, 5.0));
    let first = sketch.add_point(Point2::new(10.0, 0.0));
    let second = sketch.add_point(Point2::new(30.0, 0.0));
    for point in [first, second] {
        sketch
            .add_constraint(Constraint::Coincident(point, line))
            .unwrap();
    }

    let piece = sketch.split_at(line, second).unwrap();
    let inner = sketch.split_at(line, first).unwrap();
    let family = sketch.pieces_of(inner);
    for id in sketch.constraints_using(inner) {
        sketch.remove_constraint(id).unwrap();
    }
    sketch.remove_unused_entity(inner).unwrap();

    assert_eq!(family, [line, inner, piece].into());
    assert_eq!(sketch.split_from(line), None);
    assert_eq!(sketch.split_from(piece), Some(line));
    assert_eq!(sketch.split_from(inner), None);
    assert_eq!(sketch.pieces_of(other), [other].into());
    assert_eq!(sketch.pieces_of(piece), [line, piece].into());
    assert_eq!(
        sketch.set_split_from(line, Some(line)),
        Err(crate::SketchError::SplitFromItself {
            entity: line,
            label: sketch.entity_label(line),
        })
    );
}

#[test]
fn a_trimmed_piece_and_a_broken_one_remember_their_curve_and_the_kept_part_stays_itself() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    sketch.add_line(Point2::new(10.0, -5.0), Point2::new(10.0, 5.0));
    sketch.add_line(Point2::new(20.0, -5.0), Point2::new(20.0, 5.0));
    sketch.add_line(Point2::new(30.0, -5.0), Point2::new(30.0, 5.0));

    let trimmed = sketch.trim(line, Point2::new(15.0, 0.0)).unwrap();
    let crate::Trimmed::Split { piece } = trimmed else {
        panic!("the line is split, found {trimmed:?}");
    };
    let broken = sketch.break_curve(piece).unwrap();

    assert_eq!(sketch.split_from(line), None);
    assert_eq!(sketch.split_from(piece), Some(line));
    assert!(!broken.is_empty());
    for made in broken.iter().filter(|made| **made != piece) {
        assert_eq!(sketch.split_from(*made), Some(line));
    }
    assert_eq!(
        sketch.pieces_of(line).len(),
        2 + broken.iter().filter(|made| **made != piece).count()
    );
}
