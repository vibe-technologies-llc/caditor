use caditor_document::{
    BodyOperation, Edit, Extrude, ExtrudeExtent, FaceAttachment, FeatureId, FeatureKind,
    RegionChoice, SketchFeature, SolidFeature, Transaction,
};
use caditor_expression::Expression;
use caditor_geometry::{Maths, Plane, Point2, Point3};
use caditor_kernel::{FaceOrigin, RegionKey};
use caditor_sketch::Sketch;

use super::{CAMERA_SETTLE, Harness, rectangle};
use crate::{model::Action, scene_palette, selection::Pickable};

const RADIUS: f64 = 8.0;
const THICKNESS: f64 = 3.0;
const WINDOW_EDGE: f64 = 5.0;
const HALF_WINDOW: f64 = 5.0;
const DEPTH: f64 = 4.0;

struct Scene {
    sketch: FeatureId,
    plane: Plane,
    window_point: Point2,
    material_point: Point2,
}

fn window_piece_area() -> f64 {
    let half_chord = |x: f64| {
        0.5 * (x * (RADIUS * RADIUS - x * x).sqrt() + RADIUS * RADIUS * (x / RADIUS).asin_libm())
    };
    let corner = (RADIUS * RADIUS - HALF_WINDOW * HALF_WINDOW).sqrt();
    2.0 * HALF_WINDOW * (corner - WINDOW_EDGE) + 2.0 * (half_chord(RADIUS) - half_chord(corner))
}

fn circle_over_a_window(harness: &mut Harness) -> Scene {
    let mut outline = Sketch::new(Plane::XY);

    rectangle(
        &mut outline,
        Point2::new(-20.0, -20.0),
        Point2::new(40.0, 20.0),
    );
    rectangle(
        &mut outline,
        Point2::new(WINDOW_EDGE, -HALF_WINDOW),
        Point2::new(WINDOW_EDGE + 10.0, HALF_WINDOW),
    );

    let mut transaction = harness.document().transaction("Plate");

    let sketch = transaction.add_feature("Windowed outline", FeatureKind::from(outline));
    let plate = transaction.add_feature(
        "Windowed plate",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("3 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );

    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();

    let solid = harness
        .model
        .evaluation()
        .body(plate)
        .expect("the plate is built")
        .clone();
    let top = solid
        .faces()
        .find(|(_, face)| {
            face.origin()
                == Some(FaceOrigin::EndCap {
                    feature: plate.raw(),
                })
        })
        .map(|(id, _)| id)
        .expect("the plate has a top face");

    let (attachment, plane) = FaceAttachment::capture(plate, &solid, top).unwrap();
    let mut disc = Sketch::new(plane);

    disc.add_circle(plane.to_local(Point3::new(0.0, 0.0, THICKNESS)), RADIUS);

    let mut transaction = harness.document().transaction("Disc");

    let sketch = transaction.add_feature(
        "Disc",
        FeatureKind::Sketch(SketchFeature::on_face(disc, attachment)),
    );

    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    harness.edit(sketch);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    Scene {
        sketch,
        plane,
        window_point: plane.to_local(Point3::new(WINDOW_EDGE + 1.0, 0.0, THICKNESS)),
        material_point: plane.to_local(Point3::new(-1.0, 0.0, THICKNESS)),
    }
}

fn piece(harness: &Harness, scene: &Scene, point: Point2) -> Pickable {
    let region = crate::selection::sketch_regions(harness.model.evaluation(), scene.sketch)
        .expect("the sketch has regions")
        .iter()
        .find(|region| region.region.contains(point))
        .map(|region| region.region.key())
        .expect("a piece holds the point");
    Pickable::SketchRegion {
        feature: scene.sketch,
        region,
    }
}

fn key_of(pickable: Pickable) -> RegionKey {
    match pickable {
        Pickable::SketchRegion { region, .. } => region,
        _ => panic!("not a region"),
    }
}

#[test]
fn hovering_inside_the_window_lights_only_the_piece_of_the_circle_inside_it() {
    let mut harness = Harness::new();

    let scene = circle_over_a_window(&mut harness);
    let window = piece(&harness, &scene, scene.window_point);
    let material = piece(&harness, &scene, scene.material_point);

    let offered = harness
        .built()
        .picks
        .pickables()
        .filter(|pickable| matches!(pickable, Pickable::SketchRegion { .. }))
        .count();

    harness.hover_pickable(scene.plane, scene.window_point, window);

    let built = harness.built();
    let fill_of = |pickable: Pickable| {
        built
            .scene
            .fills()
            .find(|fill| fill.pick == built.picks.id_of(pickable))
            .expect("the piece is drawn")
            .color
    };

    assert_eq!(offered, 2);
    assert_ne!(window, material);
    assert_ne!(
        fill_of(window),
        scene_palette::STANDARD.closed_region,
        "the piece under the pointer is lit"
    );
    assert_eq!(
        fill_of(material),
        scene_palette::STANDARD.closed_region,
        "the rest of the circle is not"
    );
}

#[test]
fn the_piece_picked_inside_the_window_is_extruded_alone() {
    let mut harness = Harness::new();

    let scene = circle_over_a_window(&mut harness);
    let window = piece(&harness, &scene, scene.window_point);

    harness.click_pickable(scene.plane, scene.window_point, window);
    harness.click("Extrude");
    harness.settle();

    let extrude = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    let RegionChoice::Chosen(references) = harness.solid(extrude).regions().clone() else {
        panic!("the extrusion sweeps every region");
    };
    let SolidFeature::Extrude(opened) = harness.solid(extrude).clone() else {
        panic!("the feature is an extrusion");
    };
    let alone = Extrude {
        extent: ExtrudeExtent::one_side(Expression::parse_stored("4 mm").unwrap(), false),
        operation: BodyOperation::NewBody,
        ..opened
    };

    harness.perform(Action::Apply(Transaction::single(
        "A body of its own",
        Edit::SetFeatureKind {
            id: extrude,
            kind: FeatureKind::Solid(SolidFeature::Extrude(alone)),
        },
    )));
    harness.settle();

    let volume = harness.body_volume(extrude);

    assert_eq!(
        references
            .iter()
            .map(caditor_kernel::RegionReference::key)
            .collect::<Vec<_>>(),
        [key_of(window)]
    );
    assert!(
        (volume - window_piece_area() * DEPTH).abs() < 0.1,
        "{volume} vs {}",
        window_piece_area() * DEPTH
    );
}
