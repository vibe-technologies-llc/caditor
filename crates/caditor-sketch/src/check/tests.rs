use caditor_geometry::{Plane, Point2};

use crate::{Constraint, Entity, EntityId, Fix, Flaw, Sketch, Tolerance};

fn ends(sketch: &Sketch, line: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(line) {
        Some(&Entity::Line { start, end }) => (start, end),
        other => panic!("expected a line, found {other:?}"),
    }
}

fn applied(sketch: &Sketch, fix: &Fix) -> Sketch {
    let mut fixed = sketch.clone();
    for constraint in fixed.constraints().map(|(id, _)| id).collect::<Vec<_>>() {
        let touches = fixed.constraint(constraint).is_some_and(|constraint| {
            constraint
                .entities()
                .iter()
                .any(|id| fix.remove.contains(id))
        });
        if touches {
            fixed.remove_constraint(constraint).unwrap();
        }
    }
    for entity in &fix.remove {
        fixed.remove_unused_entity(*entity).unwrap();
    }
    for constraint in &fix.add {
        fixed.add_constraint(constraint.clone()).unwrap();
    }
    fixed
}

#[test]
fn ends_a_hair_apart_are_named_and_joined_by_the_fix() {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::ZERO, Point2::new(20.0, 0.0));
    let second = sketch.add_line(Point2::new(20.004, 0.002), Point2::new(20.0, 15.0));
    let (_, first_end) = ends(&sketch, first);
    let (second_start, _) = ends(&sketch, second);

    let flaws = sketch.flaws(Tolerance::of(&sketch));

    let [Flaw::NearlyJoined { first, second, gap }] = flaws.as_slice() else {
        panic!("expected one pair of ends, found {flaws:?}");
    };
    assert_eq!((*first, *second), (first_end, second_start));
    assert!((gap - 0.004_472).abs() < 1e-5);
    let fixed = applied(&sketch, &sketch.fix(&flaws[0]));
    assert!(fixed.flaws(Tolerance::of(&fixed)).is_empty());
}

#[test]
fn joined_ends_and_ends_of_one_curve_are_not_flaws() {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::ZERO, Point2::new(20.0, 0.0));
    let second = sketch.add_line(Point2::new(20.0, 0.0), Point2::new(20.0, 15.0));
    let (_, first_end) = ends(&sketch, first);
    let (second_start, _) = ends(&sketch, second);
    sketch
        .add_constraint(Constraint::Coincident(first_end, second_start))
        .unwrap();
    sketch.add_arc(
        Point2::new(50.0, 0.0),
        Point2::new(55.0, 0.0),
        Point2::new(55.0, -0.001),
    );

    assert!(sketch.flaws(Tolerance::of(&sketch)).is_empty());
}

#[test]
fn a_line_drawn_twice_is_removed_and_its_joined_ends_kept_on_the_other() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 0.0));
    let copy = sketch.add_line(Point2::new(5.0, 0.001), Point2::new(25.0, 0.0));
    let stub = sketch.add_line(Point2::new(5.0, 0.001), Point2::new(5.0, 10.0));
    let (copy_start, copy_end) = ends(&sketch, copy);
    let (stub_start, _) = ends(&sketch, stub);
    sketch
        .add_constraint(Constraint::Coincident(copy_start, stub_start))
        .unwrap();

    let flaws = sketch.flaws(Tolerance::of(&sketch));

    assert_eq!(
        flaws,
        vec![Flaw::LiesOn {
            curve: copy,
            on: line
        }]
    );
    let fix = sketch.fix(&flaws[0]);
    assert_eq!(fix.remove, vec![copy, copy_end]);
    assert_eq!(fix.add, vec![Constraint::Coincident(copy_start, line)]);
    let fixed = applied(&sketch, &fix);
    assert!(fixed.flaws(Tolerance::of(&fixed)).is_empty());
}

#[test]
fn circles_and_arcs_on_one_circle_overlap_and_lines_sharing_a_stretch_are_named() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(40.0, 40.0), 6.0);
    let arc = sketch.add_arc(
        Point2::new(40.0, 40.0),
        Point2::new(46.0, 40.0),
        Point2::new(40.0, 46.0),
    );
    let first = sketch.add_line(Point2::ZERO, Point2::new(20.0, 0.0));
    let second = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(30.0, 0.0));

    let flaws = sketch.flaws(Tolerance::of(&sketch));

    assert!(flaws.contains(&Flaw::LiesOn {
        curve: arc,
        on: circle
    }));
    assert!(flaws.contains(&Flaw::Overlaps {
        curve: second,
        other: first
    }));
    assert!(
        sketch
            .fix(&Flaw::Overlaps {
                curve: second,
                other: first
            })
            .is_empty()
    );
}

#[test]
fn a_line_of_no_length_between_two_curves_is_removed_and_they_are_joined() {
    let mut sketch = Sketch::new(Plane::XY);
    let left = sketch.add_line(Point2::new(-20.0, 0.0), Point2::ZERO);
    let speck = sketch.add_line(Point2::ZERO, Point2::new(0.0, 0.000_1));
    let right = sketch.add_line(Point2::new(0.0, 0.000_1), Point2::new(20.0, 5.0));
    let (_, left_end) = ends(&sketch, left);
    let (speck_start, speck_end) = ends(&sketch, speck);
    let (right_start, _) = ends(&sketch, right);
    sketch
        .add_constraint(Constraint::Coincident(left_end, speck_start))
        .unwrap();
    sketch
        .add_constraint(Constraint::Coincident(speck_end, right_start))
        .unwrap();

    let flaws = sketch.flaws(Tolerance::of(&sketch));

    assert_eq!(flaws, vec![Flaw::NoLength { curve: speck }]);
    let fix = sketch.fix(&flaws[0]);
    assert_eq!(fix.remove, vec![speck]);
    assert_eq!(
        fix.add,
        vec![Constraint::Coincident(speck_start, speck_end)]
    );
    let fixed = applied(&sketch, &fix);
    assert!(fixed.flaws(Tolerance::of(&fixed)).is_empty());
    assert!(fixed.open_ends().len() == 2);
}
