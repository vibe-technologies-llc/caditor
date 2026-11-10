use caditor_expression::{EvalError, ParameterId, Quantity};
use caditor_geometry::{Plane, Point2};

use crate::{
    BSpline, Constraint, DegreeError, Entity, EntityId, MAX_DEGREE, Sketch, SketchError,
    SplineKind, Trimmed, intersect::closest_spline_parameter,
};

const EXACT: f64 = 1e-7;
const SAMPLES: usize = 64;

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn wave() -> Vec<Point2> {
    vec![
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 12.0),
        Point2::new(20.0, -8.0),
        Point2::new(30.0, 10.0),
        Point2::new(40.0, -6.0),
        Point2::new(50.0, 8.0),
        Point2::new(60.0, 0.0),
    ]
}

fn kind_of(sketch: &Sketch, spline: EntityId) -> SplineKind {
    sketch.entity(spline).and_then(Entity::spline_kind).unwrap()
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

#[test]
fn a_control_spline_takes_the_degree_chosen_for_it_and_keeps_its_points() {
    for degree in [1, 2, 4, 5, 6] {
        let mut sketch = Sketch::new(Plane::XY);
        let spline = sketch.add_spline(&wave());
        let points = sketch.entity(spline).unwrap().points();

        sketch.set_spline_degree(spline, degree).unwrap();

        let curve = sketch.spline(spline).unwrap();
        assert_eq!(curve.degree(), degree);
        assert_eq!(sketch.spline_degree(spline), Some(degree));
        assert_eq!(sketch.entity(spline).unwrap().points(), points);
        assert_eq!(curve.point_at(0.0), wave()[0]);
        assert!(curve.point_at(1.0).distance(wave()[6]) < EXACT);
    }
}

#[test]
fn a_degree_of_three_is_the_plain_control_spline_again() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&wave());
    let before = sketch.spline(spline).unwrap();

    sketch.set_spline_degree(spline, 5).unwrap();
    sketch.set_spline_degree(spline, 3).unwrap();

    assert_eq!(kind_of(&sketch, spline), SplineKind::OPEN);
    lies_on(&sketch.spline(spline).unwrap(), &before);
}

#[test]
fn a_degree_the_points_cannot_carry_or_a_spline_without_one_is_refused_in_words() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&wave()[..4]);
    let fit = sketch.add_spline_of(&wave(), SplineKind::fit(false));
    let closed = sketch.add_spline_of(&wave(), SplineKind::Control { closed: true });
    let conic = sketch.add_spline_of(&wave()[..3], SplineKind::Conic { rho: 0.4 });
    let before = sketch.clone();

    assert!(matches!(
        sketch.set_spline_degree(spline, 4),
        Err(DegreeError::TooFewPoints { most: 3, .. })
    ));
    assert!(matches!(
        sketch.set_spline_degree(spline, 0),
        Err(DegreeError::OutOfRange { degree: 0 })
    ));
    assert!(matches!(
        sketch.set_spline_degree(spline, MAX_DEGREE + 1),
        Err(DegreeError::OutOfRange { .. })
    ));
    assert!(matches!(
        sketch.set_spline_degree(spline, 3),
        Err(DegreeError::Unchanged { degree: 3, .. })
    ));
    assert!(matches!(
        sketch.set_spline_degree(fit, 5),
        Err(DegreeError::FitSpline { .. })
    ));
    assert!(matches!(
        sketch.set_spline_degree(closed, 5),
        Err(DegreeError::ClosedSpline { .. })
    ));
    assert!(matches!(
        sketch.set_spline_degree(conic, 5),
        Err(DegreeError::Conic { .. })
    ));
    assert!(sketch.same_content(&before));
}

#[test]
fn a_spline_of_degree_one_drops_a_curvature_and_a_higher_one_keeps_every_other_constraint() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&wave());
    let other = sketch.add_spline(&[
        Point2::new(60.0, 0.0),
        Point2::new(70.0, 10.0),
        Point2::new(80.0, -4.0),
        Point2::new(90.0, 0.0),
    ]);
    let (_, end) = sketch.entity(spline).and_then(Entity::spline_ends).unwrap();
    let (start, _) = sketch.entity(other).and_then(Entity::spline_ends).unwrap();
    sketch
        .add_constraint(Constraint::Coincident(end, start))
        .unwrap();
    sketch
        .add_constraint(Constraint::Tangent(spline, other))
        .unwrap();
    sketch
        .add_constraint(Constraint::Curvature(spline, other))
        .unwrap();

    sketch.set_spline_degree(spline, 5).unwrap();
    sketch.set_spline_degree(spline, 1).unwrap();

    assert!(
        sketch
            .constraints()
            .any(|(_, constraint)| *constraint == Constraint::Tangent(spline, other))
    );
    assert!(
        !sketch
            .constraints()
            .any(|(_, constraint)| matches!(constraint, Constraint::Curvature(..)))
    );
}

#[test]
fn a_spline_of_a_chosen_degree_solves_in_place_with_a_point_held_on_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&wave());
    sketch.set_spline_degree(spline, 5).unwrap();
    let on = sketch
        .spline(spline)
        .map(|curve| curve.point_at(0.4))
        .unwrap();
    let point = sketch.add_point(on);
    sketch
        .add_constraint(Constraint::Coincident(point, spline))
        .unwrap();

    let solved = sketch.solve(&no_parameters, &|| false).unwrap();

    for (id, entity) in sketch.entities() {
        if let Entity::Point(position) = entity {
            let moved = solved.geometry.point(id).unwrap().distance(*position);
            assert!(moved < 1e-7, "point {id:?} moved by {moved}");
        }
    }
    assert!(solved.solution.redundancies().is_empty());
}

#[test]
fn a_spline_of_a_chosen_degree_is_trimmed_split_and_given_a_point_without_changing_its_shape() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&wave());
    sketch.set_spline_degree(spline, 5).unwrap();
    let whole = sketch.spline(spline).unwrap();
    let cutter = sketch.add_line(Point2::new(25.0, -30.0), Point2::new(25.0, 30.0));
    let near = sketch.spline(spline).unwrap().point_at(0.8);

    assert!(matches!(sketch.trim(spline, near), Ok(Trimmed::Shortened)));

    let trimmed = sketch.spline(spline).unwrap();
    assert_eq!(trimmed.degree(), 5);
    assert!(sketch.entity(cutter).is_some());
    lies_on(&trimmed, &whole);

    let before = sketch.spline(spline).unwrap();
    let count = sketch.entity(spline).unwrap().points().len();
    sketch.insert_control_point(spline, None).unwrap();

    assert_eq!(sketch.entity(spline).unwrap().points().len(), count + 1);
    assert_eq!(sketch.spline(spline).unwrap().degree(), 5);
    lies_on(&sketch.spline(spline).unwrap(), &before);
}

#[test]
fn a_spline_of_a_chosen_degree_splits_at_a_point_on_it_into_two_exact_pieces() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&wave());
    sketch.set_spline_degree(spline, 4).unwrap();
    let whole = sketch.spline(spline).unwrap();
    let cut = sketch.add_point(whole.point_at(0.45));
    sketch
        .add_constraint(Constraint::Coincident(cut, spline))
        .unwrap();

    let piece = sketch.split_at(spline, cut).unwrap();

    for part in [spline, piece] {
        assert_eq!(sketch.spline(part).unwrap().degree(), 4);
        lies_on(&sketch.spline(part).unwrap(), &whole);
    }
}

#[test]
fn an_invalid_degree_or_span_list_is_refused_when_a_spline_is_added() {
    let mut sketch = Sketch::new(Plane::XY);
    let points: Vec<EntityId> = wave()
        .iter()
        .map(|position| sketch.add_point(*position))
        .collect();
    let next = sketch.next_id();

    let mut insert = |degree: usize, spans: Vec<f64>| {
        sketch.insert_entity(
            EntityId::from_raw(next),
            Entity::Spline {
                points: points.clone(),
                kind: SplineKind::Graded { degree, spans },
            },
        )
    };

    assert_eq!(insert(0, vec![0.25; 4]), Err(SketchError::InvalidDegree));
    assert_eq!(
        insert(MAX_DEGREE + 1, vec![1.0]),
        Err(SketchError::InvalidDegree)
    );
    assert_eq!(
        insert(4, vec![0.5, -0.5, 1.0]),
        Err(SketchError::InvalidGradedSpans)
    );
    assert_eq!(insert(4, vec![0.5; 4]), Err(SketchError::GradedSpanCount));
    assert!(insert(4, vec![0.25, 0.25, 0.5]).is_ok());
}

#[test]
fn a_spline_drawn_at_a_degree_has_that_degree_once_it_has_the_points_for_it() {
    for degree in [1, 2, 4, 5, 6] {
        let kind = SplineKind::drawn(degree, wave().len());
        let curve = kind.curve(&wave()).unwrap();

        assert_eq!(curve.degree(), degree);
        assert_eq!(kind.degree_of(wave().len()), Some(degree));
        assert_eq!(kind.fewest_points(), degree + 1);
    }
}

#[test]
fn a_spline_drawn_at_a_degree_its_points_cannot_carry_takes_the_most_they_can() {
    assert_eq!(SplineKind::drawn(5, 4), SplineKind::OPEN);
    assert_eq!(SplineKind::drawn(5, 2), SplineKind::OPEN);
    assert_eq!(SplineKind::drawn(3, 7), SplineKind::OPEN);
    assert_eq!(SplineKind::drawn(5, 1), SplineKind::OPEN);

    let kind = SplineKind::drawn(5, 5);
    assert_eq!(kind.degree_of(5), Some(4));
    assert_eq!(kind, SplineKind::graded(4, 5).unwrap());
}

#[test]
fn a_spline_drawn_at_a_degree_outside_the_range_is_clamped_into_it() {
    let points: Vec<Point2> = (0..30)
        .map(|index| Point2::new(index as f64, (index % 4) as f64))
        .collect();

    assert_eq!(
        SplineKind::drawn(0, points.len()).degree_of(points.len()),
        Some(1)
    );
    assert_eq!(
        SplineKind::drawn(99, points.len()).degree_of(points.len()),
        Some(MAX_DEGREE)
    );
    assert!(SplineKind::drawn(99, points.len()).curve(&points).is_some());
}

#[test]
fn a_spline_added_at_a_drawn_degree_is_the_one_set_afterwards() {
    let mut drawn = Sketch::new(Plane::XY);
    let mut set = Sketch::new(Plane::XY);

    let at_five = drawn.add_spline_of(&wave(), SplineKind::drawn(5, wave().len()));
    let later = set.add_spline(&wave());
    set.set_spline_degree(later, 5).unwrap();

    assert_eq!(kind_of(&drawn, at_five), kind_of(&set, later));
    assert_eq!(drawn.spline(at_five), set.spline(later));
}
