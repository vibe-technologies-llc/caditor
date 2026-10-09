use std::collections::BTreeSet;

use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2};

use super::*;

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn every_kind() -> BTreeSet<RelationKind> {
    RelationKind::ALL.into_iter().collect()
}

fn infer(sketch: &Sketch) -> Kept {
    sketch
        .inferred_relations(
            Tolerance::of(sketch),
            &every_kind(),
            &no_parameters,
            &|| false,
        )
        .unwrap()
}

fn count(kept: &Kept, kind: RelationKind) -> usize {
    kept.constraints
        .iter()
        .filter(|constraint| RelationKind::of(constraint) == Some(kind))
        .count()
}

fn relates(
    kept: &Kept,
    relation: fn(EntityId, EntityId) -> Constraint,
    a: EntityId,
    b: EntityId,
) -> bool {
    kept.constraints.contains(&relation(a, b)) || kept.constraints.contains(&relation(b, a))
}

fn with(sketch: &Sketch, kept: &Kept) -> Sketch {
    let mut constrained = sketch.clone();
    for constraint in &kept.constraints {
        constrained.add_constraint(constraint.clone()).unwrap();
    }
    constrained
}

fn ends(sketch: &Sketch, line: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(line) {
        Some(&Entity::Line { start, end }) => (start, end),
        other => panic!("expected a line, found {other:?}"),
    }
}

fn loose_outline(sketch: &mut Sketch, corners: &[Point2]) -> Vec<EntityId> {
    let hair = Vector2::new(0.002, -0.001);
    (0..corners.len())
        .map(|index| sketch.add_line(corners[index], corners[(index + 1) % corners.len()] + hair))
        .collect()
}

#[test]
fn a_loose_rectangle_gets_its_joints_and_directions_and_nothing_that_repeats_them() {
    let mut sketch = Sketch::new(Plane::XY);
    let sides = loose_outline(
        &mut sketch,
        &[
            Point2::new(20.0, 10.0),
            Point2::new(60.0, 10.05),
            Point2::new(60.03, 35.0),
            Point2::new(20.0, 35.0),
        ],
    );

    let kept = infer(&sketch);

    assert_eq!(count(&kept, RelationKind::Coincident), 4);
    assert_eq!(count(&kept, RelationKind::Horizontal), 2);
    assert_eq!(count(&kept, RelationKind::Vertical), 2);
    assert_eq!(count(&kept, RelationKind::Parallel), 0);
    assert_eq!(count(&kept, RelationKind::Perpendicular), 0);
    assert_eq!(count(&kept, RelationKind::Equal), 0);
    assert_eq!(kept.degrees_of_freedom, 4);
    assert!(kept.constraints.contains(&Constraint::Horizontal(sides[0])));

    let constrained = with(&sketch, &kept);
    let solved = constrained.solve(&no_parameters, &|| false).unwrap();

    assert!(solved.solution.redundancies().is_empty());
    assert!(constrained.open_ends().is_empty());
}

#[test]
fn a_square_keeps_one_equal_length_beside_its_directions() {
    let mut sketch = Sketch::new(Plane::XY);
    loose_outline(
        &mut sketch,
        &[
            Point2::new(5.0, 5.0),
            Point2::new(25.0, 5.0),
            Point2::new(25.0, 25.01),
            Point2::new(5.0, 25.0),
        ],
    );

    let kept = infer(&sketch);

    assert_eq!(count(&kept, RelationKind::Equal), 1);
    assert_eq!(kept.degrees_of_freedom, 3);
}

#[test]
fn a_relation_already_in_the_sketch_is_not_found_again() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(1.0, 2.0), Point2::new(30.0, 2.0));
    let upright = sketch.add_line(Point2::new(40.0, 2.0), Point2::new(40.0, 20.0));
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();

    let kept = infer(&sketch);

    assert_eq!(kept.constraints, vec![Constraint::Vertical(upright)]);
}

#[test]
fn a_relation_the_sketch_cannot_hold_is_left_out() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(40.0, 10.2));
    let tilt = sketch
        .line_direction(line)
        .map(|direction| direction.y.atan2(direction.x).to_degrees())
        .unwrap();
    sketch
        .add_constraint(Constraint::Angle {
            from: EntityId::HORIZONTAL_AXIS,
            to: line,
            reversed: false,
            value: Expression::Measure(tilt, Unit::Degree),
        })
        .unwrap();

    let found = sketch.shown_relations(Tolerance::of(&sketch), &every_kind());
    let kept = infer(&sketch);

    assert!(found.contains(&Constraint::Horizontal(line)));
    assert!(!kept.constraints.contains(&Constraint::Horizontal(line)));
}

#[test]
fn a_sketch_that_does_not_solve_is_refused() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(40.0, 25.0));
    let (start, end) = ends(&sketch, line);
    for value in [10.0, 20.0] {
        sketch
            .add_constraint(Constraint::Distance {
                from: start,
                to: end,
                value: Expression::Measure(value, Unit::Millimetre),
            })
            .unwrap();
    }

    let refused = sketch.inferred_relations(
        Tolerance::of(&sketch),
        &every_kind(),
        &no_parameters,
        &|| false,
    );

    assert!(matches!(refused, Err(InferenceError::Unsolved(_))));
}

#[test]
fn a_line_leaving_an_arc_along_it_is_tangent_and_so_is_one_grazing_a_circle() {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_arc(Point2::ZERO, Point2::new(10.0, 0.0), Point2::new(0.0, 10.0));
    let line = sketch.add_line(Point2::new(10.0, 0.001), Point2::new(10.05, -20.0));
    let circle = sketch.add_circle(Point2::new(50.0, 0.0), 5.0);
    let grazing = sketch.add_line(Point2::new(40.0, 5.002), Point2::new(60.0, 5.0));

    let kept = infer(&sketch);

    assert!(relates(&kept, Constraint::Tangent, arc, line));
    assert!(relates(&kept, Constraint::Tangent, grazing, circle));
    assert_eq!(count(&kept, RelationKind::Coincident), 1);
}

#[test]
fn circles_nearly_sharing_a_centre_are_concentric() {
    let mut sketch = Sketch::new(Plane::XY);
    let inner = sketch.add_circle(Point2::new(30.0, 20.0), 4.0);
    let outer = sketch.add_circle(Point2::new(30.002, 20.001), 9.0);

    let kept = infer(&sketch);

    assert_eq!(kept.constraints, vec![Constraint::Concentric(inner, outer)]);
}

#[test]
fn circles_mirrored_about_an_axis_are_symmetric_and_equal() {
    let mut sketch = Sketch::new(Plane::XY);
    let left = sketch.add_circle(Point2::new(-15.0, 8.0), 3.0);
    let right = sketch.add_circle(Point2::new(15.002, 8.001), 3.001);

    let kept = infer(&sketch);

    let (Some(left_centre), Some(right_centre)) = (sketch.center_of(left), sketch.center_of(right))
    else {
        panic!("circles have centres");
    };
    assert!(kept.constraints.contains(&Constraint::Symmetric {
        first: left_centre,
        second: right_centre,
        about: EntityId::VERTICAL_AXIS,
    }));
    assert!(kept.constraints.contains(&Constraint::Equal(left, right)));
}

#[test]
fn a_rectangle_centred_on_the_origin_keeps_three_symmetries_and_two_freedoms() {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        Point2::new(-20.0, -10.0),
        Point2::new(20.0, -10.0),
        Point2::new(20.0, 10.0),
        Point2::new(-20.0, 10.0),
    ];
    let sides: Vec<EntityId> = (0..4)
        .map(|index| sketch.add_line(corners[index], corners[(index + 1) % 4]))
        .collect();
    for index in 0..4 {
        let (_, end) = ends(&sketch, sides[index]);
        let (start, _) = ends(&sketch, sides[(index + 1) % 4]);
        sketch
            .add_constraint(Constraint::Coincident(end, start))
            .unwrap();
    }

    let kept = infer(&sketch);

    assert_eq!(count(&kept, RelationKind::Symmetric), 3);
    assert_eq!(count(&kept, RelationKind::Horizontal), 0);
    assert_eq!(count(&kept, RelationKind::Vertical), 0);
    assert_eq!(kept.degrees_of_freedom, 2);
}

#[test]
fn slanted_lines_find_parallels_and_one_right_angle_between_their_directions() {
    let mut sketch = Sketch::new(Plane::XY);
    let direction = Vector2::from_angle(30f64.to_radians());
    let square = direction.perp();
    let first = sketch.add_line(Point2::ZERO, direction * 30.0);
    let second = sketch.add_line(
        Point2::new(0.0, 15.0),
        Point2::new(0.0, 15.0) + direction * 20.0,
    );
    let across = sketch.add_line(
        Point2::new(50.0, 0.0),
        Point2::new(50.0, 0.0) + square * 12.0 + Vector2::new(0.01, 0.0),
    );

    let kept = infer(&sketch);

    assert!(relates(&kept, Constraint::Parallel, first, second));
    assert_eq!(count(&kept, RelationKind::Perpendicular), 1);
    assert!(
        relates(&kept, Constraint::Perpendicular, first, across)
            || relates(&kept, Constraint::Perpendicular, second, across)
    );
}

#[test]
fn only_the_chosen_kinds_are_found() {
    let mut sketch = Sketch::new(Plane::XY);
    loose_outline(
        &mut sketch,
        &[
            Point2::new(20.0, 10.0),
            Point2::new(60.0, 10.0),
            Point2::new(60.0, 35.0),
            Point2::new(20.0, 35.0),
        ],
    );

    let kinds = BTreeSet::from([RelationKind::Coincident]);
    let kept = sketch
        .inferred_relations(Tolerance::of(&sketch), &kinds, &no_parameters, &|| false)
        .unwrap();

    assert_eq!(kept.constraints.len(), 4);
    assert_eq!(count(&kept, RelationKind::Coincident), 4);
}

#[test]
fn the_ends_of_one_short_line_are_never_joined_to_each_other() {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(5.0, 5.0), Point2::new(5.001, 5.0));
    sketch.add_line(Point2::new(1.0, 1.0), Point2::new(40.0, 30.0));

    let found = sketch.shown_relations(Tolerance::of(&sketch), &every_kind());

    assert_eq!(count_found(&found, RelationKind::Coincident), 0);
}

fn count_found(found: &[Constraint], kind: RelationKind) -> usize {
    found
        .iter()
        .filter(|constraint| RelationKind::of(constraint) == Some(kind))
        .count()
}

#[test]
fn a_loosely_drawn_centred_rectangle_settles_where_it_was_drawn() {
    let mut sketch = Sketch::new(Plane::XY);
    loose_outline(
        &mut sketch,
        &[
            Point2::new(-20.0, -10.01),
            Point2::new(20.02, -10.0),
            Point2::new(20.0, 10.0),
            Point2::new(-20.01, 10.02),
        ],
    );

    let kept = infer(&sketch);
    let solved = with(&sketch, &kept)
        .solve(&no_parameters, &|| false)
        .unwrap();

    assert_eq!(kept.degrees_of_freedom, 2);
    assert!(solved.solution.redundancies().is_empty());
    for (id, entity) in sketch.entities() {
        if let Entity::Point(at) = entity {
            assert!(solved.geometry.point(id).unwrap().distance(*at) < 0.1);
        }
    }
}
