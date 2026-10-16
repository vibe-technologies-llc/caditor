use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2};

use crate::{
    BSpline, Constraint, Entity, EntityId, InsertError, Sketch, SketchError, Solved, SplineKind,
    intersect::closest_spline_parameter,
};

const EXACT: f64 = 1e-9;
const SAMPLES: usize = 128;

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn solve(sketch: &Sketch) -> Solved {
    sketch.solve(&no_parameters, &|| false).unwrap()
}

fn long_wave() -> Vec<Point2> {
    vec![
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 15.0),
        Point2::new(20.0, -10.0),
        Point2::new(30.0, 12.0),
        Point2::new(40.0, -14.0),
        Point2::new(50.0, 8.0),
        Point2::new(60.0, 0.0),
    ]
}

fn bezier() -> Vec<Point2> {
    vec![
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 20.0),
        Point2::new(30.0, -20.0),
        Point2::new(40.0, 0.0),
    ]
}

fn points_of(sketch: &Sketch, spline: EntityId) -> Vec<EntityId> {
    match sketch.entity(spline) {
        Some(Entity::Spline { points, .. }) => points.clone(),
        other => panic!("expected a spline, found {other:?}"),
    }
}

fn kind_of(sketch: &Sketch, spline: EntityId) -> SplineKind {
    sketch.entity(spline).and_then(Entity::spline_kind).unwrap()
}

fn fixed(sketch: &Sketch, point: EntityId) -> Constraint {
    Constraint::Fix {
        point,
        at: sketch.point(point).unwrap(),
    }
}

fn assert_same_curve(before: &BSpline, after: &BSpline) {
    for step in 0..=SAMPLES {
        let parameter = step as f64 / SAMPLES as f64;
        let moved = before
            .point_at(parameter)
            .distance(after.point_at(parameter));
        assert!(moved < EXACT, "the curve moved by {moved} at {parameter}");
    }
}

fn assert_solves_in_place(sketch: &Sketch) {
    let solved = solve(sketch);
    for (id, entity) in sketch.entities() {
        if let Entity::Point(position) = entity {
            let moved = solved.geometry.point(id).unwrap().distance(*position);
            assert!(moved < 1e-7, "point {id:?} moved by {moved}");
        }
    }
    assert!(solved.solution.redundancies().is_empty());
}

#[test]
fn a_point_added_to_a_spline_keeps_its_curve_exactly_wherever_it_is_added() {
    for (points, at) in [
        (bezier(), 0.5),
        (bezier(), 0.1),
        (long_wave(), 0.1),
        (long_wave(), 0.3),
        (long_wave(), 0.5),
        (long_wave(), 0.74),
        (long_wave(), 0.95),
    ] {
        let mut sketch = Sketch::new(Plane::XY);
        let spline = sketch.add_spline(&points);
        let before = sketch.spline(spline).unwrap();
        let near = before.point_at(at);

        let added = sketch.insert_control_point(spline, Some(near)).unwrap();

        let after = sketch.spline(spline).unwrap();
        assert_eq!(points_of(&sketch, spline).len(), points.len() + 1);
        assert!(points_of(&sketch, spline).contains(&added));
        assert_same_curve(&before, &after);
        assert_solves_in_place(&sketch);
    }
}

#[test]
fn a_spline_given_no_place_gets_its_point_in_its_longest_stretch_and_keeps_its_curve() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&long_wave());
    let before = sketch.spline(spline).unwrap();

    sketch.insert_control_point(spline, None).unwrap();

    assert_eq!(points_of(&sketch, spline).len(), 8);
    assert_same_curve(&before, &sketch.spline(spline).unwrap());
}

#[test]
fn points_added_again_and_again_keep_the_curve_and_the_ends() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&long_wave());
    let before = sketch.spline(spline).unwrap();
    let ends = (
        points_of(&sketch, spline)[0],
        *points_of(&sketch, spline).last().unwrap(),
    );

    for step in 0..6 {
        let near = before.point_at(0.13 + 0.15 * step as f64);
        sketch.insert_control_point(spline, Some(near)).unwrap();
    }

    let points = points_of(&sketch, spline);
    assert_eq!(points.len(), 13);
    assert_eq!((points[0], *points.last().unwrap()), ends);
    assert!(matches!(
        kind_of(&sketch, spline),
        SplineKind::Knotted { .. }
    ));
    assert_same_curve(&before, &sketch.spline(spline).unwrap());
    assert_solves_in_place(&sketch);
}

#[test]
fn a_trimmed_piece_keeps_its_curve_when_a_point_is_added() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&long_wave());
    sketch.add_line(Point2::new(13.0, -40.0), Point2::new(13.0, 40.0));
    sketch.trim(spline, Point2::new(2.0, 0.0)).unwrap();
    let before = sketch.spline(spline).unwrap();

    sketch
        .insert_control_point(spline, Some(before.point_at(0.4)))
        .unwrap();
    let inserted = sketch.spline(spline).unwrap();
    sketch
        .insert_control_point(spline, Some(before.point_at(0.8)))
        .unwrap();

    assert_same_curve(&before, &inserted);
    assert_same_curve(&before, &sketch.spline(spline).unwrap());
}

#[test]
fn a_spline_with_an_added_point_splits_and_trims_into_exact_pieces() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&long_wave());
    let whole = sketch.spline(spline).unwrap();
    sketch
        .insert_control_point(spline, Some(whole.point_at(0.3)))
        .unwrap();
    let cut = sketch.add_point(whole.point_at(0.6));
    sketch
        .add_constraint(Constraint::Coincident(cut, spline))
        .unwrap();

    let piece = sketch.split_at(spline, cut).unwrap();

    for part in [spline, piece] {
        let part = sketch.spline(part).unwrap();
        for step in 0..=SAMPLES {
            let at = part.point_at(step as f64 / SAMPLES as f64);
            let off = whole
                .point_at(closest_spline_parameter(&whole, at))
                .distance(at);
            assert!(off < 1e-7, "{at} lies {off} off the original spline");
        }
    }
}

#[test]
fn a_point_that_would_move_a_constrained_control_point_is_refused_naming_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&long_wave());
    let held = points_of(&sketch, spline)[1];
    let constraint = fixed(&sketch, held);
    sketch.add_constraint(constraint).unwrap();
    let before = sketch.clone();
    let near = sketch.spline(spline).unwrap().point_at(0.05);

    let refused = sketch.insert_control_point(spline, Some(near));

    assert!(matches!(
        refused,
        Err(InsertError::HeldPoint { point, .. }) if point == held
    ));
    assert!(sketch.same_content(&before));
}

#[test]
fn a_spline_with_a_held_point_takes_its_point_in_a_stretch_that_leaves_it_alone() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&long_wave());
    let held: Vec<EntityId> = points_of(&sketch, spline)[1..4].to_vec();
    for point in &held {
        let constraint = fixed(&sketch, *point);
        sketch.add_constraint(constraint).unwrap();
    }
    let before = sketch.spline(spline).unwrap();

    sketch.insert_control_point(spline, None).unwrap();

    assert_same_curve(&before, &sketch.spline(spline).unwrap());
    let kept = points_of(&sketch, spline);
    assert!(held.iter().all(|point| kept.contains(point)));
    for point in &held {
        assert!(
            sketch
                .constraints()
                .any(|(_, constraint)| *constraint == fixed(&sketch, *point))
        );
    }
}

#[test]
fn constraints_on_the_spline_itself_survive_an_added_point() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&long_wave());
    let on = sketch.add_point(sketch.spline(spline).unwrap().point_at(0.5));
    let coincident = sketch
        .add_constraint(Constraint::Coincident(on, spline))
        .unwrap();
    let distance = sketch
        .add_constraint(Constraint::Distance {
            from: points_of(&sketch, spline)[0],
            to: on,
            value: Expression::Measure(
                sketch
                    .spline(spline)
                    .unwrap()
                    .point_at(0.0)
                    .distance(sketch.spline(spline).unwrap().point_at(0.5)),
                Unit::Millimetre,
            ),
        })
        .unwrap();

    sketch
        .insert_control_point(spline, Some(Point2::new(5.0, 7.0)))
        .unwrap();

    assert!(sketch.constraint(coincident).is_some());
    assert!(sketch.constraint(distance).is_some());
}

#[test]
fn splines_that_cannot_take_a_point_exactly_are_refused_in_words() {
    let mut sketch = Sketch::new(Plane::XY);
    let fit = sketch.add_spline_of(&long_wave(), SplineKind::fit(false));
    let closed = sketch.add_spline_of(&long_wave(), SplineKind::Control { closed: true });
    let conic = sketch.add_spline_of(
        &[
            Point2::new(0.0, 0.0),
            Point2::new(20.0, 30.0),
            Point2::new(40.0, 0.0),
        ],
        SplineKind::Conic { rho: 0.7 },
    );
    let quadratic = sketch.add_spline(&bezier()[..3]);
    let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    let before = sketch.clone();

    assert!(matches!(
        sketch.insert_control_point(fit, None),
        Err(InsertError::FitSpline { .. })
    ));
    assert!(matches!(
        sketch.insert_control_point(closed, None),
        Err(InsertError::ClosedSpline { .. })
    ));
    assert!(matches!(
        sketch.insert_control_point(conic, None),
        Err(InsertError::Conic { .. })
    ));
    assert!(matches!(
        sketch.insert_control_point(quadratic, None),
        Err(InsertError::TooFewPoints { points: 3, .. })
    ));
    assert!(matches!(
        sketch.insert_control_point(line, None),
        Err(InsertError::NotASpline { .. })
    ));
    assert!(sketch.same_content(&before));
}

fn assert_lies_on(piece: &BSpline, whole: &BSpline) {
    for step in 0..=SAMPLES {
        let at = piece.point_at(step as f64 / SAMPLES as f64);
        let off = whole
            .point_at(closest_spline_parameter(whole, at))
            .distance(at);
        assert!(off < 1e-7, "{at} lies {off} off the original spline");
    }
}

#[test]
fn a_spline_with_added_points_trims_and_extends_along_its_own_curve() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&long_wave());
    let whole = sketch.spline(spline).unwrap();
    for parameter in [0.2, 0.5, 0.8] {
        sketch
            .insert_control_point(spline, Some(whole.point_at(parameter)))
            .unwrap();
    }
    sketch.add_line(Point2::new(13.0, -40.0), Point2::new(13.0, 40.0));
    sketch.add_line(Point2::new(47.0, -40.0), Point2::new(47.0, 40.0));

    sketch.trim(spline, whole.point_at(0.02)).unwrap();
    sketch.trim(spline, whole.point_at(0.98)).unwrap();
    let wall = sketch.add_line(Point2::new(8.0, -40.0), Point2::new(8.0, 40.0));

    let trimmed = sketch.spline(spline).unwrap();
    assert!(matches!(
        kind_of(&sketch, spline),
        SplineKind::Knotted { .. }
    ));
    assert_lies_on(&trimmed, &whole);
    assert!((trimmed.point_at(0.0).x - 13.0).abs() < 1e-7);

    let extension = sketch.extend(spline, trimmed.point_at(0.01)).unwrap();

    assert_eq!(extension.target, wall);
    let extended = sketch.spline(spline).unwrap();
    assert_lies_on(&trimmed, &extended);
    assert!((extended.point_at(0.0).x - 8.0).abs() < 1e-7);
    assert_solves_in_place(&sketch);
}

#[test]
fn a_spline_with_spans_that_do_not_fit_its_points_is_refused_in_words() {
    let mut sketch = Sketch::new(Plane::XY);
    let points: Vec<EntityId> = (0..7)
        .map(|index| sketch.add_point(Point2::new(f64::from(index) * 10.0, 0.0)))
        .collect();
    let knotted = |spans: Vec<f64>| Entity::Spline {
        points: points.clone(),
        kind: SplineKind::Knotted { spans },
    };
    let id = EntityId::from_raw(100);

    assert_eq!(
        sketch.insert_entity(id, knotted(vec![1.0, 1.0, 1.0])),
        Err(SketchError::KnotSpanCount)
    );
    assert_eq!(
        sketch.insert_entity(id, knotted(vec![1.0, 0.0, 1.0, 1.0])),
        Err(SketchError::InvalidKnotSpans)
    );
    assert_eq!(
        sketch.insert_entity(id, knotted(vec![1.0])),
        Err(SketchError::InvalidKnotSpans)
    );
    assert!(
        sketch
            .insert_entity(id, knotted(vec![1.0, 2.0, 1.0, 1.0]))
            .is_ok()
    );
    assert!(sketch.spline(id).is_some());
}
