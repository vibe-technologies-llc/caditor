use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{EdgeReference, FaceId, FaceReference, SamplingTolerance, Solid, Surface};
use caditor_sketch::Sketch;

use crate::*;

const LONG: f64 = 10.0;
const WIDE: f64 = 8.0;
const HIGH: f64 = 4.0;

fn rectangle(from: Point2, size: Point2) -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        from,
        Point2::new(from.x + size.x, from.y),
        from + size,
        Point2::new(from.x, from.y + size.y),
    ];
    for index in 0..4 {
        sketch.add_line(corners[index], corners[(index + 1) % 4]);
    }
    sketch
}

fn text(text: &str) -> Expression {
    Expression::parse_stored(text).unwrap()
}

fn evaluate(document: &Document, engine: &mut Recompute) -> Evaluation {
    engine.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn assert_volume(evaluation: &Evaluation, body: FeatureId, expected: f64) {
    let found = evaluation
        .body(body)
        .unwrap()
        .tessellate(&SamplingTolerance::new(1e-3, 0.05).unwrap())
        .unwrap()
        .mass_properties()
        .volume;
    assert!(
        (found - expected).abs() < 2e-3 * expected,
        "volume {found} instead of {expected}"
    );
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

fn flat_face(solid: &Solid, normal: Vector3, through: Point3) -> FaceId {
    solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                let outward = plane.frame().normal() * face.sense().sign();
                outward.dot(normal) > 0.999 && plane.frame().signed_distance(through).abs() < 1e-9
            }
            _ => false,
        })
        .map(|(id, _)| id)
        .unwrap()
}

fn extruded(
    transaction: &mut TransactionBuilder<'_>,
    name: &str,
    sketch: Sketch,
    height: &str,
) -> FeatureId {
    let outline = transaction.add_feature(format!("{name} outline"), FeatureKind::from(sketch));
    transaction.add_feature(
        name,
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(text(height), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    )
}

struct Model {
    document: Document,
    engine: Recompute,
    offset: ParameterId,
    base: FeatureId,
    tower: FeatureId,
    datum: FeatureId,
    replace: FeatureId,
}

fn model() -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let offset = transaction.add_parameter("height", transaction.parse("6 mm").unwrap());
    let base = extruded(
        &mut transaction,
        "Base",
        rectangle(Point2::ZERO, Point2::new(LONG, WIDE)),
        "4 mm",
    );
    let tower = extruded(
        &mut transaction,
        "Tower",
        rectangle(Point2::new(20.0, 0.0), Point2::new(3.0, 3.0)),
        "9 mm",
    );
    let datum = transaction.add_feature(
        "Plane 1",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Xy),
            rotation: None,
            offset: Expression::Parameter(offset),
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    let solid = evaluation.body(base).unwrap();
    let top = flat_face(solid, Vector3::Z, Point3::new(1.0, 1.0, HIGH));
    let mut transaction = document.transaction("Replace face");
    let replace = transaction.add_feature(
        "Replace face 1",
        FeatureKind::ReplaceFace(ReplaceFace {
            body: base,
            faces: vec![FaceReference::capture(solid, top).unwrap()],
            target: Some(PlaneReference::Datum(datum)),
        }),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        engine,
        offset,
        base,
        tower,
        datum,
        replace,
    }
}

fn set_replace(model: &mut Model, change: impl FnOnce(&mut ReplaceFace, &Solid)) {
    let evaluation = evaluate(&model.document, &mut model.engine);
    let before = evaluation
        .body_seen_by(model.replace, model.base)
        .unwrap()
        .clone();
    let mut definition = model
        .document
        .feature(model.replace)
        .unwrap()
        .kind
        .replace_face()
        .unwrap()
        .clone();
    change(&mut definition, &before);
    model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: model.replace,
                kind: FeatureKind::ReplaceFace(definition),
            },
        ))
        .unwrap();
}

fn tower_top(model: &mut Model) -> PlaneReference {
    let evaluation = evaluate(&model.document, &mut model.engine);
    let tower = evaluation.body(model.tower).unwrap();
    let top = flat_face(tower, Vector3::Z, Point3::new(21.0, 1.0, 9.0));
    PlaneReference::Face(FaceAttachment::capture(model.tower, tower, top).unwrap().0)
}

#[test]
fn a_top_replaced_by_a_datum_plane_follows_the_plane_and_undoes() {
    let mut model = model();

    let evaluation = evaluate(&model.document, &mut model.engine);
    let kind = &model.document.feature(model.replace).unwrap().kind;
    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, LONG * WIDE * 6.0);
    assert!(kind.dependencies().contains(&model.datum));

    let inverse = model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetParameterExpression {
                id: model.offset,
                expression: model.document.parse("2.5 mm").unwrap(),
            },
        ))
        .unwrap();
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_volume(&evaluation, model.base, LONG * WIDE * 2.5);

    model.document.apply(inverse).unwrap();
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_volume(&evaluation, model.base, LONG * WIDE * 6.0);
}

#[test]
fn a_top_takes_the_plane_of_another_body_s_face() {
    let mut model = model();
    let target = tower_top(&mut model);

    set_replace(&mut model, |replace, _| replace.target = Some(target));
    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, LONG * WIDE * 9.0);
    assert!(
        model
            .document
            .feature(model.replace)
            .unwrap()
            .kind
            .dependencies()
            .contains(&model.tower)
    );
}

#[test]
fn a_face_of_an_imported_body_is_replaced_by_a_slanted_plane() {
    let mut model = model();
    let evaluation = evaluate(&model.document, &mut model.engine);
    let solid = evaluation
        .body_seen_by(model.replace, model.base)
        .unwrap()
        .clone();
    let mut document = Document::default();
    let mut transaction = document.transaction("Import");
    let imported = transaction.add_feature(
        "Block",
        FeatureKind::Import(Import::new("block.step", solid, "")),
    );
    let slanted = transaction.add_feature(
        "Slant",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Xy),
            rotation: Some(PlaneRotation {
                axis: AxisReference::Principal(PrincipalAxis::Y),
                angle: text("10 deg"),
            }),
            offset: text("6 mm"),
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    let read = evaluation.body(imported).unwrap();
    let top = flat_face(read, Vector3::Z, Point3::new(1.0, 1.0, HIGH));
    let mut transaction = document.transaction("Replace face");
    let replace = transaction.add_feature(
        "Replace face 1",
        FeatureKind::ReplaceFace(ReplaceFace {
            body: imported,
            faces: vec![FaceReference::capture(read, top).unwrap()],
            target: Some(PlaneReference::Datum(slanted)),
        }),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut engine);

    assert_eq!(
        evaluation.failed_count(),
        0,
        "{:?}",
        evaluation.feature(replace)
    );
    let result = evaluation.body(imported).unwrap();
    let (sin, cos) = 10f64.to_radians().sin_cos();
    let volume = result
        .tessellate(&SamplingTolerance::new(1e-3, 0.05).unwrap())
        .unwrap()
        .mass_properties()
        .volume;
    let leaning = |sign: f64| LONG * WIDE * (6.0 + sign * LONG / 2.0 * sin) / cos;
    assert_eq!(result.faces().count(), 6);
    assert!(
        [leaning(1.0), leaning(-1.0)]
            .iter()
            .any(|expected| (volume - expected).abs() < 2e-3 * expected),
        "volume {volume}"
    );
    assert!(result.faces().any(|(_, face)| match face.surface() {
        Surface::Plane(plane) => {
            let normal = plane.frame().normal();
            normal.z.abs() < 0.99 && normal.z.abs() > 0.9
        }
        _ => false,
    }));
}

#[test]
fn replacing_errors_name_the_face_and_the_fix() {
    let mut model = model();

    set_replace(&mut model, |replace, _| {
        replace.target = Some(PlaneReference::Principal(PrincipalPlane::Xz));
    });
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.replace);
    assert!(
        error
            .reason()
            .ends_with("runs square to the XZ plane, so it cannot be turned onto it."),
        "{}",
        error.reason()
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(model.replace)));

    set_replace(&mut model, |replace, _| {
        replace.target = Some(PlaneReference::Principal(PrincipalPlane::Xy));
    });
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.replace);
    assert!(
        error
            .reason()
            .starts_with("The XY plane lies on or beyond the far side of the body from"),
        "{}",
        error.reason()
    );
    assert!(evaluation.body(model.base).is_some());

    let base = model.base;
    set_replace(&mut model, |replace, solid| {
        let top = flat_face(solid, Vector3::Z, Point3::new(1.0, 1.0, HIGH));
        replace.target = Some(PlaneReference::Face(
            FaceAttachment::capture(base, solid, top).unwrap().0,
        ));
    });
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.replace);
    assert!(
        error
            .reason()
            .starts_with("Every chosen face already lies on"),
        "{}",
        error.reason()
    );

    set_replace(&mut model, |replace, _| replace.target = None);
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert!(matches!(
        evaluation.feature(model.replace).unwrap().state,
        FeatureState::Waiting(MissingInput::TargetPlane { .. })
    ));

    set_replace(&mut model, |replace, _| replace.faces.clear());
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert!(matches!(
        evaluation.feature(model.replace).unwrap().state,
        FeatureState::Waiting(MissingInput::FacesToReplace { .. })
    ));
}

#[test]
fn a_curved_face_is_refused_by_name() {
    let mut model = model();
    let mut transaction = model.document.transaction("Round");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let solid = evaluation
        .body_seen_by(model.replace, model.base)
        .unwrap()
        .clone();
    let corner = solid
        .edges()
        .find(|(_, edge)| {
            let middle = edge.curve().point(edge.interval().middle());
            (middle - Point3::new(LONG, WIDE, HIGH / 2.0)).length() < 1e-6
        })
        .map(|(id, _)| id)
        .unwrap();
    let round = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: model.base,
            groups: Vec::new(),
            edges: vec![EdgeReference::capture(&solid, corner).unwrap()],
            size: text("1 mm"),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    model.document.apply(transaction.finish()).unwrap();
    model
        .document
        .apply(Transaction::single(
            "Reorder",
            Edit::MoveFeature {
                id: round,
                index: model.document.feature_index(model.replace).unwrap(),
            },
        ))
        .unwrap();
    set_replace(&mut model, |replace, solid| {
        let rounded = solid
            .faces()
            .find(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
            .map(|(id, _)| id)
            .unwrap();
        replace.faces = vec![FaceReference::capture(solid, rounded).unwrap()];
    });
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.replace);

    assert!(
        error
            .reason()
            .ends_with("is curved; only flat faces can be replaced."),
        "{}",
        error.reason()
    );
    assert_eq!(error.remedy(), "Leave this face out.");
}
