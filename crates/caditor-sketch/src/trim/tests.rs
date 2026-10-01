use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2};

use crate::{Constraint, Entity, EntityId, ExtendError, Sketch, Solved, TrimError, Trimmed};

const EXACT: f64 = 1e-9;

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn solve(sketch: &Sketch) -> Solved {
    sketch.solve(&no_parameters, &|| false).unwrap()
}

fn mm(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

fn ends(sketch: &Sketch, curve: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(curve) {
        Some(Entity::Line { start, end } | Entity::Arc { start, end, .. }) => (*start, *end),
        other => panic!("expected a line or an arc, found {other:?}"),
    }
}

fn center(sketch: &Sketch, curve: EntityId) -> EntityId {
    match sketch.entity(curve) {
        Some(Entity::Circle { center, .. } | Entity::Arc { center, .. }) => *center,
        other => panic!("expected a circle or an arc, found {other:?}"),
    }
}

fn at(sketch: &Sketch, point: EntityId) -> Point2 {
    sketch.point(point).unwrap()
}

fn assert_near(actual: Point2, expected: Point2) {
    assert!(
        actual.distance(expected) < EXACT,
        "{actual} is not {expected}"
    );
}

fn has(sketch: &Sketch, constraint: &Constraint) -> bool {
    sketch
        .constraints()
        .any(|(_, existing)| existing == constraint)
}

fn assert_solves_in_place(sketch: &Sketch) -> Solved {
    let solved = solve(sketch);
    for (id, entity) in sketch.entities() {
        if let Entity::Point(position) = entity {
            assert_near(at(&solved.geometry, id), *position);
        }
    }
    assert!(solved.solution.redundancies().is_empty());
    solved
}

fn vertical(sketch: &mut Sketch, x: f64) -> EntityId {
    sketch.add_line(Point2::new(x, -10.0), Point2::new(x, 10.0))
}

#[test]
fn trimming_the_middle_of_a_line_splits_it_and_keeps_both_halves_level_and_in_line() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 0.0));
    let (start, end) = ends(&sketch, line);
    let level = sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    sketch
        .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
        .unwrap();
    let left = vertical(&mut sketch, 10.0);
    let right = vertical(&mut sketch, 20.0);
    let freedom = solve(&sketch).solution.degrees_of_freedom();
    let first_new = sketch.next_id();

    let Trimmed::Split { piece } = sketch.trim(line, Point2::new(15.0, 0.1)).unwrap() else {
        panic!("the line splits");
    };

    assert!(piece.raw() >= first_new);
    let (kept_start, near_end) = ends(&sketch, line);
    let (far_start, far_end) = ends(&sketch, piece);
    assert_eq!((kept_start, far_end), (start, end));
    assert_near(at(&sketch, near_end), Point2::new(10.0, 0.0));
    assert_near(at(&sketch, far_start), Point2::new(20.0, 0.0));
    assert_eq!(
        sketch.constraint(level),
        Some(&Constraint::Horizontal(line))
    );
    assert!(has(&sketch, &Constraint::Horizontal(piece)));
    assert!(has(&sketch, &Constraint::Coincident(far_start, line)));
    assert!(has(&sketch, &Constraint::Coincident(near_end, left)));
    assert!(has(&sketch, &Constraint::Coincident(far_start, right)));
    assert!(has(
        &sketch,
        &Constraint::Coincident(start, EntityId::ORIGIN)
    ));
    let solved = assert_solves_in_place(&sketch);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);
}

#[test]
fn a_split_line_without_a_direction_keeps_its_halves_collinear() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 10.0));
    let middle = sketch.add_point(Point2::new(15.0, 5.0));
    sketch
        .add_constraint(Constraint::Midpoint {
            point: middle,
            line,
        })
        .unwrap();
    vertical(&mut sketch, 10.0);
    vertical(&mut sketch, 20.0);

    let Trimmed::Split { piece } = sketch.trim(line, Point2::new(15.0, 5.0)).unwrap() else {
        panic!("the line splits");
    };

    assert!(has(&sketch, &Constraint::Collinear(line, piece)));
    assert!(
        sketch
            .constraints()
            .all(|(_, constraint)| !matches!(constraint, Constraint::Midpoint { .. }))
    );
    assert_solves_in_place(&sketch);
}

#[test]
fn trimming_an_end_piece_shortens_the_line_and_drops_its_old_end() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 0.0));
    let (start, end) = ends(&sketch, line);
    let other = sketch.add_line(Point2::new(30.0, 0.0), Point2::new(30.0, 20.0));
    let (corner, _) = ends(&sketch, other);
    sketch
        .add_constraint(Constraint::Coincident(end, corner))
        .unwrap();
    let cutter = vertical(&mut sketch, 10.0);
    let (_, top) = ends(&sketch, cutter);

    assert_eq!(
        sketch.trim(line, Point2::new(25.0, 0.0)),
        Ok(Trimmed::Shortened)
    );

    let (kept, new_end) = ends(&sketch, line);
    assert_eq!(kept, start);
    assert!(sketch.entity(end).is_none());
    assert_near(at(&sketch, new_end), Point2::new(10.0, 0.0));
    assert!(has(&sketch, &Constraint::Coincident(new_end, cutter)));
    assert!(
        sketch
            .constraints()
            .all(|(_, constraint)| !constraint.entities().contains(&corner))
    );
    assert!(sketch.entity(top).is_some());
    assert_solves_in_place(&sketch);
}

#[test]
fn a_shortened_line_loses_the_length_dimension_that_ran_to_a_shared_old_end() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 0.0));
    let (corner, end) = ends(&sketch, line);
    let top = sketch.add_point(Point2::new(0.0, 20.0));
    let other = EntityId::from_raw(sketch.next_id());
    sketch
        .insert_entity(
            other,
            Entity::Line {
                start: corner,
                end: top,
            },
        )
        .unwrap();
    let length = sketch
        .add_constraint(Constraint::Distance {
            from: corner,
            to: end,
            value: mm(30.0),
        })
        .unwrap();
    let across = sketch
        .add_constraint(Constraint::HorizontalDistance {
            from: end,
            to: corner,
            value: mm(30.0),
        })
        .unwrap();
    let height = sketch
        .add_constraint(Constraint::Distance {
            from: corner,
            to: top,
            value: mm(20.0),
        })
        .unwrap();
    let cutter = vertical(&mut sketch, 10.0);

    assert_eq!(
        sketch.trim(line, Point2::new(5.0, 0.0)),
        Ok(Trimmed::Shortened)
    );

    assert!(sketch.entity(corner).is_some());
    assert!(sketch.constraint(length).is_none());
    assert!(sketch.constraint(across).is_none());
    assert!(sketch.constraint(height).is_some());
    assert!(sketch.entity(cutter).is_some());
    assert_solves_in_place(&sketch);
}

#[test]
fn a_shortened_line_loses_a_dimension_from_a_point_joined_to_its_old_end() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 0.0));
    let (corner, end) = ends(&sketch, line);
    let other = sketch.add_line(Point2::ZERO, Point2::new(0.0, 20.0));
    let (joined, top) = ends(&sketch, other);
    sketch
        .add_constraint(Constraint::Coincident(corner, joined))
        .unwrap();
    let length = sketch
        .add_constraint(Constraint::Distance {
            from: joined,
            to: end,
            value: mm(30.0),
        })
        .unwrap();
    let across = sketch
        .add_constraint(Constraint::HorizontalDistance {
            from: end,
            to: joined,
            value: mm(30.0),
        })
        .unwrap();
    let height = sketch
        .add_constraint(Constraint::Distance {
            from: joined,
            to: top,
            value: mm(20.0),
        })
        .unwrap();
    vertical(&mut sketch, 10.0);

    assert_eq!(
        sketch.trim(line, Point2::new(5.0, 0.0)),
        Ok(Trimmed::Shortened)
    );

    assert!(sketch.constraint(length).is_none());
    assert!(sketch.constraint(across).is_none());
    assert!(sketch.constraint(height).is_some());
    assert_solves_in_place(&sketch);
}

#[test]
fn a_cut_at_the_end_of_a_joined_line_takes_over_its_joint() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 0.0));
    let stem = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(10.0, 20.0));
    let (foot, _) = ends(&sketch, stem);
    let on_line = sketch
        .add_constraint(Constraint::Coincident(foot, line))
        .unwrap();

    sketch.trim(line, Point2::new(5.0, 0.0)).unwrap();

    let (new_start, _) = ends(&sketch, line);
    assert!(sketch.constraint(on_line).is_none());
    assert!(has(&sketch, &Constraint::Coincident(new_start, foot)));
    assert_solves_in_place(&sketch);
}

#[test]
fn trimming_a_circle_leaves_an_arc_with_its_radius_and_centre() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::ZERO, 10.0);
    let centre = center(&sketch, circle);
    let radius = sketch
        .add_constraint(Constraint::Radius {
            entity: circle,
            value: mm(10.0),
        })
        .unwrap();
    let chord = sketch.add_line(Point2::new(-20.0, 5.0), Point2::new(20.0, 5.0));

    assert_eq!(
        sketch.trim(circle, Point2::new(0.0, 10.0)),
        Ok(Trimmed::Opened)
    );

    let Some(Entity::Arc {
        center: kept_centre,
        start,
        end,
    }) = sketch.entity(circle).cloned()
    else {
        panic!("the circle becomes an arc");
    };
    let half_chord = 75.0_f64.sqrt();
    assert_eq!(kept_centre, centre);
    assert_near(at(&sketch, start), Point2::new(-half_chord, 5.0));
    assert_near(at(&sketch, end), Point2::new(half_chord, 5.0));
    assert!(sketch.arc(circle).unwrap().sweep > std::f64::consts::PI);
    assert!(matches!(
        sketch.constraint(radius),
        Some(Constraint::Radius { entity, .. }) if *entity == circle
    ));
    assert!(has(&sketch, &Constraint::Coincident(start, chord)));
    assert!(has(&sketch, &Constraint::Coincident(end, chord)));
    assert_solves_in_place(&sketch);
}

#[test]
fn trimming_the_middle_of_an_arc_splits_it_and_the_far_piece_keeps_its_tangent() {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_arc(
        Point2::ZERO,
        Point2::new(10.0, 0.0),
        Point2::new(-10.0, 0.0),
    );
    let (arc_start, arc_end) = ends(&sketch, arc);
    sketch
        .add_constraint(Constraint::Radius {
            entity: arc,
            value: mm(10.0),
        })
        .unwrap();
    let leg = sketch.add_line(Point2::new(-10.0, 0.0), Point2::new(-10.0, -10.0));
    let (leg_top, _) = ends(&sketch, leg);
    sketch
        .add_constraint(Constraint::Coincident(leg_top, arc_end))
        .unwrap();
    let tangent = sketch
        .add_constraint(Constraint::Tangent(leg, arc))
        .unwrap();
    vertical(&mut sketch, -5.0);
    vertical(&mut sketch, 5.0);
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    let Trimmed::Split { piece } = sketch.trim(arc, Point2::new(0.0, 10.0)).unwrap() else {
        panic!("the arc splits");
    };

    let (kept_start, near_end) = ends(&sketch, arc);
    let (far_start, far_end) = ends(&sketch, piece);
    let side = 75.0_f64.sqrt();
    assert_eq!((kept_start, far_end), (arc_start, arc_end));
    assert_near(at(&sketch, near_end), Point2::new(5.0, side));
    assert_near(at(&sketch, far_start), Point2::new(-5.0, side));
    assert_near(at(&sketch, center(&sketch, piece)), Point2::ZERO);
    assert!(sketch.constraint(tangent).is_none());
    assert!(has(&sketch, &Constraint::Tangent(leg, piece)));
    assert!(has(&sketch, &Constraint::Concentric(arc, piece)));
    assert!(has(&sketch, &Constraint::Equal(arc, piece)));
    let solved = assert_solves_in_place(&sketch);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);
}

#[test]
fn a_piece_nothing_crosses_is_deleted_with_its_points() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 0.0));
    let (start, end) = ends(&sketch, line);
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    let circle = sketch.add_circle(Point2::new(0.0, 50.0), 5.0);

    assert_eq!(
        sketch.trim(line, Point2::new(3.0, 0.0)),
        Ok(Trimmed::Deleted)
    );
    assert!(sketch.entity(line).is_none());
    assert!(sketch.entity(start).is_none());
    assert!(sketch.entity(end).is_none());
    assert_eq!(sketch.constraints().len(), 0);

    assert_eq!(
        sketch.trim(circle, Point2::new(5.0, 50.0)),
        Ok(Trimmed::Deleted)
    );
    assert_eq!(sketch.entities().len(), 0);
}

#[test]
fn construction_curves_cut_and_pieces_stay_construction() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 0.0));
    sketch.set_construction(line, true).unwrap();
    let guide = vertical(&mut sketch, 10.0);
    sketch.set_construction(guide, true).unwrap();
    vertical(&mut sketch, 20.0);

    let Trimmed::Split { piece } = sketch.trim(line, Point2::new(15.0, 0.0)).unwrap() else {
        panic!("the line splits");
    };

    assert!(sketch.is_construction(line));
    assert!(sketch.is_construction(piece));
}

#[test]
fn splines_cut_other_curves_but_cannot_be_trimmed() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&[
        Point2::new(10.0, -10.0),
        Point2::new(12.0, 0.0),
        Point2::new(10.0, 10.0),
    ]);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 0.0));

    assert!(matches!(
        sketch.trim(spline, Point2::new(11.0, 0.0)),
        Err(TrimError::Spline { .. })
    ));
    assert_eq!(
        sketch.trim(line, Point2::new(25.0, 0.0)),
        Ok(Trimmed::Shortened)
    );
    let (_, end) = ends(&sketch, line);
    assert!(has(&sketch, &Constraint::Coincident(end, spline)));
    assert_solves_in_place(&sketch);
}

#[test]
fn pieces_run_between_neighbouring_crossings() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 0.0));
    let left = vertical(&mut sketch, 10.0);
    let right = vertical(&mut sketch, 20.0);

    let pieces = sketch.trim_pieces(line).unwrap();

    assert_eq!(pieces.len(), 3);
    let middle = &pieces[1];
    assert_eq!(middle.cutters(), vec![left, right]);
    assert_near(middle.middle(), Point2::new(15.0, 0.0));
    assert!(middle.contains(Point2::new(12.0, 0.0)));
    assert!(!middle.contains(Point2::new(25.0, 0.0)));
    assert_eq!(
        sketch.trim_piece(line, Point2::new(12.0, 3.0)).unwrap(),
        *middle
    );
}

#[test]
fn extending_a_line_reaches_the_nearest_circle_and_stays_on_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(5.0, 0.0));
    let (start, end) = ends(&sketch, line);
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    let near = sketch.add_circle(Point2::new(20.0, 0.0), 5.0);
    sketch.add_circle(Point2::new(40.0, 0.0), 5.0);

    let extension = sketch.extend(line, Point2::new(4.0, 0.0)).unwrap();

    assert_eq!(extension.end, end);
    assert_eq!(extension.target, near);
    assert_eq!(ends(&sketch, line), (start, end));
    assert_near(at(&sketch, end), Point2::new(15.0, 0.0));
    assert!(has(&sketch, &Constraint::Coincident(end, near)));
    assert!(has(&sketch, &Constraint::Horizontal(line)));
    assert_solves_in_place(&sketch);
}

#[test]
fn extending_an_arc_follows_its_circle_to_a_line() {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_arc(Point2::ZERO, Point2::new(10.0, 0.0), Point2::new(0.0, 10.0));
    let (_, end) = ends(&sketch, arc);
    let wall = vertical(&mut sketch, -6.0);

    let extension = sketch.extend(arc, Point2::new(1.0, 10.0)).unwrap();

    assert_eq!(extension.target, wall);
    assert_near(at(&sketch, end), Point2::new(-6.0, 8.0));
    assert!(has(&sketch, &Constraint::Coincident(end, wall)));
    assert_solves_in_place(&sketch);
}

#[test]
fn an_end_with_nothing_beyond_it_is_not_extended() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(5.0, 0.0));
    sketch.add_circle(Point2::new(-20.0, 0.0), 5.0);
    let before = sketch.clone();

    assert!(matches!(
        sketch.extend(line, Point2::new(4.0, 0.0)),
        Err(ExtendError::NothingToReach { .. })
    ));
    assert_eq!(sketch, before);
    assert!(sketch.extend(line, Point2::new(1.0, 0.0)).is_ok());
}

#[test]
fn a_joined_or_closed_end_is_not_extended() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(5.0, 0.0));
    let (_, end) = ends(&sketch, line);
    let other = sketch.add_line(Point2::new(5.0, 0.0), Point2::new(5.0, 5.0));
    let (corner, _) = ends(&sketch, other);
    sketch
        .add_constraint(Constraint::Coincident(end, corner))
        .unwrap();
    sketch.add_line(Point2::new(20.0, -5.0), Point2::new(20.0, 5.0));
    let circle = sketch.add_circle(Point2::new(0.0, 30.0), 3.0);

    assert!(matches!(
        sketch.extend(line, Point2::new(4.0, 0.0)),
        Err(ExtendError::Joined { other: ref name, .. }) if *name == format!("Line {other}")
    ));
    assert!(matches!(
        sketch.extend(circle, Point2::new(3.0, 30.0)),
        Err(ExtendError::Closed { .. })
    ));
}

#[test]
fn trimming_never_reuses_an_identifier() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 0.0));
    vertical(&mut sketch, 10.0);
    vertical(&mut sketch, 20.0);
    let used: Vec<u64> = sketch
        .entities()
        .map(|(id, _)| id.raw())
        .chain(sketch.constraints().map(|(id, _)| id.raw()))
        .collect();

    sketch.trim(line, Point2::new(15.0, 0.0)).unwrap();

    let introduced: Vec<u64> = sketch
        .entities()
        .map(|(id, _)| id.raw())
        .chain(sketch.constraints().map(|(id, _)| id.raw()))
        .filter(|id| !used.contains(id))
        .collect();
    assert!(!introduced.is_empty());
    assert!(introduced.iter().all(|id| *id >= used.len() as u64));
}
