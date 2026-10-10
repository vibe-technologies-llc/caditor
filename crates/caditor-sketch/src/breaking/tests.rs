use caditor_expression::{EvalError, ParameterId, Quantity};
use caditor_geometry::{Plane, Point2};

use crate::{BreakError, Constraint, Entity, EntityId, Sketch, Solved};

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

fn lines(sketch: &Sketch) -> Vec<(Point2, Point2)> {
    sketch
        .entities()
        .filter_map(|(id, entity)| match entity {
            Entity::Line { .. } => sketch.line_endpoints(id),
            _ => None,
        })
        .collect()
}

fn has_line(sketch: &Sketch, from: Point2, to: Point2) -> bool {
    lines(sketch)
        .iter()
        .any(|(start, end)| start.distance(from) < EXACT && end.distance(to) < EXACT)
}

fn assert_clean(sketch: &Sketch) -> Solved {
    let solved = solve(sketch);
    assert!(
        solved.solution.redundancies().is_empty(),
        "redundant: {:?}",
        solved.solution.redundancies()
    );
    solved
}

#[test]
fn a_line_is_broken_at_every_curve_crossing_it_and_the_pieces_stay_joined_to_them() {
    let mut sketch = Sketch::new(Plane::XY);
    let across = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let first = sketch.add_line(Point2::new(10.0, -5.0), Point2::new(10.0, 5.0));
    let second = sketch.add_line(Point2::new(30.0, -5.0), Point2::new(30.0, 5.0));
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    let pieces = sketch.break_curve(across).unwrap();
    let solved = assert_clean(&sketch);

    assert_eq!(pieces.len(), 2);
    assert!(has_line(&sketch, Point2::ZERO, Point2::new(10.0, 0.0)));
    assert!(has_line(
        &sketch,
        Point2::new(10.0, 0.0),
        Point2::new(30.0, 0.0)
    ));
    assert!(has_line(
        &sketch,
        Point2::new(30.0, 0.0),
        Point2::new(40.0, 0.0)
    ));
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);

    let (_, end) = ends(&sketch, across);
    assert!(
        sketch
            .constraints()
            .any(|(_, constraint)| *constraint == Constraint::Coincident(end, first))
    );
    assert!(sketch.entity(second).is_some());
}

#[test]
fn two_crossing_lines_broken_together_meet_at_one_shared_point_without_redundancy() {
    let mut sketch = Sketch::new(Plane::XY);
    let across = sketch.add_line(Point2::new(-10.0, 0.0), Point2::new(10.0, 0.0));
    let up = sketch.add_line(Point2::new(0.0, -10.0), Point2::new(0.0, 10.0));
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    let broken = sketch.break_curves(&[across, up]).unwrap();
    let solved = assert_clean(&sketch);

    assert_eq!(broken.curves, 2);
    assert_eq!(broken.pieces.len(), 2);
    assert_eq!(lines(&sketch).len(), 4);
    assert!(has_line(
        &sketch,
        Point2::new(-10.0, 0.0),
        Point2::new(0.0, 0.0)
    ));
    assert!(has_line(
        &sketch,
        Point2::new(0.0, 0.0),
        Point2::new(0.0, 10.0)
    ));
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);
    let centre: Vec<EntityId> = [across, up, broken.pieces[0], broken.pieces[1]]
        .iter()
        .map(|curve| ends(&sketch, *curve))
        .flat_map(|(start, end)| [start, end])
        .filter(|point| {
            sketch
                .point(*point)
                .is_some_and(|at| at.distance(Point2::ZERO) < EXACT)
        })
        .collect();
    assert_eq!(centre.len(), 4);
    assert!(centre.iter().all(|point| *point == centre[0]));
}

#[test]
fn an_arc_is_broken_where_a_line_crosses_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_arc(
        Point2::ZERO,
        Point2::new(10.0, 0.0),
        Point2::new(-10.0, 0.0),
    );
    sketch.add_line(Point2::new(0.0, -2.0), Point2::new(0.0, 20.0));

    let pieces = sketch.break_curve(arc).unwrap();
    let solved = assert_clean(&sketch);

    assert_eq!(pieces.len(), 1);
    let first = solved.geometry.arc(arc).unwrap();
    let second = solved.geometry.arc(pieces[0]).unwrap();
    assert!((first.sweep - std::f64::consts::FRAC_PI_2).abs() < EXACT);
    assert!((second.sweep - std::f64::consts::FRAC_PI_2).abs() < EXACT);
    assert!((first.radius - second.radius).abs() < EXACT);
}

#[test]
fn a_crossing_at_the_end_of_another_curve_breaks_at_that_end_and_joins_the_two() {
    let mut sketch = Sketch::new(Plane::XY);
    let long = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let stem = sketch.add_line(Point2::new(20.0, 15.0), Point2::new(20.0, 0.0));
    let (_, stem_end) = ends(&sketch, stem);

    let pieces = sketch.break_curve(long).unwrap();
    assert_clean(&sketch);

    assert_eq!(pieces.len(), 1);
    assert_eq!(ends(&sketch, long).1, stem_end);
    assert_eq!(ends(&sketch, pieces[0]).0, stem_end);
}

#[test]
fn a_line_is_broken_where_it_crosses_an_axis() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(-10.0, 5.0), Point2::new(10.0, 5.0));

    let pieces = sketch.break_curve(line).unwrap();
    assert_clean(&sketch);

    assert_eq!(pieces.len(), 1);
    let (_, middle) = ends(&sketch, line);
    assert!(
        sketch.constraints().any(|(_, constraint)| *constraint
            == Constraint::Coincident(middle, EntityId::VERTICAL_AXIS))
    );
}

#[test]
fn a_curve_without_crossings_or_that_cannot_be_broken_is_refused_in_words() {
    let mut sketch = Sketch::new(Plane::XY);
    let alone = sketch.add_line(Point2::new(5.0, 5.0), Point2::new(15.0, 5.0));
    let circle = sketch.add_circle(Point2::new(50.0, 50.0), 5.0);
    let spline = sketch.add_spline(&[
        Point2::new(40.0, 0.0),
        Point2::new(45.0, 10.0),
        Point2::new(50.0, 0.0),
    ]);
    let before = sketch.clone();

    assert!(matches!(
        sketch.break_curve(alone),
        Err(BreakError::NoCrossing { .. })
    ));
    assert!(matches!(
        sketch.break_curve(circle),
        Err(BreakError::NotLineOrArc { .. })
    ));
    assert!(matches!(
        sketch.break_curve(spline),
        Err(BreakError::NotLineOrArc { .. })
    ));
    assert!(matches!(
        sketch.break_curve(EntityId::HORIZONTAL_AXIS),
        Err(BreakError::Reference { .. })
    ));
    assert_eq!(sketch.break_curves(&[]), Err(BreakError::NothingSelected));
    assert_eq!(
        sketch.break_curves(&[alone, circle]),
        Err(BreakError::NothingToBreak)
    );
    assert!(sketch.same_content(&before));
}

#[test]
fn a_selection_breaks_the_curves_that_cross_and_skips_the_ones_that_cannot() {
    let mut sketch = Sketch::new(Plane::XY);
    let across = sketch.add_line(Point2::new(5.0, 5.0), Point2::new(25.0, 5.0));
    sketch.add_line(Point2::new(15.0, 0.0), Point2::new(15.0, 10.0));
    let circle = sketch.add_circle(Point2::new(50.0, 50.0), 5.0);

    let broken = sketch.break_curves(&[circle, across]).unwrap();

    assert_eq!(broken.curves, 1);
    assert_eq!(broken.pieces.len(), 1);
    assert_eq!(lines(&sketch).len(), 3);
}

#[test]
fn pieces_of_a_construction_line_are_construction_lines() {
    let mut sketch = Sketch::new(Plane::XY);
    let across = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    sketch.add_line(Point2::new(20.0, -5.0), Point2::new(20.0, 5.0));
    sketch.set_construction(across, true).unwrap();

    let pieces = sketch.break_curve(across).unwrap();

    assert!(sketch.is_construction(pieces[0]));
}

#[test]
fn an_elliptical_arc_is_broken_where_a_line_crosses_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let center = Point2::new(30.0, 20.0);
    let at = |angle: f64| center + Point2::new(10.0 * angle.cos(), 4.0 * angle.sin());
    let arc = sketch.add_elliptical_arc(
        center,
        center + Point2::new(10.0, 0.0),
        4.0,
        at(0.3),
        at(2.8),
    );
    sketch.add_line(
        center + Point2::new(2.0, -2.0),
        center + Point2::new(2.0, 9.0),
    );
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    let pieces = sketch.break_curve(arc).unwrap();
    let solved = assert_clean(&sketch);

    assert_eq!(pieces.len(), 1);
    let first = solved.geometry.ellipse(arc).unwrap();
    let second = solved.geometry.ellipse(pieces[0]).unwrap();
    assert!((first.minor_radius - second.minor_radius).abs() < EXACT);
    assert!((first.point_at(first.end()).x - center.x - 2.0).abs() < EXACT);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);
}
