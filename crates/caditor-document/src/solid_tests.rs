use std::sync::Arc;

use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{Interrupt, MeshQuality, ProfileError, interruptible};
use caditor_sketch::{EntityId, Sketch, SketchSolution};

use crate::*;

pub(crate) fn rectangle(plane: Plane, min: (f64, f64), max: (f64, f64)) -> Sketch {
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

pub(crate) fn extrude(
    sketch: FeatureId,
    distance: &str,
    reversed: bool,
    operation: BodyOperation,
) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent: ExtrudeExtent::one_side(Expression::parse_stored(distance).unwrap(), reversed),
        operation,
        start: None,
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
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (10.0, 8.0))),
    );
    let base = transaction.add_feature(
        "Base",
        extrude(outline, "4 mm", false, BodyOperation::NewBody),
    );
    let top = Plane::from_frame(Point3::new(0.0, 0.0, 4.0), Vector3::Z, Vector3::X).unwrap();
    let hole = transaction.add_feature(
        "Hole sketch",
        FeatureKind::from(rectangle(top, (2.0, 2.0), (4.0, 4.0))),
    );
    let pocket = transaction.add_feature(
        "Pocket",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: hole,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::Parameter(depth), true),
            operation: BodyOperation::Remove(base),
            start: None,
        })),
    );
    let lug = transaction.add_feature(
        "Lug sketch",
        FeatureKind::from(rectangle(top, (6.0, 2.0), (9.0, 6.0))),
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
    assert!(!document.dependents_of(&[outline]).is_empty());
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
        FeatureKind::from(rectangle(Plane::XZ, (2.0, 0.0), (4.0, 3.0))),
    );
    let ring = transaction.add_feature(
        "Ring",
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch: section,
            regions: RegionChoice::All,
            axis: RevolveAxis::Sketch(EntityId::VERTICAL_AXIS),
            extent: RevolveExtent::Full,
            operation: BodyOperation::NewBody,
            start: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut Recompute::default());
    let expected = std::f64::consts::PI * (16.0 - 4.0) * 3.0;
    assert!((volume(&evaluation, ring) - expected).abs() < 0.1);
}

#[test]
fn a_revolve_axis_must_be_a_line_of_its_own_sketch() {
    let mut document = Document::default();

    let mut section = rectangle(Plane::XZ, (2.0, 0.0), (4.0, 3.0));
    let pivot = section.add_line(Point2::new(0.0, 0.0), Point2::new(0.0, 3.0));
    let mut marks = Sketch::new(Plane::XZ);
    let mark = marks.add_point(Point2::new(1.0, 1.0));
    let mut transaction = document.transaction("Build");
    let section = transaction.add_feature("Section", FeatureKind::from(section));
    let marks = transaction.add_feature("Marks", FeatureKind::from(marks));
    document.apply(transaction.finish()).unwrap();
    let revolve = |sketch, axis| {
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch,
            regions: RegionChoice::All,
            axis: RevolveAxis::Sketch(axis),
            extent: RevolveExtent::Full,
            operation: BodyOperation::NewBody,
            start: None,
        }))
    };
    let mut transaction = document.transaction("Revolve");
    let ring = transaction.add_feature("Ring", revolve(section, pivot));
    document.apply(transaction.finish()).unwrap();
    let moved = |kind| Transaction::single("Edit", Edit::SetFeatureKind { id: ring, kind });

    assert_eq!(
        document.check(&moved(revolve(marks, pivot))),
        Err(EditError::AxisNotALine("Marks".to_owned()))
    );
    assert_eq!(
        document.check(&moved(revolve(section, EntityId::ORIGIN))),
        Err(EditError::AxisNotALine("Section".to_owned()))
    );
    assert_eq!(
        document.check(&moved(revolve(marks, mark))),
        Err(EditError::AxisNotALine("Marks".to_owned()))
    );
    assert_eq!(
        document.check(&moved(revolve(marks, EntityId::HORIZONTAL_AXIS))),
        Ok(())
    );
}

#[test]
fn an_open_sketch_is_reported_against_the_sketch() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let mut open = Sketch::new(Plane::XY);
    open.add_line(Point2::ZERO, Point2::new(5.0, 0.0));
    let sketch = transaction.add_feature("Open", FeatureKind::from(open));
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

#[test]
fn an_outline_with_a_gap_names_the_two_ends_and_how_far_apart_they_are() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let mut open = Sketch::new(Plane::XY);
    let corners = [
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 0.0),
        Point2::new(10.0, 6.0),
        Point2::new(0.0, 6.0),
    ];
    let lines: Vec<EntityId> = corners
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let end = corners
                .get(index + 1)
                .copied()
                .unwrap_or(Point2::new(0.0, 0.5));
            open.add_line(*start, end)
        })
        .collect();
    let sketch = transaction.add_feature("Plate", FeatureKind::from(open));
    let solid = transaction.add_feature(
        "Solid",
        extrude(sketch, "1 mm", false, BodyOperation::NewBody),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document, &mut Recompute::default());
    let FeatureState::Failed(error) = &evaluation.feature(solid).unwrap().state else {
        panic!("an open outline cannot be extruded");
    };

    let (first, last) = (lines.first().unwrap(), lines.last().unwrap());
    assert_eq!(
        error.reason,
        format!(
            "Plate has no closed shape to sweep: an end of Line {first} is 0.5 mm from an end of Line {last}."
        )
    );
}

#[test]
fn a_body_whose_first_feature_fails_keeps_its_last_good_shape_as_stale() {
    let mut model = model();
    let mut engine = Recompute::default();
    let good = evaluate(&model.document, &mut engine);
    assert!(!good.is_stale(model.base));
    let outline = model.document.features().next().unwrap().id();
    let flat = extrude(outline, "0 mm", false, BodyOperation::NewBody);
    model
        .document
        .apply(Transaction::single(
            "Flatten",
            Edit::SetFeatureKind {
                id: model.base,
                kind: flat,
            },
        ))
        .unwrap();
    let evaluation = evaluate(&model.document, &mut engine);
    assert!(matches!(
        evaluation.feature(model.base).unwrap().state,
        FeatureState::Failed(_)
    ));
    assert!(evaluation.is_stale(model.base));
    assert!((volume(&evaluation, model.base) - 348.0).abs() < 0.05);
    assert!(
        evaluation
            .body_result(model.base)
            .and_then(|result| result.solid())
            .is_some_and(SolidResult::is_meshed)
    );
    assert!(evaluation.is_complete());
}

#[test]
fn an_edit_that_leaves_geometry_unchanged_stops_at_the_first_equal_result() {
    let mut model = model();
    let mut engine = Recompute::default();
    let before = evaluate(&model.document, &mut engine);
    let hole = model.document.features().nth(2).unwrap().id();
    let bottom = model
        .document
        .feature(hole)
        .and_then(|feature| feature.kind.sketch())
        .and_then(|sketch| {
            sketch
                .entities()
                .find(|(_, entity)| matches!(entity, caditor_sketch::Entity::Line { .. }))
        })
        .map(|(id, _)| id)
        .unwrap();
    let mut transaction = model.document.transaction("Constrain");
    transaction.add_sketch_constraint(hole, caditor_sketch::Constraint::Horizontal(bottom));
    model.document.apply(transaction.finish()).unwrap();

    let after = evaluate(&model.document, &mut engine);
    assert_eq!(after.recomputed(), &[hole]);
    assert!(Arc::ptr_eq(
        before
            .feature(model.pocket)
            .unwrap()
            .result
            .as_ref()
            .unwrap(),
        after
            .feature(model.pocket)
            .unwrap()
            .result
            .as_ref()
            .unwrap()
    ));
}

#[test]
fn renaming_keeps_results_and_refreshes_only_failure_messages() {
    let mut model = model();
    let mut engine = Recompute::default();
    evaluate(&model.document, &mut engine);
    model
        .document
        .apply(Transaction::single(
            "Rename",
            Edit::RenameFeature {
                id: model.base,
                name: "Plate".to_owned(),
            },
        ))
        .unwrap();
    let depth = model.document.parameter_named("depth").unwrap().id();
    model
        .document
        .apply(Transaction::single(
            "Rename",
            Edit::RenameParameter {
                id: depth,
                name: "cut".to_owned(),
            },
        ))
        .unwrap();
    assert_eq!(evaluate(&model.document, &mut engine).recomputed(), &[]);

    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let mut open = Sketch::new(Plane::XY);
    open.add_line(Point2::ZERO, Point2::new(5.0, 0.0));
    let sketch = transaction.add_feature("Open", FeatureKind::from(open));
    let solid = transaction.add_feature(
        "Solid",
        extrude(sketch, "1 mm", false, BodyOperation::NewBody),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    evaluate(&document, &mut engine);
    document
        .apply(Transaction::single(
            "Rename",
            Edit::RenameFeature {
                id: sketch,
                name: "Outline".to_owned(),
            },
        ))
        .unwrap();
    let renamed = evaluate(&document, &mut engine);
    assert_eq!(renamed.recomputed(), &[solid]);
    let FeatureState::Failed(error) = &renamed.feature(solid).unwrap().state else {
        panic!("an open sketch cannot be extruded");
    };
    assert_eq!(error.reason, "Outline has no closed shape to sweep.");
}

#[test]
fn a_run_cancelled_while_meshing_is_not_complete() {
    let model = model();
    let reached_meshing = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = Arc::clone(&reached_meshing);
    let cancel = CancelToken::new(move || flag.load(std::sync::atomic::Ordering::SeqCst));
    let features = model.document.features().len();
    let evaluation =
        Recompute::default().run(&model.document, &ModelEvaluator, &cancel, &|done, _| {
            if done == features {
                reached_meshing.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        });
    assert_eq!(evaluation.failed_count(), 0);
    assert!(!evaluation.is_complete());
}

#[test]
fn both_distances_of_a_two_sided_extrusion_must_be_above_zero() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (4.0, 4.0))),
    );
    let solid = transaction.add_feature(
        "Solid",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::two_sides(
                Expression::parse_stored("5 mm").unwrap(),
                Expression::parse_stored("-2 mm").unwrap(),
            ),
            operation: BodyOperation::NewBody,
            start: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut Recompute::default());
    let FeatureState::Failed(error) = &evaluation.feature(solid).unwrap().state else {
        panic!("a negative distance should be refused");
    };
    assert_eq!(
        error.reason,
        "The backward distance must be more than zero."
    );
}

fn single_body(extent_or_revolve: FeatureKind, sketch: Sketch) -> (Document, FeatureId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let section = transaction.add_feature("Section", FeatureKind::from(sketch));
    let kind = match extent_or_revolve {
        FeatureKind::Solid(SolidFeature::Extrude(mut extrude)) => {
            extrude.sketch = section;
            FeatureKind::Solid(SolidFeature::Extrude(extrude))
        }
        FeatureKind::Solid(SolidFeature::Revolve(mut revolve)) => {
            revolve.sketch = section;
            FeatureKind::Solid(SolidFeature::Revolve(revolve))
        }
        other => other,
    };
    let body = transaction.add_feature("Body", kind);
    document.apply(transaction.finish()).unwrap();
    (document, body)
}

fn extruded(extent: ExtrudeExtent) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch: FeatureId::from_raw(0),
        regions: RegionChoice::All,
        extent,
        operation: BodyOperation::NewBody,
        start: None,
    }))
}

fn revolved(extent: RevolveExtent) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Revolve(Revolve {
        sketch: FeatureId::from_raw(0),
        regions: RegionChoice::All,
        axis: RevolveAxis::Sketch(EntityId::VERTICAL_AXIS),
        extent,
        operation: BodyOperation::NewBody,
        start: None,
    }))
}

fn stored(text: &str) -> Expression {
    Expression::parse_stored(text).unwrap()
}

fn bounds(evaluation: &Evaluation, body: FeatureId) -> caditor_geometry::Aabb {
    evaluation.body(body).unwrap().bounding_box().unwrap()
}

fn extruded_circle() -> (Document, FeatureId) {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_circle(Point2::ZERO, 10.0);
    single_body(
        extruded(ExtrudeExtent::one_side(stored("20 mm"), false)),
        sketch,
    )
}

fn shown_mesh(evaluation: &Evaluation, body: FeatureId) -> &caditor_kernel::Mesh {
    evaluation
        .body_result(body)
        .and_then(|result| result.solid())
        .and_then(SolidResult::mesh)
        .unwrap()
}

#[test]
fn bodies_are_meshed_at_the_recompute_mesh_quality_and_a_new_quality_meshes_them_again() {
    let (document, body) = extruded_circle();
    let mut engine = Recompute::default();

    let smooth = evaluate(&document, &mut engine);
    let smooth_mesh = shown_mesh(&smooth, body);
    let solid = smooth.body(body).unwrap();

    assert_eq!(engine.mesh_quality(), MeshQuality::SMOOTH);
    assert_eq!(
        smooth_mesh,
        &solid.display_mesh(&MeshQuality::SMOOTH).unwrap()
    );

    engine.set_mesh_quality(MeshQuality::COARSE);
    let coarse = evaluate(&document, &mut engine);
    let coarse_mesh = shown_mesh(&coarse, body);

    assert!(!Arc::ptr_eq(
        smooth.body_result(body).unwrap(),
        coarse.body_result(body).unwrap()
    ));
    assert_eq!(
        coarse_mesh,
        &solid.tessellate(&solid.default_tolerance()).unwrap()
    );
    assert!(smooth_mesh.triangles().len() > coarse_mesh.triangles().len());

    engine.set_mesh_quality(MeshQuality::COARSE);
    let again = evaluate(&document, &mut engine);

    assert!(Arc::ptr_eq(
        coarse.body_result(body).unwrap(),
        again.body_result(body).unwrap()
    ));
}

#[test]
fn the_worker_meshes_bodies_at_the_quality_it_was_given() {
    let (document, body) = extruded_circle();
    let coarse = evaluate(
        &document,
        &mut Recompute::with_mesh_quality(MeshQuality::COARSE),
    );
    let mut worker = Recomputer::spawn(ModelEvaluator, || {}).unwrap();

    worker.set_mesh_quality(MeshQuality::COARSE).unwrap();
    worker.submit(document, 1).unwrap();
    let update = worker.wait();

    assert_eq!(
        shown_mesh(&update.evaluation, body),
        shown_mesh(&coarse, body)
    );
}

#[test]
fn symmetric_and_two_sided_extrusions_reach_both_ways() {
    let square = rectangle(Plane::XY, (0.0, 0.0), (2.0, 2.0));
    let (document, body) = single_body(
        extruded(ExtrudeExtent::Symmetric {
            distance: stored("6 mm"),
        }),
        square.clone(),
    );
    let evaluation = evaluate(&document, &mut Recompute::default());
    assert!((volume(&evaluation, body) - 24.0).abs() < 1e-6);
    let reach = bounds(&evaluation, body);
    assert!((reach.min().z + 3.0).abs() < 1e-9 && (reach.max().z - 3.0).abs() < 1e-9);

    let (document, body) = single_body(
        extruded(ExtrudeExtent::two_sides(stored("5 mm"), stored("1 mm"))),
        square,
    );
    let evaluation = evaluate(&document, &mut Recompute::default());
    assert!((volume(&evaluation, body) - 24.0).abs() < 1e-6);
    let reach = bounds(&evaluation, body);
    assert!((reach.min().z + 1.0).abs() < 1e-9 && (reach.max().z - 5.0).abs() < 1e-9);
}

#[test]
fn one_sided_and_symmetric_revolutions_sweep_their_angle() {
    let section = rectangle(Plane::XZ, (2.0, 0.0), (4.0, 3.0));
    let full = std::f64::consts::PI * (16.0 - 4.0) * 3.0;
    for (extent, fraction) in [
        (
            RevolveExtent::OneSide {
                angle: stored("90 deg"),
                reversed: false,
            },
            0.25,
        ),
        (
            RevolveExtent::OneSide {
                angle: stored("90 deg"),
                reversed: true,
            },
            0.25,
        ),
        (
            RevolveExtent::Symmetric {
                angle: stored("120 deg"),
            },
            1.0 / 3.0,
        ),
    ] {
        let (document, body) = single_body(revolved(extent.clone()), section.clone());
        let evaluation = evaluate(&document, &mut Recompute::default());
        let expected = full * fraction;
        assert!(
            (volume(&evaluation, body) - expected).abs() < 1e-3 * expected,
            "{extent:?}"
        );
    }
}

#[test]
fn intersecting_keeps_only_what_both_bodies_share() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let first = transaction.add_feature(
        "First",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (4.0, 4.0))),
    );
    let body = transaction.add_feature(
        "Block",
        extrude(first, "4 mm", false, BodyOperation::NewBody),
    );
    let second = transaction.add_feature(
        "Second",
        FeatureKind::from(rectangle(Plane::XY, (2.0, 1.0), (6.0, 3.0))),
    );
    transaction.add_feature(
        "Common",
        extrude(second, "4 mm", false, BodyOperation::Intersect(body)),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut Recompute::default());
    assert_eq!(evaluation.failed_count(), 0);
    assert!((volume(&evaluation, body) - 16.0).abs() < 1e-6);
}

#[test]
fn an_import_makes_a_body_named_by_its_own_feature() {
    let base = model();
    let evaluation = evaluate(&base.document, &mut Recompute::default());
    let solid = evaluation.body(base.base).unwrap().clone();
    let mut document = Document::default();
    let mut transaction = document.transaction("Import");
    let imported = transaction.add_feature(
        "Bracket",
        FeatureKind::Import(Import::new("bracket.step", solid, "")),
    );
    let broken = transaction.add_feature(
        "Nothing",
        FeatureKind::Import(Import::new(
            "empty.step",
            caditor_kernel::Solid::default(),
            "",
        )),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut Recompute::default());
    assert!((volume(&evaluation, imported) - 348.0).abs() < 0.05);
    let origins: Vec<_> = evaluation
        .body(imported)
        .unwrap()
        .faces()
        .map(|(_, face)| face.origin())
        .collect();
    assert!(
        origins
            .iter()
            .all(|origin| matches!(origin, Some(caditor_kernel::FaceOrigin::Imported { .. })))
    );
    let FeatureState::Failed(error) = &evaluation.feature(broken).unwrap().state else {
        panic!("an empty import cannot make a body");
    };
    assert_eq!(
        error.reason,
        "The shape imported from “empty.step” could not be read back from the model file."
    );
}

#[test]
fn an_import_used_by_later_features_can_be_replaced_by_another_import_but_not_another_kind() {
    let base = model();
    let evaluation = evaluate(&base.document, &mut Recompute::default());
    let solid = evaluation.body(base.base).unwrap().clone();
    let mut document = Document::default();
    let mut transaction = document.transaction("Import");
    let imported = transaction.add_feature(
        "Bracket",
        FeatureKind::Import(Import::new("bracket.step", solid.clone(), "first")),
    );
    transaction.add_feature("Gone", FeatureKind::Remove(Remove { body: imported }));
    document.apply(transaction.finish()).unwrap();

    let replaced = document.apply(Transaction::single(
        "Replace",
        Edit::SetFeatureKind {
            id: imported,
            kind: FeatureKind::Import(Import::new("bracket-v2.step", solid, "second")),
        },
    ));
    let into_a_sketch = document.apply(Transaction::single(
        "Change",
        Edit::SetFeatureKind {
            id: imported,
            kind: FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (1.0, 1.0))),
        },
    ));

    assert!(replaced.is_ok());
    assert_eq!(
        document
            .feature(imported)
            .and_then(|feature| feature.kind.import())
            .map(|import| import.source.as_str()),
        Some("bracket-v2.step")
    );
    assert_eq!(
        into_a_sketch,
        Err(EditError::KindChange("Bracket".to_owned()))
    );
}

#[test]
fn a_revolve_with_regions_on_both_sides_names_the_curves_apart_from_the_rest() {
    let mut section = rectangle(Plane::XZ, (2.0, 0.0), (4.0, 3.0));
    let stray = section.add_circle(Point2::new(-3.0, 1.0), 0.5);
    let label = section.entity_label(stray);
    let (document, body) = single_body(
        revolved(RevolveExtent::OneSide {
            angle: stored("360 deg"),
            reversed: false,
        }),
        section,
    );
    let evaluation = evaluate(&document, &mut Recompute::default());
    let FeatureState::Failed(error) = &evaluation.feature(body).unwrap().state else {
        panic!("regions on both sides of the axis cannot revolve");
    };
    assert_eq!(
        error.reason,
        format!(
            "In Section, {label} lies on the other side of the revolution axis from the rest of \
             the profile."
        )
    );
    assert_eq!(
        error.remedy,
        "Choose only the regions on one side of the axis, or choose another axis."
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(body)));
}

#[test]
fn a_distance_beyond_a_kilometre_is_refused_in_words() {
    let (document, body) = single_body(
        extruded(ExtrudeExtent::one_side(stored("2000 m"), false)),
        rectangle(Plane::XY, (0.0, 0.0), (2.0, 2.0)),
    );
    let evaluation = evaluate(&document, &mut Recompute::default());
    let FeatureState::Failed(error) = &evaluation.feature(body).unwrap().state else {
        panic!("a two-kilometre extrusion should be refused");
    };
    assert_eq!(error.reason, "The distance cannot be more than 1000 m.");
    assert_eq!(error.remedy, "Enter a distance of at most 1000 m.");
}

#[test]
fn a_profile_cancelled_while_it_is_built_is_built_again_later() {
    let result = SketchResult::new(
        rectangle(Plane::XY, (0.0, 0.0), (4.0, 3.0)),
        SketchSolution::default(),
    );
    let stop = || -> Interrupt { Arc::new(|| true) };

    let stopped = interruptible(stop(), || result.profile().map(|_| ()));
    interruptible(stop(), || result.find_regions());

    assert!(matches!(stopped, Err(ProfileError::Cancelled(_))));
    assert!(result.regions().is_none());
    assert_eq!(
        result.profile().map(|profile| profile.regions().len()),
        Ok(1)
    );
    result.find_regions();
    assert!(matches!(result.regions(), Some(Ok(regions)) if regions.len() == 1));
}

#[test]
fn construction_curves_are_left_out_of_the_profile() {
    let mut outline = rectangle(Plane::XY, (0.0, 0.0), (10.0, 8.0));
    let circle = outline.add_circle(Point2::new(5.0, 4.0), 2.0);
    let (mut document, body) = single_body(
        extruded(ExtrudeExtent::one_side(stored("1 mm"), false)),
        outline,
    );
    let section = document.features().next().unwrap().id();
    let mut engine = Recompute::default();
    let hole = 80.0 - std::f64::consts::PI * 4.0;
    assert!((volume(&evaluate(&document, &mut engine), body) - hole).abs() < 0.05);

    let ordinary = document
        .apply(Transaction::single(
            "Make construction",
            Edit::SetSketchConstruction {
                feature: section,
                id: circle,
                construction: true,
            },
        ))
        .unwrap();
    let solid = evaluate(&document, &mut engine);

    assert_eq!(solid.recomputed(), &[section, body]);
    assert!((volume(&solid, body) - 80.0).abs() < 0.05);
    assert_eq!(
        ordinary.edits(),
        [Edit::SetSketchConstruction {
            feature: section,
            id: circle,
            construction: false,
        }]
    );

    document.apply(ordinary).unwrap();
    assert!((volume(&evaluate(&document, &mut engine), body) - hole).abs() < 0.05);
}

#[test]
fn a_revolve_turns_about_a_construction_centreline_and_undoing_its_deletion_keeps_it_construction()
{
    let mut section = rectangle(Plane::XZ, (2.0, 0.0), (4.0, 3.0));
    let centreline = section.add_line(Point2::new(0.0, -1.0), Point2::new(0.0, 5.0));
    section.set_construction(centreline, true).unwrap();
    let (mut document, ring) = single_body(
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch: FeatureId::from_raw(0),
            regions: RegionChoice::All,
            axis: RevolveAxis::Sketch(centreline),
            extent: RevolveExtent::Full,
            operation: BodyOperation::NewBody,
            start: None,
        })),
        section,
    );
    let sketch = document.features().next().unwrap().id();
    let evaluation = evaluate(&document, &mut Recompute::default());
    let expected = std::f64::consts::PI * (16.0 - 4.0) * 3.0;
    assert!((volume(&evaluation, ring) - expected).abs() < 0.1);

    document
        .apply(Transaction::single(
            "Delete ring",
            Edit::RemoveFeature { id: ring },
        ))
        .unwrap();
    let mut transaction = document.transaction("Delete centreline");
    transaction.remove_sketch_items(sketch, [centreline], []);
    let restore = document.apply(transaction.finish()).unwrap();
    let removed = document
        .feature(sketch)
        .unwrap()
        .kind
        .sketch()
        .unwrap()
        .clone();
    document.apply(restore).unwrap();
    let restored = document.feature(sketch).unwrap().kind.sketch().unwrap();

    assert_eq!(removed.construction().len(), 0);
    assert!(restored.is_construction(centreline));
}

fn named_vertices(
    evaluation: &Evaluation,
    body: FeatureId,
) -> std::collections::BTreeMap<caditor_kernel::VertexName, Point3> {
    let result = evaluation.body_result(body).unwrap().solid().unwrap();
    result
        .solid
        .vertices()
        .map(|(id, vertex)| {
            let name = result.names().vertex_name(id).unwrap();
            assert_eq!(result.names().vertices_named(name), [id]);
            (name, vertex.point())
        })
        .collect()
}

#[test]
fn vertices_keep_their_names_when_a_parameter_moves_them() {
    let mut model = model();
    let mut engine = Recompute::default();
    let before = named_vertices(&evaluate(&model.document, &mut engine), model.base);
    let depth = model.document.parameter_named("depth").unwrap().id();

    model
        .document
        .apply(Transaction::single(
            "Deeper",
            Edit::SetParameterExpression {
                id: depth,
                expression: Expression::parse_stored("3 mm").unwrap(),
            },
        ))
        .unwrap();
    let after = named_vertices(&evaluate(&model.document, &mut engine), model.base);
    let moved: Vec<(f64, f64)> = before
        .iter()
        .filter_map(|(name, point)| {
            let now = after.get(name)?;
            (now.distance(*point) > 1e-9).then_some((point.z, now.z))
        })
        .collect();

    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>()
    );
    assert_eq!(moved.len(), 4);
    assert!(
        moved
            .iter()
            .all(|(was, now)| (was - 2.0).abs() < 1e-9 && (now - 1.0).abs() < 1e-9)
    );
}

fn extrusion_with_start(start: Option<&str>) -> (Document, FeatureId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (10.0, 8.0))),
    );
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("4 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
            start: start.map(|text| SolidStart::Distance(Expression::parse_stored(text).unwrap())),
        })),
    );
    document.apply(transaction.finish()).unwrap();
    (document, base)
}

fn heights(evaluation: &Evaluation, body: FeatureId) -> (f64, f64) {
    let bounds = evaluation.body(body).unwrap().bounding_box().unwrap();
    (bounds.min().z, bounds.max().z)
}

#[test]
fn an_extrusion_can_start_off_its_sketch_plane_either_way() {
    let mut engine = Recompute::default();
    for (start, expected) in [
        (None, (0.0, 4.0)),
        (Some("5 mm"), (5.0, 9.0)),
        (Some("-3 mm"), (-3.0, 1.0)),
        (Some("0 mm"), (0.0, 4.0)),
    ] {
        let (document, base) = extrusion_with_start(start);

        let evaluation = evaluate(&document, &mut engine);

        assert_eq!(evaluation.failed_count(), 0, "{start:?}");
        let (low, high) = heights(&evaluation, base);
        assert!(
            (low - expected.0).abs() < 1e-9 && (high - expected.1).abs() < 1e-9,
            "{start:?} gave {low} to {high}"
        );
        assert!((volume(&evaluation, base) - 320.0).abs() < 0.05);
    }
}

#[test]
fn a_start_offset_that_cannot_be_evaluated_or_reaches_too_far_fails_the_extrusion_alone() {
    let mut engine = Recompute::default();
    for (start, reason) in [
        ("5 deg", "The start offset cannot be evaluated"),
        ("1e12 mm", "The start offset cannot be more than"),
    ] {
        let (document, base) = extrusion_with_start(Some(start));

        let evaluation = evaluate(&document, &mut engine);

        let FeatureState::Failed(error) = &evaluation.feature(base).unwrap().state else {
            panic!("the extrusion should fail for {start}");
        };
        assert!(error.reason.starts_with(reason), "{}", error.reason);
        assert_eq!(evaluation.failed_count(), 1);
    }
}

#[test]
fn a_start_offset_follows_the_parameter_it_uses() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let lift = transaction.add_parameter("lift", transaction.parse("2 mm").unwrap());
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (10.0, 8.0))),
    );
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("4 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
            start: Some(SolidStart::Distance(Expression::Parameter(lift))),
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    assert_eq!(heights(&evaluate(&document, &mut engine), base), (2.0, 6.0));

    let mut transaction = document.transaction("Lift");
    transaction.edit(Edit::SetParameterExpression {
        id: lift,
        expression: transaction.parse("7 mm").unwrap(),
    });
    document.apply(transaction.finish()).unwrap();

    assert_eq!(
        heights(&evaluate(&document, &mut engine), base),
        (7.0, 11.0)
    );
    assert_eq!(document.parameter_users(lift), vec!["Base".to_owned()]);
    assert!(document.used_parameters().contains(&lift));
}
