use std::f64::consts::{FRAC_PI_2, FRAC_PI_3, FRAC_PI_4, PI};

use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2, Vector2};

use crate::{
    ChamferSize, Constraint, ConstraintId, Dimensioned, Entity, EntityId, FilletError, Pick,
    Sketch, Solved,
};

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

fn mm(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

fn equal(distance: f64) -> ChamferSize {
    ChamferSize::Equal(dimensioned(distance))
}

fn dimensioned(distance: f64) -> Dimensioned {
    Dimensioned {
        expression: mm(distance),
        value: distance,
    }
}

fn degrees(angle: f64) -> Dimensioned {
    Dimensioned {
        expression: Expression::Measure(angle, Unit::Degree),
        value: angle,
    }
}

fn ends(sketch: &Sketch, curve: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(curve) {
        Some(Entity::Line { start, end } | Entity::Arc { start, end, .. }) => (*start, *end),
        other => panic!("expected a line or an arc, found {other:?}"),
    }
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

fn assert_clean(solved: &Solved) {
    assert!(
        solved.solution.redundancies().is_empty(),
        "redundant: {:?}",
        solved.solution.redundancies()
    );
}

struct Rectangle {
    sketch: Sketch,
    sides: [EntityId; 4],
    width: ConstraintId,
    corner: EntityId,
}

fn rectangle() -> Rectangle {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        Point2::ZERO,
        Point2::new(40.0, 0.0),
        Point2::new(40.0, 20.0),
        Point2::new(0.0, 20.0),
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
        sketch
            .add_constraint(if index % 2 == 0 {
                Constraint::Horizontal(sides[index])
            } else {
                Constraint::Vertical(sides[index])
            })
            .unwrap();
    }
    let (origin_corner, corner) = ends(&sketch, sides[0]);
    sketch
        .add_constraint(Constraint::Coincident(origin_corner, EntityId::ORIGIN))
        .unwrap();
    let width = sketch
        .add_constraint(Constraint::Distance {
            from: origin_corner,
            to: corner,
            value: mm(40.0),
        })
        .unwrap();
    let (bottom, top) = ends(&sketch, sides[1]);
    sketch
        .add_constraint(Constraint::Distance {
            from: bottom,
            to: top,
            value: mm(20.0),
        })
        .unwrap();
    Rectangle {
        sketch,
        sides: [sides[0], sides[1], sides[2], sides[3]],
        width,
        corner,
    }
}

#[test]
fn a_rectangle_corner_is_rounded_and_stays_rounded_when_the_rectangle_widens() {
    let Rectangle {
        mut sketch,
        sides,
        width,
        corner,
    } = rectangle();
    let freedom = solve(&sketch).solution.degrees_of_freedom();
    let found = sketch.corner_at(corner).unwrap();
    assert_eq!(found.curves, [sides[0], sides[1]]);
    assert_eq!(sketch.corner_between(sides[0], sides[1]).unwrap(), found);
    assert_eq!(
        sketch.corner_between(sides[1], sides[0]).unwrap().curves,
        [sides[1], sides[0]]
    );

    let arc = sketch.fillet(&found, 5.0, mm(5.0)).unwrap();

    let (_, bottom_end) = ends(&sketch, sides[0]);
    let (right_start, _) = ends(&sketch, sides[1]);
    assert_near(sketch.point(bottom_end).unwrap(), Point2::new(35.0, 0.0));
    assert_near(sketch.point(right_start).unwrap(), Point2::new(40.0, 5.0));
    let geometry = sketch.arc(arc).unwrap();
    assert_near(geometry.center, Point2::new(35.0, 5.0));
    assert!((geometry.radius - 5.0).abs() < EXACT);
    assert!((geometry.sweep - FRAC_PI_2).abs() < EXACT);
    assert!(has(&sketch, &Constraint::Tangent(sides[0], arc)));
    assert!(has(&sketch, &Constraint::Tangent(sides[1], arc)));
    assert!(has(
        &sketch,
        &Constraint::Radius {
            entity: arc,
            value: mm(5.0)
        }
    ));
    assert_eq!(sketch.point(corner), Some(Point2::new(40.0, 0.0)));
    assert!(has(&sketch, &Constraint::Coincident(corner, sides[0])));
    assert!(has(&sketch, &Constraint::Coincident(corner, sides[1])));
    assert!(sketch.entities_using(corner).is_empty());
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);

    sketch.set_dimension(width, mm(60.0)).unwrap();
    let wider = solve(&sketch).geometry;
    assert_near(wider.arc(arc).unwrap().center, Point2::new(55.0, 5.0));
    assert_near(wider.point(bottom_end).unwrap(), Point2::new(55.0, 0.0));
    assert_near(wider.point(corner).unwrap(), Point2::new(60.0, 0.0));
}

#[test]
fn a_corner_shared_by_two_lines_keeps_its_point_as_the_sharp() {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::new(-20.0, 10.0), Point2::ZERO);
    let (_, corner) = ends(&sketch, first);
    let far = sketch.add_point(Point2::new(20.0, 10.0));
    let second = EntityId::from_raw(sketch.next_id());
    sketch
        .insert_entity(
            second,
            Entity::Line {
                start: corner,
                end: far,
            },
        )
        .unwrap();
    sketch
        .add_constraint(Constraint::Fix {
            point: corner,
            at: Point2::ZERO,
        })
        .unwrap();
    sketch
        .add_constraint(Constraint::Equal(first, second))
        .unwrap();
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    let arc = sketch
        .fillet(&sketch.corner_at(corner).unwrap(), 4.0, mm(4.0))
        .unwrap();

    let centre = sketch.arc(arc).unwrap().center;
    assert!(centre.x.abs() < EXACT && centre.y > 4.0);
    assert!(has(
        &sketch,
        &Constraint::Fix {
            point: corner,
            at: Point2::ZERO
        }
    ));
    assert!(has(&sketch, &Constraint::Equal(first, second)));
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);
}

#[test]
fn a_line_meeting_an_arc_is_rounded_by_an_arc_tangent_to_both() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(-20.0, 0.0), Point2::new(-10.0, 0.0));
    let arc = sketch.add_arc(
        Point2::ZERO,
        Point2::new(10.0, 0.0),
        Point2::new(-10.0, 0.0),
    );
    let (_, line_end) = ends(&sketch, line);
    let (_, arc_end) = ends(&sketch, arc);
    sketch
        .add_constraint(Constraint::Coincident(line_end, arc_end))
        .unwrap();
    let radius = sketch
        .add_constraint(Constraint::Radius {
            entity: arc,
            value: mm(10.0),
        })
        .unwrap();
    let Some(Entity::Arc { center, .. }) = sketch.entity(arc).cloned() else {
        panic!("an arc");
    };
    sketch
        .add_constraint(Constraint::Coincident(center, EntityId::ORIGIN))
        .unwrap();
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    let freedom = solve(&sketch).solution.degrees_of_freedom();
    let corner = sketch.corner_between(line, arc).unwrap();

    let rounding = sketch.fillet(&corner, 2.0, mm(2.0)).unwrap();

    let fillet = sketch.arc(rounding).unwrap();
    assert!((fillet.radius - 2.0).abs() < EXACT);
    assert!((fillet.center.distance(Point2::ZERO) - 12.0).abs() < EXACT);
    assert!((fillet.center.y - 2.0).abs() < EXACT);
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);

    sketch.set_dimension(radius, mm(15.0)).unwrap();
    let bigger = solve(&sketch).geometry;
    let moved = bigger.arc(rounding).unwrap();
    assert!((moved.center.distance(Point2::ZERO) - 17.0).abs() < EXACT);
    assert!((moved.radius - 2.0).abs() < EXACT);
}

#[test]
fn a_radius_too_large_for_either_side_is_refused_in_words() {
    let Rectangle {
        mut sketch,
        sides,
        corner,
        ..
    } = rectangle();
    let found = sketch.corner_at(corner).unwrap();
    let before = sketch.clone();

    let refused = sketch.fillet(&found, 25.0, mm(25.0));

    assert_eq!(
        refused,
        Err(FilletError::TooLarge {
            entity: sides[1],
            label: sketch.entity_label(sides[1]),
        })
    );
    assert_eq!(
        refused.unwrap_err().to_string(),
        format!(
            "the radius is too large for {}",
            sketch.entity_label(sides[1])
        )
    );
    assert_eq!(sketch, before);
    assert_eq!(sketch.rounding(&found, 0.0), Err(FilletError::NotPositive));
    assert!(sketch.rounding(&found, 19.0).is_ok());
}

#[test]
fn only_a_corner_of_two_lines_or_arcs_can_be_filleted() {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    let straight = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(20.0, 0.0));
    let lone = sketch.add_line(Point2::new(0.0, 30.0), Point2::new(10.0, 30.0));
    let spline = sketch.add_spline(&[Point2::new(10.0, 30.0), Point2::new(20.0, 40.0)]);
    let stem = sketch.add_line(Point2::new(0.0, 50.0), Point2::new(10.0, 50.0));
    let branch = sketch.add_line(Point2::new(10.0, 50.0), Point2::new(10.0, 60.0));
    let fork = sketch.add_line(Point2::new(10.0, 50.0), Point2::new(20.0, 40.0));
    let (lone_start, _) = ends(&sketch, lone);
    let (_, stem_end) = ends(&sketch, stem);
    let (_, first_end) = ends(&sketch, first);
    let label = |id: EntityId| sketch.entity_label(id);

    assert_eq!(
        sketch.corner_at(lone_start).unwrap_err().to_string(),
        format!(
            "only {} ends at {}, so there is no corner to round",
            label(lone),
            label(lone_start)
        )
    );
    assert_eq!(
        sketch.corner_at(stem_end).unwrap_err().to_string(),
        format!(
            "3 curves meet at {}; only a corner between two can be rounded or cut",
            label(stem_end)
        )
    );
    assert_eq!(
        sketch.corner_between(lone, spline),
        Err(FilletError::NotLineOrArc {
            entity: spline,
            label: label(spline),
        })
    );
    assert_eq!(
        sketch.corner_between(first, lone).unwrap_err().to_string(),
        format!(
            "{} and {} do not meet at their ends",
            label(first),
            label(lone)
        )
    );
    let straight_corner = sketch.corner_at(first_end).unwrap();
    assert_eq!(
        sketch.rounding(&straight_corner, 1.0),
        Err(FilletError::NoCorner {
            first: label(first),
            second: label(straight),
        })
    );
    assert!(
        sketch
            .fillet_corners()
            .iter()
            .all(|corner| !corner.curves.contains(&branch) && !corner.curves.contains(&fork))
    );
}

#[test]
fn every_rectangle_corner_is_offered_once_and_the_drag_radius_passes_under_the_pointer() {
    let Rectangle {
        sketch,
        sides,
        corner,
        ..
    } = rectangle();

    let corners = sketch.fillet_corners();

    assert_eq!(corners.len(), 4);
    for side in sides {
        assert_eq!(
            corners
                .iter()
                .filter(|found| found.curves.contains(&side))
                .count(),
            2
        );
    }
    let found = sketch.corner_at(corner).unwrap();
    let radius = sketch
        .radius_through(&found, Point2::new(38.0, 2.0))
        .unwrap();
    let rounding = sketch.rounding(&found, radius).unwrap();
    let middle = rounding.center + (Point2::new(40.0, 0.0) - rounding.center).normalize() * radius;
    assert_near(middle, Point2::new(38.0, 2.0));
}

#[test]
fn a_rectangle_corner_is_chamfered_and_keeps_its_distances_when_the_rectangle_widens() {
    let Rectangle {
        mut sketch,
        sides,
        width,
        corner,
    } = rectangle();
    let freedom = solve(&sketch).solution.degrees_of_freedom();
    let found = sketch.corner_at(corner).unwrap();
    let bevel = sketch.bevel(&found, &equal(5.0)).unwrap();

    assert_near(bevel.touches[0], Point2::new(35.0, 0.0));
    assert_near(bevel.touches[1], Point2::new(40.0, 5.0));

    let line = sketch.chamfer(&found, &equal(5.0)).unwrap();
    let solved = solve(&sketch);
    let (start, end) = ends(&sketch, line);

    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);
    assert_near(sketch.point(start).unwrap(), Point2::new(35.0, 0.0));
    assert_near(sketch.point(end).unwrap(), Point2::new(40.0, 5.0));
    assert!(has(&sketch, &Constraint::Coincident(corner, sides[0])));
    assert!(has(&sketch, &Constraint::Coincident(corner, sides[1])));

    sketch.set_dimension(width, mm(60.0)).unwrap();
    let wider = solve(&sketch).geometry;

    assert_near(wider.point(start).unwrap(), Point2::new(55.0, 0.0));
    assert_near(wider.point(end).unwrap(), Point2::new(60.0, 5.0));
}

#[test]
fn a_chamfer_longer_than_a_side_or_not_above_zero_is_refused() {
    let Rectangle { sketch, corner, .. } = rectangle();
    let found = sketch.corner_at(corner).unwrap();

    assert!(matches!(
        sketch.bevel(&found, &equal(25.0)),
        Err(FilletError::TooFar { .. })
    ));
    assert_eq!(
        sketch.bevel(&found, &equal(0.0)),
        Err(FilletError::DistanceNotPositive)
    );
    assert!(
        sketch
            .distance_through(&found, Point2::new(37.5, 2.5))
            .is_some_and(|distance| (distance - 5.0).abs() < EXACT)
    );
}

#[test]
fn a_corner_of_a_line_and_an_arc_is_chamfered_on_the_arc_by_its_chord() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 30.0), Point2::new(0.0, 10.0));
    let arc = sketch.add_arc(Point2::ZERO, Point2::new(10.0, 0.0), Point2::new(0.0, 10.0));
    let (_, line_end) = ends(&sketch, line);
    let (_, arc_end) = ends(&sketch, arc);
    sketch
        .add_constraint(Constraint::Coincident(line_end, arc_end))
        .unwrap();
    let found = sketch.corner_between(line, arc).unwrap();

    let bevel = sketch.bevel(&found, &equal(4.0)).unwrap();

    assert_near(bevel.touches[0], Point2::new(0.0, 14.0));
    assert!(bevel.touches[1].x > 0.0);
    assert!((bevel.touches[1].length() - 10.0).abs() < EXACT);
    assert!((bevel.touches[1].distance(Point2::new(0.0, 10.0)) - 4.0).abs() < EXACT);
    assert!(sketch.chamfer(&found, &equal(4.0)).is_ok());
}

#[test]
fn a_corner_is_chamfered_by_a_different_distance_on_each_curve_and_each_stays_editable() {
    let Rectangle {
        mut sketch,
        sides,
        corner,
        ..
    } = rectangle();
    let freedom = solve(&sketch).solution.degrees_of_freedom();
    let found = sketch.corner_at(corner).unwrap();
    let size = ChamferSize::Distances {
        first: dimensioned(8.0),
        second: dimensioned(3.0),
    };

    let bevel = sketch.bevel(&found, &size).unwrap();

    assert_eq!(bevel.distances, [8.0, 3.0]);
    assert_near(bevel.touches[0], Point2::new(32.0, 0.0));
    assert_near(bevel.touches[1], Point2::new(40.0, 3.0));

    let line = sketch.chamfer(&found, &size).unwrap();
    let solved = solve(&sketch);
    let (start, end) = ends(&sketch, line);
    let distances: Vec<ConstraintId> = sketch
        .constraints()
        .filter(|(_, constraint)| {
            matches!(constraint, Constraint::Distance { from, to, .. } if *from == corner && (*to == start || *to == end))
        })
        .map(|(id, _)| id)
        .collect();

    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);
    assert_eq!(distances.len(), 2);
    assert!(has(&sketch, &Constraint::Coincident(corner, sides[1])));

    let second = distances[1];
    sketch.set_dimension(second, mm(6.0)).unwrap();
    let taller = solve(&sketch).geometry;

    assert_near(taller.point(start).unwrap(), Point2::new(32.0, 0.0));
    assert_near(taller.point(end).unwrap(), Point2::new(40.0, 6.0));
}

#[test]
fn a_corner_is_chamfered_by_a_distance_and_an_angle_that_stays_a_dimension() {
    let Rectangle {
        mut sketch,
        sides,
        corner,
        ..
    } = rectangle();
    let freedom = solve(&sketch).solution.degrees_of_freedom();
    let found = sketch.corner_at(corner).unwrap();
    let size = ChamferSize::DistanceAndAngle {
        distance: dimensioned(6.0),
        angle: degrees(30.0),
    };

    let bevel = sketch.bevel(&found, &size).unwrap();

    assert_near(bevel.touches[0], Point2::new(34.0, 0.0));
    assert_near(
        bevel.touches[1],
        Point2::new(40.0, 6.0 * 30f64.to_radians().tan()),
    );

    let line = sketch.chamfer(&found, &size).unwrap();
    let solved = solve(&sketch);
    let (start, end) = ends(&sketch, line);
    let angle = sketch
        .constraints()
        .find(|(_, constraint)| matches!(constraint, Constraint::Angle { .. }))
        .map(|(id, constraint)| (id, constraint.clone()))
        .unwrap();

    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);
    assert!(has(&sketch, &Constraint::Coincident(corner, sides[0])));
    assert!(
        matches!(&angle.1, Constraint::Angle { value, .. } if *value == degrees(30.0).expression)
    );
    assert!(
        sketch
            .measured(&angle.1)
            .is_some_and(|found| (found - 30.0).abs() < EXACT)
    );

    sketch
        .set_dimension(angle.0, degrees(60.0).expression)
        .unwrap();
    let steeper = solve(&sketch).geometry;

    assert_near(steeper.point(start).unwrap(), Point2::new(34.0, 0.0));
    assert_near(
        steeper.point(end).unwrap(),
        Point2::new(40.0, 6.0 * 60f64.to_radians().tan()),
    );
}

#[test]
fn a_chamfer_angle_is_measured_inside_the_cut_corner_whichever_way_the_lines_are_drawn() {
    let mut sketch = Sketch::new(Plane::XY);
    let across = sketch.add_line(Point2::new(40.0, 0.0), Point2::new(0.0, 0.0));
    let up = sketch.add_line(Point2::new(40.0, 20.0), Point2::new(40.0, 0.0));
    let (across_start, _) = ends(&sketch, across);
    let (_, up_end) = ends(&sketch, up);
    sketch
        .add_constraint(Constraint::Coincident(across_start, up_end))
        .unwrap();
    let found = sketch.corner_between(across, up).unwrap();
    let size = ChamferSize::DistanceAndAngle {
        distance: dimensioned(5.0),
        angle: degrees(45.0),
    };

    let bevel = sketch.bevel(&found, &size).unwrap();

    assert_near(bevel.touches[0], Point2::new(35.0, 0.0));
    assert_near(bevel.touches[1], Point2::new(40.0, 5.0));

    sketch.chamfer(&found, &size).unwrap();
    let angle = sketch
        .constraints()
        .find(|(_, constraint)| matches!(constraint, Constraint::Angle { .. }))
        .map(|(_, constraint)| constraint.clone())
        .unwrap();

    assert!(
        sketch
            .measured(&angle)
            .is_some_and(|found| (found - 45.0).abs() < EXACT)
    );
}

#[test]
fn a_chamfer_angle_outside_a_half_turn_or_missing_the_other_curve_is_refused() {
    let Rectangle { sketch, corner, .. } = rectangle();
    let found = sketch.corner_at(corner).unwrap();
    let at = |angle: f64| ChamferSize::DistanceAndAngle {
        distance: dimensioned(5.0),
        angle: degrees(angle),
    };

    assert_eq!(
        sketch.bevel(&found, &at(0.0)),
        Err(FilletError::AngleOutOfRange)
    );
    assert_eq!(
        sketch.bevel(&found, &at(180.0)),
        Err(FilletError::AngleOutOfRange)
    );
    assert!(matches!(
        sketch.bevel(&found, &at(150.0)),
        Err(FilletError::AngleMisses { .. })
    ));
    assert!(matches!(
        sketch.bevel(&found, &at(89.0)),
        Err(FilletError::TooFar { .. })
    ));
}

#[test]
fn a_line_and_an_arc_are_chamfered_by_a_distance_and_an_angle_held_at_the_line() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 30.0), Point2::new(0.0, 10.0));
    let arc = sketch.add_arc(Point2::ZERO, Point2::new(10.0, 0.0), Point2::new(0.0, 10.0));
    let (_, line_end) = ends(&sketch, line);
    let (_, arc_end) = ends(&sketch, arc);
    sketch
        .add_constraint(Constraint::Coincident(line_end, arc_end))
        .unwrap();
    let found = sketch.corner_between(line, arc).unwrap();
    let size = ChamferSize::DistanceAndAngle {
        distance: dimensioned(4.0),
        angle: degrees(45.0),
    };

    sketch.chamfer(&found, &size).unwrap();
    let angle = sketch
        .constraints()
        .find(|(_, constraint)| matches!(constraint, Constraint::Angle { .. }))
        .map(|(_, constraint)| constraint.clone())
        .unwrap();

    assert!(
        sketch
            .measured(&angle)
            .is_some_and(|found| (found - 45.0).abs() < EXACT)
    );
    assert!(solve(&sketch).solution.redundancies().is_empty());
}

fn elliptic_ends(sketch: &Sketch, curve: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(curve) {
        Some(Entity::EllipticalArc { start, end, .. }) => (*start, *end),
        other => panic!("expected an elliptical arc, found {other:?}"),
    }
}

fn gap_to_ellipse(sketch: &Sketch, ellipse: EntityId, point: Point2) -> f64 {
    sketch
        .closest_on_ellipse(ellipse, point)
        .unwrap()
        .distance(point)
}

fn line_on_elliptical_arc(corner_at_start: bool) -> (Sketch, EntityId, EntityId) {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_elliptical_arc(
        Point2::ZERO,
        Point2::new(10.0, 0.0),
        4.0,
        Point2::new(10.0, 0.0),
        Point2::new(0.0, 4.0),
    );
    let (start, end) = elliptic_ends(&sketch, arc);
    let (line, joint) = if corner_at_start {
        (
            sketch.add_line(Point2::new(10.0, 0.0), Point2::new(20.0, 0.0)),
            start,
        )
    } else {
        (
            sketch.add_line(Point2::new(0.0, 4.0), Point2::new(0.0, 14.0)),
            end,
        )
    };
    let (line_start, _) = ends(&sketch, line);
    sketch
        .add_constraint(Constraint::Coincident(line_start, joint))
        .unwrap();
    (sketch, line, arc)
}

#[test]
fn a_line_meeting_an_elliptical_arc_is_rounded_by_an_arc_tangent_to_both() {
    for corner_at_start in [true, false] {
        let (mut sketch, line, ellipse) = line_on_elliptical_arc(corner_at_start);
        let freedom = solve(&sketch).solution.degrees_of_freedom();
        let corner = sketch.corner_between(ellipse, line).unwrap();

        let rounding = sketch.fillet(&corner, 1.5, mm(1.5)).unwrap();

        let fillet = sketch.arc(rounding).unwrap();
        let (start, end) = sketch.line_endpoints(line).unwrap();
        let line_gap = (fillet.center - start)
            .perp_dot((end - start).normalize())
            .abs();
        assert!((fillet.radius - 1.5).abs() < EXACT);
        assert!((line_gap - 1.5).abs() < EXACT, "{line_gap}");
        assert!((gap_to_ellipse(&sketch, ellipse, fillet.center) - 1.5).abs() < 1e-6);
        assert!(has(&sketch, &Constraint::Tangent(ellipse, rounding)));
        let solved = solve(&sketch);
        assert_clean(&solved);
        assert_eq!(solved.solution.degrees_of_freedom(), freedom);
        let moved = solved.geometry.arc(rounding).unwrap();
        assert!(moved.center.distance(fillet.center) < 1e-6);
    }
}

#[test]
fn an_elliptical_arc_too_short_for_the_radius_is_refused_and_one_is_chamfered_along_it() {
    let (mut sketch, line, ellipse) = line_on_elliptical_arc(true);
    let corner = sketch.corner_between(line, ellipse).unwrap();

    assert!(matches!(
        sketch.rounding(&corner, 40.0),
        Err(FilletError::TooLarge { .. })
    ));

    sketch.chamfer(&corner, &equal(2.0)).unwrap();
    let (start, _) = elliptic_ends(&sketch, ellipse);
    let touch = sketch.point(start).unwrap();
    assert!((touch.distance(Point2::new(10.0, 0.0)) - 2.0).abs() < EXACT);
    assert!(gap_to_ellipse(&sketch, ellipse, touch) < EXACT);
    assert_clean(&solve(&sketch));
}

#[test]
fn a_corner_of_two_elliptical_arcs_is_rounded_tangent_to_both() {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_elliptical_arc(
        Point2::ZERO,
        Point2::new(10.0, 0.0),
        4.0,
        Point2::new(10.0, 0.0),
        Point2::new(0.0, 4.0),
    );
    let second = sketch.add_elliptical_arc(
        Point2::new(10.0, -5.0),
        Point2::new(16.0, -5.0),
        5.0,
        Point2::new(10.0, 0.0),
        Point2::new(4.0, -5.0),
    );
    let (corner, _) = elliptic_ends(&sketch, first);
    let (other, _) = elliptic_ends(&sketch, second);
    sketch
        .add_constraint(Constraint::Coincident(corner, other))
        .unwrap();
    let found = sketch.corner_between(first, second).unwrap();

    let rounding = sketch.fillet(&found, 1.0, mm(1.0)).unwrap();

    let fillet = sketch.arc(rounding).unwrap();
    for ellipse in [first, second] {
        assert!((gap_to_ellipse(&sketch, ellipse, fillet.center) - 1.0).abs() < 1e-6);
    }
    assert_clean(&solve(&sketch));
}

#[test]
fn an_elliptical_arc_is_chamfered_by_a_distance_along_it_and_an_angle_from_it() {
    for corner_at_start in [true, false] {
        let (mut sketch, line, ellipse) = line_on_elliptical_arc(corner_at_start);
        let corner = sketch.corner_between(ellipse, line).unwrap();
        let size = ChamferSize::DistanceAndAngle {
            distance: dimensioned(3.0),
            angle: degrees(10.0),
        };

        sketch.chamfer(&corner, &size).unwrap();
        let angle = sketch
            .constraints()
            .find(|(_, constraint)| matches!(constraint, Constraint::Angle { .. }))
            .map(|(_, constraint)| constraint.clone())
            .unwrap();

        assert!(matches!(
            angle,
            Constraint::Angle { from, to, .. } if from == ellipse || to == ellipse
        ));
        assert!(
            sketch
                .measured(&angle)
                .is_some_and(|found| (found - 10.0).abs() < 1e-6),
            "{:?}",
            sketch.measured(&angle)
        );
        let solved = solve(&sketch);
        assert_clean(&solved);
        assert!(
            solved
                .geometry
                .measured(&angle)
                .is_some_and(|found| (found - 10.0).abs() < 1e-6)
        );
    }
}

fn pick(curve: EntityId, x: f64, y: f64) -> Pick {
    Pick {
        curve,
        near: Point2::new(x, y),
    }
}

#[test]
fn two_crossing_lines_are_trimmed_to_their_crossing_and_rounded_in_one_step() {
    let mut sketch = Sketch::new(Plane::XY);
    let level = sketch.add_line(Point2::new(-10.0, 0.0), Point2::new(10.0, 0.0));
    let upright = sketch.add_line(Point2::new(0.0, -10.0), Point2::new(0.0, 10.0));

    let corner = sketch
        .join_at_crossing([pick(level, 8.0, 0.0), pick(upright, 0.0, 6.0)])
        .unwrap();
    let arc = sketch.fillet(&corner, 2.0, mm(2.0)).unwrap();

    let (level_start, level_end) = ends(&sketch, level);
    let (upright_start, upright_end) = ends(&sketch, upright);
    assert_near(sketch.point(level_start).unwrap(), Point2::new(2.0, 0.0));
    assert_near(sketch.point(level_end).unwrap(), Point2::new(10.0, 0.0));
    assert_near(sketch.point(upright_start).unwrap(), Point2::new(0.0, 2.0));
    assert_near(sketch.point(upright_end).unwrap(), Point2::new(0.0, 10.0));
    assert_near(sketch.arc(arc).unwrap().center, Point2::new(2.0, 2.0));
    assert_eq!(sketch.point(corner.point), Some(Point2::ZERO));
    assert_eq!(sketch.free_points(), vec![corner.point]);
    assert_clean(&solve(&sketch));
}

#[test]
fn two_lines_stopping_short_are_extended_to_where_they_would_meet() {
    let mut sketch = Sketch::new(Plane::XY);
    let level = sketch.add_line(Point2::new(-10.0, 0.0), Point2::new(-3.0, 0.0));
    let upright = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(0.0, 4.0));
    let mut chamfered = sketch.clone();

    let corner = sketch
        .join_at_crossing([pick(level, -6.0, 0.0), pick(upright, 0.0, 7.0)])
        .unwrap();
    sketch.fillet(&corner, 2.0, mm(2.0)).unwrap();
    let cut = chamfered
        .join_at_crossing([pick(level, -6.0, 0.0), pick(upright, 0.0, 7.0)])
        .unwrap();
    chamfered.chamfer(&cut, &equal(1.0)).unwrap();

    let (_, level_end) = ends(&sketch, level);
    let (_, upright_end) = ends(&sketch, upright);
    assert_near(sketch.point(level_end).unwrap(), Point2::new(-2.0, 0.0));
    assert_near(sketch.point(upright_end).unwrap(), Point2::new(0.0, 2.0));
    assert_eq!(sketch.open_ends().len(), 2);
    assert_clean(&solve(&sketch));
    let (_, cut_end) = ends(&chamfered, level);
    assert_near(chamfered.point(cut_end).unwrap(), Point2::new(-1.0, 0.0));
}

#[test]
fn the_lever_outline_rounds_where_each_line_meets_a_boss_in_one_step_each() {
    let mut sketch = Sketch::new(Plane::XY);
    let left = sketch.add_circle(Point2::ZERO, 10.0);
    let right = sketch.add_circle(Point2::new(50.0, 0.0), 6.0);
    let top = sketch.add_line(Point2::new(0.0, 8.0), Point2::new(50.0, 4.0));
    let bottom = sketch.add_line(Point2::new(0.0, -8.0), Point2::new(50.0, -4.0));

    let steps = [
        [pick(top, 20.0, 6.4), pick(left, 5.0, 8.66)],
        [pick(bottom, 20.0, -6.4), pick(left, 5.0, -8.66)],
        [pick(top, 30.0, 5.6), pick(right, 46.0, 4.47)],
        [pick(bottom, 30.0, -5.6), pick(right, 46.0, -4.47)],
    ];
    for picks in steps {
        let corner = sketch.join_at_crossing(picks).unwrap();
        sketch.fillet(&corner, 3.0, mm(3.0)).unwrap();
    }

    assert!(sketch.open_ends().is_empty(), "{:?}", sketch.open_ends());
    let left_arc = sketch.arc(left).unwrap();
    let right_arc = sketch.arc(right).unwrap();
    assert!(left_arc.sweep > PI);
    assert!(right_arc.sweep > PI);
    assert!(
        left_arc
            .point_at(left_arc.start_angle + left_arc.sweep / 2.0)
            .x
            < -9.0
    );
    assert!(
        right_arc
            .point_at(right_arc.start_angle + right_arc.sweep / 2.0)
            .x
            > 55.0
    );
    assert_clean(&solve(&sketch));
}

#[test]
fn selected_curves_keep_their_longer_parts_and_a_boss_its_blunter_side() {
    let mut sketch = Sketch::new(Plane::XY);
    let boss = sketch.add_circle(Point2::ZERO, 10.0);
    let top = sketch.add_line(Point2::new(0.0, 8.0), Point2::new(50.0, 4.0));

    let picks = sketch.crossing_picks(top, boss).unwrap();
    let corner = sketch.join_at_crossing(picks).unwrap();

    let arc = sketch.arc(boss).unwrap();
    assert!(arc.point_at(arc.start_angle + arc.sweep / 2.0).y > 9.0);
    let (start, end) = ends(&sketch, top);
    assert_eq!(start, corner.point);
    assert_near(sketch.point(end).unwrap(), Point2::new(50.0, 4.0));
}

#[test]
fn curves_that_never_meet_or_cannot_move_are_refused_in_words() {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    let second = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(10.0, 5.0));
    let upright = sketch.add_line(Point2::new(20.0, 10.0), Point2::new(20.0, 2.0));
    let (_, held) = ends(&sketch, upright);
    sketch
        .add_constraint(Constraint::Fix {
            point: held,
            at: Point2::new(20.0, 2.0),
        })
        .unwrap();

    assert!(matches!(
        sketch.join_at_crossing([pick(first, 5.0, 0.0), pick(second, 5.0, 5.0)]),
        Err(FilletError::NeverMeet { .. })
    ));
    assert!(matches!(
        sketch.join_at_crossing([pick(first, 5.0, 0.0), pick(upright, 20.0, 6.0)]),
        Err(FilletError::Held(_))
    ));
    assert!(matches!(
        sketch.join_at_crossing([pick(first, 5.0, 0.0), pick(first, 6.0, 0.0)]),
        Err(FilletError::SameCurve { .. })
    ));
}

const PLACED_LABEL: Vector2 = Vector2::new(3.0, -7.0);

struct DisabledOnSecondPoint {
    sketch: Sketch,
    corner: crate::Corner,
    dimension: ConstraintId,
    far: EntityId,
}

fn disabled_on_second_point() -> DisabledOnSecondPoint {
    let mut sketch = Sketch::new(Plane::XY);

    let first = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let second = sketch.add_line(Point2::new(40.0, 0.0), Point2::new(40.0, 20.0));
    let far = sketch.add_point(Point2::new(60.0, 0.0));
    let (_, first_end) = ends(&sketch, first);
    let (second_start, _) = ends(&sketch, second);
    sketch
        .add_constraint(Constraint::Coincident(first_end, second_start))
        .unwrap();
    let dimension = sketch
        .add_constraint(Constraint::Distance {
            from: second_start,
            to: far,
            value: mm(20.0),
        })
        .unwrap();
    sketch.set_active(dimension, false).unwrap();
    sketch
        .set_label_offset(dimension, Some(PLACED_LABEL))
        .unwrap();
    let corner = sketch.corner_between(first, second).unwrap();

    DisabledOnSecondPoint {
        sketch,
        corner,
        dimension,
        far,
    }
}

fn distance_to(sketch: &Sketch, far: EntityId) -> ConstraintId {
    let found: Vec<ConstraintId> = sketch
        .constraints()
        .filter(
            |(_, constraint)| matches!(constraint, Constraint::Distance { to, .. } if *to == far),
        )
        .map(|(id, _)| id)
        .collect();
    assert_eq!(found.len(), 1);
    found[0]
}

#[test]
fn a_disabled_dimension_on_the_second_corner_point_stays_disabled_and_placed_when_filleted() {
    let DisabledOnSecondPoint {
        mut sketch,
        corner,
        dimension,
        far,
    } = disabled_on_second_point();

    sketch.fillet(&corner, 5.0, mm(5.0)).unwrap();

    let moved = distance_to(&sketch, far);
    assert_eq!(moved, dimension);
    assert!(!sketch.is_active(moved));
    assert_eq!(sketch.label_offset(moved), Some(PLACED_LABEL));
}

#[test]
fn a_disabled_dimension_on_the_second_corner_point_stays_disabled_and_placed_when_chamfered() {
    let DisabledOnSecondPoint {
        mut sketch,
        corner,
        dimension,
        far,
    } = disabled_on_second_point();

    sketch.chamfer(&corner, &equal(5.0)).unwrap();

    let moved = distance_to(&sketch, far);
    assert_eq!(moved, dimension);
    assert!(!sketch.is_active(moved));
    assert_eq!(sketch.label_offset(moved), Some(PLACED_LABEL));
}

#[test]
fn a_moved_constraint_is_not_dropped_as_a_duplicate_of_an_inactive_one() {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::new(-20.0, 10.0), Point2::ZERO);
    let second = sketch.add_line(Point2::ZERO, Point2::new(20.0, 10.0));
    let (_, kept) = ends(&sketch, first);
    let (merged, _) = ends(&sketch, second);
    let inactive = sketch
        .add_constraint(Constraint::Fix {
            point: kept,
            at: Point2::ZERO,
        })
        .unwrap();
    sketch.set_active(inactive, false).unwrap();
    sketch
        .add_constraint(Constraint::Fix {
            point: merged,
            at: Point2::ZERO,
        })
        .unwrap();
    let corner = sketch.fillet_corners().into_iter().next().unwrap();

    sketch.fillet(&corner, 4.0, mm(4.0)).unwrap();

    assert!(sketch.active_constraints().any(|(_, constraint)| {
        *constraint
            == Constraint::Fix {
                point: kept,
                at: Point2::ZERO,
            }
    }));
}

const MAJOR: f64 = 10.0;
const MINOR: f64 = 4.0;

fn on_ellipse(parameter: f64) -> Point2 {
    Point2::new(MAJOR * parameter.cos(), MINOR * parameter.sin())
}

fn ellipse_between(sketch: &mut Sketch, from: f64, to: f64) -> EntityId {
    sketch.add_elliptical_arc(
        Point2::ZERO,
        Point2::new(MAJOR, 0.0),
        MINOR,
        on_ellipse(from),
        on_ellipse(to),
    )
}

fn freedom(sketch: &Sketch) -> usize {
    solve(sketch).solution.degrees_of_freedom()
}

fn assert_on_ellipse(sketch: &Sketch, ellipse: EntityId, point: EntityId) {
    let at = sketch.point(point).unwrap();
    let whole = sketch.ellipse(ellipse).unwrap();
    let axis = whole.axis();
    let offset = at - whole.center;
    let level = (offset.dot(axis) / whole.major_radius()).powi(2)
        + (offset.dot(axis.perp()) / whole.minor_radius).powi(2);
    assert!((level - 1.0).abs() < EXACT, "{at} lies off the ellipse");
}

fn assert_rounds_tangent(sketch: &Sketch, curves: [EntityId; 2], rounding: EntityId) {
    for curve in curves {
        assert!(
            has(sketch, &Constraint::Tangent(curve, rounding)),
            "the rounding is not tangent to {curve:?}"
        );
    }
}

fn assert_holds(sketch: &Sketch, freedom_before: usize, rounding: EntityId) {
    let center = sketch.arc(rounding).unwrap().center;

    let solved = solve(sketch);

    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom_before);
    let moved = solved.geometry.arc(rounding).unwrap();
    assert!(moved.center.distance(center) < 1e-6, "{center} moved");
}

#[test]
fn a_line_crossing_an_elliptical_arc_is_trimmed_to_the_crossing_and_rounded_tangent_to_both() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = ellipse_between(&mut sketch, -FRAC_PI_2, FRAC_PI_2);
    let upright = sketch.add_line(Point2::new(6.0, -10.0), Point2::new(6.0, 10.0));
    let before = freedom(&sketch);

    let corner = sketch
        .join_at_crossing([pick(upright, 6.0, 8.0), pick(ellipse, 2.0, 3.9)])
        .unwrap();
    let joined = freedom(&sketch);
    let rounding = sketch.fillet(&corner, 1.0, mm(1.0)).unwrap();

    let crossing = Point2::new(6.0, 3.2);
    let (ellipse_start, ellipse_end) = elliptic_ends(&sketch, ellipse);
    let (_, upright_end) = ends(&sketch, upright);
    let fillet = sketch.arc(rounding).unwrap();
    assert_near(corner.position, crossing);
    assert_near(sketch.point(corner.point).unwrap(), crossing);
    assert_near(sketch.point(ellipse_end).unwrap(), Point2::new(0.0, 4.0));
    assert_near(sketch.point(upright_end).unwrap(), Point2::new(6.0, 10.0));
    assert_on_ellipse(&sketch, ellipse, ellipse_start);
    assert!((fillet.center.x - 5.0).abs() < EXACT, "{}", fillet.center);
    assert!((gap_to_ellipse(&sketch, ellipse, fillet.center) - 1.0).abs() < 1e-6);
    assert_rounds_tangent(&sketch, [upright, ellipse], rounding);
    assert_eq!(joined, before - 2);
    assert_holds(&sketch, joined, rounding);
}

#[test]
fn an_elliptical_arc_stopping_short_of_a_line_is_carried_round_its_ellipse_to_meet_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = ellipse_between(&mut sketch, 0.0, FRAC_PI_2);
    let upright = sketch.add_line(Point2::new(-3.0, 10.0), Point2::new(-3.0, 5.0));
    let before = freedom(&sketch);
    let mut chamfered = sketch.clone();
    let picks = [pick(upright, -3.0, 8.0), pick(ellipse, 7.0, 2.8)];

    let corner = sketch.join_at_crossing(picks).unwrap();
    let joined = freedom(&sketch);
    let reached = sketch.ellipse(ellipse).unwrap();
    let rounding = sketch.fillet(&corner, 1.0, mm(1.0)).unwrap();
    let cut = chamfered.join_at_crossing(picks).unwrap();
    let line = chamfered.chamfer(&cut, &equal(1.0)).unwrap();

    let crossing = Point2::new(-3.0, MINOR * 0.91f64.sqrt());
    let (ellipse_start, _) = elliptic_ends(&sketch, ellipse);
    let (upright_start, _) = ends(&sketch, upright);
    assert_near(corner.position, crossing);
    assert!((reached.start).abs() < EXACT);
    assert!((reached.end() - reached.parameter_of(crossing)).abs() < EXACT);
    assert!(reached.sweep > FRAC_PI_2);
    assert_near(
        sketch.point(ellipse_start).unwrap(),
        Point2::new(MAJOR, 0.0),
    );
    assert_near(
        sketch.point(upright_start).unwrap(),
        Point2::new(-3.0, 10.0),
    );
    assert_rounds_tangent(&sketch, [upright, ellipse], rounding);
    assert_eq!(joined, before - 2);
    assert_holds(&sketch, joined, rounding);
    let (cut_start, cut_end) = chamfered.line_endpoints(line).unwrap();
    for touch in [cut_start, cut_end] {
        assert!((touch.distance(crossing) - 1.0).abs() < EXACT, "{touch}");
    }
    let (_, chamfered_end) = elliptic_ends(&chamfered, ellipse);
    assert_on_ellipse(&chamfered, ellipse, chamfered_end);
    let solved = solve(&chamfered);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), joined);
}

#[test]
fn an_arc_and_an_elliptical_arc_stopping_short_are_rounded_where_their_carriers_cross() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = ellipse_between(&mut sketch, 0.0, FRAC_PI_2);
    let center = Point2::new(-6.0, 0.0);
    let arc = sketch.add_arc(
        center,
        center + Vector2::from_angle(-FRAC_PI_3) * 5.0,
        center + Vector2::from_angle(FRAC_PI_3) * 5.0,
    );
    let before = freedom(&sketch);

    let corner = sketch
        .join_at_crossing([pick(arc, -1.0, 0.0), pick(ellipse, 7.0, 2.8)])
        .unwrap();
    let joined = freedom(&sketch);
    let reached = sketch.ellipse(ellipse).unwrap();
    let trimmed = sketch.arc(arc).unwrap();
    let (_, ellipse_end) = elliptic_ends(&sketch, ellipse);
    let (_, arc_end) = ends(&sketch, arc);
    let rounding = sketch.fillet(&corner, 0.5, mm(0.5)).unwrap();

    assert_eq!(ellipse_end, corner.point);
    assert_eq!(arc_end, corner.point);
    assert!(corner.position.y > 3.0, "{}", corner.position);
    assert!((corner.position.distance(center) - 5.0).abs() < EXACT);
    assert!(reached.sweep > FRAC_PI_2);
    assert!((reached.end() - reached.parameter_of(corner.position)).abs() < EXACT);
    assert!(trimmed.sweep < 2.0 * FRAC_PI_3);
    assert_near(trimmed.point_at(trimmed.end_angle()), corner.position);
    assert_rounds_tangent(&sketch, [arc, ellipse], rounding);
    assert_eq!(joined, before - 2);
    assert_holds(&sketch, joined, rounding);
}

#[test]
fn a_circle_meeting_an_elliptical_arc_opens_up_to_the_next_crossing_on_the_picked_side() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = ellipse_between(&mut sketch, -FRAC_PI_4, FRAC_PI_4);
    let boss = sketch.add_circle(Point2::ZERO, 7.0);

    let corner = sketch
        .join_at_crossing([pick(boss, 0.0, 7.0), pick(ellipse, 9.5, 1.0)])
        .unwrap();
    let rounding = sketch.fillet(&corner, 1.0, mm(1.0)).unwrap();

    let across = ((1.0 - 49.0 / 16.0) / (0.01 - 1.0 / 16.0f64)).sqrt();
    let up = (49.0 - across * across).sqrt();
    assert_near(corner.position, Point2::new(across, up));
    let opened = sketch.arc(boss).unwrap();
    assert_near(
        opened.point_at(opened.end_angle()),
        Point2::new(-across, up),
    );
    assert!(opened.point_at(opened.start_angle + opened.sweep / 2.0).y > 6.0);
    let far = match sketch.entity(boss) {
        Some(Entity::Arc { end, .. }) => *end,
        other => panic!("expected an arc, found {other:?}"),
    };
    assert!(has(&sketch, &Constraint::Coincident(far, ellipse)));
    assert_on_ellipse(&sketch, ellipse, far);
    assert_rounds_tangent(&sketch, [boss, ellipse], rounding);
    assert_clean(&solve(&sketch));
}

#[test]
fn two_elliptical_arcs_crossing_are_trimmed_to_their_crossing_and_chamfered() {
    let mut sketch = Sketch::new(Plane::XY);
    let first = ellipse_between(&mut sketch, -FRAC_PI_2, FRAC_PI_2);
    let narrow_center = Point2::new(8.0, 0.0);
    let on_narrow = |parameter: f64| {
        narrow_center + Vector2::new(-3.0 * parameter.sin(), 6.0 * parameter.cos())
    };
    let second = sketch.add_elliptical_arc(
        narrow_center,
        narrow_center + Vector2::new(0.0, 6.0),
        3.0,
        on_narrow(0.0),
        on_narrow(PI),
    );
    let before = freedom(&sketch);

    let corner = sketch
        .join_at_crossing([pick(first, 2.0, 3.9), pick(second, 8.0, 5.5)])
        .unwrap();
    let joined = freedom(&sketch);
    let line = sketch.chamfer(&corner, &equal(1.0)).unwrap();

    let (first_start, first_end) = elliptic_ends(&sketch, first);
    let (second_start, second_end) = elliptic_ends(&sketch, second);
    assert!(corner.position.y > 0.0, "{}", corner.position);
    assert_near(sketch.point(first_end).unwrap(), Point2::new(0.0, MINOR));
    assert_near(sketch.point(second_start).unwrap(), on_narrow(0.0));
    for (ellipse, touch) in [(first, first_start), (second, second_end)] {
        assert_on_ellipse(&sketch, ellipse, touch);
        let at = sketch.point(touch).unwrap();
        assert!((at.distance(corner.position) - 1.0).abs() < EXACT, "{at}");
    }
    let (cut_start, cut_end) = sketch.line_endpoints(line).unwrap();
    assert!(cut_start.distance(cut_end) > 0.5);
    assert_eq!(joined, before - 2);
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), joined);
}

#[test]
fn a_selected_line_and_elliptical_arc_keep_their_longer_parts() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = ellipse_between(&mut sketch, -FRAC_PI_2, FRAC_PI_2);
    let upright = sketch.add_line(Point2::new(6.0, -10.0), Point2::new(6.0, 2.0));

    let picks = sketch.crossing_picks(upright, ellipse).unwrap();
    let corner = sketch.join_at_crossing(picks).unwrap();

    let (ellipse_start, ellipse_end) = elliptic_ends(&sketch, ellipse);
    let (upright_start, upright_end) = ends(&sketch, upright);
    assert_near(corner.position, Point2::new(6.0, -3.2));
    assert_eq!(ellipse_start, corner.point);
    assert_near(sketch.point(ellipse_end).unwrap(), Point2::new(0.0, MINOR));
    assert_near(
        sketch.point(upright_start).unwrap(),
        Point2::new(6.0, -10.0),
    );
    assert_eq!(upright_end, corner.point);
}

struct Wedge {
    sketch: Sketch,
    first: EntityId,
    second: EntityId,
    corner: EntityId,
}

fn wedge() -> Wedge {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::new(-20.0, 10.0), Point2::ZERO);
    let (_, corner) = ends(&sketch, first);
    let far = sketch.add_point(Point2::new(20.0, 10.0));
    let second = EntityId::from_raw(sketch.next_id());
    sketch
        .insert_entity(
            second,
            Entity::Line {
                start: corner,
                end: far,
            },
        )
        .unwrap();
    sketch
        .add_constraint(Constraint::Fix {
            point: corner,
            at: Point2::ZERO,
        })
        .unwrap();
    Wedge {
        sketch,
        first,
        second,
        corner,
    }
}

#[test]
fn a_third_line_equal_to_a_rounded_one_is_freed_while_the_rounded_pair_stay_equal() {
    let Wedge {
        mut sketch,
        first,
        second,
        corner,
    } = wedge();
    let third = sketch.add_line(
        Point2::new(30.0, 0.0),
        Point2::new(30.0, 22.360679774997898),
    );
    sketch
        .add_constraint(Constraint::Equal(first, second))
        .unwrap();
    sketch
        .add_constraint(Constraint::Equal(first, third))
        .unwrap();

    sketch
        .fillet(&sketch.corner_at(corner).unwrap(), 4.0, mm(4.0))
        .unwrap();

    assert!(has(&sketch, &Constraint::Equal(first, second)));
    assert!(!has(&sketch, &Constraint::Equal(first, third)));
    assert_clean(&solve(&sketch));
}

#[test]
fn a_chamfer_by_unequal_distances_frees_the_equal_lengths_of_its_corner() {
    let Wedge {
        mut sketch,
        first,
        second,
        corner,
    } = wedge();
    sketch
        .add_constraint(Constraint::Equal(first, second))
        .unwrap();
    let freedom = solve(&sketch).solution.degrees_of_freedom();
    let size = ChamferSize::Distances {
        first: dimensioned(3.0),
        second: dimensioned(5.0),
    };

    sketch
        .chamfer(&sketch.corner_at(corner).unwrap(), &size)
        .unwrap();

    assert!(!has(&sketch, &Constraint::Equal(first, second)));
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom + 1);
}

#[test]
fn a_chamfer_by_one_distance_keeps_the_equal_lengths_of_its_corner() {
    let Wedge {
        mut sketch,
        first,
        second,
        corner,
    } = wedge();
    sketch
        .add_constraint(Constraint::Equal(first, second))
        .unwrap();
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    sketch
        .chamfer(&sketch.corner_at(corner).unwrap(), &equal(4.0))
        .unwrap();

    assert!(has(&sketch, &Constraint::Equal(first, second)));
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);
}

#[test]
fn a_midpoint_on_a_rounded_line_becomes_a_point_on_it_while_it_stays_on_what_is_left() {
    let Rectangle {
        mut sketch,
        sides,
        corner,
        ..
    } = rectangle();
    let middle = sketch.add_point(Point2::new(20.0, 0.0));
    let held = sketch
        .add_constraint(Constraint::Midpoint {
            point: middle,
            curve: sides[0],
        })
        .unwrap();
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    sketch
        .fillet(&sketch.corner_at(corner).unwrap(), 5.0, mm(5.0))
        .unwrap();

    assert_eq!(
        sketch.constraint(held),
        Some(&Constraint::Coincident(middle, sides[0]))
    );
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom + 1);
}

#[test]
fn a_midpoint_on_a_rounded_line_is_freed_when_the_round_takes_its_part_of_the_line() {
    let mut sketch = Sketch::new(Plane::XY);
    let across = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let (_, corner) = ends(&sketch, across);
    let up = sketch.add_line(Point2::new(40.0, 0.0), Point2::new(40.0, 100.0));
    let (joint, _) = ends(&sketch, up);
    sketch
        .add_constraint(Constraint::Coincident(corner, joint))
        .unwrap();
    let middle = sketch.add_point(Point2::new(20.0, 0.0));
    sketch
        .add_constraint(Constraint::Midpoint {
            point: middle,
            curve: across,
        })
        .unwrap();

    sketch
        .fillet(&sketch.corner_at(corner).unwrap(), 30.0, mm(30.0))
        .unwrap();

    assert!(sketch.constraints_using(middle).is_empty());
}
