use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{
    EdgeReference, FaceId, FaceName, FaceReference, SamplingTolerance, Solid, Surface,
};
use caditor_sketch::Sketch;

use crate::*;

const BLOCK: f64 = 10.0 * 8.0 * 4.0;

fn holed_plate() -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 0.0),
        Point2::new(10.0, 8.0),
        Point2::new(0.0, 8.0),
    ];
    for index in 0..4 {
        sketch.add_line(corners[index], corners[(index + 1) % 4]);
    }
    sketch.add_circle(Point2::new(5.0, 4.0), 2.0);
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

fn assert_volume(evaluation: &Evaluation, body: FeatureId, expected: f64) {
    let found = volume(evaluation, body);
    assert!(
        (found - expected).abs() < 0.002 * expected,
        "volume {found} instead of {expected}"
    );
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

fn round_faces(solid: &Solid, radius: f64) -> Vec<FaceId> {
    solid
        .faces()
        .filter(|(_, face)| match face.surface() {
            Surface::Cylinder(cylinder) => (cylinder.radius() - radius).abs() < 1e-9,
            _ => false,
        })
        .map(|(id, _)| id)
        .collect()
}

fn references(solid: &Solid, faces: &[FaceId]) -> Vec<FaceReference> {
    faces
        .iter()
        .map(|face| FaceReference::capture(solid, *face).unwrap())
        .collect()
}

fn top_face(solid: &Solid) -> FaceId {
    solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                plane.frame().normal() * face.sense().sign() == Vector3::Z
                    && plane.frame().origin().z > 0.0
            }
            _ => false,
        })
        .map(|(id, _)| id)
        .unwrap()
}

struct Model {
    document: Document,
    engine: Recompute,
    base: FeatureId,
    deletion: FeatureId,
}

fn rounded_plate() -> (Document, Recompute, FeatureId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature("Outline", FeatureKind::from(holed_plate()));
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("4 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    let solid = evaluation.body(base).unwrap();
    let front_top = solid
        .edges()
        .find(|(_, edge)| {
            let middle = edge.curve().point(edge.interval().middle());
            middle.distance(Point3::new(5.0, 0.0, 4.0)) < 1e-9
        })
        .map(|(id, _)| EdgeReference::capture(solid, id).unwrap())
        .unwrap();
    let mut transaction = document.transaction("Fillet");
    transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: base,
            groups: Vec::new(),
            edges: vec![front_top],
            size: Expression::parse_stored("1 mm").unwrap(),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    (document, engine, base)
}

fn model() -> Model {
    let (mut document, mut engine, base) = rounded_plate();
    let evaluation = evaluate(&document, &mut engine);
    let solid = evaluation.body(base).unwrap();
    let mut chosen = round_faces(solid, 1.0);
    chosen.extend(round_faces(solid, 2.0));
    let faces = references(solid, &chosen);
    let mut transaction = document.transaction("Delete face");
    let deletion = transaction.add_feature(
        "Delete face 1",
        FeatureKind::DeleteFace(DeleteFace { body: base, faces }),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        engine,
        base,
        deletion,
    }
}

fn set_faces(model: &mut Model, change: impl FnOnce(&mut DeleteFace)) {
    let mut definition = model
        .document
        .feature(model.deletion)
        .unwrap()
        .kind
        .delete_face()
        .unwrap()
        .clone();
    change(&mut definition);
    model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: model.deletion,
                kind: FeatureKind::DeleteFace(definition),
            },
        ))
        .unwrap();
}

#[test]
fn deleting_a_fillet_and_a_bore_leaves_the_plain_block() {
    let mut model = model();
    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, BLOCK);
    assert!(evaluation.body_before(model.deletion).is_some());
    assert_eq!(evaluation.body(model.base).unwrap().faces().count(), 6);
}

#[test]
fn delete_face_errors_name_the_problem_and_the_fix() {
    let mut model = model();
    let evaluation = evaluate(&model.document, &mut model.engine);
    let before = evaluation
        .body_before(model.deletion)
        .and_then(|result| result.solid())
        .map(|result| result.solid.clone())
        .unwrap();
    let top = references(&before, &[top_face(&before)]);

    set_faces(&mut model, |deletion| deletion.faces = top);
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.deletion);
    assert_eq!(
        error.reason,
        "The faces around Base end face cannot close the gap it leaves."
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(model.deletion)));
    assert!(evaluation.body(model.base).is_some());

    set_faces(&mut model, |deletion| {
        deletion
            .faces
            .push(FaceReference::new(FaceName::from_digest(7), None, []));
    });
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(
        failure(&evaluation, model.deletion).reason,
        "A face to delete is no longer part of the body of Base."
    );

    set_faces(&mut model, |deletion| deletion.faces.clear());
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert!(matches!(
        evaluation.feature(model.deletion).unwrap().state,
        FeatureState::Waiting(MissingInput::FacesToDelete { .. })
    ));
}

#[test]
fn a_deleted_face_stays_that_kind_and_keeps_its_body_in_use() {
    let model = model();
    let sketch = FeatureKind::from(holed_plate());

    assert!(matches!(
        model.document.clone().apply(Transaction::single(
            "Change",
            Edit::SetFeatureKind {
                id: model.deletion,
                kind: sketch
            }
        )),
        Err(EditError::KindChange(_))
    ));
    assert!(
        model
            .document
            .dependents_of(&[model.base])
            .contains(&model.deletion)
    );
}

#[test]
fn an_imported_body_loses_its_fillet_and_bore_and_later_features_build_on_it() {
    let (document, mut engine, base) = rounded_plate();
    let evaluation = evaluate(&document, &mut engine);
    let solid = evaluation.body(base).unwrap().clone();

    let mut document = Document::default();
    let mut transaction = document.transaction("Import");
    let imported = transaction.add_feature(
        "Bracket",
        FeatureKind::Import(Import::new("bracket.step", solid, "")),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    let read = evaluation.body(imported).unwrap();
    let mut chosen = round_faces(read, 1.0);
    chosen.extend(round_faces(read, 2.0));
    let faces = references(read, &chosen);
    let top = references(read, &[top_face(read)]);
    let mut transaction = document.transaction("Delete face");
    let deletion = transaction.add_feature(
        "Delete face 1",
        FeatureKind::DeleteFace(DeleteFace {
            body: imported,
            faces,
        }),
    );
    transaction.add_feature(
        "Offset face 1",
        FeatureKind::OffsetFace(OffsetFace {
            body: imported,
            faces: top,
            distance: Expression::parse_stored("1 mm").unwrap(),
            tangent: false,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert!(evaluation.body_before(deletion).is_some());
    assert_volume(&evaluation, imported, BLOCK + 80.0);
}
