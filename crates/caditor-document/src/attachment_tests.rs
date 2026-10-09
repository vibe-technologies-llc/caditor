use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_kernel::{FaceName, FaceOrigin, FaceReference, Solid};
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
    for (index, corner) in corners.iter().enumerate() {
        sketch.add_line(*corner, corners[(index + 1) % 4]);
    }
    sketch
}

fn extrude(sketch: FeatureId, distance: Expression, operation: BodyOperation) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent: ExtrudeExtent::one_side(distance, false),
        operation,
        start: None,
        other_bodies: Vec::new(),
        taper: None,
        wall: None,
    }))
}

fn evaluate(document: &Document, engine: &mut Recompute) -> Evaluation {
    engine.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn end_cap(solid: &Solid, feature: FeatureId) -> caditor_kernel::FaceId {
    solid
        .faces()
        .find(|(_, face)| {
            face.origin()
                == Some(FaceOrigin::EndCap {
                    feature: feature.raw(),
                })
        })
        .map(|(id, _)| id)
        .unwrap()
}

fn sketch_plane(evaluation: &Evaluation, sketch: FeatureId) -> Plane {
    evaluation
        .feature(sketch)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::sketch)
        .map(|result| result.geometry.plane())
        .unwrap()
}

fn volume(evaluation: &Evaluation, body: FeatureId) -> f64 {
    evaluation
        .body(body)
        .unwrap()
        .tessellate(&caditor_kernel::SamplingTolerance::new(1e-3, 0.1).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

struct Stack {
    document: Document,
    height: caditor_expression::ParameterId,
    base: FeatureId,
    top: FeatureId,
    boss: FeatureId,
    cap: FeatureId,
}

fn stack() -> Stack {
    let mut document = Document::default();
    let mut transaction = document.transaction("Base");
    let height = transaction.add_parameter("height", transaction.parse("4 mm").unwrap());
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (10.0, 8.0))),
    );
    let base = transaction.add_feature(
        "Base",
        extrude(
            outline,
            Expression::Parameter(height),
            BodyOperation::NewBody,
        ),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();

    let solid = evaluate(&document, &mut engine).body(base).unwrap().clone();
    let (attachment, plane) = FaceAttachment::capture(base, &solid, end_cap(&solid, base)).unwrap();
    assert_eq!(plane.origin().z, 4.0);
    assert_eq!(plane.normal().z, 1.0);
    let mut transaction = document.transaction("Boss");
    let top = transaction.add_feature(
        "Top sketch",
        FeatureKind::Sketch(SketchFeature::on_face(
            rectangle(plane, (2.0, 2.0), (4.0, 4.0)),
            attachment,
        )),
    );
    let boss = transaction.add_feature(
        "Boss",
        extrude(
            top,
            Expression::parse_stored("3").unwrap(),
            BodyOperation::Add(base),
        ),
    );
    document.apply(transaction.finish()).unwrap();

    let solid = evaluate(&document, &mut engine).body(base).unwrap().clone();
    let (attachment, plane) = FaceAttachment::capture(base, &solid, end_cap(&solid, boss)).unwrap();
    let mut transaction = document.transaction("Cap");
    let cap = transaction.add_feature(
        "Cap sketch",
        FeatureKind::Sketch(SketchFeature::on_face(
            rectangle(plane, (2.5, 2.5), (3.5, 3.5)),
            attachment,
        )),
    );
    document.apply(transaction.finish()).unwrap();
    Stack {
        document,
        height,
        base,
        top,
        boss,
        cap,
    }
}

#[test]
fn sketches_on_faces_follow_the_faces_when_the_body_changes() {
    let mut stack = stack();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&stack.document, &mut engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(sketch_plane(&evaluation, stack.top).origin().z, 4.0);
    assert_eq!(sketch_plane(&evaluation, stack.cap).origin().z, 7.0);
    assert!((volume(&evaluation, stack.base) - 332.0).abs() < 0.05);

    let taller = Transaction::single(
        "Taller",
        Edit::SetParameterExpression {
            id: stack.height,
            expression: stack.document.parse("6 mm").unwrap(),
        },
    );
    stack.document.apply(taller).unwrap();
    let evaluation = evaluate(&stack.document, &mut engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(sketch_plane(&evaluation, stack.top).origin().z, 6.0);
    assert_eq!(sketch_plane(&evaluation, stack.cap).origin().z, 9.0);
    assert!((volume(&evaluation, stack.base) - 492.0).abs() < 0.05);
    assert_eq!(
        stack
            .document
            .feature(stack.top)
            .unwrap()
            .kind
            .sketch()
            .unwrap()
            .plane()
            .origin()
            .z,
        4.0
    );
}

#[test]
fn a_lost_face_fails_the_sketch_and_its_users_only() {
    let mut stack = stack();
    let mut engine = Recompute::default();
    evaluate(&stack.document, &mut engine);
    let plane = stack
        .document
        .feature(stack.top)
        .unwrap()
        .kind
        .sketch()
        .unwrap()
        .plane();
    let lost = FaceAttachment {
        body: stack.base,
        face: FaceReference::new(FaceName::from_digest(99), None, []),
    };
    stack
        .document
        .apply(Transaction::single(
            "Lose the face",
            Edit::SetSketchPlacement {
                feature: stack.top,
                plane,
                attachment: Some(SketchAttachment::Face(lost)),
            },
        ))
        .unwrap();
    let evaluation = evaluate(&stack.document, &mut engine);

    let FeatureState::Failed(error) = &evaluation.feature(stack.top).unwrap().state else {
        panic!("the sketch should fail");
    };
    assert_eq!(
        error.reason,
        "The face this sketch lies on is no longer part of Base."
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(stack.top)));
    assert!(matches!(
        evaluation.feature(stack.boss).unwrap().state,
        FeatureState::Failed(_)
    ));
    assert_eq!(
        evaluation.feature(stack.base).unwrap().state,
        FeatureState::UpToDate
    );
    assert_eq!(sketch_plane(&evaluation, stack.top).origin().z, 4.0);
}

#[test]
fn placement_edits_are_checked_and_undone() {
    let stack = stack();
    let mut editor = Editor::new(stack.document.clone());
    let before = editor.document().clone();
    let attachment = before
        .feature(stack.top)
        .unwrap()
        .kind
        .attachment()
        .and_then(SketchAttachment::face)
        .cloned();
    let lifted = Plane::from_frame(
        caditor_geometry::Point3::new(0.0, 0.0, 20.0),
        caditor_geometry::Vector3::Z,
        caditor_geometry::Vector3::X,
    )
    .unwrap();
    editor
        .apply(Transaction::single(
            "Detach",
            Edit::SetSketchPlacement {
                feature: stack.top,
                plane: lifted,
                attachment: None,
            },
        ))
        .unwrap();
    let detached = editor.document().feature(stack.top).unwrap();
    assert!(detached.kind.attachment().is_none());
    assert_eq!(detached.kind.sketch().unwrap().plane(), lifted);
    editor.undo().unwrap();
    assert_eq!(*editor.document(), before);

    let document = editor.document();
    let outline = document.features().next().unwrap().id();
    let on_sketch = Transaction::single(
        "Attach to a sketch",
        Edit::SetSketchPlacement {
            feature: stack.top,
            plane: lifted,
            attachment: attachment.clone().map(|attachment| {
                SketchAttachment::Face(FaceAttachment {
                    body: outline,
                    ..attachment
                })
            }),
        },
    );
    assert_eq!(
        document.check(&on_sketch),
        Err(EditError::NotABody("Outline".to_owned()))
    );
    let on_a_solid = Transaction::single(
        "Place a solid",
        Edit::SetSketchPlacement {
            feature: stack.boss,
            plane: lifted,
            attachment: None,
        },
    );
    assert_eq!(
        document.check(&on_a_solid),
        Err(EditError::NotASketch("Boss".to_owned()))
    );
    assert!(!document.dependents_of(&[stack.base]).is_empty());
    let above_base = Transaction::single(
        "Move",
        Edit::MoveFeature {
            id: stack.cap,
            index: 1,
        },
    );
    assert!(matches!(
        document.check(&above_base),
        Err(EditError::AboveDependency { .. })
    ));
}

fn attached_face(document: &Document, sketch: FeatureId) -> FaceReference {
    match document.feature(sketch).unwrap().kind.attachment() {
        Some(SketchAttachment::Face(attachment)) => attachment.face.clone(),
        other => panic!("expected a face attachment, found {other:?}"),
    }
}

#[test]
fn completing_origins_gives_a_face_saved_without_one_its_origin_and_keeps_the_sketch() {
    let Stack {
        mut document,
        base,
        top,
        ..
    } = stack();
    let captured = attached_face(&document, top);
    let plane = match &document.feature(top).unwrap().kind {
        FeatureKind::Sketch(sketch) => sketch.sketch.plane(),
        other => panic!("expected a sketch, found {other:?}"),
    };
    let stripped = FaceReference::new(captured.name(), None, captured.neighbours().iter().copied());
    document
        .apply(Transaction::single(
            "Saved before origins",
            Edit::SetSketchPlacement {
                feature: top,
                plane,
                attachment: Some(SketchAttachment::Face(FaceAttachment {
                    body: base,
                    face: stripped,
                })),
            },
        ))
        .unwrap();

    let completion = complete_origins(&document, &CancelToken::never());
    document.apply(completion).unwrap();

    assert_eq!(attached_face(&document, top), captured);
    assert_eq!(
        captured.origin(),
        Some(FaceOrigin::EndCap {
            feature: base.raw()
        })
    );
}
