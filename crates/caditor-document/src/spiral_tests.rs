use std::{
    collections::BTreeMap,
    f64::consts::{PI, TAU},
};

use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{SamplingTolerance, Solid};
use caditor_sketch::Sketch;

use crate::*;

fn evaluate(document: &Document) -> Evaluation {
    Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn mm(value: f64) -> Expression {
    Expression::measure(value, Unit::Millimetre)
}

fn spiral_of(length: CoilLength) -> Helix {
    Helix {
        length,
        ..Helix::flat_spiral(AxisReference::Principal(PrincipalAxis::Z), mm(20.0))
    }
}

fn turns_and_pitch(turns: f64, pitch: f64) -> Helix {
    spiral_of(CoilLength::TurnsAndPitch {
        turns: Expression::number(turns),
        pitch: mm(pitch),
    })
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

fn alone(spiral: Helix) -> (Document, FeatureId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let feature = transaction.add_feature("Spiral 1", FeatureKind::Helix(spiral));
    document.apply(transaction.finish()).unwrap();
    (document, feature)
}

fn curve(evaluation: &Evaluation, feature: FeatureId) -> HelixResult {
    evaluation
        .feature(feature)
        .unwrap()
        .result
        .as_deref()
        .and_then(FeatureResult::helix)
        .unwrap()
        .clone()
}

fn volume(solid: &Solid) -> f64 {
    solid
        .tessellate(&SamplingTolerance::new(1e-3, 0.02).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn assert_watertight(solid: &Solid) {
    let mesh = solid.tessellate(&solid.default_tolerance()).unwrap();
    let mut directed: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for [a, b, c] in mesh.position_triangles() {
        for edge in [(a, b), (b, c), (c, a)] {
            *directed.entry(edge).or_default() += 1;
        }
    }
    assert!(!directed.is_empty());
    for ((from, to), count) in &directed {
        assert_eq!(*count, 1, "edge {from}->{to} used {count} times");
        assert_eq!(
            directed.get(&(*to, *from)),
            Some(&1),
            "edge {from}->{to} is open"
        );
    }
}

#[test]
fn a_flat_spiral_winds_out_from_its_inner_diameter_in_the_plane_square_to_its_axis() {
    let (document, feature) = alone(turns_and_pitch(2.5, 3.0));

    let evaluation = evaluate(&document);
    let result = curve(&evaluation, feature);
    let spec = result.spiral.unwrap();
    let length: f64 = result
        .turns
        .iter()
        .map(|turn| turn.curve().length(turn.range()))
        .sum();
    let growth = 3.0 / TAU;
    let primitive = |radius: f64| {
        let reach = radius.hypot(growth);
        radius * reach + growth * growth * (radius + reach).ln()
    };
    let closed_form = (primitive(17.5) - primitive(10.0)) / (2.0 * growth);

    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(result.turns.len(), 3);
    assert!(
        result
            .start()
            .unwrap()
            .distance(Point3::new(10.0, 0.0, 0.0))
            < 1e-9
    );
    assert!(result.end().unwrap().distance(Point3::new(-17.5, 0.0, 0.0)) < 1e-9);
    for turn in &result.turns {
        for step in 0..=64 {
            let point = turn.curve().point(turn.range().at(f64::from(step) / 64.0));

            assert!(point.z.abs() < 1e-12, "{point}");
        }
    }
    assert!(
        (length - closed_form).abs() < 1e-6 * closed_form,
        "{length} against {closed_form}"
    );
    assert!((spec.length() - closed_form).abs() < 1e-9 * closed_form);
    assert!(!document.feature(feature).unwrap().makes_body());
}

#[test]
fn an_outer_diameter_sizes_the_spiral_with_its_turns_or_its_pitch() {
    let by_turns = spiral_of(CoilLength::HeightAndTurns {
        height: mm(50.0),
        turns: Expression::number(5.0),
    });
    let by_pitch = spiral_of(CoilLength::HeightAndPitch {
        height: mm(50.0),
        pitch: mm(4.0),
    });

    let (document, feature) = alone(by_turns);
    let turned = curve(&evaluate(&document), feature).spiral.unwrap();
    let (document, feature) = alone(by_pitch);
    let pitched = curve(&evaluate(&document), feature).spiral.unwrap();

    assert!((turned.pitch - 3.0).abs() < 1e-12);
    assert!((turned.turns - 5.0).abs() < 1e-12);
    assert!((turned.outer_radius() - 25.0).abs() < 1e-12);
    assert!((pitched.turns - 3.75).abs() < 1e-12);
    assert!((pitched.outer_radius() - 25.0).abs() < 1e-12);
}

#[test]
fn an_outer_diameter_inside_the_inner_one_fails_the_spiral_in_words() {
    let (document, feature) = alone(spiral_of(CoilLength::HeightAndTurns {
        height: mm(15.0),
        turns: Expression::number(3.0),
    }));

    let error = failure(&evaluate(&document), feature);

    assert_eq!(
        error.reason(),
        "The outer diameter of Spiral 1 is not larger than its inner diameter."
    );
    assert_eq!(
        error.remedy(),
        "Make the outer diameter larger than the inner diameter."
    );
}

#[test]
fn a_clockwise_spiral_started_a_quarter_turn_round_winds_the_other_way() {
    let (document, feature) = alone(Helix {
        turning: CoilTurning::Clockwise,
        start: Expression::measure(90.0, Unit::Degree),
        ..turns_and_pitch(1.25, 4.0)
    });

    let result = curve(&evaluate(&document), feature);

    assert!(
        result
            .start()
            .unwrap()
            .distance(Point3::new(0.0, 10.0, 0.0))
            < 1e-9
    );
    assert!(
        result.end().unwrap().distance(Point3::new(15.0, 0.0, 0.0)) < 1e-9,
        "{:?}",
        result.end()
    );
}

struct Model {
    document: Document,
    follower: FeatureId,
}

fn piped(spiral: Helix, size: f64) -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let helix = transaction.add_feature("Spiral 1", FeatureKind::Helix(spiral));
    let follower = transaction.add_feature(
        "Pipe 1",
        FeatureKind::Pipe(Pipe::new(SweepPath::Helix { helix }, mm(size), mm(0.0))),
    );
    document.apply(transaction.finish()).unwrap();
    Model { document, follower }
}

fn triangle() -> Sketch {
    let plane = Plane::from_frame(Point3::ZERO, Vector3::Y, Vector3::X).unwrap();
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

fn swept(spiral: Helix) -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let helix = transaction.add_feature("Spiral 1", FeatureKind::Helix(spiral));
    let profile = transaction.add_feature("Profile", FeatureKind::from(triangle()));
    let follower = transaction.add_feature(
        "Sweep 1",
        FeatureKind::Solid(SolidFeature::Sweep(Sweep::new(
            profile,
            SweepPath::Helix { helix },
        ))),
    );
    document.apply(transaction.finish()).unwrap();
    Model { document, follower }
}

#[test]
fn a_pipe_along_a_flat_spiral_is_a_valid_watertight_clock_spring() {
    let model = piped(turns_and_pitch(1.5, 3.0), 1.5);

    let evaluation = evaluate(&model.document);
    let solid = evaluation.body(model.follower).unwrap();
    let growth = 3.0 / TAU;
    let sweep = 1.5 * TAU;
    let area = PI * 0.25 * 1.5 * 1.5;
    let expected = area / 10.0_f64.hypot(growth)
        * ((100.0 + growth * growth) * sweep + growth * 10.0 * sweep * sweep / 2.0);
    let found = volume(solid);

    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(solid.validate(), Ok(()));
    assert_watertight(solid);
    assert!(
        (found - expected).abs() < 1e-3 * expected,
        "{found} against {expected}"
    );
    let bounds = solid.bounding_box().unwrap();
    assert!(bounds.max().z < 0.75 + 1e-6 && bounds.min().z > -0.75 - 1e-6);
}

#[test]
fn a_triangle_swept_along_a_flat_spiral_keeps_its_place_against_the_axis() {
    let model = swept(turns_and_pitch(2.0, 3.0));

    let evaluation = evaluate(&model.document);
    let solid = evaluation.body(model.follower).unwrap();
    let sweep = 2.0 * TAU;
    let expected = 2.0 * (31.0 / 3.0 * sweep + 3.0 / TAU * sweep * sweep / 2.0);
    let found = volume(solid);

    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(solid.validate(), Ok(()));
    assert_watertight(solid);
    assert!(
        (found - expected).abs() < 1e-3 * expected,
        "{found} against {expected}"
    );
}

#[test]
fn a_pipe_too_thick_for_the_spiral_s_pitch_fails_naming_both() {
    let model = piped(turns_and_pitch(3.0, 2.0), 2.5);

    let evaluation = evaluate(&model.document);
    let error = failure(&evaluation, model.follower);

    assert!(
        error
            .reason()
            .starts_with("The turns of Pipe 1 would touch along Spiral 1: at a pitch of 2.000 mm"),
        "{}",
        error.reason()
    );
    assert_eq!(
        error.remedy(),
        "Make the pitch of Spiral 1 larger, or Pipe 1 smaller."
    );
}

#[test]
fn a_pipe_at_the_least_pitch_named_is_built() {
    let spiral = turns_and_pitch(3.0, 2.0);
    let (document, feature) = alone(spiral.clone());
    let spec = curve(&evaluate(&document), feature).spiral.unwrap();
    let least = spec.least_pitch_for(2.5);

    let model = piped(turns_and_pitch(3.0, least * 1.001), 2.5);
    let evaluation = evaluate(&model.document);

    assert!(least > 2.5 && least < 2.6, "{least}");
    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(evaluation.body(model.follower).unwrap().validate(), Ok(()));
}

#[test]
fn a_profile_wider_than_the_spiral_s_pitch_fails_the_sweep_as_running_into_itself() {
    let model = swept(turns_and_pitch(2.0, 1.5));

    let evaluation = evaluate(&model.document);
    let error = failure(&evaluation, model.follower);

    assert_eq!(error.reason(), "Sweep 1 runs into itself along its path.");
}
