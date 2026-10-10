use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2};

use crate::{
    CircularPattern, Constraint, Dimensioned, Entity, EntityId, Faceting, MAX_PATTERN_INSTANCES,
    PatternError, PatternRow, RectangularPattern, Sketch, Solved, Spread,
};

const EXACT: f64 = 1e-6;
const SPACING: ParameterId = ParameterId::from_raw(7);

fn solve_with(sketch: &Sketch, spacing: f64) -> Solved {
    let value_of = |id: ParameterId| {
        if id == SPACING {
            Ok(Quantity::length(spacing))
        } else {
            Err(EvalError::ParameterMissing)
        }
    };
    match sketch.solve(&value_of, &|| false) {
        Ok(solved) => solved,
        Err(error) => panic!("the sketch does not solve: {error:?}"),
    }
}

fn solve(sketch: &Sketch) -> Solved {
    solve_with(sketch, 0.0)
}

fn mm(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

fn degrees(value: f64) -> Dimensioned {
    Dimensioned {
        expression: Expression::Measure(value, Unit::Degree),
        value,
    }
}

fn length(value: f64) -> Dimensioned {
    Dimensioned {
        expression: Expression::measure(value, Unit::Millimetre),
        value,
    }
}

fn row(count: usize, spacing: f64, angle: f64) -> PatternRow {
    PatternRow {
        count,
        spacing: length(spacing),
        angle: degrees(angle),
    }
}

fn across(count: usize, spacing: f64) -> RectangularPattern {
    RectangularPattern {
        first: row(count, spacing, 0.0),
        second: None,
    }
}

fn turns(count: usize) -> CircularPattern {
    CircularPattern {
        count,
        spread: Spread::FullTurn,
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

fn assert_clean(solved: &Solved) {
    assert!(
        solved.solution.redundancies().is_empty(),
        "redundant: {:?}",
        solved.solution.redundancies()
    );
}

fn count_of(sketch: &Sketch, kind: &str) -> usize {
    sketch
        .constraints()
        .filter(|(_, constraint)| constraint.kind_name() == kind)
        .count()
}

struct Outline {
    lines: [EntityId; 4],
    width: crate::ConstraintId,
}

fn rectangle(sketch: &mut Sketch, corner: Point2, width: f64, height: f64) -> Outline {
    let (x, y) = (corner.x, corner.y);
    let bottom = sketch.add_line(Point2::new(x, y), Point2::new(x + width, y));
    let right = sketch.add_line(
        Point2::new(x + width, y),
        Point2::new(x + width, y + height),
    );
    let top = sketch.add_line(
        Point2::new(x + width, y + height),
        Point2::new(x, y + height),
    );
    let left = sketch.add_line(Point2::new(x, y + height), Point2::new(x, y));
    let corners = [(bottom, right), (right, top), (top, left), (left, bottom)];
    for (first, second) in corners {
        let (_, end) = ends(sketch, first);
        let (start, _) = ends(sketch, second);
        sketch
            .add_constraint(Constraint::Coincident(end, start))
            .unwrap();
    }
    sketch
        .add_constraint(Constraint::Horizontal(bottom))
        .unwrap();
    sketch.add_constraint(Constraint::Horizontal(top)).unwrap();
    sketch.add_constraint(Constraint::Vertical(right)).unwrap();
    sketch.add_constraint(Constraint::Vertical(left)).unwrap();
    let (first, second) = ends(sketch, bottom);
    let width = sketch
        .add_constraint(Constraint::Distance {
            from: first,
            to: second,
            value: mm(width),
        })
        .unwrap();
    let (first, second) = ends(sketch, right);
    sketch
        .add_constraint(Constraint::Distance {
            from: first,
            to: second,
            value: mm(height),
        })
        .unwrap();
    Outline {
        lines: [bottom, right, top, left],
        width,
    }
}

#[test]
fn a_row_of_a_constrained_rectangle_follows_every_edit_of_the_original() {
    let mut sketch = Sketch::new(Plane::XY);
    let original = rectangle(&mut sketch, Point2::ZERO, 10.0, 6.0);
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    let copies = sketch
        .rectangular_pattern(&original.lines, &across(3, 25.0))
        .unwrap();

    assert_eq!(copies.len(), 8);
    let (start, end) = ends(&sketch, copies[0]);
    assert_near(sketch.point(start).unwrap(), Point2::new(25.0, 0.0));
    assert_near(sketch.point(end).unwrap(), Point2::new(35.0, 0.0));
    let (_, far) = ends(&sketch, copies[4]);
    assert_near(sketch.point(far).unwrap(), Point2::new(60.0, 0.0));
    assert_eq!(count_of(&sketch, "Horizontal distance"), 16);
    assert_eq!(count_of(&sketch, "Horizontal"), 2 + 16);
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);

    sketch.set_dimension(original.width, mm(14.0)).unwrap();
    let wider = solve(&sketch).geometry;
    let (home, _) = ends(&wider, original.lines[0]);
    let x = |point: EntityId| wider.point(point).unwrap().x;
    let (first_start, first_end) = ends(&wider, copies[0]);
    let (second_start, second_end) = ends(&wider, copies[4]);
    assert!((x(first_end) - x(first_start) - 14.0).abs() < EXACT);
    assert!((x(first_start) - x(home) - 25.0).abs() < EXACT);
    assert!((x(second_start) - x(first_start) - 25.0).abs() < EXACT);
    assert!((x(second_end) - x(second_start) - 14.0).abs() < EXACT);
}

#[test]
fn a_grid_repeats_along_a_second_direction_square_to_the_first() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(5.0, 5.0), 2.0);
    sketch.set_construction(circle, true).unwrap();
    let pattern = RectangularPattern {
        first: row(3, 10.0, 0.0),
        second: Some(row(2, 12.0, 90.0)),
    };

    let copies = sketch.rectangular_pattern(&[circle], &pattern).unwrap();

    assert_eq!(copies.len(), 5);
    let centres: Vec<Point2> = copies
        .iter()
        .map(|copy| sketch.circle(*copy).unwrap().0)
        .collect();
    for (centre, expected) in centres.iter().zip([
        Point2::new(15.0, 5.0),
        Point2::new(25.0, 5.0),
        Point2::new(5.0, 17.0),
        Point2::new(15.0, 17.0),
        Point2::new(25.0, 17.0),
    ]) {
        assert_near(*centre, expected);
    }
    assert!(copies.iter().all(|copy| sketch.is_construction(*copy)));
    assert_eq!(count_of(&sketch, "Equal"), 5);
    assert_eq!(count_of(&sketch, "Vertical"), 1);
    assert_eq!(count_of(&sketch, "Horizontal"), 4);
    assert_clean(&solve(&sketch));
}

#[test]
fn a_slanted_row_and_a_negative_spacing_run_the_way_they_were_asked() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(4.0, 0.0));
    let pattern = RectangularPattern {
        first: row(3, 10.0, 30.0),
        second: None,
    };

    let copies = sketch.rectangular_pattern(&[line], &pattern).unwrap();

    let (start, _) = ends(&sketch, copies[1]);
    let expected = Point2::new(20.0 * 30.0_f64.to_radians().cos(), 10.0);
    assert_near(sketch.point(start).unwrap(), expected);
    assert_eq!(count_of(&sketch, "Horizontal distance"), 4);
    assert_eq!(count_of(&sketch, "Vertical distance"), 4);
    assert_clean(&solve(&sketch));

    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(4.0, 0.0));
    let backwards = across(2, -15.0);

    let copies = sketch.rectangular_pattern(&[line], &backwards).unwrap();

    let (start, _) = ends(&sketch, copies[0]);
    assert_near(sketch.point(start).unwrap(), Point2::new(-15.0, 0.0));
    let solved = solve(&sketch);
    assert_clean(&solved);
    let (start, _) = ends(&solved.geometry, copies[0]);
    assert_near(
        solved.geometry.point(start).unwrap(),
        Point2::new(-15.0, 0.0),
    );
}

#[test]
fn a_spacing_naming_a_parameter_moves_every_copy_when_the_parameter_changes() {
    let mut sketch = Sketch::new(Plane::XY);
    let point = sketch.add_point(Point2::ZERO);
    let pattern = RectangularPattern {
        first: PatternRow {
            count: 4,
            spacing: Dimensioned {
                expression: Expression::Parameter(SPACING),
                value: 10.0,
            },
            angle: degrees(0.0),
        },
        second: None,
    };

    let copies = sketch.rectangular_pattern(&[point], &pattern).unwrap();

    assert_eq!(copies.len(), 3);
    let wider = solve_with(&sketch, 12.5).geometry;
    let home = wider.point(point).unwrap();
    for (copy, steps) in copies.iter().zip([1.0, 2.0, 3.0]) {
        assert_near(
            wider.point(*copy).unwrap(),
            home + Point2::new(12.5 * steps, 0.0),
        );
    }
}

#[test]
fn a_circular_pattern_turns_copies_about_the_origin_and_keeps_them_tied_to_the_original() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(14.0, 0.0));
    let circle = sketch.add_circle(Point2::new(12.0, 3.0), 1.0);
    let radius = sketch
        .add_constraint(Constraint::Radius {
            entity: circle,
            value: mm(1.0),
        })
        .unwrap();
    let (start, _) = ends(&sketch, line);
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    let copies = sketch
        .circular_pattern(&[line, circle], EntityId::ORIGIN, &turns(4))
        .unwrap();

    assert_eq!(copies.len(), 6);
    let (turned, _) = ends(&sketch, copies[0]);
    assert_near(sketch.point(turned).unwrap(), Point2::new(0.0, 10.0));
    let (opposite, _) = ends(&sketch, copies[2]);
    assert_near(sketch.point(opposite).unwrap(), Point2::new(-10.0, 0.0));
    assert_near(
        sketch.circle(copies[3]).unwrap().0,
        Point2::new(-12.0, -3.0),
    );
    assert_eq!(count_of(&sketch, "Angle"), 9);
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);

    sketch.set_dimension(radius, mm(2.0)).unwrap();
    let thicker = solve(&sketch).geometry;
    assert!((thicker.circle(copies[5]).unwrap().1 - 2.0).abs() < EXACT);

    let mut moved = sketch.clone();
    moved
        .add_constraint(Constraint::Fix {
            point: start,
            at: Point2::new(0.0, 10.0),
        })
        .unwrap();
    let held = solve(&moved).geometry;
    let (turned, _) = ends(&held, copies[0]);
    assert_near(held.point(turned).unwrap(), Point2::new(-10.0, 0.0));
}

#[test]
fn a_circular_pattern_about_a_sketch_point_over_a_total_angle_spaces_its_ends_evenly() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::new(5.0, 5.0));
    let line = sketch.add_line(Point2::new(5.0, 5.0), Point2::new(15.0, 5.0));
    let (hub, tip) = ends(&sketch, line);
    sketch.set_construction(line, true).unwrap();
    let pattern = CircularPattern {
        count: 3,
        spread: Spread::Total(degrees(90.0)),
    };

    let copies = sketch.circular_pattern(&[line], centre, &pattern).unwrap();

    assert_eq!(copies.len(), 2);
    let (copy_hub, middle) = ends(&sketch, copies[0]);
    assert_eq!(copy_hub, hub);
    assert_near(
        sketch.point(middle).unwrap(),
        Point2::new(
            5.0 + 10.0 * 45.0_f64.to_radians().cos(),
            5.0 + 10.0 * 45.0_f64.to_radians().sin(),
        ),
    );
    let (_, last) = ends(&sketch, copies[1]);
    assert_near(sketch.point(last).unwrap(), Point2::new(5.0, 15.0));
    assert!(copies.iter().all(|copy| sketch.is_construction(*copy)));
    assert!(
        sketch
            .constraints()
            .any(|(_, constraint)| { *constraint == Constraint::Coincident(hub, centre) })
    );
    assert!(!sketch.constraints().any(|(_, constraint)| {
        matches!(constraint, Constraint::Coincident(a, b) if *a == tip || *b == tip)
    }));
    let solved = solve(&sketch);
    assert_clean(&solved);
    let (_, solved_last) = ends(&solved.geometry, copies[1]);
    assert_near(
        solved.geometry.point(solved_last).unwrap(),
        Point2::new(5.0, 15.0),
    );
}

#[test]
fn a_negative_total_angle_turns_clockwise() {
    let mut sketch = Sketch::new(Plane::XY);
    let point = sketch.add_point(Point2::new(10.0, 0.0));
    let pattern = CircularPattern {
        count: 2,
        spread: Spread::Total(Dimensioned {
            expression: Expression::Negate(Box::new(Expression::Measure(90.0, Unit::Degree))),
            value: -90.0,
        }),
    };

    let copies = sketch
        .circular_pattern(&[point], EntityId::ORIGIN, &pattern)
        .unwrap();

    assert_near(sketch.point(copies[0]).unwrap(), Point2::new(0.0, -10.0));
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_near(
        solved.geometry.point(copies[0]).unwrap(),
        Point2::new(0.0, -10.0),
    );
}

#[test]
fn geometry_on_the_centre_stays_there_and_a_circle_about_it_is_left_out() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(8.0, 0.0));
    let rim = sketch.add_circle(Point2::ZERO, 3.0);
    let (root, _) = ends(&sketch, line);

    let copies = sketch
        .circular_pattern(&[line, rim], EntityId::ORIGIN, &turns(3))
        .unwrap();

    assert_eq!(copies.len(), 2);
    assert!(
        sketch
            .constraints()
            .any(|(_, constraint)| *constraint == Constraint::Coincident(root, EntityId::ORIGIN))
    );
    assert_eq!(count_of(&sketch, "Equal"), 2);
    assert_clean(&solve(&sketch));

    let mut only_the_rim = Sketch::new(Plane::XY);
    let rim = only_the_rim.add_circle(Point2::ZERO, 3.0);
    assert_eq!(
        only_the_rim.circular_pattern(&[rim], EntityId::ORIGIN, &turns(3)),
        Err(PatternError::NothingToTurn)
    );
}

#[test]
fn a_pattern_is_previewed_without_changing_the_sketch() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(1.0, 1.0), Point2::new(4.0, 2.0));
    let lone = sketch.add_point(Point2::new(3.0, 7.0));
    let before = sketch.clone();

    let image = sketch
        .rectangular_image(&[line, lone], &across(3, 10.0), Faceting::within(0.01))
        .unwrap();

    assert_eq!(sketch, before);
    assert_eq!(image.curves.len(), 2);
    assert_near(image.curves[0][0], Point2::new(11.0, 1.0));
    assert_near(image.curves[1][0], Point2::new(21.0, 1.0));
    assert_eq!(image.points.len(), 2);
    assert_near(image.points[1], Point2::new(23.0, 7.0));

    let turned = sketch
        .circular_image(&[line], EntityId::ORIGIN, &turns(4), Faceting::within(0.01))
        .unwrap();
    assert_eq!(sketch, before);
    assert_eq!(turned.curves.len(), 3);
    assert_near(turned.curves[0][0], Point2::new(-1.0, 1.0));
}

#[test]
fn nothing_to_pattern_a_small_count_or_a_zero_spacing_is_refused_in_words() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(4.0, 0.0));
    let centre = sketch.add_point(Point2::new(9.0, 9.0));
    let before = sketch.clone();

    assert_eq!(
        sketch.rectangular_pattern(&[], &across(3, 5.0)),
        Err(PatternError::NothingSelected)
    );
    assert_eq!(
        sketch.rectangular_pattern(
            &[EntityId::ORIGIN, EntityId::VERTICAL_AXIS],
            &across(3, 5.0)
        ),
        Err(PatternError::NothingSelected)
    );
    assert_eq!(
        sketch.rectangular_pattern(&[line], &across(1, 5.0)),
        Err(PatternError::CountTooSmall { count: 1 })
    );
    assert_eq!(
        sketch.rectangular_pattern(&[line], &across(3, 0.0)),
        Err(PatternError::ZeroSpacing)
    );
    let beside = RectangularPattern {
        first: row(3, 5.0, 0.0),
        second: Some(row(2, 5.0, 180.0)),
    };
    assert_eq!(
        sketch.rectangular_pattern(&[line], &beside),
        Err(PatternError::ParallelDirections)
    );
    let many = RectangularPattern {
        first: row(20, 5.0, 0.0),
        second: Some(row(20, 5.0, 90.0)),
    };
    assert_eq!(
        sketch.rectangular_pattern(&[line], &many),
        Err(PatternError::TooManyInstances {
            count: 400,
            most: MAX_PATTERN_INSTANCES
        })
    );
    assert_eq!(
        sketch.rectangular_pattern(&[line], &across(3, 5.0e9)),
        Err(PatternError::OutOfReach)
    );
    assert_eq!(
        sketch.circular_pattern(&[line], centre, &turns(1)),
        Err(PatternError::CountTooSmall { count: 1 })
    );
    assert_eq!(
        sketch.circular_pattern(&[line], line, &turns(3)),
        Err(PatternError::NotAPoint {
            entity: line,
            label: sketch.entity_label(line),
        })
    );
    assert_eq!(
        sketch.circular_pattern(&[line], EntityId::HORIZONTAL_AXIS, &turns(3)),
        Err(PatternError::NotAPoint {
            entity: EntityId::HORIZONTAL_AXIS,
            label: "Horizontal axis".to_owned(),
        })
    );
    for total in [0.0, 360.0, -400.0] {
        let pattern = CircularPattern {
            count: 3,
            spread: Spread::Total(degrees(total)),
        };
        assert_eq!(
            sketch.circular_pattern(&[line], centre, &pattern),
            Err(PatternError::SpreadOutsideTurn)
        );
    }
    assert_eq!(sketch, before);
    assert_eq!(
        PatternError::CountTooSmall { count: 1 }.to_string(),
        "a pattern needs a count of at least 2, counting the original"
    );
}

#[test]
fn ellipses_repeat_with_their_minor_radius_held_by_equal() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::new(20.0, 5.0), Point2::new(28.0, 9.0), 3.0);
    let free = solve(&sketch).solution.degrees_of_freedom();

    let copies = sketch
        .rectangular_pattern(&[ellipse], &across(3, 40.0))
        .unwrap();
    let solved = solve(&sketch);
    let equal = sketch
        .constraints()
        .filter(|(_, constraint)| matches!(constraint, Constraint::Equal(..)))
        .count();
    let last = solved.geometry.ellipse(copies[1]).unwrap();

    assert!(solved.solution.redundancies().is_empty());
    assert_eq!(solved.solution.degrees_of_freedom(), free);
    assert_eq!(equal, 2);
    assert!(last.center.distance(Point2::new(100.0, 5.0)) < EXACT);
    assert!((last.minor_radius - 3.0).abs() < EXACT);
}

#[test]
fn an_elliptical_arc_repeats_on_its_points_alone() {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_elliptical_arc(
        Point2::new(20.0, 30.0),
        Point2::new(28.0, 30.0),
        3.0,
        Point2::new(20.0 + 8.0 * 0.5_f64.cos(), 30.0 + 3.0 * 0.5_f64.sin()),
        Point2::new(20.0, 33.0),
    );

    let copies = sketch
        .rectangular_pattern(&[arc], &across(2, 40.0))
        .unwrap();
    let solved = solve(&sketch);
    let original = solved.geometry.ellipse(arc).unwrap();
    let copy = solved.geometry.ellipse(copies[0]).unwrap();

    assert!(
        !sketch
            .constraints()
            .any(|(_, constraint)| matches!(constraint, Constraint::Equal(..)))
    );
    assert!((copy.minor_radius - 3.0).abs() < EXACT);
    assert!((copy.sweep - original.sweep).abs() < EXACT);
    assert!((copy.center - original.center).distance(Point2::new(40.0, 0.0)) < EXACT);
}

#[test]
fn an_ellipse_turns_about_a_centre_with_its_minor_radius_held() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::new(20.0, 0.0), Point2::new(28.0, 0.0), 3.0);
    let free = solve(&sketch).solution.degrees_of_freedom();

    let copies = sketch
        .circular_pattern(&[ellipse], EntityId::ORIGIN, &turns(4))
        .unwrap();
    let solved = solve(&sketch);
    let quarter = solved.geometry.ellipse(copies[0]).unwrap();

    assert!(solved.solution.redundancies().is_empty());
    assert_eq!(solved.solution.degrees_of_freedom(), free);
    assert!(quarter.center.distance(Point2::new(0.0, 20.0)) < EXACT);
    assert!((quarter.center + quarter.major).distance(Point2::new(0.0, 28.0)) < EXACT);
}
