use std::{f64::consts::PI, sync::Arc};

use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3};
use caditor_kernel::{
    EdgeName, EdgeReference, FaceName, FaceReference, SamplingTolerance, Solid, VertexName,
};
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
            groups: Vec::new(),
            edges,
            size: Expression::Parameter(radius),
            form: ChamferForm::Equal,
            flipped: false,
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
    assert_eq!(error.reason(), "The radius must be more than zero.");
    assert_eq!(error.fix, Some(FixTarget::Feature(model.fillet)));

    let id = model.radius;
    set(&mut model, id, "0.0000005 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.fillet);
    assert_eq!(error.reason(), "The radius must be more than 0.000001 mm.");
    assert_eq!(error.remedy(), "Enter a larger radius.");

    let id = model.radius;
    set(&mut model, id, "5 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.fillet);
    assert!(
        error
            .reason()
            .starts_with("The radius is too large for the faces next to the edge between Base"),
        "{}",
        error.reason()
    );
    assert_eq!(
        error.remedy(),
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
        error.reason(),
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
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
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
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let slotted = evaluate(&document, &mut Recompute::default());
    let solid = slotted.body(base).unwrap();
    let blend = Blend {
        kind: BlendKind::Fillet,
        body: base,
        groups: Vec::new(),
        edges: vec![front_top],
        size: Expression::parse_stored("1 mm").unwrap(),
        form: ChamferForm::Equal,
        flipped: false,
    };
    let pieces = blend.resolve(&document, solid).unwrap();
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
        listed.resolutions(&document, solid),
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
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
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
    let rim = edge_at(evaluation.body(base).unwrap(), Point3::new(5.0, 3.0, 4.0));
    let mut transaction = document.transaction("Fillet");
    let fillet = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: base,
            groups: Vec::new(),
            edges: vec![rim],
            size: Expression::parse_stored("5 mm").unwrap(),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    let before = failure(&evaluate(&document, &mut engine), fillet).reason();
    document
        .apply(Transaction::single(
            "Rename",
            Edit::RenameFeature {
                id: cut,
                name: "Pocket".to_owned(),
            },
        ))
        .unwrap();
    let after = failure(&evaluate(&document, &mut engine), fillet).reason();

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
        start: None,
        other_bodies: Vec::new(),
        taper: None,
        wall: None,
        direction: None,
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
            groups: Vec::new(),
            edges,
            size: mm("1"),
            form: ChamferForm::Equal,
            flipped: false,
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

    let healing = evaluation.feature(fillet).unwrap().healing.clone().unwrap();
    assert_eq!(healing.matched, ["its edge"]);
    assert_eq!(
        healing.reason(),
        "After an upstream change, its edge was matched to the most similar geometry."
    );
    let update = healing.update(&document).unwrap();
    assert_eq!(update.label(), "Update references of Fillet 1");
    document.apply(update).unwrap();
    let updated = evaluate(&document, &mut engine);

    assert_eq!(updated.feature(fillet).unwrap().healing, None);
    assert_eq!(
        updated.feature(fillet).unwrap().state,
        FeatureState::UpToDate
    );
    assert!((volume(&updated, base) - found).abs() < 1e-9);
}

struct Boss {
    document: Document,
    base: FeatureId,
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
            groups: Vec::new(),
            edges,
            size: mm("0.5"),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    Boss {
        document,
        base,
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
        ..
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
        document.check_moves(&Transaction::single(
            "Move",
            Edit::MoveFeature {
                id: fillet,
                index: index - 1,
            },
        )),
        Err(EditError::AboveDependency { .. })
    ));
    assert!(matches!(
        document.check_moves(&Transaction::single(
            "Move",
            Edit::MoveFeature { id: boss, index },
        )),
        Err(EditError::BelowDependent { .. })
    ));
}

fn reshape(model: &mut Model, form: ChamferForm, flipped: bool) -> Evaluation {
    let FeatureKind::Blend(mut definition) =
        model.document.feature(model.fillet).unwrap().kind.clone()
    else {
        panic!("the fillet should be a blend");
    };
    definition.kind = BlendKind::Chamfer;
    definition.form = form;
    definition.flipped = flipped;
    model
        .document
        .apply(Transaction::single(
            "Reshape",
            Edit::SetFeatureKind {
                id: model.fillet,
                kind: FeatureKind::Blend(definition),
            },
        ))
        .unwrap();
    evaluate(&model.document, &mut model.engine)
}

fn assert_volume(evaluation: &Evaluation, body: FeatureId, expected: f64) {
    let found = volume(evaluation, body);
    assert!(
        (found - expected).abs() < 0.02,
        "volume {found} instead of {expected}"
    );
}

#[test]
fn a_chamfer_takes_two_distances_or_a_distance_and_an_angle() {
    let mut model = model();
    let angle = model.document.parse("30 deg").unwrap();
    let mut transaction = model.document.transaction("Angle");
    let tilt = transaction.add_parameter("tilt", angle);
    model.document.apply(transaction.finish()).unwrap();
    let two = |text: &str| ChamferForm::TwoDistances {
        second: Expression::parse_stored(text).unwrap(),
    };
    let tangent = 30f64.to_radians().tan();

    let on_top = reshape(&mut model, two("2 mm"), false);
    assert_eq!(on_top.failed_count(), 0);
    assert_volume(&on_top, model.base, 320.0 - 18.0 + 2.0 / 3.0);
    let on_sides = reshape(&mut model, two("2 mm"), true);
    assert_volume(&on_sides, model.base, 320.0 - 18.0 + 4.0 / 3.0);
    let angled = reshape(
        &mut model,
        ChamferForm::DistanceAngle {
            angle: Expression::Parameter(tilt),
        },
        false,
    );
    assert_volume(
        &angled,
        model.base,
        320.0 - 18.0 * 0.5 * tangent + tangent / 3.0,
    );
    let definition = model.document.feature(model.fillet).unwrap();
    assert!(definition.kind.uses_parameter(tilt));
    assert!(definition.kind.parameters().contains(&tilt));

    let flat = reshape(&mut model, two("0 mm"), false);
    assert_eq!(
        failure(&flat, model.fillet).reason(),
        "The second distance must be more than zero."
    );
    let past = reshape(
        &mut model,
        ChamferForm::DistanceAngle {
            angle: Expression::parse_stored("200 deg").unwrap(),
        },
        false,
    );
    assert_eq!(
        failure(&past, model.fillet).reason(),
        "The angle must be less than 180 degrees."
    );
    let square = reshape(
        &mut model,
        ChamferForm::DistanceAngle {
            angle: Expression::parse_stored("90 deg").unwrap(),
        },
        false,
    );
    let error = failure(&square, model.fillet);
    assert!(
        error
            .reason()
            .starts_with("At this angle the cut never meets the other face next to"),
        "{}",
        error.reason()
    );
    assert!(error.remedy().contains("flip the chamfer"));
}

struct Kept {
    model: Model,
    second: caditor_expression::ParameterId,
}

fn fillet_keeping_a_second_distance() -> Kept {
    let mut model = model();
    let value = model.document.parse("2 mm").unwrap();
    let mut transaction = model.document.transaction("Second distance");
    let second = transaction.add_parameter("d2", value);
    model.document.apply(transaction.finish()).unwrap();

    let FeatureKind::Blend(mut definition) =
        model.document.feature(model.fillet).unwrap().kind.clone()
    else {
        panic!("the fillet should be a blend");
    };
    definition.edges = Vec::new();
    definition.groups = vec![EdgeGroup::Body];
    definition.form = ChamferForm::TwoDistances {
        second: Expression::Parameter(second),
    };
    model
        .document
        .apply(Transaction::single(
            "Keep",
            Edit::SetFeatureKind {
                id: model.fillet,
                kind: FeatureKind::Blend(definition),
            },
        ))
        .unwrap();
    Kept { model, second }
}

#[test]
fn a_fillet_keeps_the_parameters_of_the_chamfer_form_it_hides() {
    let Kept { mut model, second } = fillet_keeping_a_second_distance();
    let definition = model.document.feature(model.fillet).unwrap();
    assert_eq!(definition.kind.blend().unwrap().kind, BlendKind::Fillet);

    assert!(definition.kind.uses_parameter(second));
    assert!(definition.kind.parameters().contains(&second));
    assert!(model.document.used_parameters().contains(&second));
    assert!(feature_parameters(definition).contains(&second));

    let removal = model.document.apply(Transaction::single(
        "Delete",
        Edit::RemoveParameter { id: second },
    ));
    assert!(
        matches!(removal, Err(EditError::ParameterInUse { .. })),
        "{removal:?}"
    );

    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
}

#[test]
fn inlining_a_parameter_rewrites_the_chamfer_form_a_fillet_keeps() {
    let Kept { mut model, second } = fillet_keeping_a_second_distance();

    let inlining = model.document.inline_parameter(second).unwrap();
    model.document.apply(inlining).unwrap();

    let definition = model.document.feature(model.fillet).unwrap();
    let blend = definition.kind.blend().unwrap();
    assert!(!definition.kind.uses_parameter(second));
    assert!(model.document.parameter(second).is_none());
    assert_eq!(blend.kind, BlendKind::Fillet);
    let ChamferForm::TwoDistances { second: kept } = &blend.form else {
        panic!("the form should be kept");
    };
    assert!(kept.parameters().is_empty());
    assert_eq!(
        evaluate(&model.document, &mut model.engine).failed_count(),
        0
    );
}

#[test]
fn a_pasted_fillet_takes_the_parameters_of_the_chamfer_form_it_keeps() {
    let Kept { model, second } = fillet_keeping_a_second_distance();
    let sketch = model
        .document
        .feature(model.base)
        .unwrap()
        .kind
        .dependencies();
    let features: Vec<Feature> = sketch
        .into_iter()
        .chain([model.base, model.fillet])
        .map(|id| model.document.feature(id).unwrap().clone())
        .collect();
    let values = ParameterValues::evaluate(&model.document);
    let carried = CarriedParameters::of(
        &model.document,
        &values,
        features.iter().flat_map(feature_parameters),
    );

    let mut target = Document::default();
    let paste = target
        .paste_features(&features, &carried, PasteOrigin::Elsewhere)
        .unwrap();
    assert!(paste.left_out.is_empty());
    target.apply(paste.transaction).unwrap();
    let pasted = target.feature(paste.pasted[2]).unwrap();
    assert!(pasted.kind.parameters().is_empty());
    assert!(!pasted.kind.uses_parameter(second));

    let mut named = Document::default();
    let value = named.parse("9 mm").unwrap();
    let mut transaction = named.transaction("Own");
    let own = transaction.add_parameter("d2", value);
    named.apply(transaction.finish()).unwrap();
    let paste = named
        .paste_features(&features, &carried, PasteOrigin::Elsewhere)
        .unwrap();
    named.apply(paste.transaction).unwrap();
    let pasted = named.feature(paste.pasted[2]).unwrap();
    assert!(pasted.kind.uses_parameter(own));
    assert!(!pasted.kind.uses_parameter(second));
}

fn outline_of(document: &Document, base: FeatureId) -> FeatureId {
    match &document.feature(base).unwrap().kind {
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => extrude.sketch,
        _ => panic!("the base is an extrusion"),
    }
}

fn sketch_of(document: &Document, outline: FeatureId) -> Sketch {
    document
        .feature(outline)
        .unwrap()
        .kind
        .sketch()
        .unwrap()
        .clone()
}

fn front_line_through(sketch: &Sketch, x: f64) -> caditor_sketch::EntityId {
    sketch
        .entities()
        .map(|(id, _)| id)
        .find(|id| {
            sketch.line_endpoints(*id).is_some_and(|(start, end)| {
                start.y.abs() < 1e-9
                    && end.y.abs() < 1e-9
                    && (start.x.min(end.x)..=start.x.max(end.x)).contains(&x)
            })
        })
        .unwrap()
}

fn notched(sketch: &Sketch, from: f64, to: f64) -> Sketch {
    let mut after = sketch.clone();
    after.add_line(Point2::new(from, 0.0), Point2::new(from, 2.0));
    after.add_line(Point2::new(from, 2.0), Point2::new(to, 2.0));
    after.add_line(Point2::new(to, 2.0), Point2::new(to, 0.0));
    let front = front_line_through(&after, 0.5 * (from + to));
    after
        .trim(front, Point2::new(0.5 * (from + to), 0.0))
        .unwrap();
    after
}

fn reshape_outline(document: &mut Document, outline: FeatureId, after: &Sketch) {
    let before = sketch_of(document, outline);
    let mut transaction = document.transaction("Notch");
    transaction.reshape_sketch(outline, &before, after);
    document.apply(transaction.finish()).unwrap();
}

fn notch(document: &mut Document, outline: FeatureId, from: f64, to: f64) {
    let after = notched(&sketch_of(document, outline), from, to);
    reshape_outline(document, outline, &after);
}

fn front_top_fillet(document: &mut Document, base: FeatureId, at: f64) -> FeatureId {
    let mut engine = Recompute::default();
    let evaluation = evaluate(document, &mut engine);
    let edge = edge_at(evaluation.body(base).unwrap(), Point3::new(at, 0.0, 4.0));
    let mut transaction = document.transaction("Fillet");
    let fillet = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: base,
            groups: Vec::new(),
            edges: vec![edge],
            size: mm("0.5"),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    fillet
}

fn block() -> (Document, FeatureId, FeatureId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((0.0, 0.0), (10.0, 8.0))),
    );
    let base = transaction.add_feature("Base", extruded(outline, "4", BodyOperation::NewBody));
    document.apply(transaction.finish()).unwrap();
    (document, outline, base)
}

fn rounded_lengths(document: &Document, fillet: FeatureId) -> Vec<f64> {
    let evaluation = evaluate(document, &mut Recompute::default());
    let solid = &evaluation
        .body_before(fillet)
        .unwrap()
        .solid()
        .unwrap()
        .solid;
    let blend = document.feature(fillet).unwrap().kind.blend().unwrap();
    let mut lengths: Vec<f64> = blend
        .resolve(document, solid)
        .unwrap()
        .into_iter()
        .map(|edge| {
            let edge = solid.edge(edge).unwrap();
            edge.curve().length(edge.interval())
        })
        .collect();
    lengths.sort_by(f64::total_cmp);
    lengths
}

#[test]
fn a_notch_trimmed_into_a_filleted_edge_keeps_both_pieces_rounded() {
    let mut model = model();
    let outline = outline_of(&model.document, model.base);
    notch(&mut model.document, outline, 4.0, 6.0);

    let evaluation = evaluate(&model.document, &mut model.engine);
    let notched = rounded_volume(4.0, 1.0) - 2.0 * 2.0 * 4.0 + 2.0 * spandrel(1.0);
    let found = volume(&evaluation, model.base);
    let healing = evaluation.feature(model.fillet).unwrap().healing.clone();
    let blend = model
        .document
        .feature(model.fillet)
        .unwrap()
        .kind
        .blend()
        .unwrap();

    assert_eq!(evaluation.failed_count(), 0);
    assert!(
        (found - notched).abs() < 0.02,
        "volume {found} instead of {notched}"
    );
    if let Some(healing) = healing {
        let FeatureKind::Blend(refreshed) = &healing.refreshed else {
            panic!("a blend heals into a blend");
        };
        assert_eq!(refreshed.edges.first(), blend.edges.first());
    }
}

#[test]
fn a_notch_trimmed_twice_keeps_every_piece_rounded_and_undoes() {
    let (mut document, outline, base) = block();
    let fillet = front_top_fillet(&mut document, base, 5.0);
    let original = sketch_of(&document, outline);

    notch(&mut document, outline, 3.0, 4.0);
    notch(&mut document, outline, 6.0, 7.0);
    let pieces = rounded_lengths(&document, fillet);
    let mut transaction = document.transaction("Back");
    transaction.reshape_sketch(outline, &sketch_of(&document, outline), &original);
    document.apply(transaction.finish()).unwrap();

    assert_eq!(pieces.len(), 3);
    assert!(
        (pieces.iter().sum::<f64>() - 8.0).abs() < 1e-6,
        "{pieces:?}"
    );
    assert_eq!(rounded_lengths(&document, fillet).len(), 1);
}

#[test]
fn a_fillet_chosen_after_a_notch_keeps_to_the_piece_chosen() {
    let (mut document, outline, base) = block();
    notch(&mut document, outline, 4.0, 6.0);
    let fillet = front_top_fillet(&mut document, base, 2.0);

    let alone = rounded_lengths(&document, fillet);
    add_hole_in(&mut document, outline);
    let after_hole = rounded_lengths(&document, fillet);
    notch(&mut document, outline, 1.0, 2.0);
    let split_again = rounded_lengths(&document, fillet);

    assert_eq!(alone.len(), 1);
    assert!((alone.iter().sum::<f64>() - 4.0).abs() < 1e-6, "{alone:?}");
    assert_eq!(after_hole, alone);
    assert_eq!(split_again.len(), 2);
    assert!(
        (split_again.iter().sum::<f64>() - 3.0).abs() < 1e-6,
        "{split_again:?}"
    );
}

#[test]
fn a_notch_trimmed_into_a_filleted_arc_keeps_both_arc_pieces_rounded() {
    let at = |radius: f64, degrees: f64| {
        let angle = degrees.to_radians();
        Point2::new(5.0 + radius * angle.cos(), radius * angle.sin())
    };
    let mut document = Document::default();
    let mut outline = Sketch::new(Plane::XY);
    outline.add_line(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0));
    let arc = outline.add_arc(
        Point2::new(5.0, 0.0),
        Point2::new(10.0, 0.0),
        Point2::new(0.0, 0.0),
    );
    let mut transaction = document.transaction("Build");
    let outline_id = transaction.add_feature("Outline", FeatureKind::from(outline.clone()));
    let base = transaction.add_feature("Base", extruded(outline_id, "4", BodyOperation::NewBody));
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut Recompute::default());
    let edge = edge_at(evaluation.body(base).unwrap(), Point3::new(5.0, 5.0, 4.0));
    let mut transaction = document.transaction("Fillet");
    let fillet = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: base,
            groups: Vec::new(),
            edges: vec![edge],
            size: mm("0.5"),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    document.apply(transaction.finish()).unwrap();

    let mut after = outline.clone();
    after.add_line(at(5.0, 80.0), at(3.0, 80.0));
    after.add_line(at(3.0, 80.0), at(3.0, 100.0));
    after.add_line(at(3.0, 100.0), at(5.0, 100.0));
    after.trim(arc, Point2::new(5.0, 5.0)).unwrap();
    reshape_outline(&mut document, outline_id, &after);
    let pieces = rounded_lengths(&document, fillet);
    let evaluation = evaluate(&document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(pieces.len(), 2);
    assert!(
        (pieces.iter().sum::<f64>() - 5.0 * PI * 160.0 / 180.0).abs() < 1e-6,
        "{pieces:?}"
    );
}

#[test]
fn a_split_off_piece_moved_off_the_line_fails_the_fillet_in_words() {
    let mut model = model();
    let outline = outline_of(&model.document, model.base);
    let mut after = notched(&sketch_of(&model.document, outline), 4.0, 6.0);
    let collinear: Vec<_> = after
        .constraints()
        .filter(|(_, constraint)| matches!(constraint, caditor_sketch::Constraint::Collinear(..)))
        .map(|(id, _)| id)
        .collect();
    for id in collinear {
        after.remove_constraint(id).unwrap();
    }
    let at_notch: Vec<_> = after
        .entities()
        .filter(|(_, entity)| {
            matches!(entity, caditor_sketch::Entity::Point(point)
                if point.distance(Point2::new(6.0, 0.0)) < 1e-9)
        })
        .map(|(id, _)| id)
        .collect();
    for id in at_notch {
        after
            .replace_entity(id, caditor_sketch::Entity::Point(Point2::new(6.0, -1.0)))
            .unwrap();
    }
    reshape_outline(&mut model.document, outline, &after);

    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.fillet);

    assert!(
        error
            .reason()
            .contains("split into pieces by a sketch edit"),
        "{}",
        error.reason()
    );
    assert!(
        error.remedy().contains("Choose the pieces"),
        "{}",
        error.remedy()
    );
}

struct Grouped {
    document: Document,
    engine: Recompute,
    outline: FeatureId,
    base: FeatureId,
    fillet: FeatureId,
}

fn grouped_fillet(group: impl Fn(&Solid) -> EdgeGroup) -> Grouped {
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
    let groups = vec![group(evaluation.body(base).unwrap())];
    let mut transaction = document.transaction("Fillet");
    let fillet = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: base,
            edges: Vec::new(),
            groups,
            size: mm("0.5"),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    Grouped {
        document,
        engine,
        outline,
        base,
        fillet,
    }
}

fn top_face(solid: &Solid) -> EdgeGroup {
    let (top, _) = solid
        .faces()
        .find(|(id, _)| face_plane(solid, *id).is_some_and(|plane| plane.origin().z > 3.9))
        .unwrap();
    EdgeGroup::Face(FaceReference::capture(solid, top).unwrap())
}

fn add_hole_in(document: &mut Document, outline: FeatureId) {
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
}

fn rounded_count(document: &Document, evaluation: &Evaluation, fillet: FeatureId) -> usize {
    let blend = document.feature(fillet).unwrap().kind.blend().unwrap();
    blend
        .resolve(
            document,
            &evaluation
                .body_before(fillet)
                .unwrap()
                .solid()
                .unwrap()
                .solid,
        )
        .unwrap()
        .len()
}

#[test]
fn a_fillet_of_a_face_rounds_an_edge_an_upstream_change_adds_to_that_face() {
    let mut grouped = grouped_fillet(top_face);
    let before = evaluate(&grouped.document, &mut grouped.engine);
    let rounded = volume(&before, grouped.base);

    add_hole_in(&mut grouped.document, grouped.outline);
    let after = evaluate(&grouped.document, &mut grouped.engine);
    let hole = PI * 1.5 * 1.5 * 4.0;

    assert_eq!(rounded_count(&grouped.document, &before, grouped.fillet), 4);
    assert_eq!(
        after.feature(grouped.fillet).unwrap().state,
        FeatureState::UpToDate
    );
    assert_eq!(rounded_count(&grouped.document, &after, grouped.fillet), 5);
    assert!(volume(&after, grouped.base) < rounded - hole - 0.01);
}

#[test]
fn a_fillet_of_a_whole_body_rounds_every_edge_it_has_now() {
    let mut grouped = grouped_fillet(|_| EdgeGroup::Body);
    let before = evaluate(&grouped.document, &mut grouped.engine);

    add_hole_in(&mut grouped.document, grouped.outline);
    let after = evaluate(&grouped.document, &mut grouped.engine);

    assert_eq!(
        rounded_count(&grouped.document, &before, grouped.fillet),
        12
    );
    assert_eq!(
        after.feature(grouped.fillet).unwrap().state,
        FeatureState::UpToDate
    );
    assert_eq!(rounded_count(&grouped.document, &after, grouped.fillet), 14);
}

struct Pocketed {
    document: Document,
    engine: Recompute,
    base: FeatureId,
    pocket: FeatureId,
}

fn pocketed_box() -> Pocketed {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((0.0, 0.0), (10.0, 8.0))),
    );
    let base = transaction.add_feature("Base", extruded(outline, "4", BodyOperation::NewBody));
    document.apply(transaction.finish()).unwrap();

    let top = Plane::from_frame(
        Point3::new(0.0, 0.0, 4.0),
        caditor_geometry::Vector3::Z,
        caditor_geometry::Vector3::X,
    )
    .unwrap();
    let mut footprint = rectangle((3.0, 2.0), (7.0, 6.0));
    footprint.set_plane(top);
    let mut transaction = document.transaction("Pocket");
    let footprint = transaction.add_feature("Pocket sketch", FeatureKind::from(footprint));
    let pocket = transaction.add_feature(
        "Pocket",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: footprint,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(mm("2"), true),
            operation: BodyOperation::Remove(base),
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();

    Pocketed {
        document,
        engine: Recompute::default(),
        base,
        pocket,
    }
}

fn ruled_edges(pocketed: &mut Pocketed, rule: EdgeRule) -> Vec<Point3> {
    let evaluation = evaluate(&pocketed.document, &mut pocketed.engine);
    let solid = evaluation.body(pocketed.base).unwrap();
    let blend = Blend {
        kind: BlendKind::Fillet,
        body: pocketed.base,
        edges: Vec::new(),
        groups: vec![EdgeGroup::Rule(rule)],
        size: mm("0.2"),
        form: ChamferForm::Equal,
        flipped: false,
    };
    let mut middles: Vec<Point3> = blend
        .resolve(&pocketed.document, solid)
        .unwrap()
        .into_iter()
        .map(|id| {
            let edge = solid.edge(id).unwrap();
            edge.curve().point(edge.interval().middle())
        })
        .collect();
    middles.sort_by(|a, b| a.to_array().partial_cmp(&b.to_array()).unwrap());
    middles
}

fn inside_the_pocket(middle: &Point3) -> bool {
    (3.0..=7.0).contains(&middle.x) && (2.0..=6.0).contains(&middle.y) && middle.z < 4.0 - 1e-9
}

#[test]
fn the_convex_rule_takes_the_outer_edges_and_the_pocket_rim_but_nothing_inside_it() {
    let mut pocketed = pocketed_box();

    let convex = ruled_edges(&mut pocketed, EdgeRule::Convex);

    assert_eq!(convex.len(), 16);
    assert!(!convex.iter().any(inside_the_pocket));
}

#[test]
fn the_concave_rule_takes_only_the_floor_and_the_corners_of_the_pocket() {
    let mut pocketed = pocketed_box();

    let concave = ruled_edges(&mut pocketed, EdgeRule::Concave);

    assert_eq!(concave.len(), 8);
    assert!(concave.iter().all(inside_the_pocket));
}

#[test]
fn a_feature_rule_takes_the_edges_that_feature_made() {
    let mut pocketed = pocketed_box();
    let base = pocketed.base;
    let pocket = pocketed.pocket;

    let made_by_pocket = ruled_edges(&mut pocketed, EdgeRule::MadeBy(pocket));
    let made_by_base = ruled_edges(&mut pocketed, EdgeRule::MadeBy(base));

    assert_eq!(made_by_pocket.len(), 12);
    assert_eq!(made_by_base.len(), 12);
    assert!(
        made_by_pocket
            .iter()
            .all(|middle| made_by_base.iter().all(|other| other != middle))
    );
    assert_eq!(
        made_by_pocket
            .iter()
            .filter(|middle| (middle.z - 4.0).abs() < 1e-9)
            .count(),
        4
    );
}

#[test]
fn a_rule_stays_empty_for_a_kind_of_edge_the_body_lacks() {
    let mut grouped = grouped_fillet(|_| EdgeGroup::Rule(EdgeRule::Concave));

    let evaluation = evaluate(&grouped.document, &mut grouped.engine);

    assert!(matches!(
        evaluation.feature(grouped.fillet).unwrap().state,
        FeatureState::Waiting(_)
    ));
}

#[test]
fn a_fillet_of_the_edges_a_feature_made_follows_an_upstream_change() {
    let mut grouped = grouped_fillet(|_| EdgeGroup::Body);
    let base = grouped.base;
    let mut changed = grouped
        .document
        .feature(grouped.fillet)
        .unwrap()
        .kind
        .blend()
        .unwrap()
        .clone();
    changed.groups = vec![EdgeGroup::Rule(EdgeRule::MadeBy(base))];
    grouped
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: grouped.fillet,
                kind: FeatureKind::Blend(changed),
            },
        ))
        .unwrap();
    let before = evaluate(&grouped.document, &mut grouped.engine);
    let rounded = volume(&before, grouped.base);

    add_hole_in(&mut grouped.document, grouped.outline);
    let after = evaluate(&grouped.document, &mut grouped.engine);

    assert_eq!(
        rounded_count(&grouped.document, &before, grouped.fillet),
        12
    );
    assert_eq!(
        after.feature(grouped.fillet).unwrap().state,
        FeatureState::UpToDate
    );
    assert_eq!(rounded_count(&grouped.document, &after, grouped.fillet), 14);
    assert!(volume(&after, grouped.base) < rounded - PI * 1.5 * 1.5 * 4.0 + 0.2);
}

#[test]
fn a_feature_rule_makes_the_blend_depend_on_that_feature() {
    let mut pocketed = pocketed_box();
    let pocket = pocketed.pocket;
    let base = pocketed.base;
    let mut transaction = pocketed.document.transaction("Fillet");
    let fillet = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: base,
            edges: Vec::new(),
            groups: vec![EdgeGroup::Rule(EdgeRule::MadeBy(pocket))],
            size: mm("0.2"),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    pocketed.document.apply(transaction.finish()).unwrap();

    let kind = &pocketed.document.feature(fillet).unwrap().kind;

    assert!(kind.dependencies().contains(&pocket));
}

#[test]
fn a_lost_face_fails_the_blend_in_words_about_the_face() {
    let mut grouped = grouped_fillet(top_face);
    let mut changed = grouped
        .document
        .feature(grouped.fillet)
        .unwrap()
        .kind
        .blend()
        .unwrap()
        .clone();
    changed.groups = vec![EdgeGroup::Face(FaceReference::new(
        FaceName::from_digest(7),
        None,
        Vec::new(),
    ))];

    grouped
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: grouped.fillet,
                kind: FeatureKind::Blend(changed),
            },
        ))
        .unwrap();
    let evaluation = evaluate(&grouped.document, &mut grouped.engine);

    assert_eq!(
        failure(&evaluation, grouped.fillet).reason(),
        "A chosen face is no longer part of the body of Base."
    );
}

fn rounded_rectangle() -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    let point = Point2::new;
    sketch.add_line(point(3.0, 0.0), point(17.0, 0.0));
    sketch.add_arc(point(17.0, 3.0), point(17.0, 0.0), point(20.0, 3.0));
    sketch.add_line(point(20.0, 3.0), point(20.0, 13.0));
    sketch.add_arc(point(17.0, 13.0), point(20.0, 13.0), point(17.0, 16.0));
    sketch.add_line(point(17.0, 16.0), point(3.0, 16.0));
    sketch.add_arc(point(3.0, 13.0), point(3.0, 16.0), point(0.0, 13.0));
    sketch.add_line(point(0.0, 13.0), point(0.0, 3.0));
    sketch.add_arc(point(3.0, 3.0), point(0.0, 3.0), point(3.0, 0.0));
    sketch
}

fn through_hole(document: &mut Document, body: FeatureId, sketch: SketchFeature) {
    let mut transaction = document.transaction("Drill");
    let sketch = transaction.add_feature("Side hole sketch", FeatureKind::Sketch(sketch));
    transaction.add_feature(
        "Side hole",
        FeatureKind::Hole(Hole {
            sketch,
            body,
            diameter: mm("4"),
            depth: HoleDepth::ThroughAll,
            style: HoleStyle::Plain,
            reversed: false,
            shape: HoleShape::Round,
            standard: None,
            sizing: HoleSizing::Typed,
            bottom: HoleBottom::Flat,
            thread: TappedThread::default(),
        }),
    );
    document.apply(transaction.finish()).unwrap();
}

fn blended(document: &Document, body: FeatureId, edge: EdgeReference, kind: BlendKind) -> Document {
    let mut blended = document.clone();
    let mut transaction = blended.transaction("Blend");
    transaction.add_feature(
        "Rim blend",
        FeatureKind::Blend(Blend {
            kind,
            body,
            groups: Vec::new(),
            edges: vec![edge],
            size: mm("0.2"),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    blended.apply(transaction.finish()).unwrap();
    blended
}

#[test]
fn the_rim_a_hole_leaves_through_the_far_tapered_side_is_rounded_and_chamfered() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature("Outline", FeatureKind::from(rounded_rectangle()));
    let block = transaction.add_feature(
        "Block",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(mm("20"), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: Some(Box::new(Expression::parse("7 deg", &|_| None).unwrap())),
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let solid = evaluate(&document, &mut Recompute::default())
        .body(block)
        .unwrap()
        .clone();
    let slant = 7.0_f64.to_radians().tan();
    let at = Point3::new(10.0, 12.0 * slant, 12.0);
    let (front, frame) = solid
        .faces()
        .find_map(|(id, face)| match face.surface() {
            caditor_kernel::Surface::Plane(plane)
                if (at - plane.frame().origin())
                    .dot(plane.frame().normal())
                    .abs()
                    < 1e-6 =>
            {
                Some((id, *plane.frame()))
            }
            _ => None,
        })
        .unwrap();
    let outward = frame.normal() * solid.face(front).unwrap().sense().sign();
    let plane = Plane::from_frame(frame.origin(), outward, caditor_geometry::Vector3::X).unwrap();
    let mut sketch = Sketch::new(plane);
    sketch.add_point(plane.to_local(at));
    through_hole(
        &mut document,
        block,
        SketchFeature {
            sketch,
            attachment: Some(SketchAttachment::Face(Box::new(FaceAttachment {
                body: block,
                face: FaceReference::capture(&solid, front).unwrap(),
            }))),
            projections: std::collections::BTreeMap::new(),
        },
    );
    let evaluation = evaluate(&document, &mut Recompute::default());
    let drilled = evaluation.body(block).unwrap().clone();
    let before = volume(&evaluation, block);
    let (far_rim, _) = drilled
        .edges()
        .find(|(_, edge)| {
            matches!(edge.curve(), caditor_kernel::Curve::Ellipse(_))
                && edge.curve().point(edge.interval().start()).y > 8.0
        })
        .unwrap();
    let far_rim = EdgeReference::capture(&drilled, far_rim).unwrap();

    let rounded = blended(&document, block, far_rim, BlendKind::Fillet);
    let chamfered = blended(&document, block, far_rim, BlendKind::Chamfer);
    let rounded_evaluation = evaluate(&rounded, &mut Recompute::default());
    let chamfered_evaluation = evaluate(&chamfered, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(rounded_evaluation.failed_count(), 0);
    assert_eq!(chamfered_evaluation.failed_count(), 0);
    let rounded_away = before - volume(&rounded_evaluation, block);
    let chamfered_away = before - volume(&chamfered_evaluation, block);
    assert!(rounded_away > 0.0 && chamfered_away > rounded_away);
}

fn fillet_edges_of(
    document: &mut Document,
    name: &str,
    body: FeatureId,
    edges: Vec<EdgeReference>,
    size: &str,
) {
    let mut transaction = document.transaction("Fillet");
    transaction.add_feature(
        name,
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body,
            groups: Vec::new(),
            edges,
            size: mm(size),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    document.apply(transaction.finish()).unwrap();
}

#[test]
fn the_top_rim_of_a_tapered_box_is_filleted_after_its_corners_and_bottom_rim() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((0.0, 0.0), (40.0, 40.0))),
    );
    let block = transaction.add_feature(
        "Block",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(mm("30"), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: Some(Box::new(Expression::parse("3 deg", &|_| None).unwrap())),
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let tapered = evaluate(&document, &mut Recompute::default())
        .body(block)
        .unwrap()
        .clone();
    let lower: Vec<EdgeReference> = tapered
        .edges()
        .filter(|(_, edge)| {
            let start = edge.curve().point(edge.interval().start());
            let end = edge.curve().point(edge.interval().end());
            start.z.min(end.z) < 1e-6
        })
        .map(|(id, _)| EdgeReference::capture(&tapered, id).unwrap())
        .collect();
    fillet_edges_of(&mut document, "Fillet 1", block, lower, "3");
    let cornered = evaluate(&document, &mut Recompute::default());
    let rounded_below = cornered.body(block).unwrap().clone();
    let (top, _) = rounded_below
        .edges()
        .find(|(_, edge)| {
            matches!(edge.curve(), caditor_kernel::Curve::Line(_))
                && edge.curve().point(edge.interval().middle()).z > 30.0 - 1e-6
        })
        .unwrap();
    let before = volume(&cornered, block);

    fillet_edges_of(
        &mut document,
        "Fillet 2",
        block,
        vec![EdgeReference::capture(&rounded_below, top).unwrap()],
        "2",
    );
    let topped = evaluate(&document, &mut Recompute::default());

    assert_eq!(cornered.failed_count(), 0);
    assert_eq!(topped.failed_count(), 0);
    assert!(volume(&topped, block) < before - 50.0);
}

fn fillet_of(body: FeatureId, edges: Vec<EdgeReference>, size: &str) -> FeatureKind {
    FeatureKind::Blend(Blend {
        kind: BlendKind::Fillet,
        body,
        groups: Vec::new(),
        edges,
        size: mm(size),
        form: ChamferForm::Equal,
        flipped: false,
    })
}

#[test]
fn the_uprights_of_a_box_rounded_on_top_are_filleted_after_it() {
    let Model {
        mut document,
        mut engine,
        base,
        fillet,
        ..
    } = model();
    let removal = document.deletion(&[fillet], "Delete");
    document.apply(removal).unwrap();
    let evaluation = evaluate(&document, &mut engine);
    let solid = evaluation.body(base).unwrap();
    let rim = [(5.0, 0.0), (10.0, 4.0), (5.0, 8.0), (0.0, 4.0)]
        .map(|(x, y)| edge_at(solid, Point3::new(x, y, 4.0)))
        .to_vec();
    let mut transaction = document.transaction("Fillet");
    transaction.add_feature("Rim fillet", fillet_of(base, rim, "0.5"));
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut engine);
    let rounded = evaluation.body(base).unwrap();
    let verticals = [(0.0, 0.0), (10.0, 0.0), (10.0, 8.0), (0.0, 8.0)]
        .map(|(x, y)| edge_at(rounded, Point3::new(x, y, 1.0)))
        .to_vec();
    let before = volume(&evaluation, base);

    let mut transaction = document.transaction("Fillet");
    transaction.add_feature("Fillet 2", fillet_of(base, verticals, "0.25"));
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(evaluation.body(base).unwrap().validate(), Ok(()));
    assert!(volume(&evaluation, base) < before - 4.0 * spandrel(0.25) * 3.5);
}

#[test]
fn an_upright_rising_from_a_foot_an_earlier_blend_rounded_names_it_and_says_to_move_above_it() {
    let Boss {
        mut document,
        base,
        fillet,
        ..
    } = filleted_boss();
    let removal = document.deletion(&[fillet], "Delete");
    document.apply(removal).unwrap();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    let solid = evaluation.body(base).unwrap();
    let foot = [(3.0, 2.0), (4.0, 3.0), (3.0, 4.0), (2.0, 3.0)]
        .map(|(x, y)| edge_at(solid, Point3::new(x, y, 4.0)))
        .to_vec();
    let mut transaction = document.transaction("Fillet");
    transaction.add_feature("Foot fillet", fillet_of(base, foot, "0.5"));
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut engine);
    let rounded = evaluation.body(base).unwrap();
    let verticals = [(2.0, 2.0), (4.0, 2.0), (4.0, 4.0), (2.0, 4.0)]
        .map(|(x, y)| edge_at(rounded, Point3::new(x, y, 5.5)))
        .to_vec();

    let mut transaction = document.transaction("Fillet");
    let corners = transaction.add_feature("Fillet 2", fillet_of(base, verticals, "0.25"));
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut engine);
    let error = failure(&evaluation, corners);

    assert!(
        error
            .reason()
            .ends_with("Foot fillet already blended the corner there."),
        "{}",
        error.reason()
    );
    assert_eq!(
        error.remedy(),
        "Move Fillet 2 above Foot fillet in the tree, so the corner is blended after this edge, or \
         leave this edge out."
    );
}

#[test]
fn an_equal_blend_has_no_side_to_flip() {
    let mut model = model();
    let mut flipped = model
        .document
        .feature(model.fillet)
        .unwrap()
        .kind
        .blend()
        .unwrap()
        .clone();
    flipped.kind = BlendKind::Chamfer;
    flipped.flipped = true;

    model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: model.fillet,
                kind: FeatureKind::Blend(flipped),
            },
        ))
        .unwrap();
    let kept = model.document.feature(model.fillet).unwrap().kind.clone();

    assert!(!kept.blend().unwrap().flipped);
}

fn blended_beside_touching_bores(
    edges: impl FnOnce(&Solid) -> Vec<EdgeReference>,
    groups: Vec<EdgeGroup>,
) -> (Evaluation, FeatureId, FeatureId, f64) {
    let mut document = Document::default();
    let block = crate::hole_tests::block_with_touching_bores(&mut document);
    let mut engine = Recompute::default();
    let before = evaluate(&document, &mut engine);
    let edges = edges(before.body(block).unwrap());
    let bores = volume(&before, block);
    let blend = Blend {
        kind: BlendKind::Fillet,
        body: block,
        edges,
        groups,
        size: mm("0.3"),
        form: ChamferForm::Equal,
        flipped: false,
    };
    let mut transaction = document.transaction("Round");
    let fillet = transaction.add_feature("Fillet 1", FeatureKind::Blend(blend));
    document.apply(transaction.finish()).unwrap();
    (evaluate(&document, &mut engine), block, fillet, bores)
}

#[test]
fn a_rim_beside_two_bores_that_touch_takes_a_fillet() {
    let (evaluation, block, fillet, bores) = blended_beside_touching_bores(
        |solid| vec![edge_at(solid, Point3::new(-5.0, -29.0, 16.0))],
        Vec::new(),
    );

    assert_eq!(
        evaluation.failed_count(),
        0,
        "{:?}",
        evaluation.feature(fillet)
    );
    let found = volume(&evaluation, block);
    let removed = spandrel(0.3) * 2.0 * PI * 2.0;
    assert!((bores - removed - found).abs() < 0.2 * removed, "{found}");
}

#[test]
fn every_edge_of_a_block_whose_bores_touch_leaves_out_where_they_touch() {
    let (evaluation, _, fillet, _) =
        blended_beside_touching_bores(|_| Vec::new(), vec![EdgeGroup::Body]);

    assert_eq!(
        evaluation.failed_count(),
        0,
        "{:?}",
        evaluation.feature(fillet)
    );
}

#[test]
fn the_edge_where_two_bores_touch_is_refused_as_meeting_back_to_back() {
    let (evaluation, _, fillet, _) = blended_beside_touching_bores(
        |solid| vec![edge_at(solid, Point3::new(-5.0, -25.0, 8.0))],
        Vec::new(),
    );

    let reason = failure(&evaluation, fillet).reason();
    assert!(reason.contains("meet back to back"), "{reason}");
    assert!(reason.contains("no fillet of any size"), "{reason}");
}
