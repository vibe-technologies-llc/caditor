use std::{f64::consts::PI, sync::Arc};

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
            extent: ExtrudeExtent::one_side(Expression::Parameter(height), false),
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
fn a_blend_is_not_recomputed_when_its_body_comes_out_the_same() {
    let mut model = model();
    let result = |evaluation: &Evaluation, feature: FeatureId| {
        evaluation
            .feature(feature)
            .and_then(|status| status.result.clone())
            .unwrap()
    };

    let before = evaluate(&model.document, &mut model.engine);
    let mut base = model.document.feature(model.base).unwrap().kind.clone();
    let FeatureKind::Solid(SolidFeature::Extrude(extrude)) = &mut base else {
        panic!("the base is an extrusion");
    };
    extrude.extent = ExtrudeExtent::one_side(model.document.parse("4 mm").unwrap(), false);
    model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: model.base,
                kind: base,
            },
        ))
        .unwrap();
    let after = evaluate(&model.document, &mut model.engine);

    assert!(!Arc::ptr_eq(
        &result(&before, model.base),
        &result(&after, model.base)
    ));
    assert!(Arc::ptr_eq(
        &result(&before, model.fillet),
        &result(&after, model.fillet)
    ));
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
fn a_cancel_that_comes_after_a_recompute_finished_does_not_block_a_requested_mesh() {
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
    wait();
    recomputer.cancel();
    recomputer
        .mesh(std::sync::Arc::clone(input), "Fillet".to_owned())
        .unwrap();
    wait();

    assert!(solid.is_meshed());
}

#[test]
fn a_requested_mesh_is_served_after_a_cancelled_recompute() {
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
    set(&mut model, id, "0.0000005 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.fillet);
    assert_eq!(error.reason, "The radius must be more than 0.000001 mm.");
    assert_eq!(error.remedy, "Enter a larger radius.");

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
    assert!(
        model
            .document
            .dependents_of(&[model.base])
            .contains(&model.fillet)
    );
}

#[test]
fn ambiguous_edges_and_faces_count_only_when_their_pieces_are_one_edge_or_face() {
    let mut document = Document::default();

    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((0.0, 0.0), (10.0, 8.0))),
    );
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("4 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let plain = evaluate(&document, &mut Recompute::default());
    let front_top = edge_at(plain.body(base).unwrap(), Point3::new(5.0, 0.0, 4.0));
    let top = Plane::from_frame(
        Point3::new(0.0, 0.0, 4.0),
        caditor_geometry::Vector3::Z,
        caditor_geometry::Vector3::X,
    )
    .unwrap();
    let mut slot = rectangle((4.0, -1.0), (6.0, 9.0));
    slot.set_plane(top);
    let mut transaction = document.transaction("Slot");
    let slot = transaction.add_feature("Slot sketch", FeatureKind::from(slot));
    transaction.add_feature(
        "Slot",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: slot,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("2 mm").unwrap(), true),
            operation: BodyOperation::Remove(base),
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let slotted = evaluate(&document, &mut Recompute::default());
    let solid = slotted.body(base).unwrap();
    let blend = Blend {
        kind: BlendKind::Fillet,
        body: base,
        edges: vec![front_top],
        size: Expression::parse_stored("1 mm").unwrap(),
    };
    let pieces = blend.resolve(solid).unwrap();
    let back_top_reference = edge_at(solid, Point3::new(2.0, 8.0, 4.0));
    let back_top = back_top_reference.resolve(solid).unwrap();
    let lost = FaceName::from_digest(5);
    let gone = EdgeReference::new(
        EdgeName::from_digest(9),
        [lost, lost],
        [VertexName::from_digest(1), VertexName::from_digest(2)],
    );
    let listed = Blend {
        edges: vec![blend.edges[0], back_top_reference, gone],
        ..blend.clone()
    };
    let face_at = |z: f64| {
        solid
            .faces()
            .filter(|(_, face)| {
                matches!(face.surface(), caditor_kernel::Surface::Plane(plane)
                    if (plane.frame().origin().z - z).abs() < 1e-9
                        && plane.frame().normal().z.abs() > 0.99)
            })
            .map(|(id, _)| id)
            .collect::<Vec<_>>()
    };
    let tops = face_at(4.0);
    let bottoms = face_at(0.0);

    assert_eq!(pieces.len(), 2);
    assert_eq!(
        listed.resolutions(solid),
        vec![
            Resolution::Pieces(pieces.clone()),
            Resolution::One(back_top),
            Resolution::Missing,
        ]
    );
    assert_eq!(
        crate::pieces::tally(vec![
            Resolution::One(back_top),
            Resolution::Tied(pieces.clone())
        ]),
        Err(Unresolved::Unrelated(1))
    );
    assert_eq!(
        crate::pieces::tally(vec![
            Resolution::Pieces(pieces.clone()),
            Resolution::One(back_top)
        ])
        .map(|found| found.len()),
        Ok(3)
    );
    assert!(crate::pieces::pieces_of_one_edge(solid, &pieces));
    assert!(!crate::pieces::pieces_of_one_edge(
        solid,
        &[pieces[0], back_top]
    ));
    assert_eq!(tops.len(), 2);
    assert!(crate::pieces::pieces_of_one_face(solid, &tops));
    assert!(!crate::pieces::pieces_of_one_face(
        solid,
        &[tops[0], bottoms[0]]
    ));
}

#[test]
fn a_failure_message_follows_the_renaming_of_a_feature_that_made_a_face() {
    let mut document = Document::default();

    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((0.0, 0.0), (10.0, 8.0))),
    );
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("4 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
        })),
    );
    let mut hole = rectangle((3.0, 3.0), (7.0, 5.0));
    hole.set_plane(
        Plane::from_frame(
            Point3::new(0.0, 0.0, 4.0),
            caditor_geometry::Vector3::Z,
            caditor_geometry::Vector3::X,
        )
        .unwrap(),
    );
    let hole = transaction.add_feature("Hole", FeatureKind::from(hole));
    let cut = transaction.add_feature(
        "Cut",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: hole,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("2 mm").unwrap(), true),
            operation: BodyOperation::Remove(base),
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    let rim = edge_at(evaluation.body(base).unwrap(), Point3::new(5.0, 3.0, 4.0));
    let mut transaction = document.transaction("Fillet");
    let fillet = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: base,
            edges: vec![rim],
            size: Expression::parse_stored("5 mm").unwrap(),
        }),
    );
    document.apply(transaction.finish()).unwrap();
    let before = failure(&evaluate(&document, &mut engine), fillet).reason;
    document
        .apply(Transaction::single(
            "Rename",
            Edit::RenameFeature {
                id: cut,
                name: "Pocket".to_owned(),
            },
        ))
        .unwrap();
    let after = failure(&evaluate(&document, &mut engine), fillet).reason;

    assert!(before.contains("Cut"), "{before}");
    assert!(
        after.contains("Pocket") && !after.contains("Cut"),
        "{after}"
    );
}

fn mm(text: &str) -> Expression {
    Expression::parse(&format!("{text} mm"), &|_| None).unwrap()
}

fn extruded(sketch: FeatureId, height: &str, operation: BodyOperation) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent: ExtrudeExtent::one_side(mm(height), false),
        operation,
    }))
}

#[test]
fn a_fillet_on_a_cap_edge_survives_a_hole_added_inside_the_outline() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((0.0, 0.0), (10.0, 8.0))),
    );
    let base = transaction.add_feature("Base", extruded(outline, "4", BodyOperation::NewBody));
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    let solid = evaluation.body(base).unwrap();
    let edges = vec![edge_at(solid, Point3::new(5.0, 0.0, 4.0))];
    let mut transaction = document.transaction("Fillet");
    let fillet = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: base,
            edges,
            size: mm("1"),
        }),
    );
    document.apply(transaction.finish()).unwrap();

    let mut transaction = document.transaction("Add hole");
    let center = transaction.add_sketch_entity(
        outline,
        caditor_sketch::Entity::Point(Point2::new(5.0, 4.0)),
    );
    transaction.add_sketch_entity(
        outline,
        caditor_sketch::Entity::Circle {
            center,
            radius: 1.5,
        },
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut engine);

    assert_eq!(
        evaluation.feature(fillet).unwrap().state,
        FeatureState::UpToDate
    );
    let expected = 320.0 - PI * 1.5 * 1.5 * 4.0 - 10.0 * spandrel(1.0);
    let found = volume(&evaluation, base);
    assert!((found - expected).abs() < 0.02, "volume {found}");
}

struct Boss {
    document: Document,
    boss: FeatureId,
    fillet: FeatureId,
}

fn filleted_boss() -> Boss {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((0.0, 0.0), (10.0, 8.0))),
    );
    let base = transaction.add_feature("Base", extruded(outline, "4", BodyOperation::NewBody));
    let square = transaction.add_feature(
        "Square",
        FeatureKind::from(rectangle((2.0, 2.0), (4.0, 4.0))),
    );
    let boss = transaction.add_feature("Boss", extruded(square, "6", BodyOperation::Add(base)));
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut Recompute::default());
    let solid = evaluation.body(base).unwrap();
    let edges = vec![edge_at(solid, Point3::new(2.0, 2.0, 5.0))];
    let mut transaction = document.transaction("Fillet");
    let fillet = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: base,
            edges,
            size: mm("0.5"),
        }),
    );
    document.apply(transaction.finish()).unwrap();
    Boss {
        document,
        boss,
        fillet,
    }
}

#[test]
fn a_fillet_on_an_edge_another_feature_made_depends_on_that_feature() {
    let Boss {
        document,
        boss,
        fillet,
    } = filleted_boss();

    let index = document.feature_index(fillet).unwrap();

    assert!(
        !document
            .feature(fillet)
            .unwrap()
            .kind
            .features()
            .contains(&boss)
    );
    assert_eq!(document.dependents_of(&[boss]), vec![fillet]);
    assert!(matches!(
        document.check(&Transaction::single(
            "Move",
            Edit::MoveFeature {
                id: fillet,
                index: index - 1,
            },
        )),
        Err(EditError::AboveDependency { .. })
    ));
    assert!(matches!(
        document.check(&Transaction::single(
            "Move",
            Edit::MoveFeature { id: boss, index },
        )),
        Err(EditError::BelowDependent { .. })
    ));
}
