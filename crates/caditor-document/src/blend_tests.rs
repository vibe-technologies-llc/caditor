use std::f64::consts::PI;

use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3};
use caditor_kernel::{EdgeName, EdgeReference, FaceName, SamplingTolerance, Solid, VertexName};
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

fn spandrel(radius: f64) -> f64 {
    (1.0 - PI / 4.0) * radius * radius
}

fn edge_at(solid: &Solid, point: Point3) -> EdgeReference {
    let (id, _) = solid
        .edges()
        .find(|(_, edge)| {
            let parameter = edge.curve().closest_parameter(point, edge.interval());
            edge.curve().point(parameter).distance(point) < 1e-6
        })
        .unwrap();
    EdgeReference::capture(solid, id).unwrap()
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
    height: caditor_expression::ParameterId,
    radius: caditor_expression::ParameterId,
    base: FeatureId,
    fillet: FeatureId,
}

fn model() -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let height = transaction.add_parameter("height", transaction.parse("4 mm").unwrap());
    let radius = transaction.add_parameter("r", transaction.parse("1 mm").unwrap());
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
    let solid = evaluation.body(base).unwrap();
    let edges = vec![
        edge_at(solid, Point3::new(5.0, 0.0, 4.0)),
        edge_at(solid, Point3::new(0.0, 4.0, 4.0)),
    ];
    let mut transaction = document.transaction("Fillet");
    let fillet = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: base,
            edges,
            size: Expression::Parameter(radius),
        }),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        engine,
        height,
        radius,
        base,
        fillet,
    }
}

fn set(model: &mut Model, parameter: caditor_expression::ParameterId, text: &str) {
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

fn rounded_volume(height: f64, radius: f64) -> f64 {
    let area = spandrel(radius);
    let corner = radius * radius * radius * (5.0 / 3.0 - PI / 2.0);
    80.0 * height - (10.0 + 8.0) * area + corner
}

#[test]
fn a_fillet_changes_its_body_and_follows_upstream_edits() {
    let mut model = model();
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(
        evaluation.bodies().collect::<Vec<_>>(),
        vec![(model.base, model.fillet)]
    );
    let found = volume(&evaluation, model.base);
    assert!(
        (found - rounded_volume(4.0, 1.0)).abs() < 0.02,
        "volume {found}"
    );

    let id = model.height;
    set(&mut model, id, "6 mm");
    let id = model.radius;
    set(&mut model, id, "2 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    let found = volume(&evaluation, model.base);
    assert!(
        (found - rounded_volume(6.0, 2.0)).abs() < 0.05,
        "volume {found}"
    );
}

#[test]
fn the_body_before_a_blend_is_kept_and_meshed_only_when_asked() {
    let model = model();
    let evaluation = evaluate(&model.document, &mut Recompute::default());
    let input = evaluation.body_before(model.fillet).unwrap();
    let solid = input.solid().unwrap();
    assert!(!solid.is_meshed());
    let (wake, woken) = std::sync::mpsc::channel();
    let recomputer = Recomputer::spawn(ModelEvaluator, move || {
        let _ = wake.send(());
    })
    .unwrap();
    recomputer
        .mesh(std::sync::Arc::clone(input), "Fillet".to_owned())
        .unwrap();
    woken
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert!(solid.is_meshed());
    assert_eq!(solid.solid.faces().count(), 6);
    assert!(evaluation.body_before(model.base).is_none());
}

#[test]
fn a_requested_mesh_waits_while_recompute_is_cancelled_and_follows_the_next_one() {
    let model = model();

    let evaluation = evaluate(&model.document, &mut Recompute::default());
    let input = evaluation.body_before(model.fillet).unwrap();
    let solid = input.solid().unwrap();
    let (wake, woken) = std::sync::mpsc::channel();
    let mut recomputer = Recomputer::spawn(ModelEvaluator, move || {
        let _ = wake.send(());
    })
    .unwrap();
    let wait = || {
        woken
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
    };

    recomputer.submit(model.document.clone(), 1).unwrap();
    recomputer.cancel();
    wait();
    recomputer
        .mesh(std::sync::Arc::clone(input), "Fillet".to_owned())
        .unwrap();
    assert!(
        woken
            .recv_timeout(std::time::Duration::from_millis(300))
            .is_err()
    );
    assert!(!solid.is_meshed());

    recomputer.submit(model.document.clone(), 2).unwrap();
    wait();
    wait();
    assert!(solid.is_meshed());
}

#[test]
fn blend_errors_name_the_edge_and_the_fix() {
    let mut model = model();
    let id = model.radius;
    set(&mut model, id, "0 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.fillet);
    assert_eq!(error.reason, "The radius must be more than zero.");
    assert_eq!(error.fix, Some(FixTarget::Feature(model.fillet)));

    let id = model.radius;
    set(&mut model, id, "5 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.fillet);
    assert!(
        error
            .reason
            .starts_with("The radius is too large for the faces next to the edge between Base"),
        "{}",
        error.reason
    );
    assert_eq!(
        error.remedy,
        "Enter a smaller radius, or leave this edge out."
    );
    assert!(evaluation.body(model.base).is_some());

    let id = model.radius;
    set(&mut model, id, "1 mm");
    let FeatureKind::Blend(mut definition) =
        model.document.feature(model.fillet).unwrap().kind.clone()
    else {
        panic!("the fillet should be a blend");
    };
    let lost = FaceName::from_digest(7);
    definition.edges.push(EdgeReference::new(
        EdgeName::from_digest(9),
        [lost, lost],
        [VertexName::from_digest(1), VertexName::from_digest(2)],
    ));
    model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: model.fillet,
                kind: FeatureKind::Blend(definition),
            },
        ))
        .unwrap();
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.fillet);
    assert_eq!(
        error.reason,
        "A chosen edge is no longer part of the body of Base."
    );
}

#[test]
fn a_blend_switches_between_fillet_and_chamfer_but_nothing_else() {
    let mut model = model();
    let FeatureKind::Blend(mut definition) =
        model.document.feature(model.fillet).unwrap().kind.clone()
    else {
        panic!("the fillet should be a blend");
    };
    definition.kind = BlendKind::Chamfer;
    model
        .document
        .apply(Transaction::single(
            "Chamfer",
            Edit::SetFeatureKind {
                id: model.fillet,
                kind: FeatureKind::Blend(definition),
            },
        ))
        .unwrap();
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    let corner = 1.0 / 3.0;
    let expected = 320.0 - 18.0 * 0.5 + corner;
    let found = volume(&evaluation, model.base);
    assert!((found - expected).abs() < 0.02, "volume {found}");

    let sketch = FeatureKind::from(rectangle((0.0, 0.0), (1.0, 1.0)));
    assert!(matches!(
        model.document.apply(Transaction::single(
            "Change",
            Edit::SetFeatureKind {
                id: model.fillet,
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
