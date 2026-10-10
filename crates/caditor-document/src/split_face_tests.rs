use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Plane, Point2, Vector3};
use caditor_kernel::{FaceReference, SamplingTolerance, Solid, Surface};
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

fn line(from: (f64, f64), to: (f64, f64)) -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(from.0, from.1), Point2::new(to.0, to.1));
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

fn top_faces(solid: &Solid) -> Vec<FaceReference> {
    solid
        .faces()
        .filter(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                plane.frame().normal() * face.sense().sign() == Vector3::Z
                    && plane.frame().origin().z > 0.0
            }
            _ => false,
        })
        .map(|(id, _)| FaceReference::capture(solid, id).unwrap())
        .collect()
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
    base: FeatureId,
    split: FeatureId,
}

fn model(along: impl FnOnce(&mut TransactionBuilder) -> SplitAlong) -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let height = transaction.add_parameter("height", transaction.parse("4 mm").unwrap());
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((-5.0, -4.0), (5.0, 4.0))),
    );
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::Parameter(height), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    let along = along(&mut transaction);
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    let faces = top_faces(evaluation.body(base).unwrap());
    let mut transaction = document.transaction("Split face");
    let split = transaction.add_feature(
        "Split face 1",
        FeatureKind::SplitFace(SplitFace {
            body: base,
            faces,
            along,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        engine,
        height,
        base,
        split,
    }
}

fn set_height(model: &mut Model, text: &str) {
    let expression = model.document.parse(text).unwrap();
    model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetParameterExpression {
                id: model.height,
                expression,
            },
        ))
        .unwrap();
}

#[test]
fn a_plane_splits_the_top_face_and_its_pieces_survive_an_upstream_edit() {
    let mut model = model(|_| SplitAlong::Plane(PlaneReference::Principal(PrincipalPlane::Yz)));

    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    let solid = evaluation.body(model.base).unwrap();
    assert_eq!(solid.faces().count(), 7);
    assert!((volume(&evaluation, model.base) - 320.0).abs() < 0.01);
    assert!(evaluation.body_before(model.split).is_some());
    let pieces = top_faces(solid);
    assert_eq!(pieces.len(), 2);

    set_height(&mut model, "6 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    let taller = evaluation.body(model.base).unwrap();
    for piece in &pieces {
        let found = piece.resolve(taller).unwrap();
        assert!(
            top_faces(taller)
                .iter()
                .any(|top| top.name() == piece.name())
        );
        assert_eq!(taller.face(found).unwrap().name(), piece.name());
    }
}

#[test]
fn an_open_sketch_curve_splits_the_faces_it_is_drawn_across() {
    let mut model = model(|transaction| {
        let sketch = transaction.add_feature(
            "Part line",
            FeatureKind::from(line((2.0, -10.0), (2.0, 10.0))),
        );
        SplitAlong::Sketch(sketch)
    });

    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(top_faces(evaluation.body(model.base).unwrap()).len(), 2);
}

#[test]
fn a_tool_missing_the_faces_fails_the_split_alone_in_words() {
    let mut model = model(|transaction| {
        let sketch = transaction.add_feature(
            "Far line",
            FeatureKind::from(line((20.0, -10.0), (20.0, 10.0))),
        );
        SplitAlong::Sketch(sketch)
    });

    let evaluation = evaluate(&model.document, &mut model.engine);

    let error = failure(&evaluation, model.split);
    assert_eq!(
        error.reason,
        "The curve of Far line does not cross any of the chosen faces."
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(model.split)));
    assert!(evaluation.body(model.base).is_some());
}
