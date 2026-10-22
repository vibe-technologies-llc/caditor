use std::sync::atomic::{AtomicUsize, Ordering};

use caditor_expression::{Expression, ParameterId, Unit};
use caditor_geometry::{Plane, Point2, Point3};
use caditor_kernel::{EdgeReference, FaceOrigin, Solid, VertexName, vertex_names};
use caditor_sketch::Sketch;

use crate::{
    paste_picks::{Described, Origins, Unmatched, choose},
    *,
};

fn rectangle() -> Sketch {
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
    sketch
}

fn extrude(sketch: FeatureId, height: Expression) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent: ExtrudeExtent::one_side(height, false),
        operation: BodyOperation::NewBody,
        start: None,
        other_bodies: Vec::new(),
        taper: None,
        direction: None,
        wall: None,
    }))
}

fn blend(kind: BlendKind, body: FeatureId, edges: Vec<EdgeReference>) -> FeatureKind {
    FeatureKind::Blend(Blend {
        kind,
        body,
        groups: Vec::new(),
        edges,
        size: Expression::measure(1.0, Unit::Millimetre),
        form: ChamferForm::Equal,
        flipped: false,
    })
}

fn evaluate(document: &Document) -> Evaluation {
    Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
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

fn corner_at(solid: &Solid, point: Point3) -> VertexName {
    let (id, _) = solid
        .vertices()
        .find(|(_, vertex)| vertex.point().distance(point) < 1e-6)
        .unwrap();
    vertex_names(solid)[&id]
}

struct Model {
    document: Document,
    outline: FeatureId,
    base: FeatureId,
}

fn model() -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let height = transaction.add_parameter("height", transaction.parse("4 mm").unwrap());
    let outline = transaction.add_feature("Outline", FeatureKind::from(rectangle()));
    let base = transaction.add_feature("Base", extrude(outline, Expression::Parameter(height)));
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        outline,
        base,
    }
}

fn add(document: &mut Document, name: &str, kind: FeatureKind) -> FeatureId {
    let mut transaction = document.transaction(name);
    let id = transaction.add_feature(name, kind);
    document.apply(transaction.finish()).unwrap();
    id
}

fn copied(document: &Document, ids: &[FeatureId]) -> Vec<Feature> {
    ids.iter()
        .map(|id| document.feature(*id).unwrap().clone())
        .collect()
}

fn carried(document: &Document, features: &[Feature]) -> CarriedParameters {
    CarriedParameters::of(
        document,
        &ParameterValues::evaluate(document),
        features.iter().flat_map(feature_parameters),
    )
}

fn finding(
    document: &Document,
    features: &[Feature],
    parameters: &CarriedParameters,
    origin: PasteOrigin,
    cancel: &CancelToken,
) -> Result<FeaturePaste, PasteError> {
    document.paste_features_finding_picks(
        features,
        parameters,
        origin,
        &PickSearch {
            evaluator: &ModelEvaluator,
            cancel,
            progress: &|_, _| {},
            threads_cosmetic: false,
        },
        &mut Recompute::default(),
    )
}

#[derive(Default)]
struct Counted {
    evaluated: AtomicUsize,
}

impl Counted {
    fn take(&self) -> usize {
        self.evaluated.swap(0, Ordering::SeqCst)
    }
}

impl Evaluator for Counted {
    fn evaluate(
        &self,
        feature: &Feature,
        inputs: &Inputs<'_>,
        cancel: &CancelToken,
    ) -> Result<FeatureResult, Failure> {
        self.evaluated.fetch_add(1, Ordering::SeqCst);
        ModelEvaluator.evaluate(feature, inputs, cancel)
    }
}

fn blend_of(document: &Document, feature: FeatureId) -> Blend {
    match &document.feature(feature).unwrap().kind {
        FeatureKind::Blend(blend) => blend.clone(),
        other => panic!("expected a blend, found {other:?}"),
    }
}

fn faces_made_by(solid: &Solid, made: FaceOrigin) -> usize {
    solid
        .faces()
        .filter(|(_, face)| face.origin() == Some(made))
        .count()
}

#[test]
fn a_fillet_copied_with_its_extrusion_rounds_the_copy_of_its_edge() {
    let Model {
        mut document,
        outline,
        base,
    } = model();
    let before = evaluate(&document);
    let edge = edge_at(before.body(base).unwrap(), Point3::new(10.0, 4.0, 4.0));
    let fillet = add(
        &mut document,
        "Fillet 1",
        blend(BlendKind::Fillet, base, vec![edge]),
    );
    let features = copied(&document, &[outline, base, fillet]);

    let paste = finding(
        &document,
        &features,
        &carried(&document, &features),
        PasteOrigin::ThisDocument,
        &CancelToken::never(),
    )
    .unwrap();
    document.apply(paste.transaction).unwrap();
    let evaluation = evaluate(&document);

    let [_, base_copy, fillet_copy] = paste.pasted[..] else {
        panic!("expected three copies, got {:?}", paste.pasted);
    };
    let copy_edge = blend_of(&document, fillet_copy).edges[0];
    let rounded = evaluation.body(base_copy).unwrap();
    let original = evaluation.body(base).unwrap();
    let seen = evaluation.body_seen_by(fillet_copy, base_copy).unwrap();

    assert!(paste.left_out.is_empty(), "{:?}", paste.left_out);
    assert_eq!(
        evaluation.feature(fillet_copy).unwrap().state,
        FeatureState::UpToDate
    );
    assert!(copy_edge.resolve(seen).is_ok());
    assert!(
        copy_edge
            .origins()
            .iter()
            .flatten()
            .all(|origin| origin.feature() == base_copy.raw())
    );
    assert_eq!(
        faces_made_by(
            rounded,
            FaceOrigin::Fillet {
                feature: fillet_copy.raw()
            }
        ),
        1
    );
    assert_eq!(
        faces_made_by(
            original,
            FaceOrigin::Fillet {
                feature: fillet.raw()
            }
        ),
        1
    );
}

fn block(document: &mut Document, index: u32) {
    let mut sketch = Sketch::new(Plane::XY);
    let left = 20.0 * f64::from(index + 1);
    let corners = [
        Point2::new(left, 0.0),
        Point2::new(left + 5.0, 0.0),
        Point2::new(left + 5.0, 5.0),
        Point2::new(left, 5.0),
    ];
    for corner in 0..4 {
        sketch.add_line(corners[corner], corners[(corner + 1) % 4]);
    }
    let outline = add(document, &format!("Block outline {index}"), sketch.into());
    add(
        document,
        &format!("Block {index}"),
        extrude(outline, Expression::measure(3.0, Unit::Millimetre)),
    );
}

#[test]
fn a_paste_on_the_models_cache_evaluates_only_the_copies_its_picks_lie_on() {
    let Model {
        mut document,
        outline,
        base,
    } = model();
    let before = evaluate(&document);
    let edge = edge_at(before.body(base).unwrap(), Point3::new(10.0, 4.0, 4.0));
    let fillet = add(
        &mut document,
        "Fillet 1",
        blend(BlendKind::Fillet, base, vec![edge]),
    );
    for index in 0..6 {
        block(&mut document, index);
    }
    let features = copied(&document, &[outline, base, fillet]);
    let parameters = carried(&document, &features);
    let counted = Counted::default();
    let search = PickSearch {
        evaluator: &counted,
        cancel: &CancelToken::never(),
        progress: &|_, _| {},
        threads_cosmetic: false,
    };
    let mut model_cache = Recompute::default();

    model_cache.run(&document, &counted, &CancelToken::never(), &|_, _| {});
    let model_features = counted.take();
    let warm = document.paste_features_finding_picks(
        &features,
        &parameters,
        PasteOrigin::ThisDocument,
        &search,
        &mut model_cache.draft_copy(),
    );
    let on_the_cache = counted.take();
    let cold = document.paste_features_finding_picks(
        &features,
        &parameters,
        PasteOrigin::ThisDocument,
        &search,
        &mut Recompute::default(),
    );
    let from_nothing = counted.take();

    assert_eq!(model_features, document.features().len());
    assert_eq!(model_features, 15);
    assert_eq!(on_the_cache, 2, "only the copies of the outline and base");
    assert_eq!(from_nothing, model_features + 2);
    assert_eq!(warm, cold);
    assert!(warm.unwrap().left_out.is_empty());
}

#[test]
fn a_chamfer_on_a_copied_fillet_follows_the_copy_of_the_fillet_it_was_picked_on() {
    let Model {
        mut document,
        outline,
        base,
    } = model();
    let before = evaluate(&document);
    let body = before.body(base).unwrap();
    let corners = vec![
        edge_at(body, Point3::new(10.0, 0.0, 2.0)),
        edge_at(body, Point3::new(0.0, 8.0, 2.0)),
    ];
    let fillet = add(
        &mut document,
        "Fillet 1",
        blend(BlendKind::Fillet, base, corners),
    );
    let filleted = evaluate(&document);
    let round = filleted.body(base).unwrap();
    let rim = round
        .edges()
        .find(|(id, edge)| {
            let faces = edge_faces(round, *id);
            let middle = edge.curve().point(edge.interval().middle());
            middle.z > 3.9
                && middle.x > 5.0
                && faces.iter().any(|face| {
                    round.face(*face).and_then(|face| face.origin())
                        == Some(FaceOrigin::Fillet {
                            feature: fillet.raw(),
                        })
                })
        })
        .map(|(id, _)| EdgeReference::capture(round, id).unwrap())
        .unwrap();
    let chamfer = add(
        &mut document,
        "Chamfer 1",
        blend(BlendKind::Chamfer, base, vec![rim]),
    );
    let features = copied(&document, &[outline, base, fillet, chamfer]);

    let paste = finding(
        &document,
        &features,
        &carried(&document, &features),
        PasteOrigin::ThisDocument,
        &CancelToken::never(),
    )
    .unwrap();
    document.apply(paste.transaction).unwrap();
    let evaluation = evaluate(&document);

    let [_, base_copy, fillet_copy, chamfer_copy] = paste.pasted[..] else {
        panic!("expected four copies, got {:?}", paste.pasted);
    };
    let picked = blend_of(&document, chamfer_copy).edges[0];
    let seen = evaluation.body_seen_by(chamfer_copy, base_copy).unwrap();
    let found = picked.resolve(seen).unwrap();
    let middle = seen
        .edge(found)
        .map(|edge| edge.curve().point(edge.interval().middle()))
        .unwrap();

    assert!(paste.left_out.is_empty(), "{:?}", paste.left_out);
    assert_eq!(
        evaluation.feature(chamfer_copy).unwrap().state,
        FeatureState::UpToDate
    );
    assert!(picked.origins().contains(&Some(FaceOrigin::Fillet {
        feature: fillet_copy.raw()
    })));
    assert!(middle.x > 5.0 && middle.z > 3.9, "{middle:?}");
}

#[test]
fn features_pasted_into_another_model_find_their_picks_on_the_copies() {
    let Model {
        mut document,
        outline,
        base,
    } = model();
    let before = evaluate(&document);
    let body = before.body(base).unwrap();
    let edge = edge_at(body, Point3::new(5.0, 0.0, 4.0));
    let corner = corner_at(body, Point3::new(10.0, 8.0, 4.0));
    let fillet = add(
        &mut document,
        "Fillet 1",
        blend(BlendKind::Fillet, base, vec![edge]),
    );
    let point = add(
        &mut document,
        "Point 1",
        FeatureKind::Datum(Datum::Point(DatumPoint {
            base: PointReference::Vertex {
                body: base,
                vertex: corner,
            },
            offset: [
                Expression::Number(0.0),
                Expression::Number(0.0),
                Expression::Number(0.0),
            ],
        })),
    );
    let features = copied(&document, &[outline, base, fillet, point]);
    let parameters = carried(&document, &features);
    let mut other = Document::default();
    add(&mut other, "Elsewhere", FeatureKind::from(rectangle()));

    let paste = finding(
        &other,
        &features,
        &parameters,
        PasteOrigin::Elsewhere,
        &CancelToken::never(),
    )
    .unwrap();
    other.apply(paste.transaction).unwrap();
    let evaluation = evaluate(&other);

    assert!(paste.left_out.is_empty(), "{:?}", paste.left_out);
    for copy in &paste.pasted {
        assert_eq!(
            evaluation.feature(*copy).unwrap().state,
            FeatureState::UpToDate,
            "{:?}",
            other.feature(*copy).map(|feature| &feature.name)
        );
    }
    assert_eq!(
        evaluation
            .feature(paste.pasted[3])
            .and_then(|status| status.result.as_deref())
            .and_then(FeatureResult::datum),
        Some(&DatumResult::Point(Point3::new(10.0, 8.0, 4.0)))
    );
}

#[test]
fn a_pick_on_a_copy_that_cannot_be_computed_is_left_out_with_its_reason() {
    let Model {
        mut document,
        outline,
        base,
    } = model();
    let before = evaluate(&document);
    let edge = edge_at(before.body(base).unwrap(), Point3::new(5.0, 0.0, 4.0));
    let fillet = add(
        &mut document,
        "Fillet 1",
        blend(BlendKind::Fillet, base, vec![edge]),
    );
    let features = copied(&document, &[outline, base, fillet]);
    let parameters = carried(&document, &features);
    let mut other = Document::default();
    let mut flat = other.transaction("Flat");
    flat.add_parameter("height", flat.parse("0 mm").unwrap());
    other.apply(flat.finish()).unwrap();

    let paste = finding(
        &other,
        &features,
        &parameters,
        PasteOrigin::Elsewhere,
        &CancelToken::never(),
    )
    .unwrap();

    assert_eq!(paste.pasted.len(), 2);
    assert_eq!(
        paste.left_out,
        vec![LeftOut {
            name: "Fillet 1".to_owned(),
            reason: PasteRefusal::CopyUncomputed {
                name: "Base".to_owned()
            },
        }]
    );
}

#[test]
fn a_cancelled_search_pastes_nothing() {
    let Model {
        mut document,
        outline,
        base,
    } = model();
    let before = evaluate(&document);
    let edge = edge_at(before.body(base).unwrap(), Point3::new(5.0, 0.0, 4.0));
    let fillet = add(
        &mut document,
        "Fillet 1",
        blend(BlendKind::Fillet, base, vec![edge]),
    );
    let features = copied(&document, &[outline, base, fillet]);

    let cancelled = finding(
        &document,
        &features,
        &carried(&document, &features),
        PasteOrigin::ThisDocument,
        &CancelToken::new(|| true),
    );

    assert_eq!(copied_picks(&features), 1);
    assert_eq!(copied_picks(&features[..2]), 0);
    assert!(
        matches!(cancelled, Err(PasteError::Cancelled)),
        "{cancelled:?}"
    );
}

#[test]
fn a_copied_mirror_follows_the_copy_of_the_datum_plane_it_mirrors_across() {
    let Model {
        mut document, base, ..
    } = model();
    let datum = add(
        &mut document,
        "Plane 1",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Yz),
            rotation: None,
            offset: Expression::measure(30.0, Unit::Millimetre),
        })),
    );
    let mirror = add(
        &mut document,
        "Mirror 1",
        FeatureKind::Mirror(Mirror {
            body: base,
            plane: PlaneReference::Datum(datum),
            outcome: MirrorOutcome::AddToBody,
            mirrored: Vec::new(),
            faces: Vec::new(),
        }),
    );
    let features = copied(&document, &[datum, mirror]);

    let paste = document
        .paste_features(
            &features,
            &CarriedParameters::default(),
            PasteOrigin::ThisDocument,
        )
        .unwrap();
    document.apply(paste.transaction).unwrap();

    let FeatureKind::Mirror(pasted) = &document.feature(paste.pasted[1]).unwrap().kind else {
        panic!("expected a mirror");
    };
    assert_eq!(pasted.body, base);
    assert_eq!(pasted.plane, PlaneReference::Datum(paste.pasted[0]));
}

fn described(id: u32, key: Origins, around: Origins, place: Point3) -> Described<u32> {
    Described {
        id,
        key,
        around: vec![around],
        places: vec![place],
    }
}

#[test]
fn picks_alike_in_structure_are_told_apart_only_by_where_they_lie() {
    let fillet = vec![Some(FaceOrigin::Fillet { feature: 9 })];
    let cap = vec![Some(FaceOrigin::EndCap { feature: 8 })];
    let here = Point3::new(1.0, 2.0, 3.0);
    let there = Point3::new(7.0, 2.0, 3.0);
    let originals = [
        described(0, fillet.clone(), cap.clone(), here),
        described(1, fillet.clone(), cap.clone(), there),
    ];
    let copies = [
        described(10, fillet.clone(), cap.clone(), there),
        described(11, fillet.clone(), cap.clone(), here),
    ];
    let moved = [
        described(20, fillet.clone(), cap.clone(), Point3::new(1.0, 5.0, 3.0)),
        described(21, fillet.clone(), cap.clone(), Point3::new(7.0, 5.0, 3.0)),
    ];
    let lone = [described(30, fillet.clone(), vec![None], there)];
    let stacked = [
        described(40, fillet.clone(), cap.clone(), here),
        described(41, fillet, cap, here),
    ];

    assert_eq!(choose(0, &originals, &copies), Ok(11));
    assert_eq!(choose(1, &originals, &copies), Ok(10));
    assert_eq!(choose(0, &originals, &moved), Err(Unmatched::Missing));
    assert_eq!(choose(0, &originals[..1], &lone), Ok(30));
    assert_eq!(choose(0, &originals, &stacked), Err(Unmatched::Ambiguous));
}

fn level_plane(document: &mut Document, level: ParameterId) -> FeatureId {
    add(
        document,
        "Level",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Xy),
            rotation: None,
            offset: Expression::Parameter(level),
        })),
    )
}

fn cut_on(document: &mut Document, body: FeatureId, datum: FeatureId) -> FeatureId {
    let evaluation = evaluate(document);
    let solid = evaluation.body(body).unwrap().clone();
    let Some(DatumResult::Plane(plane)) = evaluation
        .feature(datum)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::datum)
    else {
        panic!("expected a plane");
    };
    let mut transaction = document.transaction("Cut");
    let sketch = transaction.add_feature(
        "Cut",
        FeatureKind::Sketch(SketchFeature::on_datum(Sketch::new(Plane::XY), datum)),
    );
    for curve in section_curves(body, &solid, plane, sketch).unwrap() {
        transaction.add_projection(sketch, curve.source, &curve.outline);
    }
    document.apply(transaction.finish()).unwrap();
    sketch
}

fn projected_ends(evaluation: &Evaluation, sketch: FeatureId) -> Vec<[Point3; 2]> {
    let geometry = &evaluation
        .feature(sketch)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::sketch)
        .unwrap()
        .geometry;
    let plane = geometry.plane();
    geometry
        .entities()
        .filter(|(id, _)| geometry.is_projected(*id))
        .filter_map(|(id, _)| geometry.line_endpoints(id))
        .map(|(start, end)| [plane.to_world(start), plane.to_world(end)])
        .collect()
}

fn section_bodies(document: &Document, sketch: FeatureId) -> Vec<FeatureId> {
    let FeatureKind::Sketch(definition) = &document.feature(sketch).unwrap().kind else {
        panic!("expected a sketch");
    };
    definition
        .projections
        .values()
        .filter_map(|source| match source {
            ProjectionSource::Section { body, .. } => Some(*body),
            _ => None,
        })
        .collect()
}

fn model_with_level() -> (Model, FeatureId, FeatureId) {
    let mut model = model();
    let mut transaction = model.document.transaction("Level");
    let level = transaction.add_parameter("level", transaction.parse("2 mm").unwrap());
    model.document.apply(transaction.finish()).unwrap();
    let datum = level_plane(&mut model.document, level);
    let cut = cut_on(&mut model.document, model.base, datum);
    (model, datum, cut)
}

#[test]
fn a_cut_through_a_copied_body_is_pasted_cutting_the_copy() {
    let (
        Model {
            mut document,
            outline,
            base,
        },
        datum,
        cut,
    ) = model_with_level();
    let features = copied(&document, &[outline, base, datum, cut]);

    let paste = finding(
        &document,
        &features,
        &carried(&document, &features),
        PasteOrigin::ThisDocument,
        &CancelToken::never(),
    )
    .unwrap();
    document.apply(paste.transaction).unwrap();
    let evaluation = evaluate(&document);

    let [_, base_copy, _, cut_copy] = paste.pasted[..] else {
        panic!(
            "expected four copies, got {:?}, {:?}",
            paste.pasted, paste.left_out
        );
    };
    assert!(paste.left_out.is_empty(), "{:?}", paste.left_out);
    assert_eq!(
        evaluation.feature(cut_copy).unwrap().state,
        FeatureState::UpToDate
    );
    let ends = projected_ends(&evaluation, cut_copy);
    assert_eq!(section_bodies(&document, cut_copy), vec![base_copy; 4]);
    assert_eq!(ends.len(), 4);
    assert!(
        ends.iter()
            .flatten()
            .all(|point| (point.z - 2.0).abs() < 1e-6)
    );
}

#[test]
fn a_cut_pasted_into_another_model_is_taken_where_that_model_puts_its_plane() {
    let (
        Model {
            document,
            outline,
            base,
        },
        datum,
        cut,
    ) = model_with_level();
    let features = copied(&document, &[outline, base, datum, cut]);
    let parameters = carried(&document, &features);
    let mut other = Document::default();
    let mut transaction = other.transaction("Level");
    transaction.add_parameter("level", transaction.parse("3 mm").unwrap());
    other.apply(transaction.finish()).unwrap();

    let paste = finding(
        &other,
        &features,
        &parameters,
        PasteOrigin::Elsewhere,
        &CancelToken::never(),
    )
    .unwrap();
    other.apply(paste.transaction).unwrap();
    let evaluation = evaluate(&other);

    let cut_copy = paste.pasted[3];
    let ends = projected_ends(&evaluation, cut_copy);

    assert!(paste.left_out.is_empty(), "{:?}", paste.left_out);
    assert_eq!(
        evaluation.feature(cut_copy).unwrap().state,
        FeatureState::UpToDate
    );
    assert_eq!(ends.len(), 4);
    assert!(
        ends.iter()
            .flatten()
            .all(|point| (point.z - 3.0).abs() < 1e-6)
    );
}

#[test]
fn a_cut_whose_copy_the_plane_no_longer_reaches_is_left_out_with_its_reason() {
    let (
        Model {
            document,
            outline,
            base,
        },
        datum,
        cut,
    ) = model_with_level();
    let features = copied(&document, &[outline, base, datum, cut]);
    let parameters = carried(&document, &features);
    let mut other = Document::default();
    let mut transaction = other.transaction("Level");
    transaction.add_parameter("level", transaction.parse("9 mm").unwrap());
    other.apply(transaction.finish()).unwrap();

    let paste = finding(
        &other,
        &features,
        &parameters,
        PasteOrigin::Elsewhere,
        &CancelToken::never(),
    )
    .unwrap();

    assert_eq!(paste.left_out.len(), 1);
    assert_eq!(paste.left_out[0].name, "Cut");
    assert!(
        matches!(paste.left_out[0].reason, PasteRefusal::PickNotFound { .. }),
        "{:?}",
        paste.left_out
    );
}

fn patterned_twice() -> (Document, FeatureId, FeatureId, FeatureId) {
    let Model {
        mut document, base, ..
    } = model();
    let pattern = add(
        &mut document,
        "Pattern 1",
        FeatureKind::from(Pattern::new(
            base,
            PatternKind::Linear {
                first: LinearDirection {
                    axis: AxisReference::Principal(PrincipalAxis::X),
                    count: Expression::Number(2.0),
                    spacing: Expression::measure(20.0, Unit::Millimetre),
                    measured: LinearSpacing::BetweenCopies,
                    reversed: false,
                },
                second: None,
            },
        )),
    );
    let corner = corner_at(
        evaluate(&document).body(base).unwrap(),
        Point3::new(30.0, 8.0, 4.0),
    );
    let point = add(
        &mut document,
        "Point 1",
        FeatureKind::Datum(Datum::Point(DatumPoint {
            base: PointReference::Vertex {
                body: base,
                vertex: corner,
            },
            offset: [
                Expression::Number(0.0),
                Expression::Number(0.0),
                Expression::Number(0.0),
            ],
        })),
    );
    (document, base, pattern, point)
}

#[test]
fn a_corner_on_a_body_outside_the_copy_follows_the_copied_faces_around_it() {
    let (mut document, base, pattern, point) = patterned_twice();
    let features = copied(&document, &[pattern, point]);

    let paste = finding(
        &document,
        &features,
        &carried(&document, &features),
        PasteOrigin::ThisDocument,
        &CancelToken::never(),
    )
    .unwrap();
    document.apply(paste.transaction).unwrap();
    let evaluation = evaluate(&document);

    let result = |feature: FeatureId| {
        evaluation
            .feature(feature)
            .and_then(|status| status.result.as_deref())
            .and_then(FeatureResult::datum)
            .cloned()
    };

    assert!(paste.left_out.is_empty(), "{:?}", paste.left_out);
    assert_eq!(copied_picks(&features), 1);
    assert_eq!(
        result(point),
        Some(DatumResult::Point(Point3::new(30.0, 8.0, 4.0)))
    );
    assert_eq!(
        result(paste.pasted[1]),
        Some(DatumResult::Point(Point3::new(50.0, 8.0, 4.0)))
    );
    assert!(evaluation.body(base).is_some());
}

#[test]
fn a_corner_the_copies_leave_untouched_stays_where_it_was_picked() {
    let (mut document, _, pattern, point) = patterned_twice();
    let features = copied(&document, &[point]);

    let paste = finding(
        &document,
        &features,
        &carried(&document, &features),
        PasteOrigin::ThisDocument,
        &CancelToken::never(),
    )
    .unwrap();
    document.apply(paste.transaction).unwrap();
    let evaluation = evaluate(&document);

    assert_eq!(copied_picks(&features), 0);
    assert_eq!(
        evaluation
            .feature(paste.pasted[0])
            .and_then(|status| status.result.as_deref())
            .and_then(FeatureResult::datum),
        Some(&DatumResult::Point(Point3::new(30.0, 8.0, 4.0)))
    );
    assert!(document.feature(pattern).is_some());
}
