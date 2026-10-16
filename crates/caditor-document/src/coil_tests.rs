use std::f64::consts::{PI, TAU};

use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2};
use caditor_kernel::{CoilPlacement, CoilShape, SamplingTolerance, Surface};
use caditor_sketch::Sketch;

use crate::*;

fn evaluate(document: &Document) -> Evaluation {
    Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn volume(evaluation: &Evaluation, body: FeatureId) -> f64 {
    evaluation
        .body(body)
        .unwrap()
        .tessellate(&SamplingTolerance::new(1e-3, 0.05).unwrap())
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

fn spring() -> Coil {
    Coil {
        length: CoilLength::TurnsAndPitch {
            turns: Expression::number(3.0),
            pitch: mm(4.0),
        },
        ..Coil::new(
            AxisReference::Principal(PrincipalAxis::Z),
            mm(20.0),
            mm(2.0),
        )
    }
}

fn single(coil: Coil) -> (Document, FeatureId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let feature = transaction.add_feature("Coil 1", FeatureKind::Coil(coil));
    document.apply(transaction.finish()).unwrap();
    (document, feature)
}

fn assert_near(found: f64, expected: f64) {
    assert!(
        (found - expected).abs() < 1e-3 * expected,
        "{found} mm³, expected {expected} mm³"
    );
}

#[test]
fn a_coil_on_an_axis_makes_a_body_the_volume_of_its_wire() {
    let (document, coil) = single(spring());

    let evaluation = evaluate(&document);

    assert_eq!(evaluation.failed_count(), 0);
    assert_near(volume(&evaluation, coil), PI * TAU * 10.0 * 3.0);
    let feature = document.feature(coil).unwrap();
    assert!(feature.makes_body());
    assert!(feature.kind.parameters().is_empty());
    let bounds = evaluation.body(coil).unwrap().bounding_box().unwrap();
    assert!(bounds.min().z > -1e-6, "{bounds:?}");
}

#[test]
fn any_two_of_height_pitch_and_turns_size_the_coil() {
    let by_height = Coil {
        length: CoilLength::HeightAndTurns {
            height: mm(12.0),
            turns: Expression::number(3.0),
        },
        ..spring()
    };
    let by_pitch = Coil {
        length: CoilLength::HeightAndPitch {
            height: mm(10.0),
            pitch: mm(4.0),
        },
        shape: CoilShape::Square,
        placement: CoilPlacement::Inside,
        ..spring()
    };

    let (first, first_coil) = single(by_height);
    let (second, second_coil) = single(by_pitch);

    assert_near(volume(&evaluate(&first), first_coil), PI * TAU * 10.0 * 3.0);
    assert_near(
        volume(&evaluate(&second), second_coil),
        4.0 * TAU * 9.0 * 2.5,
    );
}

#[test]
fn a_tapered_clockwise_coil_keeps_its_volume_formula() {
    let (document, coil) = single(Coil {
        taper: Expression::measure(10.0, Unit::Degree),
        turning: CoilTurning::Clockwise,
        placement: CoilPlacement::Outside,
        ..spring()
    });

    let evaluation = evaluate(&document);
    let rise = 12.0 * 10.0_f64.to_radians().tan();

    assert_near(
        volume(&evaluation, coil),
        PI * TAU * 3.0 * (11.0 + 0.5 * rise),
    );
}

#[test]
fn a_coil_cut_round_a_cylinder_s_axis_grooves_it_and_repeats_as_a_cut() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Shaft");
    let shaft = transaction.add_feature(
        "Shaft",
        FeatureKind::Primitive(Primitive {
            shape: PrimitiveShape::Cylinder {
                diameter: mm(20.0),
                height: mm(20.0),
            },
            plane: PlaneReference::Principal(PrincipalPlane::Xy),
            at: [mm(0.0), mm(0.0)],
            anchor: PrimitiveAnchor::BaseCentre,
            reversed: false,
            operation: BodyOperation::NewBody,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document);
    let solid = evaluation.body(shaft).unwrap();
    let (wall, _) = solid
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .unwrap();
    let axis = AxisReference::capture_face(shaft, solid, wall).unwrap();
    let mut transaction = document.transaction("Groove");
    let groove = transaction.add_feature(
        "Groove",
        FeatureKind::Coil(Coil {
            axis,
            operation: BodyOperation::Remove(shaft),
            ..spring()
        }),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);

    assert_eq!(
        evaluation.failed_count(),
        0,
        "{:?}",
        evaluation.feature(groove)
    );
    let removed = 0.5 * PI * TAU * 3.0 * (10.0 - 4.0 / (3.0 * PI));
    assert_near(volume(&evaluation, shaft), PI * 100.0 * 20.0 - removed);
    let kind = &document.feature(groove).unwrap().kind;
    assert_eq!(repeatable_on(kind), Some(shaft));
    assert!(kind.bodies_used().contains(&shaft));
    assert!(kind.origin_features().contains(&shaft));
    assert!(!document.feature(groove).unwrap().makes_body());
}

#[test]
fn a_coil_round_a_sketch_line_uses_the_sketch() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let mut sketch = Sketch::new(Plane::XZ);
    let line = sketch.add_line(Point2::ZERO, Point2::new(0.0, 30.0));
    let sketch = transaction.add_feature("Axis sketch", FeatureKind::from(sketch));
    let coil = transaction.add_feature(
        "Coil 1",
        FeatureKind::Coil(Coil {
            axis: AxisReference::Sketch {
                sketch,
                entity: line,
            },
            ..spring()
        }),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);

    assert_eq!(evaluation.failed_count(), 0);
    let kind = &document.feature(coil).unwrap().kind;
    assert!(kind.reference_sketches().contains(&sketch));
    assert!(kind.features().contains(&sketch));
    assert_near(volume(&evaluation, coil), PI * TAU * 10.0 * 3.0);
}

#[test]
fn turns_that_would_touch_or_a_section_reaching_the_axis_fail_in_words() {
    let (touching, touching_coil) = single(Coil {
        length: CoilLength::TurnsAndPitch {
            turns: Expression::number(3.0),
            pitch: mm(2.0),
        },
        ..spring()
    });
    let (narrow, narrow_coil) = single(Coil {
        diameter: mm(1.5),
        placement: CoilPlacement::Inside,
        ..spring()
    });
    let (few, few_coil) = single(Coil {
        length: CoilLength::HeightAndPitch {
            height: mm(0.1),
            pitch: mm(4.0),
        },
        ..spring()
    });

    let touching = failure(&evaluate(&touching), touching_coil);
    let narrow = failure(&evaluate(&narrow), narrow_coil);
    let few = failure(&evaluate(&few), few_coil);

    assert!(
        touching.reason().contains("would touch"),
        "{}",
        touching.reason()
    );
    assert!(
        touching.remedy().contains("pitch larger"),
        "{}",
        touching.remedy()
    );
    assert_eq!(narrow.reason(), "The section of Coil 1 reaches its axis.");
    assert!(
        few.reason().contains("makes 0.03 turns"),
        "{}",
        few.reason()
    );
    assert_eq!(touching.fix, Some(FixTarget::Feature(touching_coil)));
}

#[test]
fn a_square_coil_s_faces_are_worded_by_side() {
    let (document, coil) = single(Coil {
        shape: CoilShape::Square,
        ..spring()
    });
    let evaluation = evaluate(&document);
    let solid = evaluation.body(coil).unwrap();

    let words: Vec<String> = solid
        .faces()
        .map(|(_, face)| describe_origin(&document, face.origin()))
        .collect();

    for side in ["inside", "outside", "top", "underside", "start", "end"] {
        let word = format!("Coil 1 {side}");
        assert!(words.contains(&word), "{word} not in {words:?}");
    }
}

#[test]
fn a_coil_s_values_may_be_parameters() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let pitch = transaction.add_parameter("pitch", mm(5.0));
    let coil = transaction.add_feature(
        "Coil 1",
        FeatureKind::Coil(Coil {
            length: CoilLength::TurnsAndPitch {
                turns: Expression::number(2.0),
                pitch: Expression::Parameter(pitch),
            },
            ..spring()
        }),
    );
    document.apply(transaction.finish()).unwrap();

    let kind = &document.feature(coil).unwrap().kind;

    assert!(kind.uses_parameter(pitch));
    assert_eq!(evaluate(&document).failed_count(), 0);
}
