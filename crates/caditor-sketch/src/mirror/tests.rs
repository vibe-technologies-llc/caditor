use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2};

use crate::{Constraint, Entity, EntityId, Faceting, MirrorError, Sketch, Solved};

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

#[test]
fn half_a_profile_mirrored_about_the_axis_shares_its_ends_there_and_follows_its_width() {
    let mut sketch = Sketch::new(Plane::XY);
    let bottom = sketch.add_line(Point2::ZERO, Point2::new(20.0, 0.0));
    let side = sketch.add_line(Point2::new(20.0, 0.0), Point2::new(20.0, 30.0));
    let top = sketch.add_line(Point2::new(20.0, 30.0), Point2::new(0.0, 30.0));
    let (foot, corner) = ends(&sketch, bottom);
    let (side_start, side_end) = ends(&sketch, side);
    let (top_start, head) = ends(&sketch, top);
    for (a, b) in [(corner, side_start), (side_end, top_start)] {
        sketch.add_constraint(Constraint::Coincident(a, b)).unwrap();
    }
    sketch
        .add_constraint(Constraint::Coincident(foot, EntityId::ORIGIN))
        .unwrap();
    sketch
        .add_constraint(Constraint::Coincident(head, EntityId::VERTICAL_AXIS))
        .unwrap();
    sketch
        .add_constraint(Constraint::Horizontal(bottom))
        .unwrap();
    sketch.add_constraint(Constraint::Horizontal(top)).unwrap();
    sketch.add_constraint(Constraint::Vertical(side)).unwrap();
    let width = sketch
        .add_constraint(Constraint::Distance {
            from: foot,
            to: corner,
            value: mm(20.0),
        })
        .unwrap();
    sketch
        .add_constraint(Constraint::Distance {
            from: side_start,
            to: side_end,
            value: mm(30.0),
        })
        .unwrap();
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    let copies = sketch
        .mirror(&[bottom, side, top], EntityId::VERTICAL_AXIS)
        .unwrap();

    assert_eq!(copies.len(), 3);
    let (copy_foot, copy_corner) = ends(&sketch, copies[0]);
    assert_eq!(copy_foot, foot);
    assert_near(sketch.point(copy_corner).unwrap(), Point2::new(-20.0, 0.0));
    let (_, copy_head) = ends(&sketch, copies[2]);
    assert_eq!(copy_head, head);
    assert_eq!(count_of(&sketch, "Symmetric"), 4);
    assert!(!sketch.constraints().any(
        |(_, constraint)| *constraint == Constraint::Coincident(foot, EntityId::VERTICAL_AXIS)
    ));
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);

    sketch.set_dimension(width, mm(35.0)).unwrap();
    let wider = solve(&sketch).geometry;
    assert_near(wider.point(copy_corner).unwrap(), Point2::new(-35.0, 0.0));
}

#[test]
fn arcs_and_circles_are_mirrored_about_a_slanted_line_and_keep_their_size() {
    let mut sketch = Sketch::new(Plane::XY);
    let mirror = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(10.0, 10.0));
    sketch.set_construction(mirror, true).unwrap();
    let arc = sketch.add_arc(
        Point2::new(20.0, 0.0),
        Point2::new(25.0, 0.0),
        Point2::new(20.0, 5.0),
    );
    let circle = sketch.add_circle(Point2::new(10.0, -5.0), 3.0);
    let radius = sketch
        .add_constraint(Constraint::Radius {
            entity: circle,
            value: mm(3.0),
        })
        .unwrap();
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    let copies = sketch.mirror(&[arc, circle, mirror], mirror).unwrap();

    let [arc_copy, circle_copy] = copies.as_slice() else {
        panic!("two copies");
    };
    let original = sketch.arc(arc).unwrap();
    let image = sketch.arc(*arc_copy).unwrap();
    assert_near(image.center, Point2::new(0.0, 20.0));
    assert!((image.sweep - original.sweep).abs() < EXACT);
    assert_near(image.point_at(image.start_angle + image.sweep / 2.0), {
        let middle = original.point_at(original.start_angle + original.sweep / 2.0);
        Point2::new(middle.y, middle.x)
    });
    assert_near(
        sketch.circle(*circle_copy).unwrap().0,
        Point2::new(-5.0, 10.0),
    );
    assert!(
        sketch
            .constraints()
            .any(|(_, constraint)| *constraint == Constraint::Equal(circle, *circle_copy))
    );
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);

    sketch.set_dimension(radius, mm(4.0)).unwrap();
    assert!((solve(&sketch).geometry.circle(*circle_copy).unwrap().1 - 4.0).abs() < EXACT);
}

#[test]
fn a_point_merely_lying_on_the_mirror_line_is_shared_and_held_on_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(10.0, 5.0));
    let (start, _) = ends(&sketch, line);

    let copies = sketch.mirror(&[line], EntityId::VERTICAL_AXIS).unwrap();

    let (copy_start, copy_end) = ends(&sketch, copies[0]);
    assert_eq!(copy_start, start);
    assert_near(sketch.point(copy_end).unwrap(), Point2::new(-10.0, 5.0));
    assert!(sketch.constraints().any(
        |(_, constraint)| *constraint == Constraint::Coincident(start, EntityId::VERTICAL_AXIS)
    ));
    assert_clean(&solve(&sketch));
}

#[test]
fn a_mirror_image_is_previewed_without_changing_the_sketch() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(1.0, 1.0), Point2::new(4.0, 2.0));
    let lone = sketch.add_point(Point2::new(3.0, 7.0));
    let before = sketch.clone();

    let image = sketch
        .mirror_image(
            &[line, lone],
            EntityId::HORIZONTAL_AXIS,
            Faceting::within(0.01),
        )
        .unwrap();

    assert_eq!(sketch, before);
    assert_eq!(image.curves.len(), 1);
    assert_near(image.curves[0][0], Point2::new(1.0, -1.0));
    assert_near(image.points[0], Point2::new(3.0, -7.0));
    let copies = sketch
        .mirror(&[line, lone], EntityId::HORIZONTAL_AXIS)
        .unwrap();
    assert_eq!(copies.len(), 1);
    assert_eq!(count_of(&sketch, "Symmetric"), 3);
}

#[test]
fn mirroring_needs_geometry_off_a_line_or_axis() {
    let mut sketch = Sketch::new(Plane::XY);
    let axis_line = sketch.add_line(Point2::new(0.0, -5.0), Point2::new(0.0, 5.0));
    let across = sketch.add_line(Point2::new(-5.0, 0.0), Point2::new(5.0, 0.0));
    let circle = sketch.add_circle(Point2::new(20.0, 0.0), 2.0);
    let label = |id: EntityId| sketch.entity_label(id);

    assert_eq!(
        sketch.mirror_image(&[], axis_line, Faceting::within(0.1)),
        Err(MirrorError::NothingSelected)
    );
    assert_eq!(
        sketch
            .mirror_image(&[across], circle, Faceting::within(0.1))
            .unwrap_err()
            .to_string(),
        format!(
            "{} is not a line or an axis, so nothing can be mirrored about it",
            label(circle)
        )
    );
    assert_eq!(
        sketch.mirror_image(&[across], axis_line, Faceting::within(0.1)),
        Err(MirrorError::NothingToMirror {
            entity: axis_line,
            label: label(axis_line),
        })
    );
    assert_eq!(
        sketch.mirror_image(&[axis_line], axis_line, Faceting::within(0.1)),
        Err(MirrorError::NothingSelected)
    );
    let before = sketch.clone();
    assert!(sketch.mirror(&[across], EntityId::ORIGIN).is_err());
    assert_eq!(sketch, before);
}

#[test]
fn a_mirrored_ellipse_keeps_its_minor_radius_by_equal_and_an_arc_by_its_ends() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::new(20.0, 5.0), Point2::new(28.0, 9.0), 3.0);
    let arc = sketch.add_elliptical_arc(
        Point2::new(20.0, 30.0),
        Point2::new(28.0, 30.0),
        3.0,
        Point2::new(20.0 + 8.0 * 0.5_f64.cos(), 30.0 + 3.0 * 0.5_f64.sin()),
        Point2::new(20.0, 33.0),
    );
    let free = solve(&sketch).solution.degrees_of_freedom();

    let copies = sketch
        .mirror(&[ellipse, arc], EntityId::VERTICAL_AXIS)
        .unwrap();
    let solved = solve(&sketch);

    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), free);
    assert_eq!(count_of(&sketch, "Equal"), 1);
    let ellipse_copy = solved.geometry.ellipse(copies[0]).unwrap();
    assert_near(ellipse_copy.center, Point2::new(-20.0, 5.0));
    assert_near(
        ellipse_copy.center + ellipse_copy.major,
        Point2::new(-28.0, 9.0),
    );
    let arc_copy = solved.geometry.ellipse(copies[1]).unwrap();
    assert_near(arc_copy.point_at(arc_copy.start), Point2::new(-20.0, 33.0));
    assert_near(
        arc_copy.point_at(arc_copy.end()),
        Point2::new(-20.0 - 8.0 * 0.5_f64.cos(), 30.0 + 3.0 * 0.5_f64.sin()),
    );
    assert!((arc_copy.sweep - std::f64::consts::FRAC_PI_2 + 0.5).abs() < EXACT);
}

#[test]
fn an_ellipse_on_the_mirror_line_is_its_own_image() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::new(0.0, 5.0), Point2::new(0.0, 12.0), 3.0);

    assert!(matches!(
        sketch.mirror(&[ellipse], EntityId::VERTICAL_AXIS),
        Err(MirrorError::NothingToMirror { .. })
    ));
}

#[test]
fn mirrored_arcs_and_elliptical_arcs_add_no_redundancy_and_no_freedom() {
    let mut sketch = Sketch::new(Plane::XY);
    let mirror = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(10.0, 4.0));
    sketch.set_construction(mirror, true).unwrap();
    let arc = sketch.add_arc(
        Point2::new(20.0, 5.0),
        Point2::new(23.0, 9.0),
        Point2::new(16.0, 8.0),
    );
    let slanted = sketch.add_elliptical_arc(
        Point2::new(20.0, 30.0),
        Point2::new(28.0, 30.0),
        3.0,
        Point2::new(20.0 + 8.0 * 0.5_f64.cos(), 30.0 + 3.0 * 0.5_f64.sin()),
        Point2::new(20.0 + 8.0 * 2.0_f64.cos(), 30.0 + 3.0 * 2.0_f64.sin()),
    );
    let half = sketch.add_elliptical_arc(
        Point2::new(20.0, 50.0),
        Point2::new(28.0, 50.0),
        3.0,
        Point2::new(28.0, 50.0),
        Point2::new(12.0, 50.0),
    );
    let free = solve(&sketch).solution.degrees_of_freedom();

    sketch.mirror(&[arc, slanted, half], mirror).unwrap();
    let solved = solve(&sketch);

    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), free);
}

#[test]
fn a_curve_mirrored_after_its_neighbour_joins_the_image_already_there() {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(20.0, 10.0));
    let (_, corner) = ends(&sketch, first);
    let top = sketch.add_point(Point2::new(20.0, 30.0));
    let second = sketch.insert(Entity::Line {
        start: corner,
        end: top,
    });

    let first_copies = sketch.mirror(&[first], EntityId::VERTICAL_AXIS).unwrap();
    let second_copies = sketch.mirror(&[second], EntityId::VERTICAL_AXIS).unwrap();

    let (_, image_corner) = ends(&sketch, first_copies[0]);
    let (image_start, image_top) = ends(&sketch, second_copies[0]);
    assert_eq!(image_start, image_corner);
    assert_near(sketch.point(image_top).unwrap(), Point2::new(-20.0, 30.0));
    assert_eq!(count_of(&sketch, "Symmetric"), 2);
    assert_clean(&solve(&sketch));
}

#[test]
fn a_curve_joined_by_coincident_to_a_mirrored_point_takes_its_image() {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(20.0, 10.0));
    let second = sketch.add_line(Point2::new(20.0, 10.0), Point2::new(20.0, 30.0));
    let (_, corner) = ends(&sketch, first);
    let (joined, _) = ends(&sketch, second);
    sketch
        .add_constraint(Constraint::Coincident(corner, joined))
        .unwrap();

    let first_copies = sketch.mirror(&[first], EntityId::VERTICAL_AXIS).unwrap();
    let second_copies = sketch.mirror(&[second], EntityId::VERTICAL_AXIS).unwrap();

    let (_, image_corner) = ends(&sketch, first_copies[0]);
    let (image_start, _) = ends(&sketch, second_copies[0]);
    assert_eq!(image_start, image_corner);
    assert_eq!(count_of(&sketch, "Symmetric"), 2);
    assert_clean(&solve(&sketch));
}
