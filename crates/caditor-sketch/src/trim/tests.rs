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
            curve: line,
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
    let circle = sketch.add_circle(Point2::new(30.0, 30.0), 10.0);
    let centre = center(&sketch, circle);
    let radius = sketch
        .add_constraint(Constraint::Radius {
            entity: circle,
            value: mm(10.0),
        })
        .unwrap();
    let chord = sketch.add_line(Point2::new(10.0, 35.0), Point2::new(50.0, 35.0));

    assert_eq!(
        sketch.trim(circle, Point2::new(30.0, 40.0)),
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
    assert_near(at(&sketch, start), Point2::new(30.0 - half_chord, 35.0));
    assert_near(at(&sketch, end), Point2::new(30.0 + half_chord, 35.0));
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
        Point2::new(30.0, 30.0),
        Point2::new(40.0, 30.0),
        Point2::new(20.0, 30.0),
    );
    let (arc_start, arc_end) = ends(&sketch, arc);
    sketch
        .add_constraint(Constraint::Radius {
            entity: arc,
            value: mm(10.0),
        })
        .unwrap();
    let leg = sketch.add_line(Point2::new(20.0, 30.0), Point2::new(20.0, 20.0));
    let (leg_top, _) = ends(&sketch, leg);
    sketch
        .add_constraint(Constraint::Coincident(leg_top, arc_end))
        .unwrap();
    let tangent = sketch
        .add_constraint(Constraint::Tangent(leg, arc))
        .unwrap();
    sketch.add_line(Point2::new(25.0, 20.0), Point2::new(25.0, 50.0));
    sketch.add_line(Point2::new(35.0, 20.0), Point2::new(35.0, 50.0));
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    let Trimmed::Split { piece } = sketch.trim(arc, Point2::new(30.0, 40.0)).unwrap() else {
        panic!("the arc splits");
    };

    let (kept_start, near_end) = ends(&sketch, arc);
    let (far_start, far_end) = ends(&sketch, piece);
    let side = 75.0_f64.sqrt();
    assert_eq!((kept_start, far_end), (arc_start, arc_end));
    assert_near(at(&sketch, near_end), Point2::new(35.0, 30.0 + side));
    assert_near(at(&sketch, far_start), Point2::new(25.0, 30.0 + side));
    assert_near(at(&sketch, center(&sketch, piece)), Point2::new(30.0, 30.0));
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
    let circle = sketch.add_circle(Point2::new(20.0, 50.0), 5.0);

    assert_eq!(
        sketch.trim(line, Point2::new(3.0, 0.0)),
        Ok(Trimmed::Deleted)
    );
    assert!(sketch.entity(line).is_none());
    assert!(sketch.entity(start).is_none());
    assert!(sketch.entity(end).is_none());
    assert_eq!(sketch.constraints().len(), 0);

    assert_eq!(
        sketch.trim(circle, Point2::new(25.0, 50.0)),
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
fn ellipses_cut_other_curves_but_cannot_be_split_or_extended_whole() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::new(20.0, 0.0), Point2::new(28.0, 0.0), 3.0);
    let arc = sketch.add_elliptical_arc(
        Point2::new(0.0, 30.0),
        Point2::new(5.0, 30.0),
        2.0,
        Point2::new(5.0, 30.0),
        Point2::new(0.0, 32.0),
    );
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));

    assert!(matches!(
        sketch.extend(ellipse, Point2::new(28.0, 0.0)),
        Err(ExtendError::Closed { .. })
    ));
    assert_eq!(
        sketch
            .extension(arc, Point2::new(0.0, 32.0))
            .map(|extension| extension.target),
        Ok(EntityId::VERTICAL_AXIS)
    );
    let on_ellipse = sketch.add_point(Point2::new(20.0, 3.0));
    assert!(matches!(
        sketch.check_split(ellipse, on_ellipse),
        Err(crate::SplitError::NotLineOrArc { .. })
    ));
    assert!(matches!(
        sketch.trim(line, Point2::new(20.0, 0.0)),
        Ok(Trimmed::Split { .. })
    ));
    let (_, end) = ends(&sketch, line);
    assert!(has(&sketch, &Constraint::Coincident(end, ellipse)));
    assert!((sketch.point(end).unwrap().x - 12.0).abs() < 1e-9);
}

fn elliptic_ends(sketch: &Sketch, curve: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(curve) {
        Some(Entity::EllipticalArc { start, end, .. }) => (*start, *end),
        other => panic!("expected an elliptical arc, found {other:?}"),
    }
}

const AWAY: Point2 = Point2::new(40.0, 30.0);

fn upright(sketch: &mut Sketch, x: f64) -> EntityId {
    sketch.add_line(AWAY + Point2::new(x, -10.0), AWAY + Point2::new(x, 10.0))
}

fn top_arc(sketch: &mut Sketch) -> EntityId {
    let at = |angle: f64| AWAY + Point2::new(10.0 * angle.cos(), 4.0 * angle.sin());
    sketch.add_elliptical_arc(AWAY, AWAY + Point2::new(10.0, 0.0), 4.0, at(0.3), at(2.8))
}

#[test]
fn an_elliptical_arc_extends_round_its_ellipse_to_the_nearest_crossing_at_either_end() {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = top_arc(&mut sketch);
    let (start, end) = elliptic_ends(&sketch, arc);
    let far = upright(&mut sketch, -9.8);
    let near = upright(&mut sketch, 9.8);

    let extension = sketch.extend(arc, at(&sketch, end)).unwrap();
    assert_eq!(extension.target, far);
    let reached = at(&sketch, end);
    assert!((reached.x - (AWAY.x - 9.8)).abs() < 1e-9);
    assert!(reached.y > AWAY.y);
    assert!(has(&sketch, &Constraint::Coincident(end, far)));

    let extension = sketch.extend(arc, at(&sketch, start)).unwrap();
    assert_eq!(extension.target, near);
    assert!((at(&sketch, start).x - (AWAY.x + 9.8)).abs() < 1e-9);
    let shape = sketch.ellipse(arc).unwrap();
    assert!((shape.minor_radius - 4.0).abs() < EXACT);
    assert!((shape.point_at(shape.end()).distance(reached)) < 1e-9);
    assert_solves_in_place(&sketch);
}

#[test]
fn trimming_an_ellipse_between_two_cuts_opens_it_into_an_elliptical_arc() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(AWAY, AWAY + Point2::new(10.0, 0.0), 4.0);
    let line = upright(&mut sketch, 5.0);
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    assert_eq!(
        sketch.trim(ellipse, AWAY + Point2::new(10.0, 0.0)),
        Ok(Trimmed::Opened)
    );
    let (start, end) = elliptic_ends(&sketch, ellipse);
    let shape = sketch.ellipse(ellipse).unwrap();
    let height = 4.0 * 0.75_f64.sqrt();
    let kept = shape.parameter_of(AWAY + Point2::new(-10.0, 0.0));
    let removed = shape.parameter_of(AWAY + Point2::new(10.0, 0.0));

    assert_near(at(&sketch, start), AWAY + Point2::new(5.0, height));
    assert_near(at(&sketch, end), AWAY + Point2::new(5.0, -height));
    assert!(shape.within_sweep(kept).is_some());
    assert!(shape.within_sweep(removed).is_none());
    assert!(has(&sketch, &Constraint::Coincident(start, line)));
    assert!(has(&sketch, &Constraint::Coincident(end, line)));
    assert_eq!(
        assert_solves_in_place(&sketch)
            .solution
            .degrees_of_freedom(),
        freedom
    );
}

#[test]
fn trimming_the_end_of_an_elliptical_arc_shortens_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = top_arc(&mut sketch);
    let line = upright(&mut sketch, 3.0);
    let (_, old_end) = elliptic_ends(&sketch, arc);

    assert_eq!(
        sketch.trim(arc, AWAY + Point2::new(-8.0, 2.0)),
        Ok(Trimmed::Shortened)
    );
    let (start, end) = elliptic_ends(&sketch, arc);

    assert!(sketch.entity(old_end).is_none());
    assert!(at(&sketch, start).x > AWAY.x + 3.0);
    assert!((at(&sketch, end).x - AWAY.x - 3.0).abs() < EXACT);
    assert!(has(&sketch, &Constraint::Coincident(end, line)));
    assert_solves_in_place(&sketch);
}

#[test]
fn trimming_the_middle_of_an_elliptical_arc_leaves_two_pieces_of_one_ellipse() {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = top_arc(&mut sketch);
    upright(&mut sketch, -3.0);
    upright(&mut sketch, 3.0);
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    let Ok(Trimmed::Split { piece }) = sketch.trim(arc, AWAY + Point2::new(0.0, 4.0)) else {
        panic!("expected a split");
    };

    let Some(&Entity::EllipticalArc { center, major, .. }) = sketch.entity(piece) else {
        panic!("expected an elliptical arc");
    };
    let Some(&Entity::EllipticalArc {
        center: own_center,
        major: own_major,
        ..
    }) = sketch.entity(arc)
    else {
        panic!("expected an elliptical arc");
    };
    assert_eq!((center, major), (own_center, own_major));
    assert!(has(&sketch, &Constraint::Equal(arc, piece)));
    assert_eq!(
        assert_solves_in_place(&sketch)
            .solution
            .degrees_of_freedom(),
        freedom
    );
}

#[test]
fn an_ellipse_crossing_another_is_cut_where_they_cross() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(AWAY, AWAY + Point2::new(10.0, 0.0), 4.0);
    sketch.add_ellipse(AWAY, AWAY + Point2::new(0.0, 8.0), 6.0);

    let pieces = sketch.trim_pieces(ellipse).unwrap();

    assert_eq!(pieces.len(), 4);
    for piece in &pieces {
        for cut in [piece.start, piece.end].into_iter().flatten() {
            let local = cut.position - AWAY;
            let other = (local.x / 6.0).powi(2) + (local.y / 8.0).powi(2);
            assert!((other - 1.0).abs() < 1e-6, "{other}");
        }
    }
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
        Err(TrimError::NotTrimmable { .. })
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
fn an_arc_trimmed_or_extended_loses_its_midpoint_but_keeps_its_radius() {
    let midpoints = |sketch: &Sketch| {
        sketch
            .constraints()
            .filter(|(_, constraint)| matches!(constraint, Constraint::Midpoint { .. }))
            .count()
    };
    let held_middle = |sketch: &mut Sketch, arc: EntityId| {
        let middle = sketch.add_point(Point2::new(50f64.sqrt(), 50f64.sqrt()));
        sketch
            .add_constraint(Constraint::Midpoint {
                point: middle,
                curve: arc,
            })
            .unwrap();
        sketch
            .add_constraint(Constraint::Radius {
                entity: arc,
                value: mm(10.0),
            })
            .unwrap();
    };
    let mut trimmed = Sketch::new(Plane::XY);
    let arc = trimmed.add_arc(Point2::ZERO, Point2::new(10.0, 0.0), Point2::new(0.0, 10.0));
    held_middle(&mut trimmed, arc);
    vertical(&mut trimmed, 3.0);
    let mut extended = Sketch::new(Plane::XY);
    let reaching = extended.add_arc(Point2::ZERO, Point2::new(10.0, 0.0), Point2::new(0.0, 10.0));
    held_middle(&mut extended, reaching);
    vertical(&mut extended, -6.0);

    assert_eq!(
        trimmed.trim(arc, Point2::new(1.0, 9.9)),
        Ok(Trimmed::Shortened)
    );
    extended.extend(reaching, Point2::new(1.0, 10.0)).unwrap();

    for sketch in [&trimmed, &extended] {
        assert_eq!(midpoints(sketch), 0);
        assert!(
            sketch
                .constraints()
                .any(|(_, constraint)| matches!(constraint, Constraint::Radius { .. }))
        );
        assert_solves_in_place(sketch);
    }
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

#[test]
fn a_line_is_trimmed_back_to_the_vertical_axis_and_joined_to_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(-20.0, 5.0), Point2::new(30.0, 5.0));
    let (_, end) = ends(&sketch, line);

    let pieces = sketch.trim_pieces(line).unwrap();
    assert_eq!(pieces.len(), 2);
    assert_eq!(pieces[0].cutters(), vec![EntityId::VERTICAL_AXIS]);

    assert_eq!(
        sketch.trim(line, Point2::new(-10.0, 5.0)),
        Ok(Trimmed::Shortened)
    );

    let (start, kept_end) = ends(&sketch, line);
    assert_eq!(kept_end, end);
    assert_near(at(&sketch, start), Point2::new(0.0, 5.0));
    assert!(has(
        &sketch,
        &Constraint::Coincident(start, EntityId::VERTICAL_AXIS)
    ));
    assert_solves_in_place(&sketch);
}

#[test]
fn a_line_crossing_both_axes_splits_into_pieces_joined_to_each() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(-20.0, -10.0), Point2::new(20.0, 30.0));

    let Trimmed::Split { piece } = sketch.trim(line, Point2::new(-5.0, 5.0)).unwrap() else {
        panic!("the line splits");
    };

    let (_, near_end) = ends(&sketch, line);
    let (far_start, _) = ends(&sketch, piece);
    assert_near(at(&sketch, near_end), Point2::new(-10.0, 0.0));
    assert_near(at(&sketch, far_start), Point2::new(0.0, 10.0));
    assert!(has(
        &sketch,
        &Constraint::Coincident(near_end, EntityId::HORIZONTAL_AXIS)
    ));
    assert!(has(
        &sketch,
        &Constraint::Coincident(far_start, EntityId::VERTICAL_AXIS)
    ));
    assert!(has(&sketch, &Constraint::Collinear(line, piece)));
    assert_solves_in_place(&sketch);
}

#[test]
fn both_axes_meeting_at_the_origin_cut_a_line_through_it_once() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(-10.0, -10.0), Point2::new(10.0, 10.0));

    let pieces = sketch.trim_pieces(line).unwrap();

    assert_eq!(pieces.len(), 2);
    assert_eq!(pieces[0].cutters().len(), 1);
    assert_near(pieces[0].middle(), Point2::new(-5.0, -5.0));
}

#[test]
fn a_circle_about_the_origin_is_cut_by_the_axes_into_quarters() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::ZERO, 10.0);
    let centre = center(&sketch, circle);

    let pieces = sketch.trim_pieces(circle).unwrap();
    assert_eq!(pieces.len(), 4);

    assert_eq!(
        sketch.trim(circle, Point2::new(7.0, 7.0)),
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
    assert_eq!(kept_centre, centre);
    assert_near(at(&sketch, start), Point2::new(0.0, 10.0));
    assert_near(at(&sketch, end), Point2::new(10.0, 0.0));
    assert!(has(
        &sketch,
        &Constraint::Coincident(start, EntityId::VERTICAL_AXIS)
    ));
    assert!(has(
        &sketch,
        &Constraint::Coincident(end, EntityId::HORIZONTAL_AXIS)
    ));
    assert_solves_in_place(&sketch);
}

#[test]
fn a_line_ending_on_an_axis_is_not_cut_there() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(30.0, 5.0));
    let (start, _) = ends(&sketch, line);
    sketch
        .add_constraint(Constraint::Coincident(start, EntityId::VERTICAL_AXIS))
        .unwrap();

    let pieces = sketch.trim_pieces(line).unwrap();

    assert_eq!(pieces.len(), 1);
    assert!(pieces[0].is_whole());
}

#[test]
fn extending_a_line_reaches_the_axis_and_joins_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(10.0, 5.0), Point2::new(20.0, 5.0));

    let extension = sketch.extend(line, Point2::new(11.0, 5.0)).unwrap();

    assert_eq!(extension.target, EntityId::VERTICAL_AXIS);
    let (start, _) = ends(&sketch, line);
    assert_near(at(&sketch, start), Point2::new(0.0, 5.0));
    assert!(has(
        &sketch,
        &Constraint::Coincident(start, EntityId::VERTICAL_AXIS)
    ));
    assert_solves_in_place(&sketch);
}

#[test]
fn an_arc_extends_round_to_an_axis_it_meets_and_is_joined_to_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_arc(
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 10.0),
        Point2::new(10.0, 20.0),
    );

    let extension = sketch.extend(arc, Point2::new(10.0, 20.0)).unwrap();

    assert_eq!(extension.target, EntityId::VERTICAL_AXIS);
    let (_, end) = ends(&sketch, arc);
    assert_near(at(&sketch, end), Point2::new(0.0, 10.0));
    assert!(has(
        &sketch,
        &Constraint::Coincident(end, EntityId::VERTICAL_AXIS)
    ));
    assert_solves_in_place(&sketch);
}

#[test]
fn the_axes_and_the_origin_are_never_trimmed_or_extended() {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(-20.0, 5.0), Point2::new(30.0, 5.0));
    let before = sketch.clone();

    for reference in EntityId::REFERENCES {
        assert!(matches!(
            sketch.trim(reference, Point2::new(3.0, 0.0)),
            Err(TrimError::Reference { .. })
        ));
        assert!(matches!(
            sketch.extend(reference, Point2::new(3.0, 0.0)),
            Err(ExtendError::Reference { .. })
        ));
        assert!(matches!(
            sketch.trim_pieces(reference),
            Err(TrimError::Reference { .. })
        ));
    }

    assert_eq!(sketch, before);
}

#[test]
fn a_collinear_line_cuts_at_both_ends_of_its_overlap() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(30.0, 5.0));
    let other = sketch.add_line(Point2::new(10.0, 5.0), Point2::new(20.0, 5.0));
    let (other_start, other_end) = ends(&sketch, other);

    let pieces = sketch.trim_pieces(line).unwrap();
    assert_eq!(pieces.len(), 3);
    assert_eq!(pieces[1].cutters(), vec![other]);

    let Trimmed::Split { piece } = sketch.trim(line, Point2::new(15.0, 5.0)).unwrap() else {
        panic!("the line splits");
    };

    let (_, near_end) = ends(&sketch, line);
    let (far_start, _) = ends(&sketch, piece);
    assert_near(at(&sketch, near_end), Point2::new(10.0, 5.0));
    assert_near(at(&sketch, far_start), Point2::new(20.0, 5.0));
    assert!(has(&sketch, &Constraint::Coincident(near_end, other_start)));
    assert!(has(&sketch, &Constraint::Coincident(far_start, other_end)));
    assert!(has(&sketch, &Constraint::Collinear(line, piece)));
    assert_solves_in_place(&sketch);
}

#[test]
fn a_collinear_line_overlapping_one_end_cuts_both_lines_where_the_other_ends() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(30.0, 5.0));
    let other = sketch.add_line(Point2::new(20.0, 5.0), Point2::new(50.0, 5.0));
    let (other_start, _) = ends(&sketch, other);
    let (_, line_end) = ends(&sketch, line);

    assert_eq!(sketch.trim_pieces(line).unwrap().len(), 2);
    assert_eq!(sketch.trim_pieces(other).unwrap().len(), 2);

    assert_eq!(
        sketch.trim(line, Point2::new(25.0, 5.0)),
        Ok(Trimmed::Shortened)
    );

    let (_, kept_end) = ends(&sketch, line);
    assert_near(at(&sketch, kept_end), Point2::new(20.0, 5.0));
    assert!(has(&sketch, &Constraint::Coincident(kept_end, other_start)));
    assert!(sketch.entity(line_end).is_none());
    assert_solves_in_place(&sketch);
}

#[test]
fn collinear_lines_that_only_touch_or_miss_each_other_cut_nothing() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(10.0, 5.0));
    let touching = sketch.add_line(Point2::new(10.0, 5.0), Point2::new(20.0, 5.0));
    sketch.add_line(Point2::new(30.0, 5.0), Point2::new(40.0, 5.0));
    sketch.add_line(Point2::new(0.0, 6.0), Point2::new(10.0, 6.0));

    assert!(sketch.trim_pieces(line).unwrap()[0].is_whole());
    assert!(sketch.trim_pieces(touching).unwrap()[0].is_whole());
}

#[test]
fn a_line_inside_another_collinear_line_is_not_cut_by_the_longer_one() {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(0.0, 5.0), Point2::new(30.0, 5.0));
    let inner = sketch.add_line(Point2::new(10.0, 5.0), Point2::new(20.0, 5.0));

    assert!(sketch.trim_pieces(inner).unwrap()[0].is_whole());
}

#[test]
fn an_arc_on_the_same_circle_cuts_the_circle_at_its_ends() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(30.0, 30.0), 10.0);
    let arc = sketch.add_arc(
        Point2::new(30.0, 30.0),
        Point2::new(40.0, 30.0),
        Point2::new(30.0, 40.0),
    );
    let (arc_start, arc_end) = ends(&sketch, arc);

    let pieces = sketch.trim_pieces(circle).unwrap();
    assert_eq!(pieces.len(), 2);

    assert_eq!(
        sketch.trim(circle, Point2::new(37.0, 37.0)),
        Ok(Trimmed::Opened)
    );

    let (start, end) = ends(&sketch, circle);
    assert_near(at(&sketch, start), Point2::new(30.0, 40.0));
    assert_near(at(&sketch, end), Point2::new(40.0, 30.0));
    assert!(has(&sketch, &Constraint::Coincident(start, arc_end)));
    assert!(has(&sketch, &Constraint::Coincident(end, arc_start)));
    assert_solves_in_place(&sketch);
}

#[test]
fn an_overlapping_arc_on_one_circle_shortens_the_other_where_it_begins() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = Point2::new(30.0, 30.0);
    let arc = sketch.add_arc(centre, Point2::new(40.0, 30.0), Point2::new(20.0, 30.0));
    let other = sketch.add_arc(centre, Point2::new(30.0, 40.0), Point2::new(30.0, 20.0));
    let (other_start, _) = ends(&sketch, other);

    assert_eq!(sketch.trim_pieces(arc).unwrap().len(), 2);

    assert_eq!(
        sketch.trim(arc, Point2::new(23.0, 37.0)),
        Ok(Trimmed::Shortened)
    );

    let (_, end) = ends(&sketch, arc);
    assert_near(at(&sketch, end), Point2::new(30.0, 40.0));
    assert!(has(&sketch, &Constraint::Coincident(end, other_start)));
    assert_solves_in_place(&sketch);
}

#[test]
fn a_whole_circle_or_a_concentric_circle_of_another_size_does_not_cut_an_arc() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = Point2::new(30.0, 30.0);
    let arc = sketch.add_arc(centre, Point2::new(40.0, 30.0), Point2::new(20.0, 30.0));
    sketch.add_circle(centre, 10.0);
    sketch.add_circle(centre, 12.0);

    let pieces = sketch.trim_pieces(arc).unwrap();

    assert_eq!(pieces.len(), 1);
    assert!(pieces[0].is_whole());
}

#[test]
fn a_trim_at_an_overlap_that_is_refused_leaves_the_sketch_as_it_was() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(30.0, 5.0));
    sketch.add_line(Point2::new(10.0, 5.0), Point2::new(20.0, 5.0));
    sketch.set_projected(line, true).unwrap();
    let before = sketch.clone();

    assert!(matches!(
        sketch.trim(line, Point2::new(15.0, 5.0)),
        Err(TrimError::Projected { .. })
    ));

    assert_eq!(sketch, before);
}
