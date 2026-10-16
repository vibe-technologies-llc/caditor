use caditor_geometry::{Plane, Point2};

use crate::{
    BSpline, BreakError, Constraint, Entity, EntityId, Sketch, SplineKind, SplitError,
    intersect::closest_spline_parameter,
};

const EXACT: f64 = 1e-7;
const SAMPLES: usize = 96;

fn wavy() -> Vec<Point2> {
    vec![
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 20.0),
        Point2::new(30.0, -20.0),
        Point2::new(40.0, 0.0),
    ]
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

fn distance_from(spline: &BSpline, point: Point2) -> f64 {
    spline
        .point_at(closest_spline_parameter(spline, point))
        .distance(point)
}

fn samples(spline: &BSpline) -> impl Iterator<Item = Point2> + '_ {
    (0..=SAMPLES).map(|index| spline.point_at(index as f64 / SAMPLES as f64))
}

fn assert_pieces_keep_the_shape(whole: &BSpline, pieces: &[&BSpline]) {
    for piece in pieces {
        for point in samples(piece) {
            let off = distance_from(whole, point);
            assert!(off < EXACT, "{point} lies {off} off the original spline");
        }
    }
    for point in samples(whole) {
        let nearest = pieces
            .iter()
            .map(|piece| distance_from(piece, point))
            .fold(f64::INFINITY, f64::min);
        assert!(nearest < EXACT, "{point} is {nearest} from every piece");
    }
}

fn on_spline(sketch: &mut Sketch, spline: EntityId, parameter: f64) -> EntityId {
    let at = sketch.spline(spline).unwrap().point_at(parameter);
    let point = sketch.add_point(at);
    sketch
        .add_constraint(Constraint::Coincident(point, spline))
        .unwrap();
    point
}

fn ends(sketch: &Sketch, spline: EntityId) -> (EntityId, EntityId) {
    sketch.entity(spline).and_then(Entity::spline_ends).unwrap()
}

fn has(sketch: &Sketch, constraint: &Constraint) -> bool {
    sketch
        .constraints()
        .any(|(_, existing)| existing == constraint)
}

#[test]
fn a_control_spline_splits_at_a_point_on_it_into_two_splines_of_the_same_curve() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&wavy());
    let point = on_spline(&mut sketch, spline, 0.37);
    let whole = sketch.spline(spline).unwrap();
    let (start, end) = ends(&sketch, spline);

    let piece = sketch.split_at(spline, point).unwrap();

    let near = sketch.spline(spline).unwrap();
    let far = sketch.spline(piece).unwrap();
    assert_eq!(ends(&sketch, spline), (start, point));
    assert_eq!(ends(&sketch, piece), (point, end));
    assert_eq!(sketch.split_from(piece), Some(spline));
    assert!(!has(&sketch, &Constraint::Coincident(point, spline)));
    assert_pieces_keep_the_shape(&whole, &[&near, &far]);
}

#[test]
fn a_long_spline_split_inside_a_span_and_at_a_knot_keeps_both_pieces_exact() {
    for parameter in [0.1, 0.25, 0.5, 0.61, 0.9] {
        let mut sketch = Sketch::new(Plane::XY);
        let spline = sketch.add_spline(&long_wave());
        let point = on_spline(&mut sketch, spline, parameter);
        let whole = sketch.spline(spline).unwrap();

        let piece = sketch.split_at(spline, point).unwrap();

        let near = sketch.spline(spline).unwrap();
        let far = sketch.spline(piece).unwrap();
        assert!(
            near.point_at(1.0).distance(far.point_at(0.0)) < EXACT,
            "the pieces do not meet at {parameter}"
        );
        assert_pieces_keep_the_shape(&whole, &[&near, &far]);
    }
}

#[test]
fn a_spline_split_twice_keeps_three_exact_pieces() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&long_wave());
    let first = on_spline(&mut sketch, spline, 0.3);
    let second = on_spline(&mut sketch, spline, 0.7);
    let whole = sketch.spline(spline).unwrap();

    let rest = sketch.split_at(spline, first).unwrap();
    let last = sketch.split_at(rest, second).unwrap();

    let pieces: Vec<BSpline> = [spline, rest, last]
        .iter()
        .map(|piece| sketch.spline(*piece).unwrap())
        .collect();
    assert_pieces_keep_the_shape(&whole, &pieces.iter().collect::<Vec<_>>());
}

#[test]
fn a_conic_splits_into_two_conics_of_the_same_curve() {
    let mut sketch = Sketch::new(Plane::XY);
    let conic = sketch.add_spline_of(
        &[
            Point2::new(0.0, 0.0),
            Point2::new(20.0, 30.0),
            Point2::new(40.0, 0.0),
        ],
        SplineKind::Conic { rho: 0.7 },
    );
    let point = on_spline(&mut sketch, conic, 0.4);
    let whole = sketch.spline(conic).unwrap();

    let piece = sketch.split_at(conic, point).unwrap();

    let near = sketch.spline(conic).unwrap();
    let far = sketch.spline(piece).unwrap();
    assert!(matches!(
        sketch.entity(piece).and_then(Entity::spline_kind),
        Some(SplineKind::Conic { .. })
    ));
    assert_pieces_keep_the_shape(&whole, &[&near, &far]);
}

#[test]
fn a_constraint_on_the_far_stretch_follows_it_to_the_far_piece() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&long_wave());
    let near_point = on_spline(&mut sketch, spline, 0.15);
    let far_point = on_spline(&mut sketch, spline, 0.85);
    let cut = on_spline(&mut sketch, spline, 0.5);

    let piece = sketch.split_at(spline, cut).unwrap();

    assert!(has(&sketch, &Constraint::Coincident(near_point, spline)));
    assert!(has(&sketch, &Constraint::Coincident(far_point, piece)));
    assert!(!has(&sketch, &Constraint::Coincident(far_point, spline)));
}

#[test]
fn a_fit_spline_a_closed_spline_and_a_point_off_the_curve_are_refused_in_words() {
    let mut sketch = Sketch::new(Plane::XY);
    let fit = sketch.add_spline_of(&wavy(), SplineKind::fit(false));
    let closed = sketch.add_spline_of(&wavy(), SplineKind::Control { closed: true });
    let control = sketch.add_spline(&wavy());
    let on_fit = on_spline(&mut sketch, fit, 0.4);
    let on_closed = on_spline(&mut sketch, closed, 0.4);
    let off = sketch.add_point(Point2::new(20.0, 50.0));
    let before = sketch.clone();

    assert!(matches!(
        sketch.check_split(fit, on_fit),
        Err(SplitError::FitSpline { .. })
    ));
    assert!(matches!(
        sketch.check_split(closed, on_closed),
        Err(SplitError::ClosedSpline { .. })
    ));
    assert!(matches!(
        sketch.check_split(control, off),
        Err(SplitError::NotOnCurve { .. })
    ));
    assert!(sketch.split_at(fit, on_fit).is_err());
    assert!(sketch.same_content(&before));
}

#[test]
fn a_point_at_an_end_of_a_spline_does_not_split_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&wavy());
    let at_start = sketch.add_point(Point2::new(0.0, 0.0));

    assert!(matches!(
        sketch.check_split(spline, at_start),
        Err(SplitError::NotOnCurve { .. })
    ));
}

#[test]
fn a_spline_is_broken_where_other_curves_cross_it_into_pieces_of_the_same_curve() {
    let mut sketch = Sketch::new(Plane::XY);
    let raised: Vec<Point2> = long_wave()
        .iter()
        .map(|point| *point + Point2::new(0.0, 100.0))
        .collect();
    let spline = sketch.add_spline(&raised);
    sketch.add_line(Point2::new(15.0, 60.0), Point2::new(15.0, 140.0));
    sketch.add_line(Point2::new(45.0, 60.0), Point2::new(45.0, 140.0));
    let whole = sketch.spline(spline).unwrap();

    let pieces = sketch.break_curve(spline).unwrap();

    assert_eq!(pieces.len(), 2);
    let curves: Vec<BSpline> = [spline]
        .iter()
        .chain(&pieces)
        .map(|piece| sketch.spline(*piece).unwrap())
        .collect();
    assert_pieces_keep_the_shape(&whole, &curves.iter().collect::<Vec<_>>());
    let first_cut = sketch.spline(spline).unwrap().point_at(1.0);
    assert!((first_cut.x - 15.0).abs() < EXACT);
}

#[test]
fn breaking_refuses_fit_and_closed_splines_in_words() {
    let mut sketch = Sketch::new(Plane::XY);
    let fit = sketch.add_spline_of(&long_wave(), SplineKind::fit(false));
    let closed = sketch.add_spline_of(&long_wave(), SplineKind::Control { closed: true });
    sketch.add_line(Point2::new(15.0, -40.0), Point2::new(15.0, 40.0));
    let before = sketch.clone();

    assert!(matches!(
        sketch.break_curve(fit),
        Err(BreakError::FitSpline { .. })
    ));
    assert!(matches!(
        sketch.break_curve(closed),
        Err(BreakError::ClosedSpline { .. })
    ));
    assert!(sketch.same_content(&before));
}
