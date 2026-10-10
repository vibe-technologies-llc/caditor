use caditor_document::FeatureId;
use caditor_geometry::{Plane, Point2, Vector3};
use caditor_sketch::Sketch;

use super::{
    CAMERA_SETTLE, Harness, drag_screen, edit_base_sketch, extruded_plate, plate_on_screen,
    rectangle, run_from_palette,
};
use crate::{
    section_panel,
    selection::{Pickable, SelectionFilter},
};

fn shown_normals(harness: &mut Harness) -> Vec<Vector3> {
    harness
        .built()
        .scene
        .section
        .iter()
        .map(|section| section.plane.normal())
        .collect()
}

fn close(normals: &[Vector3], expected: &[Vector3]) -> bool {
    normals.len() == expected.len()
        && normals
            .iter()
            .zip(expected)
            .all(|(normal, expected)| normal.distance(*expected) < 1e-9)
}

fn run(harness: &mut Harness, query: &str) {
    run_from_palette(harness, query);
    harness.frame();
    harness.frame();
}

#[test]
fn a_section_view_cuts_from_the_palette_follows_a_selected_face_and_never_changes_the_model() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    let revision = harness.model.revision();

    run(&mut harness, "show or hide the section view");

    assert!(harness.workspace.section.open);
    assert!(harness.shows(section_panel::TITLE));
    assert!(close(&shown_normals(&mut harness), &[Vector3::NEG_Y]));
    assert!((harness.built().scene.section[0].plane.origin().y - 20.0).abs() < 1e-9);

    harness.select([top]);
    harness.frame();
    run(
        &mut harness,
        "put the current section plane on the selected",
    );

    assert_eq!(harness.workspace.section.cuts[0].base, top);
    assert!(close(&shown_normals(&mut harness), &[Vector3::Z]));
    let lid = harness.built().scene.section[0].plane.origin().z;

    run(&mut harness, "flip the side the current section plane");

    assert!(close(&shown_normals(&mut harness), &[Vector3::NEG_Z]));
    assert!((harness.built().scene.section[0].plane.origin().z - lid).abs() < 1e-9);

    run(&mut harness, "add a section plane");

    assert_eq!(harness.workspace.section.current, 1);
    assert!(close(
        &shown_normals(&mut harness),
        &[Vector3::NEG_Z, Vector3::NEG_Y]
    ));

    run(&mut harness, "remove the current section plane");

    assert_eq!(harness.workspace.section.cuts.len(), 1);

    run(&mut harness, "show or hide the section view");

    assert!(!harness.workspace.section.open);
    assert!(harness.built().scene.section.is_empty());
    assert_eq!(harness.model.revision(), revision);
}

#[test]
fn slicing_while_sketching_cuts_the_bodies_in_front_of_the_sketch_plane_only_while_it_is_edited() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    edit_base_sketch(&mut harness);

    assert!(harness.built().scene.section.is_empty());

    run(&mut harness, "slice the bodies at the sketch plane");

    let section = harness.built().scene.section;
    assert_eq!(section.len(), 1);
    assert!(section[0].plane.normal().distance(Vector3::Z) < 1e-9);
    assert!(section[0].plane.origin().z.abs() < 1e-9);

    harness.click("Finish sketch");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.frame();

    assert!(harness.built().scene.section.is_empty());
    assert!(harness.workspace.viewport.sketch_slice());
}

fn selected_curves(harness: &Harness, sketch: FeatureId) -> usize {
    harness
        .workspace
        .viewport
        .selection()
        .iter()
        .filter(|pickable| matches!(pickable, Pickable::SketchEntity { feature, .. } if *feature == sketch))
        .count()
}

#[test]
fn a_box_over_sketch_curves_leaves_out_what_a_section_plane_cuts_away() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    let plate = harness.add_sketch(sketch);
    harness.select([]);
    run_from_palette(&mut harness, "fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.frame();
    let (low, high) = plate_on_screen(&harness);
    let margin = egui::vec2(12.0, 12.0);
    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::SketchGeometry);

    drag_screen(&mut harness, low - margin, high + margin);
    let whole = selected_curves(&harness, plate);

    run(&mut harness, "show or hide the section view");
    let (low, high) = plate_on_screen(&harness);
    drag_screen(&mut harness, low - margin, high + margin);
    let cut = selected_curves(&harness, plate);

    assert_eq!(whole, 4);
    assert_eq!(cut, 3);
}
