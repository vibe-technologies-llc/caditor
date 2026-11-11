use caditor_expression::{EvalError, ParameterId, Quantity};
use caditor_geometry::{Plane, Point2};

use crate::{
    BSpline, Constraint, DegreeError, Entity, EntityId, MAX_DEGREE, Sketch, SketchError,
    SplineKind, Trimmed, intersect::closest_spline_parameter,
};

const EXACT: f64 = 1e-7;
const SAMPLES: usize = 96;
const DEGREES: [usize; 5] = [1, 2, 4, 5, 6];

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn ring() -> Vec<Point2> {
    vec![
        Point2::new(100.0, 100.0),
        Point2::new(130.0, 94.0),
        Point2::new(160.0, 110.0),
        Point2::new(172.0, 140.0),
        Point2::new(150.0, 170.0),
        Point2::new(116.0, 164.0),
        Point2::new(96.0, 142.0),
        Point2::new(88.0, 118.0),
    ]
}

fn closed_at(sketch: &mut Sketch, degree: usize, points: &[Point2]) -> EntityId {
    let kind = SplineKind::graded_loop(degree, points.len()).unwrap();
    sketch.add_spline_of(points, kind)
}

fn kind_of(sketch: &Sketch, spline: EntityId) -> SplineKind {
    sketch.entity(spline).and_then(Entity::spline_kind).unwrap()
}

fn point_count(sketch: &Sketch, spline: EntityId) -> usize {
    sketch.entity(spline).unwrap().points().len()
}

fn lies_on(piece: &BSpline, whole: &BSpline) {
    for index in 0..=SAMPLES {
        let point = piece.point_at(index as f64 / SAMPLES as f64);
        let off = whole
            .point_at(closest_spline_parameter(whole, point))
            .distance(point);
        assert!(off < EXACT, "{point} lies {off} off the spline");
    }
}

fn assert_solves_in_place(sketch: &Sketch) {
    let solved = sketch.solve(&no_parameters, &|| false).unwrap();

    for (id, entity) in sketch.entities() {
        if let Entity::Point(position) = entity {
            let moved = solved.geometry.point(id).unwrap().distance(*position);
            assert!(moved < EXACT, "point {id:?} moved by {moved}");
        }
    }
    assert!(solved.solution.redundancies().is_empty());
}

#[test]
fn a_closed_control_spline_takes_the_degree_chosen_for_it_and_stays_a_closed_loop() {
    for degree in DEGREES {
        let mut sketch = Sketch::new(Plane::XY);
        let spline = sketch.add_spline_of(&ring(), SplineKind::Control { closed: true });
        let points = sketch.entity(spline).unwrap().points();

        sketch.set_spline_degree(spline, degree).unwrap();

        let curve = sketch.spline(spline).unwrap();
        assert_eq!(curve.degree(), degree);
        assert_eq!(sketch.spline_degree(spline), Some(degree));
        assert!(kind_of(&sketch, spline).is_closed());
        assert!(sketch.entity(spline).unwrap().spline_ends().is_none());
        assert_eq!(sketch.entity(spline).unwrap().points(), points);
        assert!(curve.point_at(0.0).distance(curve.point_at(1.0)) < EXACT);
        if degree > 1 {
            let [start, _] = curve.derivatives(0.0);
            let [end, _] = curve.derivatives(1.0);
            assert!((start - end).length() < EXACT * start.length().max(1.0));
        }
    }
}

#[test]
fn a_closed_spline_of_any_degree_is_the_same_loop_whichever_control_point_comes_first() {
    for degree in DEGREES {
        let mut sketch = Sketch::new(Plane::XY);
        let spline = closed_at(&mut sketch, degree, &ring());
        let mut turned = ring();
        turned.rotate_left(3);
        let again = closed_at(&mut sketch, degree, &turned);

        lies_on(
            &sketch.spline(spline).unwrap(),
            &sketch.spline(again).unwrap(),
        );
        lies_on(
            &sketch.spline(again).unwrap(),
            &sketch.spline(spline).unwrap(),
        );
    }
}

#[test]
fn a_closed_spline_of_degree_one_is_the_polygon_of_its_control_points() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = closed_at(&mut sketch, 1, &ring());
    let polygon = sketch.spline(spline).unwrap();

    for corner in ring() {
        let off = polygon
            .point_at(closest_spline_parameter(&polygon, corner))
            .distance(corner);
        assert!(off < EXACT, "{corner} lies {off} off the polygon");
    }
    for index in 0..ring().len() {
        let next = ring()[(index + 1) % ring().len()];
        let middle = ring()[index].lerp(next, 0.5);
        let off = polygon
            .point_at(closest_spline_parameter(&polygon, middle))
            .distance(middle);
        assert!(off < EXACT, "the middle of side {index} lies {off} off");
    }
}

#[test]
fn a_degree_of_three_is_the_plain_closed_control_spline_again() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline_of(&ring(), SplineKind::Control { closed: true });
    let before = sketch.spline(spline).unwrap();

    sketch.set_spline_degree(spline, 5).unwrap();
    sketch.set_spline_degree(spline, 3).unwrap();

    assert_eq!(
        kind_of(&sketch, spline),
        SplineKind::Control { closed: true }
    );
    lies_on(&sketch.spline(spline).unwrap(), &before);
}

#[test]
fn a_closed_spline_with_too_few_points_for_a_degree_is_refused_in_words_and_left_alone() {
    let mut sketch = Sketch::new(Plane::XY);
    let triangle = sketch.add_spline_of(&ring()[..3], SplineKind::Control { closed: true });
    let square = sketch.add_spline_of(&ring()[..4], SplineKind::Control { closed: true });
    let before = sketch.clone();

    assert!(matches!(
        sketch.set_spline_degree(triangle, 4),
        Err(DegreeError::TooFewPoints { most: 3, .. })
    ));
    assert!(matches!(
        sketch.set_spline_degree(square, 4),
        Err(DegreeError::TooFewPoints { most: 3, .. })
    ));
    assert!(matches!(
        sketch.set_spline_degree(square, 3),
        Err(DegreeError::Unchanged { degree: 3, .. })
    ));
    assert!(matches!(
        sketch.set_spline_degree(square, MAX_DEGREE + 1),
        Err(DegreeError::OutOfRange { .. })
    ));
    assert!(sketch.same_content(&before));
    assert!(sketch.set_spline_degree(triangle, 2).is_ok());
    assert!(sketch.set_spline_degree(triangle, 1).is_ok());
}

#[test]
fn a_closed_spline_keeps_every_constraint_on_it_when_its_degree_changes() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline_of(&ring(), SplineKind::Control { closed: true });
    let on = sketch.spline(spline).unwrap().point_at(0.3);
    let point = sketch.add_point(on);
    sketch
        .add_constraint(Constraint::Coincident(point, spline))
        .unwrap();

    sketch.set_spline_degree(spline, 5).unwrap();

    assert!(
        sketch
            .constraints()
            .any(|(_, constraint)| *constraint == Constraint::Coincident(point, spline))
    );
}

#[test]
fn a_closed_spline_of_a_chosen_degree_solves_in_place_with_a_point_held_on_it() {
    for degree in DEGREES {
        let mut sketch = Sketch::new(Plane::XY);
        let spline = closed_at(&mut sketch, degree, &ring());
        let on = sketch.spline(spline).unwrap().point_at(0.4);
        let point = sketch.add_point(on);
        sketch
            .add_constraint(Constraint::Coincident(point, spline))
            .unwrap();

        assert_solves_in_place(&sketch);
    }
}

#[test]
fn a_point_added_to_a_closed_spline_of_a_chosen_degree_keeps_its_curve_and_its_degree() {
    for degree in DEGREES {
        for share in [0.03, 0.2, 0.5, 0.77, 0.97] {
            let mut sketch = Sketch::new(Plane::XY);
            let spline = closed_at(&mut sketch, degree, &ring());
            let before = sketch.spline(spline).unwrap();

            let added = sketch
                .insert_control_point(spline, Some(before.point_at(share)))
                .unwrap();

            let after = sketch.spline(spline).unwrap();
            assert_eq!(point_count(&sketch, spline), ring().len() + 1);
            assert!(sketch.entity(spline).unwrap().points().contains(&added));
            assert!(matches!(
                kind_of(&sketch, spline),
                SplineKind::Graded { closed: true, degree: kept, .. } if kept == degree
            ));
            assert_eq!(after.degree(), degree);
            for step in 0..=SAMPLES {
                let at = step as f64 / SAMPLES as f64;
                let moved = before.point_at(at).distance(after.point_at(at));
                assert!(moved < EXACT, "degree {degree}: moved {moved} at {at}");
            }
            assert_solves_in_place(&sketch);
        }
    }
}

#[test]
fn points_added_again_and_again_to_a_closed_spline_of_a_chosen_degree_keep_its_curve() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = closed_at(&mut sketch, 5, &ring());
    let before = sketch.spline(spline).unwrap();

    for step in 0..6 {
        let near = before.point_at((0.09 + 0.29 * step as f64).fract());
        sketch.insert_control_point(spline, Some(near)).unwrap();
    }
    sketch.insert_control_point(spline, None).unwrap();

    assert_eq!(point_count(&sketch, spline), ring().len() + 7);
    lies_on(&sketch.spline(spline).unwrap(), &before);
    lies_on(&before, &sketch.spline(spline).unwrap());
}

#[test]
fn a_closed_spline_of_a_chosen_degree_refuses_a_point_that_would_move_a_held_control_point() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = closed_at(&mut sketch, 5, &ring());
    let held = sketch.entity(spline).unwrap().points()[3];
    let at = sketch.point(held).unwrap();
    sketch
        .add_constraint(Constraint::Fix { point: held, at })
        .unwrap();
    let before = sketch.clone();
    let near = sketch.spline(spline).unwrap().point_at(0.15);

    assert!(matches!(
        sketch.insert_control_point(spline, Some(near)),
        Err(crate::InsertError::HeldPoint { point, .. }) if point == held
    ));
    assert!(sketch.same_content(&before));
}

#[test]
fn a_closed_spline_of_a_chosen_degree_cut_twice_opens_into_the_rest_of_its_loop_at_that_degree() {
    for degree in DEGREES {
        let mut sketch = Sketch::new(Plane::XY);
        let spline = closed_at(&mut sketch, degree, &ring());
        let whole = sketch.spline(spline).unwrap();
        sketch.add_line(Point2::new(125.0, 60.0), Point2::new(125.0, 200.0));

        let trimmed = sketch.trim(spline, Point2::new(160.0, 125.0));

        assert_eq!(trimmed, Ok(Trimmed::Opened), "degree {degree}");
        let opened = sketch.spline(spline).unwrap();
        assert!(!kind_of(&sketch, spline).is_closed());
        assert_eq!(
            opened.degree(),
            degree.min(point_count(&sketch, spline) - 1)
        );
        assert!(sketch.entity(spline).unwrap().spline_ends().is_some());
        lies_on(&opened, &whole);
        assert_solves_in_place(&sketch);
    }
}

#[test]
fn a_closed_spline_with_points_added_at_a_chosen_degree_trims_along_its_own_curve() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = closed_at(&mut sketch, 4, &ring());
    let whole = sketch.spline(spline).unwrap();
    for at in [0.15, 0.62] {
        sketch
            .insert_control_point(spline, Some(whole.point_at(at)))
            .unwrap();
    }
    sketch.add_line(Point2::new(125.0, 60.0), Point2::new(125.0, 200.0));
    sketch.add_line(Point2::new(165.0, 60.0), Point2::new(165.0, 200.0));

    sketch.trim(spline, Point2::new(100.0, 130.0)).unwrap();

    assert_eq!(sketch.spline(spline).unwrap().degree(), 4);
    lies_on(&sketch.spline(spline).unwrap(), &whole);
}

#[test]
fn a_closed_spline_of_any_degree_is_added_only_with_points_and_spans_that_fit_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let points: Vec<EntityId> = ring()[..5]
        .iter()
        .map(|position| sketch.add_point(*position))
        .collect();
    let id = EntityId::from_raw(100);
    let looped = |degree: usize, spans: Vec<f64>| Entity::Spline {
        points: points.clone(),
        kind: SplineKind::Graded {
            degree,
            spans,
            closed: true,
        },
    };

    assert_eq!(
        sketch.insert_entity(id, looped(0, vec![0.2; 5])),
        Err(SketchError::InvalidDegree)
    );
    assert_eq!(
        sketch.insert_entity(id, looped(4, vec![0.25; 4])),
        Err(SketchError::LoopedKnotSpanCount)
    );
    assert_eq!(
        sketch.insert_entity(id, looped(4, vec![0.2, 0.2, -0.2, 0.4, 0.4])),
        Err(SketchError::InvalidGradedSpans)
    );
    assert_eq!(
        sketch.insert_entity(id, looped(5, vec![0.2; 5])),
        Err(SketchError::TooFewClosedPoints)
    );
    assert!(
        sketch
            .insert_entity(id, looped(4, vec![0.1, 0.2, 0.3, 0.2, 0.2]))
            .is_ok()
    );
    assert_eq!(sketch.spline(id).unwrap().degree(), 4);
}

#[test]
fn a_closed_spline_is_drawn_at_the_degree_its_points_can_carry() {
    for (degree, points, drawn) in [
        (5, 3, 3),
        (5, 4, 3),
        (5, 5, 4),
        (5, 6, 5),
        (5, 12, 5),
        (4, 5, 4),
        (2, 3, 2),
        (1, 3, 1),
        (3, 3, 3),
    ] {
        let kind = SplineKind::drawn_loop(degree, points);

        assert!(kind.is_closed(), "{kind:?}");
        assert_eq!(kind.degree_of(points), Some(drawn), "{degree} on {points}");
        assert!(points >= kind.fewest_points());
    }
    assert_eq!(
        SplineKind::drawn_loop(3, 6),
        SplineKind::Control { closed: true }
    );
    assert_eq!(SplineKind::graded_loop(5, 5), None);
    assert_eq!(SplineKind::graded_loop(0, 5), None);
}
