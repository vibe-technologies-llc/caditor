use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2};

use crate::{Constraint, Entity, EntityId, EntityState, Sketch, SketchError, Solved};

const EXACT: f64 = 1e-9;

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn solve(sketch: &Sketch) -> Solved {
    sketch.solve(&no_parameters, &|| false).unwrap()
}

fn ends(sketch: &Sketch, line: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(line) {
        Some(Entity::Line { start, end }) => (*start, *end),
        other => panic!("expected a line, found {other:?}"),
    }
}

fn project(sketch: &mut Sketch, curve: EntityId) {
    let points = sketch.entity(curve).unwrap().points();
    for id in points.into_iter().chain([curve]) {
        sketch.set_projected(id, true).unwrap();
    }
}

#[test]
fn a_projected_line_holds_still_and_what_is_joined_to_it_follows() {
    let mut sketch = Sketch::new(Plane::XY);
    let edge = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(20.0, 10.0));
    project(&mut sketch, edge);
    let drawn = sketch.add_line(Point2::new(1.0, 1.0), Point2::new(5.0, 3.0));
    let (edge_start, edge_end) = ends(&sketch, edge);
    let (start, end) = ends(&sketch, drawn);

    sketch
        .add_constraint(Constraint::Coincident(start, edge_start))
        .unwrap();
    sketch
        .add_constraint(Constraint::Perpendicular(drawn, edge))
        .unwrap();
    let solved = solve(&sketch);

    assert_eq!(
        solved.geometry.point(edge_start),
        Some(Point2::new(0.0, 10.0))
    );
    assert_eq!(
        solved.geometry.point(edge_end),
        Some(Point2::new(20.0, 10.0))
    );
    assert!(
        solved
            .geometry
            .point(start)
            .unwrap()
            .distance(Point2::new(0.0, 10.0))
            < EXACT
    );
    assert!(solved.geometry.point(end).unwrap().x.abs() < EXACT);
    assert_eq!(
        solved.solution.entity_state(edge),
        Some(EntityState::FullyConstrained)
    );
}

#[test]
fn projected_geometry_adds_no_freedom_and_a_projected_circle_keeps_its_radius() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(3.0, 4.0), 2.5);
    project(&mut sketch, circle);
    let point = sketch.add_point(Point2::new(9.0, 9.0));

    sketch
        .add_constraint(Constraint::Coincident(point, circle))
        .unwrap();
    let solved = solve(&sketch);

    assert_eq!(solved.solution.degrees_of_freedom(), 1);
    assert_eq!(
        solved.geometry.circle(circle),
        Some((Point2::new(3.0, 4.0), 2.5))
    );
    assert!(
        (solved
            .geometry
            .point(point)
            .unwrap()
            .distance(Point2::new(3.0, 4.0))
            - 2.5)
            .abs()
            < EXACT
    );
}

#[test]
fn a_constraint_between_projected_geometry_alone_is_refused() {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::ZERO, Point2::X);
    let second = sketch.add_line(Point2::Y, Point2::new(1.0, 2.0));
    project(&mut sketch, first);
    project(&mut sketch, second);

    let refused = sketch.add_constraint(Constraint::Parallel(first, second));
    let distance = sketch.add_constraint(Constraint::Distance {
        from: first,
        to: second,
        value: Expression::Measure(1.0, Unit::Millimetre),
    });

    assert_eq!(refused, Err(SketchError::OnlyReference));
    assert_eq!(distance, Err(SketchError::OnlyReference));
}

#[test]
fn a_moved_projection_is_solved_afresh_from_the_memo() {
    let mut sketch = Sketch::new(Plane::XY);
    let edge = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    project(&mut sketch, edge);
    let (_, edge_end) = ends(&sketch, edge);
    let point = sketch.add_point(Point2::new(4.0, 4.0));
    sketch
        .add_constraint(Constraint::Coincident(point, edge_end))
        .unwrap();
    let first = sketch
        .solve_from(&no_parameters, &|| false, &[], None)
        .unwrap();

    let mut moved = sketch.clone();
    moved
        .replace_entity(edge_end, Entity::Point(Point2::new(30.0, 5.0)))
        .unwrap();
    let again = moved
        .solve_from(&no_parameters, &|| false, &[], Some(&first.memo))
        .unwrap();

    assert!(
        again
            .geometry
            .point(point)
            .unwrap()
            .distance(Point2::new(30.0, 5.0))
            < EXACT
    );
}

#[test]
fn projected_curves_cannot_be_trimmed_extended_or_filleted() {
    let mut sketch = Sketch::new(Plane::XY);
    let edge = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    let other = sketch.add_line(Point2::new(5.0, -5.0), Point2::new(5.0, 5.0));
    project(&mut sketch, edge);

    assert!(matches!(
        sketch.trim(edge, Point2::new(8.0, 0.0)),
        Err(crate::TrimError::Projected { .. })
    ));
    assert!(matches!(
        sketch.extend(edge, Point2::new(9.0, 0.0)),
        Err(crate::ExtendError::Projected { .. })
    ));
    assert!(sketch.trim(other, Point2::new(5.0, 4.0)).is_ok());
}
