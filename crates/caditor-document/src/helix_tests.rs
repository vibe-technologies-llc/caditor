use std::f64::consts::TAU;

use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{SamplingTolerance, vertex_names};
use caditor_sketch::{Entity, EntityId, Sketch};

use crate::*;

fn evaluate(document: &Document) -> Evaluation {
    Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn volume(evaluation: &Evaluation, body: FeatureId) -> f64 {
    evaluation
        .body(body)
        .unwrap()
        .tessellate(&SamplingTolerance::new(1e-3, 0.02).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

fn mm(value: f64) -> Expression {
    Expression::measure(value, Unit::Millimetre)
}

fn helix_of(turns: f64, pitch: f64) -> Helix {
    Helix {
        length: CoilLength::TurnsAndPitch {
            turns: Expression::number(turns),
            pitch: mm(pitch),
        },
        ..Helix::new(AxisReference::Principal(PrincipalAxis::Z), mm(20.0))
    }
}

fn triangle() -> Sketch {
    triangle_facing(Vector3::Y, Vector3::X)
}

fn triangle_facing(normal: Vector3, outward: Vector3) -> Sketch {
    let plane = Plane::from_frame(Point3::ZERO, normal, outward).unwrap();
    let mut sketch = Sketch::new(plane);
    let corners = [
        Point2::new(11.0, -1.0),
        Point2::new(11.0, 1.0),
        Point2::new(9.0, 0.0),
    ];
    for (from, to) in corners.iter().zip(corners.iter().cycle().skip(1)) {
        sketch.add_line(*from, *to);
    }
    sketch
}

struct Model {
    document: Document,
    helix: FeatureId,
    sweep: FeatureId,
}

fn thread_along(helix: Helix) -> Model {
    thread_of(helix, triangle())
}

fn thread_of(helix: Helix, profile: Sketch) -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let helix = transaction.add_feature("Helix 1", FeatureKind::Helix(helix));
    let profile = transaction.add_feature("Profile", FeatureKind::from(profile));
    let sweep = transaction.add_feature(
        "Sweep 1",
        FeatureKind::Solid(SolidFeature::Sweep(Sweep::new(
            profile,
            SweepPath::Helix { helix },
        ))),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        helix,
        sweep,
    }
}

#[test]
fn a_helix_is_a_curve_feature_of_one_piece_a_turn_making_no_body() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let helix = transaction.add_feature("Helix 1", FeatureKind::Helix(helix_of(2.5, 3.0)));
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);
    let result = evaluation.feature(helix).unwrap().result.clone().unwrap();
    let curve = result.helix().unwrap();

    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(curve.turns.len(), 3);
    assert!(curve.start().unwrap().distance(Point3::new(10.0, 0.0, 0.0)) < 1e-9);
    assert!(curve.end().unwrap().distance(Point3::new(-10.0, 0.0, 7.5)) < 1e-9);
    let feature = document.feature(helix).unwrap();
    assert!(!feature.makes_body());
    assert_eq!(feature.body(), None);
}

#[test]
fn a_triangle_swept_along_a_helix_makes_a_thread_of_the_analytic_volume() {
    let model = thread_along(helix_of(2.0, 3.0));

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    let solid = evaluation.body(model.sweep).unwrap();
    assert_eq!(solid.validate(), Ok(()));
    let expected = 2.0 * TAU * 2.0 * (31.0 / 3.0);
    let found = volume(&evaluation, model.sweep);
    assert!(
        (found - expected).abs() < 1e-3 * expected,
        "{found} against {expected}"
    );
    let sweep = model.document.feature(model.sweep).unwrap();
    assert!(sweep.kind.features().contains(&model.helix));
}

#[test]
fn a_clockwise_reversed_helix_winds_the_other_way_down_its_axis() {
    let helix = Helix {
        reversed: true,
        turning: CoilTurning::Clockwise,
        ..helix_of(1.5, 3.0)
    };
    let model = thread_along(helix);

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    let bounds = evaluation
        .body(model.sweep)
        .unwrap()
        .bounding_box()
        .unwrap();
    assert!(bounds.max().z < 1.0 + 1e-6, "{bounds:?}");
    assert!(bounds.min().z < -4.4, "{bounds:?}");
}

#[test]
fn turns_too_close_for_the_profile_fail_the_sweep_naming_it() {
    let model = thread_along(helix_of(2.0, 1.5));

    let evaluation = evaluate(&model.document);
    let error = failure(&evaluation, model.sweep);

    assert_eq!(error.reason(), "Sweep 1 runs into itself along its path.");
    assert_eq!(evaluation.failed_count(), 1);
}

#[test]
fn a_taper_drawing_the_helix_into_its_axis_fails_it_in_words() {
    let model = thread_along(Helix {
        taper: Expression::measure(-70.0, Unit::Degree),
        ..helix_of(2.0, 6.0)
    });

    let evaluation = evaluate(&model.document);
    let error = failure(&evaluation, model.helix);

    assert_eq!(
        error.reason(),
        "The taper draws Helix 1 in until it reaches its axis."
    );
    assert!(matches!(
        evaluation.feature(model.sweep).unwrap().state,
        FeatureState::Failed(_)
    ));
}

#[test]
fn deleting_the_helix_fails_the_sweep_until_undone() {
    let mut model = thread_along(helix_of(1.0, 3.0));

    let dependents = model.document.dependents_of(&[model.helix]);
    let deletion = model.document.deletion(&[model.helix], "Delete");
    let undo = model.document.apply(deletion).unwrap();
    let evaluation = evaluate(&model.document);

    assert_eq!(dependents, vec![model.sweep]);
    assert!(matches!(
        evaluation.feature(model.sweep).unwrap().state,
        FeatureState::Failed(_)
    ));

    model.document.apply(undo).unwrap();

    assert_eq!(evaluate(&model.document).failed_count(), 0);
}

#[test]
fn the_helix_s_values_are_expressions_it_scales_and_inlines() {
    let mut helix = helix_of(4.0, 3.0);

    assert_eq!(helix.expressions().len(), 5);
    assert_eq!(helix.lengths_mut().len(), 2);
    helix.length = CoilLength::HeightAndPitch {
        height: mm(12.0),
        pitch: mm(3.0),
    };
    assert_eq!(helix.lengths_mut().len(), 3);
}

#[test]
fn a_start_angle_starts_the_helix_and_the_sweep_along_it_further_round_its_axis() {
    let helix = Helix {
        start: Expression::measure(90.0, Unit::Degree),
        ..helix_of(2.0, 3.0)
    };
    let model = thread_of(helix, triangle_facing(Vector3::NEG_X, Vector3::Y));

    let evaluation = evaluate(&model.document);
    let curve = evaluation
        .feature(model.helix)
        .unwrap()
        .result
        .as_deref()
        .and_then(FeatureResult::helix)
        .unwrap();

    assert_eq!(evaluation.failed_count(), 0);
    assert!(curve.start().unwrap().distance(Point3::new(0.0, 10.0, 0.0)) < 1e-9);
    assert!(curve.end().unwrap().distance(Point3::new(0.0, 10.0, 6.0)) < 1e-9);
    let solid = evaluation.body(model.sweep).unwrap();
    assert_eq!(solid.validate(), Ok(()));
    let expected = 2.0 * TAU * 2.0 * (31.0 / 3.0);
    let found = volume(&evaluation, model.sweep);
    assert!(
        (found - expected).abs() < 1e-3 * expected,
        "{found} against {expected}"
    );
}

#[test]
fn a_start_angle_that_is_not_an_angle_fails_the_helix_in_words() {
    let model = thread_along(Helix {
        start: mm(5.0),
        ..helix_of(2.0, 3.0)
    });

    let evaluation = evaluate(&model.document);
    let error = failure(&evaluation, model.helix);

    assert!(error.reason().contains("start angle"), "{}", error.reason());
}

struct Pointed {
    document: Document,
    marks: FeatureId,
    mark: EntityId,
    helix: FeatureId,
}

fn marks_at(position: Point2) -> (Sketch, EntityId) {
    let mut sketch = Sketch::new(Plane::XY);
    let mark = sketch.add_point(position);
    (sketch, mark)
}

fn pointed(position: Point2, helix: impl FnOnce(PointReference) -> Helix) -> Pointed {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let (sketch, mark) = marks_at(position);
    let marks = transaction.add_feature("Marks", FeatureKind::from(sketch));
    let helix = transaction.add_feature(
        "Helix 1",
        FeatureKind::Helix(helix(PointReference::Sketch {
            sketch: marks,
            entity: mark,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    Pointed {
        document,
        marks,
        mark,
        helix,
    }
}

fn started_at(point: PointReference) -> Helix {
    Helix {
        start_point: Some(point),
        ..helix_of(2.0, 3.0)
    }
}

fn start_of(evaluation: &Evaluation, helix: FeatureId) -> Point3 {
    evaluation
        .feature(helix)
        .unwrap()
        .result
        .as_deref()
        .and_then(FeatureResult::helix)
        .and_then(HelixResult::start)
        .unwrap()
}

#[test]
fn a_helix_started_at_a_sketch_point_takes_the_angle_of_that_point_round_its_axis() {
    let model = pointed(Point2::new(0.0, 7.0), started_at);

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    assert!(
        start_of(&evaluation, model.helix).distance(Point3::new(0.0, 10.0, 0.0)) < 1e-9,
        "{:?}",
        start_of(&evaluation, model.helix)
    );
}

#[test]
fn the_picked_point_wins_over_the_typed_angle_which_returns_once_the_point_is_cleared() {
    let mut model = pointed(Point2::new(0.0, 7.0), |point| Helix {
        start: Expression::measure(180.0, Unit::Degree),
        ..started_at(point)
    });

    let with_point = start_of(&evaluate(&model.document), model.helix);
    let mut cleared = model
        .document
        .feature(model.helix)
        .unwrap()
        .kind
        .helix()
        .unwrap()
        .clone();
    cleared.start_point = None;
    model
        .document
        .apply(Transaction::single(
            "Clear",
            Edit::SetFeatureKind {
                id: model.helix,
                kind: FeatureKind::Helix(cleared),
            },
        ))
        .unwrap();
    let without_point = start_of(&evaluate(&model.document), model.helix);

    assert!(with_point.distance(Point3::new(0.0, 10.0, 0.0)) < 1e-9);
    assert!(without_point.distance(Point3::new(-10.0, 0.0, 0.0)) < 1e-9);
}

#[test]
fn the_start_follows_the_point_when_the_sketch_moves_it() {
    let mut model = pointed(Point2::new(7.0, 0.0), started_at);
    let before = start_of(&evaluate(&model.document), model.helix);

    model
        .document
        .apply(Transaction::single(
            "Move",
            Edit::SetSketchEntity {
                feature: model.marks,
                id: model.mark,
                entity: Entity::Point(Point2::new(-7.0, 0.0)),
            },
        ))
        .unwrap();
    let after = start_of(&evaluate(&model.document), model.helix);

    assert!(before.distance(Point3::new(10.0, 0.0, 0.0)) < 1e-9);
    assert!(after.distance(Point3::new(-10.0, 0.0, 0.0)) < 1e-9);
}

#[test]
fn a_reversed_helix_still_starts_at_the_angle_of_the_point_in_the_world() {
    let model = pointed(Point2::new(0.0, 7.0), |point| Helix {
        reversed: true,
        ..started_at(point)
    });

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    assert!(start_of(&evaluation, model.helix).distance(Point3::new(0.0, 10.0, 0.0)) < 1e-9);
}

#[test]
fn a_point_on_the_axis_fails_the_helix_in_words() {
    let model = pointed(Point2::new(0.0, 0.0), started_at);

    let evaluation = evaluate(&model.document);
    let error = failure(&evaluation, model.helix);

    assert_eq!(
        error.reason(),
        "The start point of Helix 1 lies on its axis, so it has no angle round it."
    );
    assert_eq!(
        error.remedy(),
        "Pick a point off the axis, or type a start angle instead."
    );
}

#[test]
fn a_helix_started_at_a_corner_of_a_body_follows_it_by_name() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let mut outline = Sketch::new(Plane::XY);
    let corners = [
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 0.0),
        Point2::new(10.0, 8.0),
        Point2::new(0.0, 8.0),
    ];
    for index in 0..4 {
        outline.add_line(corners[index], corners[(index + 1) % 4]);
    }
    let outline = transaction.add_feature("Outline", FeatureKind::from(outline));
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(mm(4.0), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document);
    let solid = evaluation.body(base).unwrap();
    let names = vertex_names(solid);
    let corner = Point3::new(10.0, 8.0, 4.0);
    let (id, _) = solid
        .vertices()
        .find(|(_, vertex)| vertex.point().distance(corner) < 1e-9)
        .unwrap();
    let vertex = names
        .iter()
        .find(|(vertex, _)| **vertex == id)
        .map(|(_, name)| *name)
        .unwrap();
    let mut transaction = document.transaction("Helix");
    let helix = transaction.add_feature(
        "Helix 1",
        FeatureKind::Helix(started_at(PointReference::Vertex { body: base, vertex })),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);
    let angle = 8.0_f64.atan2(10.0);

    assert_eq!(evaluation.failed_count(), 0);
    assert!(
        start_of(&evaluation, helix).distance(Point3::new(
            10.0 * angle.cos(),
            10.0 * angle.sin(),
            0.0
        )) < 1e-9
    );
    assert!(
        document
            .feature(helix)
            .unwrap()
            .kind
            .features()
            .contains(&base)
    );
}

#[test]
fn a_helix_depends_on_the_sketch_holding_its_start_point() {
    let model = pointed(Point2::new(0.0, 7.0), started_at);

    let helix = model.document.feature(model.helix).unwrap();

    assert!(helix.kind.features().contains(&model.marks));
    assert!(helix.kind.reference_sketches().contains(&model.marks));
    assert_eq!(
        model.document.dependents_of(&[model.marks]),
        vec![model.helix]
    );
}

fn tabled(turns: f64, pitch: f64, rows: &[(f64, f64)]) -> Helix {
    Helix {
        pitch_table: rows
            .iter()
            .map(|&(turns, pitch)| PitchRow::new(Expression::number(turns), mm(pitch)))
            .collect(),
        ..helix_of(turns, pitch)
    }
}

#[test]
fn a_helix_follows_a_table_of_pitches_by_turn_and_ends_where_the_table_says() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let helix = transaction.add_feature(
        "Helix 1",
        FeatureKind::Helix(tabled(6.0, 2.0, &[(2.0, 2.0), (4.0, 6.0)])),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);
    let result = evaluation.feature(helix).unwrap().result.clone().unwrap();
    let end = result.helix().unwrap().end().unwrap();

    assert_eq!(evaluation.failed_count(), 0);
    assert!(end.distance(Point3::new(10.0, 0.0, 24.0)) < 1e-9, "{end}");
}

#[test]
fn a_table_sizes_the_helix_by_its_height_and_by_its_turns() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let mut by_height = tabled(6.0, 2.0, &[(2.0, 2.0), (4.0, 6.0)]);
    by_height.length = CoilLength::HeightAndTurns {
        height: mm(24.0),
        turns: Expression::number(6.0),
    };
    let mut by_pitch = tabled(6.0, 2.0, &[(2.0, 2.0), (4.0, 6.0)]);
    by_pitch.length = CoilLength::HeightAndPitch {
        height: mm(24.0),
        pitch: mm(2.0),
    };
    let first = transaction.add_feature("By height", FeatureKind::Helix(by_height));
    let second = transaction.add_feature("By pitch", FeatureKind::Helix(by_pitch));
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);

    assert_eq!(evaluation.failed_count(), 0);
    for feature in [first, second] {
        let result = evaluation.feature(feature).unwrap().result.clone().unwrap();
        let end = result.helix().unwrap().end().unwrap();
        assert!(end.distance(Point3::new(10.0, 0.0, 24.0)) < 1e-6, "{end}");
    }
}

#[test]
fn a_table_row_past_the_turns_or_beside_a_final_pitch_fails_the_helix_alone() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let beyond = transaction.add_feature(
        "Beyond",
        FeatureKind::Helix(tabled(4.0, 2.0, &[(5.0, 3.0)])),
    );
    let closed = transaction.add_feature(
        "Closed",
        FeatureKind::Helix(Helix {
            final_pitch: Some(mm(4.0)),
            ..tabled(8.0, 2.0, &[(5.0, 3.0)])
        }),
    );
    let fine = transaction.add_feature("Fine", FeatureKind::Helix(helix_of(2.0, 3.0)));
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);

    assert_eq!(evaluation.failed_count(), 2);
    assert!(failure(&evaluation, beyond).reason().contains("table"));
    assert!(failure(&evaluation, closed).reason().contains("table"));
    assert!(evaluation.feature(fine).unwrap().result.is_some());
}

fn tabled_between_ends(turns: f64, pitch: f64, rows: &[(f64, f64)]) -> Helix {
    Helix {
        ends: Some(Box::default()),
        ..tabled(turns, pitch, rows)
    }
}

fn end_of(helix: Helix) -> Point3 {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let feature = transaction.add_feature("Helix 1", FeatureKind::Helix(helix));
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);
    let result = evaluation.feature(feature).unwrap().result.clone().unwrap();

    assert_eq!(evaluation.failed_count(), 0);
    result.helix().unwrap().end().unwrap()
}

#[test]
fn a_table_between_closed_ends_counts_its_turns_from_the_body_between_them() {
    let end = end_of(tabled_between_ends(9.0, 2.0, &[(2.0, 2.0), (4.0, 6.0)]));

    assert!(end.distance(Point3::new(10.0, 0.0, 32.25)) < 1e-9, "{end}");
}

#[test]
fn a_table_between_closed_ends_sizes_the_helix_by_its_height_and_by_its_turns() {
    let law = tabled_between_ends(9.0, 2.0, &[(2.0, 2.0), (4.0, 6.0)]);
    let height = 32.25;
    let by_height = Helix {
        length: CoilLength::HeightAndTurns {
            height: mm(height),
            turns: Expression::number(9.0),
        },
        ..law.clone()
    };
    let by_pitch = Helix {
        length: CoilLength::HeightAndPitch {
            height: mm(height),
            pitch: mm(2.0),
        },
        ..law
    };

    for sized in [by_height, by_pitch] {
        let end = end_of(sized);
        assert!(end.distance(Point3::new(10.0, 0.0, height)) < 1e-6, "{end}");
    }
}

#[test]
fn a_table_row_past_the_body_between_closed_ends_fails_the_helix_in_words() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let beyond = transaction.add_feature(
        "Beyond",
        FeatureKind::Helix(tabled_between_ends(6.0, 2.0, &[(3.5, 3.0)])),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);

    let reason = failure(&evaluation, beyond).reason();
    assert!(reason.contains("between its closed ends"), "{reason}");
    assert!(reason.contains("3.00"), "{reason}");
}
