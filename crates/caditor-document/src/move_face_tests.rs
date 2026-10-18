use std::f64::consts::PI;

use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{EdgeId, FaceId, FaceName, FaceReference, SamplingTolerance, Solid, Surface};
use caditor_sketch::Sketch;

use crate::*;

const BLOCK: f64 = 10.0 * 8.0 * 4.0;
const BORE: f64 = 4.0 * PI * 4.0;

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
        (found - expected).abs() < 0.01 * expected,
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
                (plane.frame().normal() * face.sense().sign()).dot(normal) > 0.999
                    && plane.frame().signed_distance(through).abs() < 1e-9
            }
            _ => false,
        })
        .map(|(id, _)| id)
        .unwrap()
}

fn bore(solid: &Solid) -> FaceId {
    solid
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .map(|(id, _)| id)
        .unwrap()
}

fn bore_centre(solid: &Solid) -> Point3 {
    solid
        .faces()
        .find_map(|(_, face)| match face.surface() {
            Surface::Cylinder(cylinder) => Some(cylinder.frame().origin()),
            _ => None,
        })
        .unwrap()
}

fn edge_through(solid: &Solid, middle: Point3) -> EdgeId {
    solid
        .edges()
        .find(|(_, edge)| (edge.curve().point(edge.interval().middle()) - middle).length() < 1e-9)
        .map(|(id, _)| id)
        .unwrap()
}

fn reference(solid: &Solid, face: FaceId) -> FaceReference {
    FaceReference::capture(solid, face).unwrap()
}

fn length(text: &str) -> Expression {
    Expression::parse_stored(text).unwrap()
}

struct Model {
    document: Document,
    engine: Recompute,
    lift: ParameterId,
    base: FeatureId,
    moving: FeatureId,
}

fn model() -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let lift = transaction.add_parameter("lift", transaction.parse("1 mm").unwrap());
    let outline = transaction.add_feature("Outline", FeatureKind::from(holed_plate()));
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(length("4 mm"), false),
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
    let top = flat_face(solid, Vector3::Z, Point3::new(1.0, 1.0, 4.0));
    let faces = vec![reference(solid, top)];
    let mut transaction = document.transaction("Move face");
    let moving = transaction.add_feature(
        "Move face 1",
        FeatureKind::MoveFace(MoveFace {
            body: base,
            faces,
            distance: Expression::Parameter(lift),
            direction: None,
            angle: length("0 deg"),
            axis: None,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        engine,
        lift,
        base,
        moving,
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

fn set_move(model: &mut Model, change: impl FnOnce(&mut MoveFace, &Solid)) {
    let evaluation = evaluate(&model.document, &mut model.engine);
    let before = evaluation
        .body_seen_by(model.moving, model.base)
        .unwrap()
        .clone();
    let mut definition = model
        .document
        .feature(model.moving)
        .unwrap()
        .kind
        .move_face()
        .unwrap()
        .clone();
    change(&mut definition, &before);
    model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: model.moving,
                kind: FeatureKind::MoveFace(definition),
            },
        ))
        .unwrap();
}

#[test]
fn a_moved_face_follows_its_distance_along_its_normal() {
    let mut model = model();

    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, (BLOCK - BORE) * 5.0 / 4.0);

    let lift = model.lift;
    set(&mut model, lift, "-1.5 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_volume(&evaluation, model.base, (BLOCK - BORE) * 2.5 / 4.0);
}

#[test]
fn a_wall_tilts_about_its_bottom_edge() {
    let mut model = model();
    let base = model.base;
    set_move(&mut model, |moving, solid| {
        let side = flat_face(solid, Vector3::X, Point3::new(10.0, 1.0, 1.0));
        let hinge = edge_through(solid, Point3::new(10.0, 4.0, 0.0));
        moving.faces = vec![reference(solid, side)];
        moving.distance = length("0 mm");
        moving.angle = length("10 deg");
        moving.axis = AxisReference::capture_edge(base, solid, hinge);
    });

    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(evaluation.failed_count(), 0);
    let lean = 0.5 * 4.0 * 4.0 * 10f64.to_radians().tan() * 8.0;
    let found = volume(&evaluation, model.base);
    let leaned_out = BLOCK - BORE + lean;
    let leaned_in = BLOCK - BORE - lean;
    assert!(
        (found - leaned_out).abs() < 0.01 * leaned_out
            || (found - leaned_in).abs() < 0.01 * leaned_in,
        "volume {found}"
    );
}

#[test]
fn an_imported_body_has_its_bore_moved_and_its_top_lowered() {
    let mut built = model();
    let evaluation = evaluate(&built.document, &mut built.engine);
    let solid = evaluation
        .body_seen_by(built.moving, built.base)
        .unwrap()
        .clone();

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
    let wall = reference(read, bore(read));
    let top = reference(
        read,
        flat_face(read, Vector3::Z, Point3::new(1.0, 1.0, 4.0)),
    );
    let mut transaction = document.transaction("Move faces");
    let shifted = transaction.add_feature(
        "Move face 1",
        FeatureKind::MoveFace(MoveFace {
            body: imported,
            faces: vec![wall],
            distance: length("2 mm"),
            direction: Some(AxisReference::Principal(PrincipalAxis::X)),
            angle: length("0 deg"),
            axis: None,
        }),
    );
    transaction.add_feature(
        "Move face 2",
        FeatureKind::MoveFace(MoveFace {
            body: imported,
            faces: vec![top],
            distance: length("-1 mm"),
            direction: None,
            angle: length("0 deg"),
            axis: None,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert!(evaluation.body_before(shifted).is_some());
    assert_volume(&evaluation, imported, (BLOCK - BORE) * 3.0 / 4.0);
    let centre = bore_centre(evaluation.body(imported).unwrap());
    assert!((centre.x - 7.0).abs() < 1e-9, "{centre:?}");
    assert!((centre.y - 4.0).abs() < 1e-9, "{centre:?}");
}

#[test]
fn move_face_errors_name_the_problem_and_the_fix() {
    let mut model = model();
    let lift = model.lift;

    set(&mut model, lift, "0 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.moving);
    assert_eq!(
        error.reason(),
        "The faces would not move: the distance and the angle are both nought."
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(model.moving)));

    set(&mut model, lift, "30 deg");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.moving);
    assert_eq!(
        error.remedy(),
        "Edit the distance so it gives a length, such as 2 mm."
    );

    set(&mut model, lift, "-5 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.moving);
    assert_eq!(error.remedy(), "Enter a smaller distance or angle.");
    assert!(evaluation.body(model.base).is_some());

    set(&mut model, lift, "1 mm");
    set_move(&mut model, |moving, _| moving.angle = length("5 deg"));
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.moving);
    assert_eq!(
        error.reason(),
        "The faces are to tilt, but no axis to tilt them about is chosen."
    );

    set_move(&mut model, |moving, solid| {
        moving.angle = length("0 deg");
        moving.faces = vec![reference(solid, bore(solid))];
    });
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.moving);
    assert!(error.reason().ends_with(
        "is curved and no flat face is chosen, so there is no single direction to move along."
    ));
    assert_eq!(
        error.remedy(),
        "Choose a direction: an axis, a straight edge or a round face's axis."
    );

    set_move(&mut model, |moving, _| {
        moving
            .faces
            .push(FaceReference::new(FaceName::from_digest(7), None, []));
    });
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.moving);
    assert_eq!(
        error.reason(),
        "A face to move is no longer part of the body of Base."
    );

    set_move(&mut model, |moving, _| moving.faces.clear());
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert!(matches!(
        evaluation.feature(model.moving).unwrap().state,
        FeatureState::Waiting(MissingInput::FacesToMove { .. })
    ));
}

#[test]
fn a_moved_face_stays_that_kind_and_tracks_what_it_uses() {
    let mut model = model();
    let sketch = FeatureKind::from(holed_plate());
    assert!(matches!(
        model.document.clone().apply(Transaction::single(
            "Change",
            Edit::SetFeatureKind {
                id: model.moving,
                kind: sketch
            }
        )),
        Err(EditError::KindChange(_))
    ));

    let kind = &model.document.feature(model.moving).unwrap().kind;
    assert!(kind.uses_parameter(model.lift));
    assert!(kind.features().contains(&model.base));

    let mut transaction = model.document.transaction("Axis");
    let datum = transaction.add_feature(
        "Axis 1",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Along(AxisReference::Principal(
            PrincipalAxis::Y,
        )))),
    );
    model.document.apply(transaction.finish()).unwrap();
    model
        .document
        .apply(Transaction::single(
            "Reorder",
            Edit::MoveFeature {
                id: datum,
                index: 2,
            },
        ))
        .unwrap();
    set_move(&mut model, |moving, _| {
        moving.direction = Some(AxisReference::Datum(datum));
    });
    let kind = &model.document.feature(model.moving).unwrap().kind;
    assert!(kind.dependencies().contains(&datum));
}
