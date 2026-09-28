use std::sync::Arc;

use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::{EntityId, Sketch};

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

fn extrude(
    sketch: FeatureId,
    distance: &str,
    reversed: bool,
    operation: BodyOperation,
) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent: ExtrudeExtent::OneSide {
            distance: Expression::parse_stored(distance).unwrap(),
            reversed,
        },
        operation,
    }))
}

fn evaluate(document: &Document, engine: &mut Recompute) -> Evaluation {
    engine.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
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

struct Model {
    document: Document,
    base: FeatureId,
    pocket: FeatureId,
    boss: FeatureId,
}

fn model() -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let depth = transaction.add_parameter("depth", transaction.parse("2 mm").unwrap());
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::Sketch(rectangle(Plane::XY, (0.0, 0.0), (10.0, 8.0))),
    );
    let base = transaction.add_feature(
        "Base",
        extrude(outline, "4 mm", false, BodyOperation::NewBody),
    );
    let top = Plane::from_frame(Point3::new(0.0, 0.0, 4.0), Vector3::Z, Vector3::X).unwrap();
    let hole = transaction.add_feature(
        "Hole sketch",
        FeatureKind::Sketch(rectangle(top, (2.0, 2.0), (4.0, 4.0))),
    );
    let pocket = transaction.add_feature(
        "Pocket",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: hole,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::OneSide {
                distance: Expression::Parameter(depth),
                reversed: true,
            },
            operation: BodyOperation::Remove(base),
        })),
    );
    let lug = transaction.add_feature(
        "Lug sketch",
        FeatureKind::Sketch(rectangle(top, (6.0, 2.0), (9.0, 6.0))),
    );
    let boss = transaction.add_feature(
        "Boss",
        extrude(lug, "3 mm", false, BodyOperation::Add(base)),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        base,
        pocket,
        boss,
    }
}

#[test]
fn features_chain_through_the_body() {
    let model = model();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&model.document, &mut engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert!((volume(&evaluation, model.base) - (320.0 - 8.0 + 36.0)).abs() < 0.05);
    assert_eq!(
        evaluation.bodies().collect::<Vec<_>>(),
        vec![(model.base, model.boss)]
    );
}

#[test]
fn only_the_final_state_of_each_body_is_meshed_on_the_worker() {
    let model = model();
    let evaluation = evaluate(&model.document, &mut Recompute::default());
    let solid_of = |feature| {
        evaluation
            .feature(feature)
            .and_then(|status| status.result.as_deref())
            .and_then(FeatureResult::solid)
            .unwrap()
    };

    let last = solid_of(model.boss);
    let mesh = last.mesh().unwrap();
    assert!((mesh.mass_properties().volume - 348.0).abs() < 0.5);
    assert!(Arc::ptr_eq(
        evaluation.body_result(model.base).unwrap(),
        evaluation
            .feature(model.boss)
            .unwrap()
            .result
            .as_ref()
            .unwrap()
    ));
    assert!(!solid_of(model.base).is_meshed());
    assert!(!solid_of(model.pocket).is_meshed());
    assert!(!last.mesh_failed());

    let outline = model.document.features().next().unwrap().id();
    let regions = evaluation
        .feature(outline)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::sketch)
        .and_then(SketchResult::regions)
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(regions.len(), 1);
    assert!(regions[0].even_depth);
    assert_eq!(regions[0].mesh.as_ref().unwrap().triangles.len(), 2);
}

#[test]
fn a_failing_feature_is_skipped_and_the_rest_still_apply() {
    let mut model = model();
    let mut engine = Recompute::default();
    evaluate(&model.document, &mut engine);
    let depth = model.document.parameter_named("depth").unwrap().id();
    model
        .document
        .apply(Transaction::single(
            "Break",
            Edit::SetParameterExpression {
                id: depth,
                expression: model.document.parse("0 mm").unwrap(),
            },
        ))
        .unwrap();
    let evaluation = evaluate(&model.document, &mut engine);
    let FeatureState::Failed(error) = &evaluation.feature(model.pocket).unwrap().state else {
        panic!("the pocket should fail");
    };
    assert_eq!(error.reason, "The distance must be more than zero.");
    assert_eq!(
        evaluation.feature(model.boss).unwrap().state,
        FeatureState::UpToDate
    );
    assert!((volume(&evaluation, model.base) - (320.0 + 36.0)).abs() < 0.05);
    assert_eq!(evaluation.recomputed(), &[model.pocket, model.boss]);
}

#[test]
fn solid_features_protect_what_they_use() {
    let model = model();
    let mut document = model.document.clone();
    let outline = document.features().next().unwrap().id();
    assert!(matches!(
        document.apply(Transaction::single(
            "Delete",
            Edit::RemoveFeature { id: outline }
        )),
        Err(EditError::FeatureInUse { .. })
    ));
    let mut transaction = document.transaction("Second body");
    let other = transaction.add_feature(
        "Other",
        extrude(outline, "1 mm", true, BodyOperation::NewBody),
    );
    transaction.edit(Edit::MoveFeature {
        id: other,
        index: 1,
    });
    document.apply(transaction.finish()).unwrap();
    let changed = extrude(outline, "5 mm", false, BodyOperation::Add(other));
    assert!(matches!(
        document.apply(Transaction::single(
            "Change",
            Edit::SetFeatureKind {
                id: model.base,
                kind: changed
            }
        )),
        Err(EditError::BodyInUse { .. })
    ));
    document = model.document.clone();
    let taller = extrude(outline, "5 mm", false, BodyOperation::NewBody);
    let undo = document
        .apply(Transaction::single(
            "Change",
            Edit::SetFeatureKind {
                id: model.base,
                kind: taller,
            },
        ))
        .unwrap();
    document.apply(undo).unwrap();
    assert_eq!(document, model.document);
    let onto_sketch = extrude(outline, "5 mm", false, BodyOperation::Add(outline));
    assert!(matches!(
        document.apply(Transaction::single(
            "Change",
            Edit::SetFeatureKind {
                id: model.boss,
                kind: onto_sketch
            }
        )),
        Err(EditError::NotABody(_))
    ));
}

#[test]
fn a_revolve_uses_a_sketch_axis_and_keeps_it() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let section = transaction.add_feature(
        "Section",
        FeatureKind::Sketch(rectangle(Plane::XZ, (2.0, 0.0), (4.0, 3.0))),
    );
    let ring = transaction.add_feature(
        "Ring",
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch: section,
            regions: RegionChoice::All,
            axis: EntityId::VERTICAL_AXIS,
            extent: RevolveExtent::Full,
            operation: BodyOperation::NewBody,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut Recompute::default());
    let expected = std::f64::consts::PI * (16.0 - 4.0) * 3.0;
    assert!((volume(&evaluation, ring) - expected).abs() < 0.1);
}

#[test]
fn an_open_sketch_is_reported_against_the_sketch() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let mut open = Sketch::new(Plane::XY);
    open.add_line(Point2::ZERO, Point2::new(5.0, 0.0));
    let sketch = transaction.add_feature("Open", FeatureKind::Sketch(open));
    let solid = transaction.add_feature(
        "Solid",
        extrude(sketch, "1 mm", false, BodyOperation::NewBody),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut Recompute::default());
    let FeatureState::Failed(error) = &evaluation.feature(solid).unwrap().state else {
        panic!("an open sketch cannot be extruded");
    };
    assert_eq!(error.reason, "Open has no closed shape to sweep.");
    assert_eq!(error.fix, Some(FixTarget::Feature(sketch)));
}
