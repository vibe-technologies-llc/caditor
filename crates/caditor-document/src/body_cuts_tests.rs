use std::collections::BTreeSet;

use caditor_expression::Expression;
use caditor_geometry::{Maths, Plane, Point2, Point3};
use caditor_kernel::{FaceOrigin, RegionKey, RegionReference, SamplingTolerance, Solid};
use caditor_sketch::{EntityId, Sketch};

use crate::*;

const RADIUS: f64 = 8.0;
const HEIGHT: f64 = 3.0;
const WINDOW_EDGE: f64 = 5.0;
const HALF_WINDOW: f64 = 5.0;

fn rectangle(sketch: &mut Sketch, min: (f64, f64), max: (f64, f64)) -> Vec<EntityId> {
    let corners = [
        Point2::new(min.0, min.1),
        Point2::new(max.0, min.1),
        Point2::new(max.0, max.1),
        Point2::new(min.0, max.1),
    ];

    corners
        .iter()
        .zip(corners.iter().cycle().skip(1))
        .map(|(start, end)| sketch.add_line(*start, *end))
        .collect()
}

fn extrude(sketch: FeatureId, regions: RegionChoice, operation: BodyOperation) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions,
        extent: ExtrudeExtent::one_side(Expression::parse_stored("3").unwrap(), false),
        operation,
        start: None,
        other_bodies: Vec::new(),
        taper: None,
        wall: None,
        direction: None,
    }))
}

fn evaluate(document: &Document) -> Evaluation {
    Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn volume(evaluation: &Evaluation, body: FeatureId) -> f64 {
    evaluation
        .body(body)
        .unwrap()
        .tessellate(&SamplingTolerance::new(1e-4, 0.02).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn sketch_result(evaluation: &Evaluation, sketch: FeatureId) -> &SketchResult {
    let result = evaluation
        .feature(sketch)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::sketch)
        .unwrap();

    result.find_regions();
    result
}

fn offered(evaluation: &Evaluation, sketch: FeatureId) -> &[SketchRegion] {
    sketch_result(evaluation, sketch)
        .regions()
        .unwrap()
        .as_ref()
        .unwrap()
}

fn holding(evaluation: &Evaluation, sketch: FeatureId, point: Point2) -> &SketchRegion {
    offered(evaluation, sketch)
        .iter()
        .find(|region| region.region.contains(point))
        .unwrap()
}

fn top_face(solid: &Solid, plate: FeatureId) -> caditor_kernel::FaceId {
    solid
        .faces()
        .find(|(_, face)| {
            face.origin()
                == Some(FaceOrigin::EndCap {
                    feature: plate.raw(),
                })
        })
        .map(|(id, _)| id)
        .unwrap()
}

fn segment_area(from: f64) -> f64 {
    let half_chord = |x: f64| {
        0.5 * (x * (RADIUS * RADIUS - x * x).sqrt() + RADIUS * RADIUS * (x / RADIUS).asin_libm())
    };
    2.0 * (half_chord(RADIUS) - half_chord(from))
}

fn window_piece_area() -> f64 {
    let corner = (RADIUS * RADIUS - HALF_WINDOW * HALF_WINDOW).sqrt();
    2.0 * HALF_WINDOW * (corner - WINDOW_EDGE) + segment_area(corner)
}

fn disc_area() -> f64 {
    std::f64::consts::PI * RADIUS * RADIUS
}

struct Plate {
    document: Document,
    outline: FeatureId,
    window: Vec<EntityId>,
    plate: FeatureId,
    disc: FeatureId,
    window_point: Point2,
    material_point: Point2,
}

fn plate() -> Plate {
    let mut document = Document::default();
    let mut sketch = Sketch::new(Plane::XY);

    rectangle(&mut sketch, (-20.0, -20.0), (40.0, 20.0));

    let window = rectangle(
        &mut sketch,
        (WINDOW_EDGE, -HALF_WINDOW),
        (WINDOW_EDGE + 10.0, HALF_WINDOW),
    );
    let mut transaction = document.transaction("Plate");

    let outline = transaction.add_feature("Outline", FeatureKind::from(sketch));
    let plate = transaction.add_feature(
        "Plate",
        extrude(outline, RegionChoice::All, BodyOperation::NewBody),
    );

    document.apply(transaction.finish()).unwrap();

    let solid = evaluate(&document).body(plate).unwrap().clone();
    let (attachment, plane) =
        FaceAttachment::capture(plate, &solid, top_face(&solid, plate)).unwrap();
    let mut disc = Sketch::new(plane);
    let centre = plane.to_local(Point3::new(0.0, 0.0, HEIGHT));

    disc.add_circle(centre, RADIUS);

    let mut transaction = document.transaction("Disc");

    let disc = transaction.add_feature(
        "Disc",
        FeatureKind::Sketch(SketchFeature::on_face(disc, attachment)),
    );

    document.apply(transaction.finish()).unwrap();
    Plate {
        document,
        outline,
        window,
        plate,
        disc,
        window_point: plane.to_local(Point3::new(WINDOW_EDGE + 1.0, 0.0, HEIGHT)),
        material_point: plane.to_local(Point3::new(-1.0, 0.0, HEIGHT)),
    }
}

fn add_extrusion(document: &mut Document, regions: RegionChoice, disc: FeatureId) -> FeatureId {
    let name = format!("Boss {}", document.features().count());
    let mut transaction = document.transaction(name.clone());

    let boss = transaction.add_feature(name, extrude(disc, regions, BodyOperation::NewBody));

    document.apply(transaction.finish()).unwrap();
    boss
}

fn chosen(evaluation: &Evaluation, sketch: FeatureId, points: &[Point2]) -> RegionChoice {
    RegionChoice::Chosen(
        points
            .iter()
            .map(|point| holding(evaluation, sketch, *point).reference())
            .collect(),
    )
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> String {
    let FeatureState::Failed(error) = &evaluation.feature(feature).unwrap().state else {
        panic!("{:?}", evaluation.feature(feature).unwrap().state);
    };
    error.reason()
}

#[test]
fn a_circle_on_a_face_crossing_its_window_offers_the_piece_inside_the_window() {
    let plate = plate();

    let evaluation = evaluate(&plate.document);
    let result = sketch_result(&evaluation, plate.disc);
    let regions = offered(&evaluation, plate.disc);
    let window = holding(&evaluation, plate.disc, plate.window_point);
    let material = holding(&evaluation, plate.disc, plate.material_point);
    let whole = result.profile().unwrap().regions().first().unwrap().key();

    assert!(!result.cuts().is_empty());
    assert_eq!(regions.len(), 2);
    assert!((window.region.area() - window_piece_area()).abs() < 1e-6);
    assert!((material.region.area() - (disc_area() - window_piece_area())).abs() < 1e-6);
    assert!(regions.iter().all(|region| region.within == whole));
    assert!(regions.iter().all(|region| region.even_depth));
    assert!(names_body_edges(&window.reference()));
}

#[test]
fn the_pieces_keep_their_names_when_the_plate_gets_thicker() {
    let mut plate = plate();

    let before = evaluate(&plate.document);
    let keys: BTreeSet<RegionKey> = offered(&before, plate.disc)
        .iter()
        .map(|region| region.region.key())
        .collect();
    let reference = holding(&before, plate.disc, plate.window_point).reference();
    let Some(FeatureKind::Solid(SolidFeature::Extrude(thicker))) = plate
        .document
        .feature(plate.plate)
        .map(|feature| feature.kind.clone())
    else {
        panic!("the plate is an extrusion");
    };
    let thicker = Extrude {
        extent: ExtrudeExtent::one_side(Expression::parse_stored("7").unwrap(), false),
        ..thicker
    };

    plate
        .document
        .apply(Transaction::single(
            "Thicker",
            Edit::SetFeatureKind {
                id: plate.plate,
                kind: FeatureKind::Solid(SolidFeature::Extrude(thicker)),
            },
        ))
        .unwrap();

    let after = evaluate(&plate.document);
    let keys_after: BTreeSet<RegionKey> = offered(&after, plate.disc)
        .iter()
        .map(|region| region.region.key())
        .collect();

    assert_eq!(keys, keys_after);
    assert!(keys_after.contains(&reference.key()));
}

#[test]
fn extruding_the_piece_inside_the_window_sweeps_that_piece_alone() {
    let mut plate = plate();

    let evaluation = evaluate(&plate.document);
    let regions = chosen(&evaluation, plate.disc, &[plate.window_point]);
    let boss = add_extrusion(&mut plate.document, regions, plate.disc);

    let evaluation = evaluate(&plate.document);
    let swept = volume(&evaluation, boss);

    assert_eq!(evaluation.failed_count(), 0);
    assert!(
        (swept - window_piece_area() * HEIGHT).abs() < 0.05,
        "{swept} vs {}",
        window_piece_area() * HEIGHT
    );
    assert_eq!(
        sketch_result(&evaluation, plate.disc).chosen_keys(
            plate
                .document
                .feature(boss)
                .and_then(|feature| feature.kind.solid())
                .unwrap()
                .regions()
        ),
        BTreeSet::from([holding(&evaluation, plate.disc, plate.window_point)
            .region
            .key()])
    );
}

#[test]
fn both_pieces_chosen_together_sweep_the_whole_disc() {
    let mut plate = plate();

    let evaluation = evaluate(&plate.document);
    let regions = chosen(
        &evaluation,
        plate.disc,
        &[plate.window_point, plate.material_point],
    );
    let boss = add_extrusion(&mut plate.document, regions, plate.disc);

    let evaluation = evaluate(&plate.document);
    let swept = volume(&evaluation, boss);

    assert_eq!(evaluation.failed_count(), 0);
    assert!((swept - disc_area() * HEIGHT).abs() < 0.05, "{swept}");
}

#[test]
fn a_whole_disc_chosen_before_the_pieces_were_offered_still_sweeps_the_whole_disc() {
    let mut plate = plate();

    let evaluation = evaluate(&plate.document);
    let result = sketch_result(&evaluation, plate.disc);
    let whole = result.profile().unwrap().regions().first().unwrap();
    let stored = RegionChoice::Chosen(vec![RegionReference::capture(whole, whole.anchor())]);
    let key_only = RegionChoice::Chosen(vec![RegionReference::of_key(whole.key())]);
    let offered_keys: BTreeSet<RegionKey> = offered(&evaluation, plate.disc)
        .iter()
        .map(|region| region.region.key())
        .collect();

    let captured = add_extrusion(&mut plate.document, stored.clone(), plate.disc);
    let keyed = add_extrusion(&mut plate.document, key_only, plate.disc);
    let every = add_extrusion(&mut plate.document, RegionChoice::All, plate.disc);

    let evaluation = evaluate(&plate.document);

    assert_eq!(evaluation.failed_count(), 0);

    for boss in [captured, keyed, every] {
        let swept = volume(&evaluation, boss);

        assert!((swept - disc_area() * HEIGHT).abs() < 0.05, "{swept}");
    }

    assert_eq!(
        sketch_result(&evaluation, plate.disc).chosen_keys(&stored),
        offered_keys
    );
    assert_eq!(
        sketch_result(&evaluation, plate.disc).chosen_keys(&RegionChoice::All),
        offered_keys
    );
}

#[test]
fn a_piece_whose_window_is_filled_in_fails_saying_its_edges_are_gone() {
    let mut plate = plate();

    let evaluation = evaluate(&plate.document);
    let regions = chosen(&evaluation, plate.disc, &[plate.window_point]);
    let boss = add_extrusion(&mut plate.document, regions, plate.disc);
    let mut transaction = plate.document.transaction("Fill the window");

    transaction.remove_sketch_items(plate.outline, plate.window.iter().copied(), []);
    plate.document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&plate.document);

    assert_eq!(offered(&evaluation, plate.disc).len(), 1);
    assert!(
        failure(&evaluation, boss).contains("edges of the face it lies on"),
        "{}",
        failure(&evaluation, boss)
    );
}

#[test]
fn a_sketch_on_a_plane_level_with_a_face_is_not_divided_by_its_edges() {
    let mut plate = plate();
    let plane = Plane::from_frame(
        Point3::new(0.0, 0.0, HEIGHT),
        caditor_geometry::Vector3::Z,
        caditor_geometry::Vector3::X,
    )
    .unwrap();
    let mut level = Sketch::new(plane);

    level.add_circle(Point2::ZERO, RADIUS);

    let mut transaction = plate.document.transaction("Level");

    let level = transaction.add_feature("Level", FeatureKind::from(level));

    plate.document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&plate.document);

    assert!(sketch_result(&evaluation, level).cuts().is_empty());
    assert_eq!(offered(&evaluation, level).len(), 1);
}

#[test]
fn the_piece_inside_the_window_pushed_into_the_plate_fills_that_part_of_the_window() {
    let mut plate = plate();

    let evaluation = evaluate(&plate.document);
    let before = volume(&evaluation, plate.plate);
    let regions = chosen(&evaluation, plate.disc, &[plate.window_point]);
    let mut transaction = plate.document.transaction("Fill");

    let fill = transaction.add_feature(
        "Fill",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: plate.disc,
            regions,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("3").unwrap(), true),
            operation: BodyOperation::Add(plate.plate),
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );

    plate.document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&plate.document);
    let after = volume(&evaluation, plate.plate);

    assert!(evaluation.feature(fill).unwrap().state == FeatureState::UpToDate);
    assert!(
        (after - before - window_piece_area() * HEIGHT).abs() < 0.05,
        "{after} - {before} vs {}",
        window_piece_area() * HEIGHT
    );
}
