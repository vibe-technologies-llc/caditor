use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Plane, Point2, Vector3};
use caditor_kernel::{FaceName, FaceReference, SamplingTolerance, Solid, Surface};
use caditor_sketch::Sketch;

use crate::*;

fn rectangle(min: (f64, f64), max: (f64, f64)) -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        Point2::new(min.0, min.1),
        Point2::new(max.0, min.1),
        Point2::new(max.0, max.1),
        Point2::new(min.0, max.1),
    ];
    for index in 0..4 {
        sketch.add_line(corners[index], corners[(index + 1) % 4]);
    }
    sketch
}

fn evaluate(document: &Document, engine: &mut Recompute) -> Evaluation {
    engine.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
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

fn top_face(solid: &Solid) -> FaceReference {
    let (id, _) = solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                plane.frame().normal() * face.sense().sign() == Vector3::Z
                    && plane.frame().origin().z > 0.0
            }
            _ => false,
        })
        .unwrap();
    FaceReference::capture(solid, id).unwrap()
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

struct Model {
    document: Document,
    engine: Recompute,
    height: ParameterId,
    wall: ParameterId,
    base: FeatureId,
    shell: FeatureId,
}

fn model() -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let height = transaction.add_parameter("height", transaction.parse("4 mm").unwrap());
    let wall = transaction.add_parameter("wall", transaction.parse("1 mm").unwrap());
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((0.0, 0.0), (10.0, 8.0))),
    );
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::OneSide {
                distance: Expression::Parameter(height),
                reversed: false,
            },
            operation: BodyOperation::NewBody,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    let open = vec![top_face(evaluation.body(base).unwrap())];
    let mut transaction = document.transaction("Shell");
    let shell = transaction.add_feature(
        "Shell 1",
        FeatureKind::Shell(Shell {
            body: base,
            open,
            thickness: Expression::Parameter(wall),
        }),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        engine,
        height,
        wall,
        base,
        shell,
    }
}

fn set(model: &mut Model, parameter: ParameterId, text: &str) {
    let expression = model.document.parse(text).unwrap();
    model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetParameterExpression {
                id: parameter,
                expression,
            },
        ))
        .unwrap();
}

fn set_shell(model: &mut Model, change: impl FnOnce(&mut Shell)) -> Result<(), EditError> {
    let mut definition = model
        .document
        .feature(model.shell)
        .unwrap()
        .kind
        .shell()
        .unwrap()
        .clone();
    change(&mut definition);
    model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: model.shell,
                kind: FeatureKind::Shell(definition),
            },
        ))
        .map(|_| ())
}

fn assert_volume(evaluation: &Evaluation, body: FeatureId, expected: f64) {
    let found = volume(evaluation, body);
    assert!(
        (found - expected).abs() < 0.01 * expected,
        "volume {found} instead of {expected}"
    );
}

#[test]
fn a_shell_hollows_its_body_and_follows_upstream_edits() {
    let mut model = model();
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(
        evaluation.bodies().collect::<Vec<_>>(),
        vec![(model.base, model.shell)]
    );
    assert_volume(&evaluation, model.base, 320.0 - 8.0 * 6.0 * 3.0);
    assert!(evaluation.body_before(model.shell).is_some());

    let id = model.height;
    set(&mut model, id, "2 mm");
    let id = model.wall;
    set(&mut model, id, "1.5 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, 160.0 - 7.0 * 5.0 * 0.5);

    set_shell(&mut model, |shell| shell.open.clear()).unwrap();
    let id = model.height;
    set(&mut model, id, "4 mm");
    let id = model.wall;
    set(&mut model, id, "1 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, 320.0 - 8.0 * 6.0 * 2.0);
}

#[test]
fn shell_errors_name_the_problem_and_the_fix() {
    let mut model = model();
    let id = model.wall;
    set(&mut model, id, "0 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.shell);
    assert_eq!(error.reason, "The thickness must be more than zero.");
    assert_eq!(error.fix, Some(FixTarget::Feature(model.shell)));

    set(&mut model, id, "30 deg");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.shell);
    assert_eq!(
        error.remedy,
        "Edit the thickness so it gives a length, such as 2 mm."
    );

    set(&mut model, id, "4.5 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.shell);
    assert_eq!(
        error.reason,
        "The wall along the edge between Base side from Line 5 and Base start face shrinks to \
         nothing at this thickness."
    );
    assert_eq!(error.remedy, "Enter a smaller thickness.");
    assert!(evaluation.body(model.base).is_some());

    set(&mut model, id, "1 mm");
    set_shell(&mut model, |shell| {
        shell
            .open
            .push(FaceReference::new(FaceName::from_digest(7), None, []));
    })
    .unwrap();
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.shell);
    assert_eq!(
        error.reason,
        "A face to open is no longer part of the body of Base."
    );
}

#[test]
fn a_shell_stays_a_shell_and_keeps_its_body_in_use() {
    let mut model = model();
    let sketch = FeatureKind::from(rectangle((0.0, 0.0), (1.0, 1.0)));
    assert!(matches!(
        model.document.apply(Transaction::single(
            "Change",
            Edit::SetFeatureKind {
                id: model.shell,
                kind: sketch
            }
        )),
        Err(EditError::KindChange(_))
    ));
    assert!(matches!(
        model.document.apply(Transaction::single(
            "Delete",
            Edit::RemoveFeature { id: model.base }
        )),
        Err(EditError::FeatureInUse { .. })
    ));
}
