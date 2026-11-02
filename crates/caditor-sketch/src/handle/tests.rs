use caditor_expression::{EvalError, ParameterId, Quantity};
use caditor_geometry::{Plane, Point2};

use super::HandledLayout;
use crate::{Constraint, Entity, EntityId, FitSpacing, Sketch, SketchError, Solved, SplineKind};

const EXACT: f64 = 1e-7;

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn solve(sketch: &Sketch) -> Solved {
    sketch.solve(&no_parameters, &|| false).unwrap()
}

fn wave() -> Vec<Point2> {
    vec![
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 12.0),
        Point2::new(25.0, -4.0),
        Point2::new(40.0, 9.0),
        Point2::new(52.0, 0.0),
    ]
}

fn fit_points(sketch: &Sketch, spline: EntityId) -> Vec<EntityId> {
    match sketch.entity(spline) {
        Some(Entity::Spline { points, .. }) => points.clone(),
        _ => Vec::new(),
    }
}

fn holds(sketch: &Sketch, spline: EntityId) {
    let points = fit_points(sketch, spline);
    let positions: Vec<Point2> = points
        .iter()
        .map(|point| sketch.point(*point).unwrap())
        .collect();
    let Some(&Entity::Spline {
        kind: SplineKind::Fit { closed, spacing },
        ..
    }) = sketch.entity(spline)
    else {
        panic!("a fit spline");
    };
    let curve = sketch.spline(spline).unwrap();
    let layout = HandledLayout::of(&positions, closed, spacing, &[]).unwrap();
    for (index, position) in positions.iter().enumerate() {
        let parameter = layout.parameter(index).unwrap();
        assert!(curve.point_at(parameter).distance(*position) < EXACT);
    }
    for handle in sketch.spline_handles(spline) {
        let parameter = layout.parameter(handle.index).unwrap();
        let [slope, _] = curve.derivatives(parameter);
        let reach = sketch.point(handle.end).unwrap() - sketch.point(handle.anchor).unwrap();
        assert!((slope * layout.arm(handle.index) - reach).length() < EXACT);
    }
}

fn move_point(sketch: &mut Sketch, point: EntityId, to: Point2) {
    sketch.replace_entity(point, Entity::Point(to)).unwrap();
}

#[test]
fn a_handle_starts_along_the_curve_and_holds_its_direction_and_pull() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline_of(&wave(), SplineKind::fit(false));
    let middle = fit_points(&sketch, spline)[2];
    let before = solve(&sketch);
    let (line, _) = sketch.add_spline_handle(spline, middle).unwrap();
    let Some(&Entity::Line { end, .. }) = sketch.entity(line) else {
        panic!("the handle is a line");
    };

    assert!(sketch.is_construction(line));
    holds(&sketch, spline);

    let after = solve(&sketch);

    assert_eq!(
        after.solution.degrees_of_freedom(),
        before.solution.degrees_of_freedom() + 2
    );
    assert!(after.solution.redundancies().is_empty());

    move_point(&mut sketch, end, Point2::new(25.0, 10.0));
    let turned = solve(&sketch);

    assert!(
        turned
            .geometry
            .point(end)
            .unwrap()
            .distance(Point2::new(25.0, 10.0))
            < EXACT
    );
    holds(&turned.geometry, spline);
}

#[test]
fn a_handle_line_takes_constraints_that_turn_the_curve() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline_of(&wave(), SplineKind::fit(false));
    let first = fit_points(&sketch, spline)[0];
    let (line, _) = sketch.add_spline_handle(spline, first).unwrap();
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    let solved = solve(&sketch);
    let curve = solved.geometry.spline(spline).unwrap();
    let [slope, _] = curve.derivatives(0.0);

    assert!(slope.y.abs() < EXACT * slope.length());
    assert!(slope.x.abs() > 0.0);
    holds(&solved.geometry, spline);
}

#[test]
fn both_handles_on_two_points_make_the_bezier_leaving_along_them() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline_of(
        &[Point2::new(0.0, 0.0), Point2::new(30.0, 0.0)],
        SplineKind::fit(false),
    );
    let [start, finish] = fit_points(&sketch, spline)[..] else {
        panic!("two points");
    };
    let (first_line, _) = sketch.add_spline_handle(spline, start).unwrap();
    let (last_line, _) = sketch.add_spline_handle(spline, finish).unwrap();
    let ends = [first_line, last_line].map(|line| match sketch.entity(line) {
        Some(&Entity::Line { end, .. }) => end,
        _ => panic!("a line"),
    });
    move_point(&mut sketch, ends[0], Point2::new(5.0, 15.0));
    move_point(&mut sketch, ends[1], Point2::new(35.0, 15.0));
    let solved = solve(&sketch);
    let curve = solved.geometry.spline(spline).unwrap();
    let expected = [
        Point2::new(0.0, 0.0),
        Point2::new(5.0, 15.0),
        Point2::new(25.0, -15.0),
        Point2::new(30.0, 0.0),
    ];

    assert_eq!(curve.control_points().len(), 4);
    for (control, expected) in curve.control_points().iter().zip(expected) {
        assert!(control.distance(expected) < EXACT);
    }
}

#[test]
fn a_second_handle_at_one_point_and_other_splines_are_refused() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline_of(&wave(), SplineKind::fit(false));
    let middle = fit_points(&sketch, spline)[2];
    sketch.add_spline_handle(spline, middle).unwrap();
    let control = sketch.add_spline_of(&wave(), SplineKind::OPEN);
    let stray = sketch.add_point(Point2::new(5.0, 5.0));

    assert!(matches!(
        sketch.add_spline_handle(spline, middle),
        Err(SketchError::AlreadyHandled { .. })
    ));
    assert!(matches!(
        sketch.add_spline_handle(spline, stray),
        Err(SketchError::NotAFitPoint { .. })
    ));
    let point = fit_points(&sketch, control)[1];

    assert!(matches!(
        sketch.add_spline_handle(control, point),
        Err(SketchError::NotAFitPoint { .. })
    ));
    let loose = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(1.0, 1.0));

    assert!(matches!(
        sketch.add_constraint(Constraint::SplineHandle {
            spline,
            line: loose
        }),
        Err(SketchError::NotJoined { .. })
    ));
}

#[test]
fn an_inactive_handle_leaves_the_plain_interpolation() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline_of(&wave(), SplineKind::fit(false));
    let plain = sketch.spline(spline).unwrap();
    let middle = fit_points(&sketch, spline)[2];
    let (line, handle) = sketch.add_spline_handle(spline, middle).unwrap();
    let Some(&Entity::Line { end, .. }) = sketch.entity(line) else {
        panic!("a line");
    };
    move_point(&mut sketch, end, Point2::new(25.0, 20.0));
    sketch.set_active(handle, false).unwrap();

    assert_eq!(sketch.spline(spline).unwrap(), plain);
    assert!(sketch.spline_handles(spline).is_empty());
    assert!(solve(&sketch).solution.redundancies().is_empty());
}

#[test]
fn a_mirrored_spline_takes_its_handle_with_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let shifted: Vec<Point2> = wave()
        .iter()
        .map(|point| *point + Point2::new(10.0, 0.0))
        .collect();
    let spline = sketch.add_spline_of(&shifted, SplineKind::fit(false));
    let middle = fit_points(&sketch, spline)[2];
    let (line, _) = sketch.add_spline_handle(spline, middle).unwrap();
    let Some(&Entity::Line { end, .. }) = sketch.entity(line) else {
        panic!("a line");
    };
    move_point(&mut sketch, end, Point2::new(40.0, 12.0));
    let copies = sketch
        .mirror(&[spline, line], EntityId::VERTICAL_AXIS)
        .unwrap();
    let copy = copies
        .iter()
        .copied()
        .find(|copy| matches!(sketch.entity(*copy), Some(Entity::Spline { .. })))
        .unwrap();

    assert_eq!(sketch.spline_handles(copy).len(), 1);

    let solved = solve(&sketch);
    let original = solved.geometry.spline(spline).unwrap();
    let image = solved.geometry.spline(copy).unwrap();
    for step in 0..=20 {
        let point = original.point_at(f64::from(step) / 20.0);
        let mirrored = Point2::new(-point.x, point.y);
        let nearest = (0..=400)
            .map(|sample| image.point_at(f64::from(sample) / 400.0).distance(mirrored))
            .fold(f64::INFINITY, f64::min);

        assert!(nearest < 0.05);
    }
}

fn ring() -> Vec<Point2> {
    vec![
        Point2::new(0.0, 0.0),
        Point2::new(30.0, -5.0),
        Point2::new(45.0, 15.0),
        Point2::new(25.0, 35.0),
        Point2::new(-5.0, 20.0),
    ]
}

fn handle_end(sketch: &Sketch, line: EntityId) -> EntityId {
    match sketch.entity(line) {
        Some(&Entity::Line { end, .. }) => end,
        _ => panic!("the handle is a line"),
    }
}

fn smooth_all_round(sketch: &Sketch, spline: EntityId) {
    let curve = sketch.spline(spline).unwrap();
    let [start_slope, start_bend] = curve.derivatives(0.0);
    let [end_slope, end_bend] = curve.derivatives(1.0);

    assert!(curve.point_at(0.0).distance(curve.point_at(1.0)) < EXACT);
    assert!((start_slope - end_slope).length() < EXACT * start_slope.length().max(1.0));
    assert!((start_bend - end_bend).length() < 1e-5 * start_bend.length().max(1.0));
    for knot in curve
        .knots()
        .iter()
        .copied()
        .filter(|knot| *knot > 0.0 && *knot < 1.0)
    {
        let [before, bend_before] = curve.derivatives(knot - 1e-9);
        let [after, bend_after] = curve.derivatives(knot + 1e-9);

        assert!((before - after).length() < 1e-5 * before.length().max(1.0));
        assert!((bend_before - bend_after).length() < 1e-3 * bend_before.length().max(1.0));
    }
}

#[test]
fn a_closed_spline_handle_starts_along_the_curve_and_holds_its_direction_and_pull() {
    for spacing in [FitSpacing::Centripetal, FitSpacing::Even] {
        let mut sketch = Sketch::new(Plane::XY);
        let spline = sketch.add_spline_of(
            &ring(),
            SplineKind::Fit {
                closed: true,
                spacing,
            },
        );
        let plain = sketch.spline(spline).unwrap();
        let third = fit_points(&sketch, spline)[2];
        let before = solve(&sketch);
        let (line, _) = sketch.add_spline_handle(spline, third).unwrap();
        let end = handle_end(&sketch, line);
        let handled = sketch.spline(spline).unwrap();

        holds(&sketch, spline);
        smooth_all_round(&sketch, spline);
        for step in 0..=40 {
            let at = f64::from(step) / 40.0;

            assert!(handled.point_at(at).distance(plain.point_at(at)) < 2.0);
        }

        let after = solve(&sketch);

        assert_eq!(
            after.solution.degrees_of_freedom(),
            before.solution.degrees_of_freedom() + 2
        );
        assert!(after.solution.redundancies().is_empty());

        move_point(&mut sketch, end, Point2::new(55.0, 30.0));
        let turned = solve(&sketch);

        assert!(
            turned
                .geometry
                .point(end)
                .unwrap()
                .distance(Point2::new(55.0, 30.0))
                < EXACT
        );
        holds(&turned.geometry, spline);
        smooth_all_round(&turned.geometry, spline);
    }
}

#[test]
fn a_handle_at_a_closed_spline_seam_turns_both_ends_of_the_loop() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline_of(&ring(), SplineKind::fit(true));
    let [first, second, ..] = fit_points(&sketch, spline)[..] else {
        panic!("five points");
    };
    let (line, _) = sketch.add_spline_handle(spline, first).unwrap();
    let (other, _) = sketch.add_spline_handle(spline, second).unwrap();
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    sketch.add_constraint(Constraint::Vertical(other)).unwrap();
    let solved = solve(&sketch);
    let curve = solved.geometry.spline(spline).unwrap();
    let [slope, _] = curve.derivatives(0.0);

    assert!(solved.solution.redundancies().is_empty());
    assert!(slope.y.abs() < EXACT * slope.length());
    assert!(slope.x.abs() > 0.0);
    holds(&solved.geometry, spline);
    smooth_all_round(&solved.geometry, spline);
}

#[test]
fn a_closed_spline_with_a_handle_on_every_point_still_solves() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline_of(&ring()[..3], SplineKind::fit(true));
    for point in fit_points(&sketch, spline) {
        sketch.add_spline_handle(spline, point).unwrap();
    }
    let solved = solve(&sketch);

    assert_eq!(solved.geometry.spline_handles(spline).len(), 3);
    assert!(solved.solution.redundancies().is_empty());
    holds(&solved.geometry, spline);
    smooth_all_round(&solved.geometry, spline);
}

#[test]
fn a_mirrored_closed_spline_takes_its_handle_with_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let shifted: Vec<Point2> = ring()
        .iter()
        .map(|point| *point + Point2::new(20.0, 0.0))
        .collect();
    let spline = sketch.add_spline_of(&shifted, SplineKind::fit(true));
    let third = fit_points(&sketch, spline)[2];
    let (line, _) = sketch.add_spline_handle(spline, third).unwrap();
    let end = handle_end(&sketch, line);
    move_point(&mut sketch, end, Point2::new(75.0, 30.0));
    let copies = sketch
        .mirror(&[spline, line], EntityId::VERTICAL_AXIS)
        .unwrap();
    let copy = copies
        .iter()
        .copied()
        .find(|copy| matches!(sketch.entity(*copy), Some(Entity::Spline { .. })))
        .unwrap();

    assert_eq!(sketch.spline_handles(copy).len(), 1);

    let solved = solve(&sketch);
    let original = solved.geometry.spline(spline).unwrap();
    let image = solved.geometry.spline(copy).unwrap();
    for step in 0..=20 {
        let point = original.point_at(f64::from(step) / 20.0);
        let mirrored = Point2::new(-point.x, point.y);
        let nearest = (0..=800)
            .map(|sample| image.point_at(f64::from(sample) / 800.0).distance(mirrored))
            .fold(f64::INFINITY, f64::min);

        assert!(nearest < 0.05);
    }
}
