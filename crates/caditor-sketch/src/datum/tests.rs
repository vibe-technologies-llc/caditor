use std::collections::BTreeSet;

use caditor_expression::{EvalError, ParameterId, Quantity};
use caditor_geometry::{Plane, Point2};

use crate::{Constraint, Entity, EntityId, InferenceError, Kept, RelationKind, Sketch, Tolerance};

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn constrained(mut sketch: Sketch, kept: &Kept) -> Sketch {
    for constraint in &kept.constraints {
        sketch.add_constraint(constraint.clone()).unwrap();
    }
    sketch
}

fn rectangle(sketch: &mut Sketch, low: Point2, high: Point2) -> Vec<EntityId> {
    let corners = [
        low,
        Point2::new(high.x, low.y),
        high,
        Point2::new(low.x, high.y),
    ];
    let sides: Vec<EntityId> = (0..4)
        .map(|index| sketch.add_line(corners[index], corners[(index + 1) % 4]))
        .collect();
    for index in 0..4 {
        let Some(&Entity::Line { end, .. }) = sketch.entity(sides[index]) else {
            panic!("expected a line");
        };
        let Some(&Entity::Line { start, .. }) = sketch.entity(sides[(index + 1) % 4]) else {
            panic!("expected a line");
        };
        sketch
            .add_constraint(Constraint::Coincident(end, start))
            .unwrap();
    }
    for (index, side) in sides.iter().enumerate() {
        let direction = if index % 2 == 0 {
            Constraint::Horizontal(*side)
        } else {
            Constraint::Vertical(*side)
        };
        sketch.add_constraint(direction).unwrap();
    }
    sides
}

fn start_of(sketch: &Sketch, line: EntityId) -> EntityId {
    match sketch.entity(line) {
        Some(&Entity::Line { start, .. }) => start,
        other => panic!("expected a line, found {other:?}"),
    }
}

fn dimensions(kept: &Kept) -> usize {
    kept.constraints
        .iter()
        .filter(|constraint| constraint.dimension().is_some())
        .count()
}

#[test]
fn a_rectangle_is_dimensioned_from_its_corner_and_the_corner_from_the_origin() {
    let mut sketch = Sketch::new(Plane::XY);
    let sides = rectangle(&mut sketch, Point2::new(10.0, 5.0), Point2::new(50.0, 30.0));
    let datum = start_of(&sketch, sides[0]);

    let kept = sketch
        .datum_dimensions(datum, Tolerance::of(&sketch), &no_parameters, &|| false)
        .unwrap();

    assert_eq!(kept.degrees_of_freedom, 0);
    assert_eq!(dimensions(&kept), 4);
    assert!(kept.constraints.iter().any(|constraint| matches!(
        constraint,
        Constraint::HorizontalDistance { from: EntityId::ORIGIN, to, .. } if *to == datum
    )));
    let solved = constrained(sketch.clone(), &kept)
        .solve(&no_parameters, &|| false)
        .unwrap();
    assert!(solved.solution.is_fully_constrained());
    assert!(solved.solution.redundancies().is_empty());
    for (id, entity) in sketch.entities() {
        if let Entity::Point(at) = entity {
            assert!(solved.geometry.point(id).unwrap().distance(*at) < 1e-6);
        }
    }
}

#[test]
fn a_corner_level_with_the_datum_is_held_level_rather_than_at_no_distance() {
    let mut sketch = Sketch::new(Plane::XY);
    let sides = rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(30.0, 20.0));
    let corner = start_of(&sketch, sides[0]);
    sketch
        .add_constraint(Constraint::Coincident(corner, EntityId::ORIGIN))
        .unwrap();

    let kept = sketch
        .datum_dimensions(
            EntityId::ORIGIN,
            Tolerance::of(&sketch),
            &no_parameters,
            &|| false,
        )
        .unwrap();

    assert_eq!(kept.degrees_of_freedom, 0);
    assert_eq!(dimensions(&kept), 2);
    assert!(kept.constraints.iter().all(|constraint| {
        constraint
            .dimension()
            .is_none_or(|_| sketch.measured(constraint).is_some_and(|value| value > 1.0))
    }));
}

#[test]
fn circles_get_a_diameter_and_their_centre_after_the_relations_found() {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_circle(Point2::new(-12.0, 4.0), 3.0);
    sketch.add_circle(Point2::new(12.0, 4.0), 3.0);
    let relations = sketch
        .inferred_relations(
            Tolerance::of(&sketch),
            &RelationKind::ALL.into_iter().collect::<BTreeSet<_>>(),
            &no_parameters,
            &|| false,
        )
        .unwrap();
    let related = constrained(sketch, &relations);

    let kept = related
        .datum_dimensions(
            EntityId::ORIGIN,
            Tolerance::of(&related),
            &no_parameters,
            &|| false,
        )
        .unwrap();

    assert_eq!(kept.degrees_of_freedom, 0);
    assert_eq!(dimensions(&kept), 3);
}

#[test]
fn a_datum_must_be_a_point() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::X);

    let refused = sketch.datum_dimensions(line, Tolerance::of(&sketch), &no_parameters, &|| false);

    assert!(matches!(refused, Err(InferenceError::NotAPoint { .. })));
}
