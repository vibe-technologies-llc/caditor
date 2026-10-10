use std::f64::consts::TAU;

use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2};

use crate::{
    BSpline, Constraint, Drag, Entity, EntityId, EntityState, FitSpacing, Sketch, SketchError,
    Solved, SplineKind,
};

const EXACT: f64 = 1e-7;

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn solve(sketch: &Sketch) -> Result<Solved, SketchError> {
    sketch.solve(&no_parameters, &|| false)
}

fn points_of(sketch: &Sketch, spline: EntityId) -> Vec<EntityId> {
    match sketch.entity(spline) {
        Some(Entity::Spline { points, .. }) => points.clone(),
        other => panic!("expected a spline, found {other:?}"),
    }
}

fn fit_points() -> Vec<Point2> {
    vec![
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 6.0),
        Point2::new(25.0, -3.0),
        Point2::new(32.0, 8.0),
        Point2::new(45.0, 2.0),
    ]
}

fn passes_through(sketch: &Sketch, spline: EntityId) -> bool {
    points_of(sketch, spline).iter().all(|point| {
        let at = sketch.point(*point).unwrap();
        sketch.closest_on_curve(spline, at).unwrap().distance(at) < 1e-6
    })
}

#[test]
fn a_fit_point_spline_passes_its_points_and_frees_two_per_point() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline_of(&fit_points(), SplineKind::fit(false));

    let solved = solve(&sketch).unwrap();

    assert_eq!(solved.solution.degrees_of_freedom(), 10);
    assert!(passes_through(&solved.geometry, spline));
    for (point, expected) in points_of(&sketch, spline).iter().zip(fit_points()) {
        assert!(solved.geometry.point(*point).unwrap().distance(expected) < EXACT);
    }
}

#[test]
fn fixing_every_fit_point_constrains_the_spline_fully() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline_of(&fit_points(), SplineKind::fit(true));
    for point in points_of(&sketch, spline) {
        let at = sketch.point(point).unwrap();
        sketch
            .add_constraint(Constraint::Fix { point, at })
            .unwrap();
    }

    let solved = solve(&sketch).unwrap();

    assert_eq!(solved.solution.degrees_of_freedom(), 0);
    assert_eq!(
        solved.solution.entity_state(spline),
        Some(EntityState::FullyConstrained)
    );
    assert!(passes_through(&solved.geometry, spline));
}

#[test]
fn dragging_a_fit_point_keeps_the_curve_through_all_of_them() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline_of(&fit_points(), SplineKind::fit(false));
    let points = points_of(&sketch, spline);
    let to = Point2::new(24.0, 12.0);

    let solved = sketch
        .solve_dragging(
            &no_parameters,
            &|| false,
            &[Drag::Point {
                point: points[2],
                to,
            }],
        )
        .unwrap();

    assert!(solved.geometry.point(points[2]).unwrap().distance(to) < 1e-6);
    assert!(passes_through(&solved.geometry, spline));
}

#[test]
fn a_point_dimensioned_on_a_closed_fit_spline_holds() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline_of(&fit_points(), SplineKind::fit(true));
    let points = points_of(&sketch, spline);
    sketch
        .add_constraint(Constraint::Distance {
            from: points[0],
            to: points[3],
            value: Expression::Measure(40.0, Unit::Millimetre),
        })
        .unwrap();

    let solved = solve(&sketch).unwrap();
    let (first, fourth) = (
        solved.geometry.point(points[0]).unwrap(),
        solved.geometry.point(points[3]).unwrap(),
    );

    assert!((first.distance(fourth) - 40.0).abs() < EXACT);
    assert!(passes_through(&solved.geometry, spline));
    assert_eq!(solved.solution.degrees_of_freedom(), 9);
}

#[test]
fn a_closed_control_spline_closes_smoothly() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline_of(&fit_points(), SplineKind::Control { closed: true });

    let solved = solve(&sketch).unwrap();
    let curve = solved.geometry.spline(spline).unwrap();
    let [start_tangent, _] = curve.derivatives(0.0);
    let [end_tangent, _] = curve.derivatives(1.0);

    assert_eq!(solved.solution.degrees_of_freedom(), 10);
    assert!(curve.point_at(0.0).distance(curve.point_at(1.0)) < EXACT);
    assert!(start_tangent.distance(end_tangent) < 1e-6 * start_tangent.length());
    assert!(
        solved
            .geometry
            .entity(spline)
            .unwrap()
            .spline_ends()
            .is_none()
    );
}

#[test]
fn a_line_tangent_to_a_conic_end_follows_its_apex() {
    let mut sketch = Sketch::new(Plane::XY);
    let conic = sketch.add_spline_of(
        &[
            Point2::new(0.0, 0.0),
            Point2::new(10.0, 10.0),
            Point2::new(20.0, 0.0),
        ],
        SplineKind::Conic { rho: 0.7 },
    );
    let [start, apex, _] = points_of(&sketch, conic)[..] else {
        panic!("a conic has three points");
    };
    let line = sketch.add_line(Point2::new(-10.0, -8.0), Point2::new(0.0, 0.0));
    let Some(&Entity::Line { start: away, end }) = sketch.entity(line) else {
        panic!("a line has two ends");
    };
    sketch
        .add_constraint(Constraint::Coincident(end, start))
        .unwrap();
    sketch
        .add_constraint(Constraint::Tangent(line, conic))
        .unwrap();

    let solved = solve(&sketch).unwrap();
    let geometry = &solved.geometry;
    let leg = geometry.point(apex).unwrap() - geometry.point(start).unwrap();
    let along = geometry.point(start).unwrap() - geometry.point(away).unwrap();

    assert!(leg.perp_dot(along).abs() < 1e-6 * leg.length() * along.length());
}

#[test]
fn a_conic_passes_its_shoulder_at_rho_of_the_way_to_its_apex() {
    let (start, apex, end) = (
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 0.0),
    );
    for rho in [0.2, 0.5, 0.8] {
        let mut sketch = Sketch::new(Plane::XY);
        let conic = sketch.add_spline_of(&[start, apex, end], SplineKind::Conic { rho });
        let curve = sketch.spline(conic).unwrap();
        let middle = start.lerp(end, 0.5);

        assert!(curve.point_at(0.5).distance(middle.lerp(apex, rho)) < EXACT);
        assert!(curve.point_at(0.0).distance(start) < EXACT);
        assert!(curve.point_at(1.0).distance(end) < EXACT);
    }
    let mut sketch = Sketch::new(Plane::XY);
    let start = sketch.add_point(start);
    let apex = sketch.add_point(apex);
    let end = sketch.add_point(end);

    assert_eq!(
        sketch.insert_entity(
            EntityId::from_raw(100),
            Entity::Spline {
                points: vec![start, apex, end],
                kind: SplineKind::Conic { rho: 1.5 },
            },
        ),
        Err(SketchError::InvalidRho)
    );
    assert_eq!(
        sketch.insert_entity(
            EntityId::from_raw(100),
            Entity::Spline {
                points: vec![start, end],
                kind: SplineKind::Control { closed: true },
            },
        ),
        Err(SketchError::TooFewClosedPoints)
    );
}

#[test]
fn a_rho_dimension_drives_the_conic_without_taking_a_degree_of_freedom() {
    let (start, apex, end) = (
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 0.0),
    );
    let mut sketch = Sketch::new(Plane::XY);
    let conic = sketch.add_spline_of(&[start, apex, end], SplineKind::Conic { rho: 0.7 });
    let rho = |value: f64| Constraint::Rho {
        conic,
        value: Expression::Number(value),
    };

    assert!(matches!(
        sketch.add_constraint(rho(1.5)),
        Err(SketchError::DimensionValue { .. })
    ));
    sketch.add_constraint(rho(0.3)).unwrap();
    let solved = solve(&sketch).unwrap();
    let shoulder = start.lerp(end, 0.5).lerp(apex, 0.3);

    assert_eq!(solved.solution.degrees_of_freedom(), 6);
    assert!(
        solved
            .geometry
            .spline(conic)
            .unwrap()
            .point_at(0.5)
            .distance(shoulder)
            < EXACT
    );
    assert_eq!(solved.geometry.measured(&rho(0.3)), Some(0.3));
}

fn uneven_points() -> Vec<Point2> {
    vec![
        Point2::new(0.0, 0.0),
        Point2::new(2.0, 3.0),
        Point2::new(4.0, 0.0),
        Point2::new(40.0, 6.0),
        Point2::new(43.0, 2.0),
    ]
}

#[test]
fn an_evenly_spaced_fit_spline_keeps_the_shape_it_was_stored_with() {
    let mut sketch = Sketch::new(Plane::XY);
    let open = sketch.add_spline_of(
        &uneven_points(),
        SplineKind::Fit {
            closed: false,
            spacing: FitSpacing::Even,
        },
    );
    let closed = sketch.add_spline_of(
        &uneven_points(),
        SplineKind::Fit {
            closed: true,
            spacing: FitSpacing::Even,
        },
    );

    let solved = solve(&sketch).unwrap();

    assert_eq!(
        solved.geometry.spline(open),
        BSpline::interpolate(&uneven_points())
    );
    assert_eq!(
        solved.geometry.spline(closed),
        BSpline::interpolate_closed(&uneven_points())
    );
    assert_ne!(
        sketch.spline(open),
        SplineKind::fit(false).curve(&uneven_points())
    );
}

#[test]
fn a_point_on_a_centripetal_spline_stays_on_the_curve_as_its_fit_points_move() {
    for closed in [false, true] {
        let mut sketch = Sketch::new(Plane::XY);
        let spline = sketch.add_spline_of(&uneven_points(), SplineKind::fit(closed));
        let points = points_of(&sketch, spline);
        let on = sketch.spline(spline).unwrap().point_at(0.3);
        let point = sketch.add_point(on);
        sketch
            .add_constraint(Constraint::Coincident(point, spline))
            .unwrap();
        sketch
            .add_constraint(Constraint::Distance {
                from: points[0],
                to: points[4],
                value: Expression::Measure(60.0, Unit::Millimetre),
            })
            .unwrap();

        let solved = solve(&sketch).unwrap();
        let geometry = &solved.geometry;
        let at = geometry.point(point).unwrap();
        let (first, last) = (
            geometry.point(points[0]).unwrap(),
            geometry.point(points[4]).unwrap(),
        );

        assert!(
            (first.distance(last) - 60.0).abs() < EXACT,
            "{closed} {}",
            first.distance(last)
        );
        assert!(
            geometry.closest_on_curve(spline, at).unwrap().distance(at) < 1e-7,
            "{closed} {}",
            geometry.closest_on_curve(spline, at).unwrap().distance(at)
        );
        assert!(passes_through(geometry, spline), "{closed}");
    }
}

#[test]
fn a_point_held_on_a_closed_spline_moves_across_its_seam() {
    for kind in [SplineKind::Control { closed: true }, SplineKind::fit(true)] {
        let mut sketch = Sketch::new(Plane::XY);
        let ring: Vec<Point2> = (0..6)
            .map(|step| Point2::from_angle(f64::from(step) * TAU / 6.0) * 20.0)
            .collect();
        let spline = sketch.add_spline_of(&ring, kind);
        for point in points_of(&sketch, spline) {
            let at = sketch.point(point).unwrap();
            sketch
                .add_constraint(Constraint::Fix { point, at })
                .unwrap();
        }
        let curve = sketch.spline(spline).unwrap();
        let point = sketch.add_point(curve.point_at(0.97));
        sketch
            .add_constraint(Constraint::Coincident(point, spline))
            .unwrap();
        let to = curve.point_at(0.03);

        let solved = sketch
            .solve_dragging(&no_parameters, &|| false, &[Drag::Point { point, to }])
            .unwrap();

        assert!(
            solved.geometry.point(point).unwrap().distance(to) < 1e-6,
            "{kind:?}"
        );
    }
}
