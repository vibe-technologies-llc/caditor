use caditor_expression::{EvalError, Expression, ParameterId, Quantity};
use caditor_geometry::{Plane, Point2};

use crate::{
    BSpline, Constraint, Entity, EntityId, ExtendError, FitSpacing, Sketch, Solved, SplineKind,
    TrimError, Trimmed, intersect::closest_spline_parameter,
};

const EXACT: f64 = 1e-9;
const SAMPLES: usize = 64;
const AWAY: f64 = 100.0;

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn solve(sketch: &Sketch) -> Solved {
    sketch.solve(&no_parameters, &|| false).unwrap()
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

fn points_of(sketch: &Sketch, spline: EntityId) -> Vec<EntityId> {
    match sketch.entity(spline) {
        Some(Entity::Spline { points, .. }) => points.clone(),
        other => panic!("expected a spline, found {other:?}"),
    }
}

fn kind_of(sketch: &Sketch, spline: EntityId) -> SplineKind {
    sketch.entity(spline).and_then(Entity::spline_kind).unwrap()
}

fn ends(sketch: &Sketch, spline: EntityId) -> (EntityId, EntityId) {
    sketch.entity(spline).and_then(Entity::spline_ends).unwrap()
}

fn has(sketch: &Sketch, constraint: &Constraint) -> bool {
    sketch
        .constraints()
        .any(|(_, existing)| existing == constraint)
}

fn samples(spline: &BSpline) -> impl Iterator<Item = Point2> + '_ {
    (0..=SAMPLES).map(|index| spline.point_at(index as f64 / SAMPLES as f64))
}

fn distance_from(spline: &BSpline, point: Point2) -> f64 {
    spline
        .point_at(closest_spline_parameter(spline, point))
        .distance(point)
}

fn assert_lies_on(piece: &BSpline, whole: &BSpline) {
    for point in samples(piece) {
        let off = distance_from(whole, point);
        assert!(off < EXACT, "{point} lies {off} off the original spline");
    }
}

fn at(x: f64, y: f64) -> Point2 {
    Point2::new(x + AWAY, y + AWAY)
}

fn wavy() -> [Point2; 4] {
    [at(0.0, 0.0), at(10.0, 20.0), at(30.0, -20.0), at(40.0, 0.0)]
}

fn long_wave() -> [Point2; 7] {
    [
        at(0.0, 0.0),
        at(10.0, 15.0),
        at(20.0, -10.0),
        at(30.0, 12.0),
        at(40.0, -14.0),
        at(50.0, 8.0),
        at(60.0, 0.0),
    ]
}

fn upright(sketch: &mut Sketch, x: f64) -> EntityId {
    sketch.add_line(at(x, -40.0), at(x, 40.0))
}

#[test]
fn trimming_the_middle_of_a_spline_leaves_two_splines_on_the_same_curve() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&wavy());
    let left = upright(&mut sketch, 12.0);
    let right = upright(&mut sketch, 28.0);
    let whole = sketch.spline(spline).unwrap();
    let (start, end) = ends(&sketch, spline);

    let trimmed = sketch.trim(spline, at(20.0, 0.0)).unwrap();

    let Trimmed::Split { piece } = trimmed else {
        panic!("expected a split, found {trimmed:?}");
    };
    let (near_start, near_end) = ends(&sketch, spline);
    let (far_start, far_end) = ends(&sketch, piece);
    assert_eq!((near_start, far_end), (start, end));
    assert_eq!(kind_of(&sketch, spline), SplineKind::OPEN);
    assert_eq!(kind_of(&sketch, piece), SplineKind::OPEN);
    assert_eq!(points_of(&sketch, spline).len(), 4);
    assert_eq!(sketch.split_from(piece), Some(spline));
    assert!((sketch.point(near_end).unwrap().x - at(12.0, 0.0).x).abs() < 1e-7);
    assert!((sketch.point(far_start).unwrap().x - at(28.0, 0.0).x).abs() < 1e-7);
    assert!(has(&sketch, &Constraint::Coincident(near_end, left)));
    assert!(has(&sketch, &Constraint::Coincident(far_start, right)));
    for kept in [spline, piece] {
        assert_lies_on(&sketch.spline(kept).unwrap(), &whole);
    }
    assert_solves_in_place(&sketch);
}

#[test]
fn trimming_an_end_piece_shortens_the_spline_and_keeps_its_far_end() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&wavy());
    let post = upright(&mut sketch, 30.0);
    let whole = sketch.spline(spline).unwrap();
    let (start, end) = ends(&sketch, spline);
    let old_points = points_of(&sketch, spline);

    assert_eq!(sketch.trim(spline, at(38.0, 0.0)), Ok(Trimmed::Shortened));

    let (kept_start, cut_end) = ends(&sketch, spline);
    assert_eq!(kept_start, start);
    assert_ne!(cut_end, end);
    assert!(sketch.entity(end).is_none());
    assert!(
        old_points
            .iter()
            .skip(1)
            .all(|point| sketch.entity(*point).is_none())
    );
    assert!((sketch.point(cut_end).unwrap().x - at(30.0, 0.0).x).abs() < 1e-7);
    assert!(has(&sketch, &Constraint::Coincident(cut_end, post)));
    assert_lies_on(&sketch.spline(spline).unwrap(), &whole);
    assert_solves_in_place(&sketch);
}

#[test]
fn trimming_both_ends_of_a_long_spline_leaves_its_middle_exactly() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&long_wave());
    upright(&mut sketch, 13.0);
    upright(&mut sketch, 47.0);
    let whole = sketch.spline(spline).unwrap();
    let inner = points_of(&sketch, spline)[3];

    assert_eq!(sketch.trim(spline, at(2.0, 0.0)), Ok(Trimmed::Shortened));
    assert_eq!(sketch.trim(spline, at(58.0, 0.0)), Ok(Trimmed::Shortened));

    let SplineKind::Piece { lead, trail } = kind_of(&sketch, spline) else {
        panic!("expected a piece, found {:?}", kind_of(&sketch, spline));
    };
    assert!(lead > 0.0 && lead < 1.0 && trail > 0.0 && trail < 1.0);
    assert!(points_of(&sketch, spline).contains(&inner));
    let piece = sketch.spline(spline).unwrap();
    assert!((piece.point_at(0.0).x - at(13.0, 0.0).x).abs() < 1e-7);
    assert!((piece.point_at(1.0).x - at(47.0, 0.0).x).abs() < 1e-7);
    assert_lies_on(&piece, &whole);
    assert_solves_in_place(&sketch);
}

#[test]
fn a_conic_is_trimmed_into_a_conic_of_the_same_curve() {
    let mut sketch = Sketch::new(Plane::XY);
    let conic = sketch.add_spline_of(
        &[at(0.0, 0.0), at(20.0, 30.0), at(40.0, 0.0)],
        SplineKind::Conic { rho: 0.7 },
    );
    let rho = sketch
        .add_constraint(Constraint::Rho {
            conic,
            value: Expression::Number(0.7),
        })
        .unwrap();
    upright(&mut sketch, 30.0);
    let whole = sketch.spline(conic).unwrap();

    assert_eq!(sketch.trim(conic, at(39.0, 1.0)), Ok(Trimmed::Shortened));

    let SplineKind::Conic { rho: trimmed } = kind_of(&sketch, conic) else {
        panic!("expected a conic");
    };
    assert!(SplineKind::rho_is_valid(trimmed));
    assert!(sketch.constraint(rho).is_none());
    assert_lies_on(&sketch.spline(conic).unwrap(), &whole);
    assert_solves_in_place(&sketch);
}

#[test]
fn a_closed_spline_cut_twice_opens_into_the_rest_of_its_loop() {
    let mut sketch = Sketch::new(Plane::XY);
    let loop_spline = sketch.add_spline_of(
        &[
            at(-20.0, -20.0),
            at(20.0, -20.0),
            at(20.0, 20.0),
            at(-20.0, 20.0),
        ],
        SplineKind::Control { closed: true },
    );
    let post = upright(&mut sketch, 5.0);
    let whole = sketch.spline(loop_spline).unwrap();

    assert_eq!(sketch.trim(loop_spline, at(10.0, 0.0)), Ok(Trimmed::Opened));

    assert!(!kind_of(&sketch, loop_spline).is_closed());
    let (start, end) = ends(&sketch, loop_spline);
    for point in [start, end] {
        assert!((sketch.point(point).unwrap().x - at(5.0, 0.0).x).abs() < 1e-7);
        assert!(has(&sketch, &Constraint::Coincident(point, post)));
    }
    let opened = sketch.spline(loop_spline).unwrap();
    assert!(opened.point_at(0.5).x < AWAY);
    assert_lies_on(&opened, &whole);
    assert_solves_in_place(&sketch);
}

#[test]
fn a_fit_spline_keeps_its_points_and_passes_through_the_cut() {
    let mut sketch = Sketch::new(Plane::XY);
    let through = [
        at(0.0, 0.0),
        at(10.0, 8.0),
        at(20.0, -4.0),
        at(30.0, 6.0),
        at(40.0, 0.0),
    ];
    let fit = sketch.add_spline_of(&through, SplineKind::fit(false));
    let kept = points_of(&sketch, fit);
    let post = upright(&mut sketch, 25.0);

    assert_eq!(sketch.trim(fit, at(39.0, 0.0)), Ok(Trimmed::Shortened));

    let points = points_of(&sketch, fit);
    assert_eq!(&points[..3], &kept[..3]);
    assert_eq!(points.len(), 4);
    assert_eq!(
        kind_of(&sketch, fit),
        SplineKind::Fit {
            closed: false,
            spacing: FitSpacing::Centripetal
        }
    );
    let cut = points[3];
    assert!((sketch.point(cut).unwrap().x - at(25.0, 0.0).x).abs() < 1e-7);
    assert!(has(&sketch, &Constraint::Coincident(cut, post)));
    assert_solves_in_place(&sketch);
}

#[test]
fn a_spline_joined_at_its_trimmed_end_loses_the_tangent_there_and_keeps_the_other() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&wavy());
    let (start, end) = ends(&sketch, spline);
    let before = sketch.add_line(at(-10.0, -20.0), at(0.0, 0.0));
    let after = sketch.add_line(at(40.0, 0.0), at(50.0, 20.0));
    let (before_end, after_start) = match (sketch.entity(before), sketch.entity(after)) {
        (Some(Entity::Line { end, .. }), Some(Entity::Line { start, .. })) => (*end, *start),
        _ => panic!("expected lines"),
    };
    sketch
        .add_constraint(Constraint::Coincident(before_end, start))
        .unwrap();
    sketch
        .add_constraint(Constraint::Coincident(after_start, end))
        .unwrap();
    let kept = sketch
        .add_constraint(Constraint::Tangent(before, spline))
        .unwrap();
    let lost = sketch
        .add_constraint(Constraint::Tangent(spline, after))
        .unwrap();
    upright(&mut sketch, 30.0);

    assert_eq!(sketch.trim(spline, at(38.0, 0.0)), Ok(Trimmed::Shortened));

    assert!(sketch.constraint(kept).is_some());
    assert!(sketch.constraint(lost).is_none());
}

#[test]
fn a_spline_end_extends_along_its_own_curve_to_the_line_in_its_way() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&wavy());
    let wall = upright(&mut sketch, 44.0);
    let whole = sketch.spline(spline).unwrap();
    let (start, end) = ends(&sketch, spline);

    let extension = sketch.extend(spline, at(39.0, 0.0)).unwrap();

    assert_eq!(extension.end, end);
    assert_eq!(extension.target, wall);
    assert_eq!(ends(&sketch, spline), (start, end));
    assert!((sketch.point(end).unwrap().x - at(44.0, 0.0).x).abs() < 1e-7);
    assert!(has(&sketch, &Constraint::Coincident(end, wall)));
    let longer = sketch.spline(spline).unwrap();
    assert_lies_on(&whole, &longer);
    assert_eq!(kind_of(&sketch, spline), SplineKind::OPEN);
    assert_solves_in_place(&sketch);
}

#[test]
fn a_long_spline_extends_its_last_span_exactly() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&long_wave());
    let wall = upright(&mut sketch, 66.0);
    let whole = sketch.spline(spline).unwrap();
    let inner = points_of(&sketch, spline)[2];

    let extension = sketch.extend(spline, at(59.0, 0.0)).unwrap();

    assert_eq!(extension.target, wall);
    let SplineKind::Piece { lead, trail } = kind_of(&sketch, spline) else {
        panic!("expected a piece, found {:?}", kind_of(&sketch, spline));
    };
    assert!((lead - 1.0).abs() < 1e-12 && trail > 1.0, "{lead} {trail}");
    assert!(points_of(&sketch, spline).contains(&inner));
    assert_lies_on(&whole, &sketch.spline(spline).unwrap());
    assert_solves_in_place(&sketch);
}

#[test]
fn a_fit_spline_extends_through_its_old_end() {
    let mut sketch = Sketch::new(Plane::XY);
    let through = [at(0.0, 0.0), at(10.0, 8.0), at(20.0, -4.0), at(30.0, 0.0)];
    let fit = sketch.add_spline_of(&through, SplineKind::fit(false));
    let wall = upright(&mut sketch, 40.0);
    let (_, end) = ends(&sketch, fit);

    sketch.extend(fit, at(29.0, 0.0)).unwrap();

    let points = points_of(&sketch, fit);
    assert_eq!(points.len(), 5);
    assert_eq!(points.last(), Some(&end));
    let old_end = sketch.point(points[3]).unwrap();
    assert!(old_end.distance(at(30.0, 0.0)) < EXACT);
    assert!((sketch.point(end).unwrap().x - at(40.0, 0.0).x).abs() < 1e-7);
    assert!(has(&sketch, &Constraint::Coincident(end, wall)));
    assert_solves_in_place(&sketch);
}

#[test]
fn an_end_running_on_past_every_curve_is_not_extended() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&wavy());
    upright(&mut sketch, 50.0);

    assert!(matches!(
        sketch.extend(spline, at(39.0, 0.0)),
        Err(ExtendError::NothingToReach { .. })
    ));
}

#[test]
fn a_closed_spline_is_not_extended() {
    let mut sketch = Sketch::new(Plane::XY);
    let closed = sketch.add_spline_of(
        &[at(0.0, 0.0), at(10.0, 0.0), at(5.0, 10.0)],
        SplineKind::Control { closed: true },
    );
    upright(&mut sketch, 30.0);

    assert!(matches!(
        sketch.extend(closed, at(10.0, 0.0)),
        Err(ExtendError::Closed { .. })
    ));
}

#[test]
fn a_spline_crossing_nothing_is_deleted_with_its_points() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&wavy());
    let points = points_of(&sketch, spline);

    assert_eq!(sketch.trim(spline, at(20.0, 0.0)), Ok(Trimmed::Deleted));

    assert!(sketch.entity(spline).is_none());
    assert!(points.iter().all(|point| sketch.entity(*point).is_none()));
    assert!(matches!(
        sketch.trim(spline, at(20.0, 0.0)),
        Err(TrimError::NoSuchCurve(_))
    ));
}

#[test]
fn a_point_on_a_trimmed_spline_follows_the_piece_it_lies_on() {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&wavy());
    upright(&mut sketch, 12.0);
    upright(&mut sketch, 28.0);
    let whole = sketch.spline(spline).unwrap();
    let [middle, far] = [0.5, 0.9].map(|parameter| sketch.add_point(whole.point_at(parameter)));
    let on_middle = sketch
        .add_constraint(Constraint::Coincident(middle, spline))
        .unwrap();
    sketch
        .add_constraint(Constraint::Coincident(far, spline))
        .unwrap();

    let Ok(Trimmed::Split { piece }) = sketch.trim(spline, whole.point_at(0.5)) else {
        panic!("expected a split");
    };

    assert!(sketch.constraint(on_middle).is_none());
    assert!(has(&sketch, &Constraint::Coincident(far, piece)));
    assert!(!has(&sketch, &Constraint::Coincident(far, spline)));
    assert_solves_in_place(&sketch);
}

#[test]
fn a_closed_fit_spline_opens_through_the_points_left_on_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let fit = sketch.add_spline_of(
        &[at(-20.0, 0.0), at(0.0, -20.0), at(20.0, 0.0), at(0.0, 20.0)],
        SplineKind::fit(true),
    );
    let points = points_of(&sketch, fit);
    upright(&mut sketch, 10.0);

    assert_eq!(sketch.trim(fit, at(20.0, 0.0)), Ok(Trimmed::Opened));

    let opened = points_of(&sketch, fit);
    assert_eq!(
        kind_of(&sketch, fit),
        SplineKind::Fit {
            closed: false,
            spacing: FitSpacing::Centripetal
        }
    );
    assert_eq!(opened.len(), 5);
    assert_eq!(&opened[1..4], &[points[3], points[0], points[1]]);
    assert!(sketch.entity(points[2]).is_none());
    assert_solves_in_place(&sketch);
}

#[test]
fn a_conic_extends_along_its_own_curve() {
    let mut sketch = Sketch::new(Plane::XY);
    let conic = sketch.add_spline_of(
        &[at(0.0, 0.0), at(20.0, 30.0), at(40.0, 0.0)],
        SplineKind::Conic { rho: 0.4 },
    );
    let floor = sketch.add_line(at(-20.0, -10.0), at(80.0, -10.0));
    let whole = sketch.spline(conic).unwrap();
    let (_, end) = ends(&sketch, conic);

    let extension = sketch.extend(conic, at(39.0, 1.0)).unwrap();

    assert_eq!(extension.target, floor);
    assert!((sketch.point(end).unwrap().y - at(0.0, -10.0).y).abs() < 1e-7);
    let SplineKind::Conic { rho } = kind_of(&sketch, conic) else {
        panic!("expected a conic");
    };
    assert!(SplineKind::rho_is_valid(rho) && rho != 0.4);
    assert_lies_on(&whole, &sketch.spline(conic).unwrap());
    assert_solves_in_place(&sketch);
}
