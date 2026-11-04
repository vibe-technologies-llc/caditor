use std::f64::consts::PI;

use caditor_document::{FeatureId, PlaneReference, ReplaceFace};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::Surface;
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{Harness, delete_face::holed_plate, extruded_plate, run_from_palette, volume_about};
use crate::{bodies, replace_face_panel, selection::Pickable};

const SIDE: f64 = 40.0;
const HEIGHT: f64 = 10.0;

fn replace_of(harness: &Harness, feature: FeatureId) -> &ReplaceFace {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.replace_face())
        .unwrap()
}

fn close_feature(harness: &mut Harness) {
    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }
}

fn datum_plane_at(harness: &mut Harness, offset: &str) -> FeatureId {
    harness.select([]);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the new plane is open");
    harness.type_into_field(Id::unique(("datum-field", "offset", plane)), offset);
    harness.settle();
    close_feature(harness);
    plane
}

#[test]
fn the_palette_replaces_a_selected_top_with_a_selected_datum_plane_and_undoes() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let plane = datum_plane_at(&mut harness, "15 mm");

    harness.select([top, Pickable::Datum(plane)]);
    run_from_palette(&mut harness, "Replace face");
    harness.settle();
    let replace = harness
        .workspace
        .editing
        .solid()
        .expect("the replacement is open");

    assert_eq!(harness.model.undo_label(), Some("Create Replace face 1"));
    assert_eq!(replace_of(&harness, replace).faces.len(), 1);
    assert_eq!(
        replace_of(&harness, replace).target,
        Some(PlaneReference::Datum(plane))
    );
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(volume_about(&harness, plate, SIDE * SIDE * 15.0));
    assert!(harness.shows("Faces to replace"));
    assert!(harness.shows(replace_face_panel::TARGET));
    assert!(harness.shows("Click faces to replace them, and again to leave one out"));

    close_feature(&mut harness);
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();

    assert!(volume_about(&harness, plate, SIDE * SIDE * HEIGHT));
}

#[test]
fn a_top_chosen_alone_waits_for_its_target() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);

    harness.select([top]);
    run_from_palette(&mut harness, "Replace face");
    harness.settle();
    let replace = harness
        .workspace
        .editing
        .solid()
        .expect("the replacement is open");

    assert_eq!(replace_of(&harness, replace).target, None);
    assert!(harness.shows("Waiting for a target"));
    assert!(harness.shows_containing("Choose the plane or face to replace the faces"));
    assert!(volume_about(&harness, plate, SIDE * SIDE * HEIGHT));
}

#[test]
fn a_bore_takes_the_radius_of_another_body_s_cylinder_on_its_axis() {
    let mut harness = Harness::new();
    let (plate, bore, _) = holed_plate(&mut harness);
    let above = Plane::from_frame(Point3::new(0.0, 0.0, 50.0), Vector3::Z, Vector3::X).unwrap();
    let mut sketch = Sketch::new(above);
    sketch.add_circle(Point2::new(20.0, 20.0), 8.0);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let wall = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            let Pickable::Face { body, face } = *pickable else {
                return false;
            };
            let Some(result) = harness
                .model
                .evaluation()
                .body_result(body)
                .and_then(|result| result.solid())
            else {
                return false;
            };
            body != plate
                && bodies::find_face(result, face)
                    .and_then(|found| result.solid.face(found))
                    .is_some_and(|found| matches!(found.surface(), Surface::Cylinder(_)))
        })
        .expect("the other body's cylinder is pickable");

    harness.select([bore, wall]);
    run_from_palette(&mut harness, "Replace face");
    harness.settle();
    let replace = harness
        .workspace
        .editing
        .solid()
        .expect("the replacement is open");

    assert!(matches!(
        replace_of(&harness, replace).target,
        Some(PlaneReference::Face(_))
    ));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(volume_about(
        &harness,
        plate,
        (SIDE * SIDE - PI * 64.0) * HEIGHT
    ));
}
