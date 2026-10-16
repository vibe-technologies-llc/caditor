use std::f64::consts::TAU;

use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::SamplingTolerance;
use caditor_sketch::Sketch;

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
