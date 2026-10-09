use caditor_geometry::Vector3;

use super::{CAMERA_SETTLE, Harness, edit_base_sketch, extruded_plate, run_from_palette};
use crate::section_panel;

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
