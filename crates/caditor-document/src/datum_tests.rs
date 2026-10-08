use std::f64::consts::PI;

use caditor_expression::{Expression, ParameterId, Unit};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{EdgeReference, FaceReference, SamplingTolerance, Solid, Surface};
use caditor_sketch::Sketch;

use crate::*;

fn rectangle(plane: Plane, min: (f64, f64), max: (f64, f64)) -> Sketch {
    let mut sketch = Sketch::new(plane);
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

fn millimetres(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

fn degrees(value: f64) -> Expression {
    Expression::Measure(value, Unit::Degree)
}

fn datum(evaluation: &Evaluation, feature: FeatureId) -> DatumResult {
    match evaluation.feature(feature).unwrap() {
        FeatureStatus {
            state: FeatureState::UpToDate,
            result: Some(result),
            ..
        } => *result.datum().unwrap(),
        other => panic!("the datum failed: {other:?}"),
    }
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

fn sketch_plane(evaluation: &Evaluation, feature: FeatureId) -> Plane {
    evaluation
        .feature(feature)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::sketch)
        .unwrap()
        .geometry
        .plane()
}

fn close(a: Vector3, b: Vector3) -> bool {
    (a - b).length() < 1e-9
}

fn set(document: &mut Document, parameter: ParameterId, text: &str) {
    let expression = document.parse(text).unwrap();
    document
        .apply(Transaction::single(
            "Edit",
            Edit::SetParameterExpression {
                id: parameter,
                expression,
            },
        ))
        .unwrap();
}

fn add(document: &mut Document, name: &str, kind: FeatureKind) -> FeatureId {
    let mut transaction = document.transaction("Add");
    let feature = transaction.add_feature(name, kind);
    document.apply(transaction.finish()).unwrap();
    feature
}

fn offset_plane(base: PlaneReference, offset: Expression) -> FeatureKind {
    FeatureKind::Datum(Datum::Plane(DatumPlane {
        base,
        rotation: None,
        offset,
    }))
}

struct Block {
    document: Document,
    height: ParameterId,
    base: FeatureId,
}

fn block() -> Block {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let height = transaction.add_parameter("height", transaction.parse("4 mm").unwrap());
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (10.0, 8.0))),
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
        })),
    );
    document.apply(transaction.finish()).unwrap();
    Block {
        document,
        height,
        base,
    }
}

fn top_face(solid: &Solid) -> FaceReference {
    let (id, _) = solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                close(plane.frame().normal() * face.sense().sign(), Vector3::Z)
                    && plane.frame().origin().z > 0.0
            }
            _ => false,
        })
        .unwrap();
    FaceReference::capture(solid, id).unwrap()
}

fn edge_along(solid: &Solid, middle: Point3) -> EdgeReference {
    let (id, _) = solid
        .edges()
        .find(|(_, edge)| {
            edge.curve()
                .point(edge.interval().middle())
                .distance(middle)
                < 1e-6
        })
        .unwrap();
    EdgeReference::capture(solid, id).unwrap()
}

#[test]
fn an_offset_plane_carries_a_sketch_and_follows_its_parameter() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let gap = transaction.add_parameter("gap", transaction.parse("15 mm").unwrap());
    document.apply(transaction.finish()).unwrap();
    let plane = add(
        &mut document,
        "Plane 1",
        offset_plane(
            PlaneReference::Principal(PrincipalPlane::Xy),
            Expression::Parameter(gap),
        ),
    );
    let sketch = add(
        &mut document,
        "Sketch 1",
        FeatureKind::Sketch(SketchFeature::on_datum(
            rectangle(Plane::XY, (0.0, 0.0), (2.0, 2.0)),
            plane,
        )),
    );
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(sketch_plane(&evaluation, sketch).origin().z, 15.0);

    set(&mut document, gap, "-5 mm");
    let evaluation = evaluate(&document, &mut engine);
    assert_eq!(sketch_plane(&evaluation, sketch).origin().z, -5.0);

    set(&mut document, gap, "5 deg");
    let evaluation = evaluate(&document, &mut engine);
    let error = failure(&evaluation, plane);
    assert_eq!(
        error.remedy,
        "Edit the offset so it gives a length, such as 10 mm."
    );
    assert_eq!(
        failure(&evaluation, sketch).fix,
        Some(FixTarget::Feature(plane))
    );
}

#[test]
fn a_rotated_plane_turns_about_its_axis_before_the_offset() {
    let mut document = Document::default();
    let plane = add(
        &mut document,
        "Plane 1",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Xy),
            rotation: Some(PlaneRotation {
                axis: AxisReference::Principal(PrincipalAxis::Y),
                angle: degrees(90.0),
            }),
            offset: millimetres(3.0),
        })),
    );
    let evaluation = evaluate(&document, &mut Recompute::default());
    let found = datum(&evaluation, plane).plane().unwrap();
    assert!(close(found.normal(), Vector3::X), "{found:?}");
    assert!(
        close(found.origin(), Point3::new(3.0, 0.0, 0.0)),
        "{found:?}"
    );
}

#[test]
fn a_plane_cannot_turn_about_an_axis_that_crosses_it() {
    let mut document = Document::default();
    let plane = add(
        &mut document,
        "Plane 1",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Xy),
            rotation: Some(PlaneRotation {
                axis: AxisReference::Principal(PrincipalAxis::Z),
                angle: degrees(45.0),
            }),
            offset: millimetres(0.0),
        })),
    );
    let evaluation = evaluate(&document, &mut Recompute::default());
    let error = failure(&evaluation, plane);
    assert_eq!(
        error.reason,
        "The Z axis does not run along the XY plane, so turning the plane about it cannot work."
    );
}

#[test]
fn datums_on_body_faces_and_edges_follow_the_body() {
    let Block {
        mut document,
        height,
        base,
    } = block();
    let evaluation = evaluate(&document, &mut Recompute::default());
    let solid = evaluation.body(base).unwrap();
    let top = top_face(solid);
    let edge = edge_along(solid, Point3::new(5.0, 0.0, 4.0));
    let above = add(
        &mut document,
        "Plane 1",
        offset_plane(
            PlaneReference::Face(FaceAttachment {
                body: base,
                face: top,
            }),
            millimetres(2.0),
        ),
    );
    let along = add(
        &mut document,
        "Axis 1",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Along(AxisReference::Edge {
            body: base,
            edge: Box::new(edge),
        }))),
    );
    let crossing = add(
        &mut document,
        "Axis 2",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Intersection(
            PlaneReference::Datum(above),
            PlaneReference::Principal(PrincipalPlane::Yz),
        ))),
    );
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(datum(&evaluation, above).plane().unwrap().origin().z, 6.0);
    let axis = datum(&evaluation, along).axis().unwrap();
    assert!(close(axis.direction().abs(), Vector3::X));
    assert!((axis.origin().z - 4.0).abs() < 1e-9);
    let line = datum(&evaluation, crossing).axis().unwrap();
    assert!(close(line.direction().abs(), Vector3::Y));
    assert!(close(
        line.origin() - Vector3::Y * line.origin().y,
        Point3::new(0.0, 0.0, 6.0)
    ));

    set(&mut document, height, "7 mm");
    let evaluation = evaluate(&document, &mut engine);
    assert_eq!(datum(&evaluation, above).plane().unwrap().origin().z, 9.0);
    assert!((datum(&evaluation, along).axis().unwrap().origin().z - 7.0).abs() < 1e-9);

    let parallel = add(
        &mut document,
        "Axis 3",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Intersection(
            PlaneReference::Datum(above),
            PlaneReference::Principal(PrincipalPlane::Xy),
        ))),
    );
    let evaluation = evaluate(&document, &mut engine);
    assert_eq!(
        failure(&evaluation, parallel).reason,
        "Plane 1 and the XY plane are parallel, so they do not meet in a line."
    );
}

#[test]
fn a_revolve_turns_about_a_datum_axis_in_its_sketch_plane() {
    let mut document = Document::default();
    let axis = add(
        &mut document,
        "Axis 1",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Intersection(
            PlaneReference::Principal(PrincipalPlane::Xz),
            PlaneReference::Principal(PrincipalPlane::Yz),
        ))),
    );
    let section = add(
        &mut document,
        "Section",
        FeatureKind::from(rectangle(Plane::XZ, (2.0, 0.0), (3.0, 1.0))),
    );
    let revolve = |axis| {
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch: section,
            regions: RegionChoice::All,
            axis,
            extent: RevolveExtent::Full,
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            side: None,
        }))
    };
    let ring = add(
        &mut document,
        "Ring",
        revolve(RevolveAxis::Model(AxisReference::Datum(axis))),
    );
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    assert_eq!(evaluation.failed_count(), 0);
    let volume = evaluation
        .body(ring)
        .unwrap()
        .tessellate(&SamplingTolerance::new(1e-3, 0.02).unwrap())
        .unwrap()
        .mass_properties()
        .volume;
    let expected = PI * (9.0 - 4.0);
    assert!((volume - expected).abs() < 0.01 * expected, "{volume}");

    let tilted = add(
        &mut document,
        "Tilted",
        revolve(RevolveAxis::Model(AxisReference::Principal(
            PrincipalAxis::Y,
        ))),
    );
    let evaluation = evaluate(&document, &mut engine);
    assert_eq!(
        failure(&evaluation, tilted).reason,
        "The Y axis does not lie in the plane of Section, so the sketch cannot turn about it."
    );
}

#[test]
fn datum_references_are_checked_and_kept_in_use() {
    let mut document = Document::default();
    let axis = add(
        &mut document,
        "Axis 1",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Along(AxisReference::Principal(
            PrincipalAxis::Z,
        )))),
    );
    let mut transaction = document.transaction("Add");
    transaction.add_feature(
        "Sketch 1",
        FeatureKind::Sketch(SketchFeature::on_datum(Sketch::new(Plane::XY), axis)),
    );
    assert_eq!(
        document.apply(transaction.finish()),
        Err(EditError::NotAPlane("Axis 1".to_owned()))
    );

    let plane = add(
        &mut document,
        "Plane 1",
        offset_plane(
            PlaneReference::Principal(PrincipalPlane::Xz),
            Expression::Number(1.0),
        ),
    );
    add(
        &mut document,
        "Sketch 1",
        FeatureKind::Sketch(SketchFeature::on_datum(Sketch::new(Plane::XZ), plane)),
    );
    assert_eq!(document.dependents_of(&[plane]).len(), 1);
    assert!(matches!(
        document.apply(Transaction::single(
            "Change",
            Edit::SetFeatureKind {
                id: plane,
                kind: FeatureKind::Datum(Datum::Axis(DatumAxis::Along(AxisReference::Principal(
                    PrincipalAxis::X
                )))),
            }
        )),
        Err(EditError::KindChange(_))
    ));
    assert!(matches!(
        document.apply(Transaction::single(
            "Move",
            Edit::MoveFeature {
                id: plane,
                index: 2
            }
        )),
        Err(EditError::BelowDependent { .. })
    ));
}

#[test]
fn a_revolve_axis_is_shown_where_the_revolve_found_it() {
    let Block {
        mut document, base, ..
    } = block();
    let evaluation = evaluate(&document, &mut Recompute::default());
    let edge = edge_along(evaluation.body(base).unwrap(), Point3::new(5.0, 0.0, 0.0));
    let axis = AxisReference::Edge {
        body: base,
        edge: Box::new(edge),
    };
    let section = add(
        &mut document,
        "Section",
        FeatureKind::from(rectangle(Plane::XZ, (2.0, 1.0), (3.0, 2.0))),
    );
    let ring = add(
        &mut document,
        "Ring",
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch: section,
            regions: RegionChoice::All,
            axis: RevolveAxis::Model(axis.clone()),
            extent: RevolveExtent::Full,
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            side: None,
        })),
    );
    let strip = add(
        &mut document,
        "Strip",
        FeatureKind::from(rectangle(Plane::XY, (-1.0, -1.0), (11.0, 1.0))),
    );
    add(
        &mut document,
        "Chamfer cut",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: strip,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(millimetres(1.0), false),
            operation: BodyOperation::Remove(base),
            start: None,
            other_bodies: Vec::new(),
        })),
    );
    let evaluation = evaluate(&document, &mut Recompute::default());
    assert_eq!(evaluation.failed_count(), 0);
    let shown = displayed_axis(&evaluation, ring, &axis).unwrap();
    assert!(close(shown.direction().abs(), Vector3::X), "{shown:?}");
    assert!(shown.origin().y.abs() < 1e-9 && shown.origin().z.abs() < 1e-9);
}

#[test]
fn restoring_an_earlier_version_brings_back_datums_attached_sketches_and_imports() {
    let Block {
        mut document,
        height,
        base,
    } = block();
    let evaluation = evaluate(&document, &mut Recompute::default());
    let solid = evaluation.body(base).unwrap().clone();
    let top = top_face(&solid);

    let plane = add(
        &mut document,
        "Plane 1",
        offset_plane(
            PlaneReference::Principal(PrincipalPlane::Xy),
            millimetres(-3.0),
        ),
    );
    let on_plane = add(
        &mut document,
        "Below",
        FeatureKind::Sketch(SketchFeature::on_datum(
            rectangle(Plane::XY, (0.0, 0.0), (2.0, 2.0)),
            plane,
        )),
    );
    let on_face = add(
        &mut document,
        "On top",
        FeatureKind::Sketch(SketchFeature::on_face(
            rectangle(Plane::XY, (1.0, 1.0), (3.0, 3.0)),
            FaceAttachment {
                body: base,
                face: top,
            },
        )),
    );
    let imported = add(
        &mut document,
        "Copy",
        FeatureKind::Import(Import::new("copy.step", solid, "")),
    );
    let earlier = document.clone();

    for feature in [imported, on_face, on_plane, plane] {
        document
            .apply(Transaction::single(
                "Delete",
                Edit::RemoveFeature { id: feature },
            ))
            .unwrap();
    }
    set(&mut document, height, "9 mm");
    let later = document.clone();
    let mut editor = Editor::new(later.clone());
    editor
        .apply(later.transaction_to(&earlier, "Restore"))
        .unwrap();
    let evaluation = evaluate(editor.document(), &mut Recompute::default());

    assert!(editor.document().same_content(&earlier));
    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(sketch_plane(&evaluation, on_plane).origin().z, -3.0);
    assert!((sketch_plane(&evaluation, on_face).origin().z - 4.0).abs() < 1e-9);
    assert!(evaluation.body(imported).is_some());

    editor.undo().unwrap();

    assert!(editor.document().same_content(&later));
    assert!(editor.document().feature(imported).is_none());
}
