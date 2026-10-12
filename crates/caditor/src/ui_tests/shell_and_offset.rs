use caditor_document::FeatureId;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, draft_volume, extruded_plate, offset_of, open_combo,
    pickable_described, removed_about, run_from_palette, shell_of, volume_about,
};
use crate::selection::Pickable;

#[test]
fn a_shell_opens_the_selected_face_and_takes_more_faces_clicked_in_the_view() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let Pickable::Face { face: top_key, .. } = top else {
        panic!("the top is a face");
    };

    harness.select([top]);
    harness.click("Shell");
    harness.settle();
    let shell = harness
        .workspace
        .editing
        .solid()
        .expect("the shell is open");
    assert_eq!(harness.model.undo_label(), Some("Create Shell 1"));
    assert_eq!(shell_of(&harness, shell).open.len(), 1);
    assert!(removed_about(&harness, plate, 38.0 * 38.0 * 9.0));
    assert!(harness.shows("Click a face to shell the body open there, and again to close it"));
    assert!(harness.shows("Thickness"));

    let built = harness.built_with_meshes(2);
    assert_eq!(built.scene.meshes.len(), 1);
    let shell_faces = built
        .picks
        .pickables()
        .filter(|pickable| matches!(pickable, Pickable::ShellFace { .. }))
        .count();
    assert_eq!(shell_faces, 7);

    let bottom = pickable_described(
        &mut harness,
        "Extrude 1 start face: click to open it in Shell 1 or close it again",
    );
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), bottom);
    harness.settle();
    assert_eq!(shell_of(&harness, shell).open.len(), 2);
    assert_eq!(harness.model.undo_label(), Some("Open a face of Shell 1"));
    assert!(removed_about(&harness, plate, 38.0 * 38.0 * 10.0));

    harness.type_into_field(Id::new(("shell-thickness", shell)), "2 mm");
    harness.settle();
    assert!(removed_about(&harness, plate, 36.0 * 36.0 * 10.0));

    harness.type_into_field(Id::new(("shell-thickness", shell)), "0 mm");
    assert!(harness.shows(crate::feature_fields::ABOVE_ZERO));
    assert_eq!(harness.workspace.editing.solid(), Some(shell));

    harness.click_pickable(
        Plane::XY,
        Point2::new(20.0, 20.0),
        Pickable::ShellFace {
            feature: shell,
            face: top_key,
        },
    );
    harness.settle();
    assert_eq!(shell_of(&harness, shell).open.len(), 1);
    assert_eq!(harness.model.undo_label(), Some("Close a face of Shell 1"));
    assert!(removed_about(&harness, plate, 36.0 * 36.0 * 8.0));

    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }
    assert_eq!(harness.workspace.editing.solid(), None);
    let inner = pickable_described(&mut harness, "Extrude 1 › Shell 1 inner face");
    assert!(matches!(inner, Pickable::Face { .. }));
}

#[test]
fn an_open_shell_draws_its_hollow_and_previews_a_typed_thickness() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let Pickable::Face { face: top_key, .. } = top else {
        panic!("the top is a face");
    };
    harness.select([top]);
    harness.click("Shell");
    harness.settle();
    let shell = harness
        .workspace
        .editing
        .solid()
        .expect("the shell is open");
    let field = Id::new(("shell-thickness", shell));

    let built = harness.built_with_meshes(2);
    let hollow = built.scene.meshes.first().expect("the shell is drawn");
    let opened = built
        .scene
        .translucent_meshes
        .first()
        .expect("the opened face is drawn over it");
    let opened_picks = opened
        .faces
        .iter()
        .filter(|face| face.pick.is_some())
        .count();
    let wall_picks = hollow
        .faces
        .iter()
        .filter(|face| face.pick.is_some())
        .count();
    assert_eq!(hollow.mesh.face_count(), 11);
    assert_eq!(wall_picks, 6);
    assert_eq!(opened_picks, 1);
    assert!(built.picks.pickables().any(|pickable| pickable
        == Pickable::ShellFace {
            feature: shell,
            face: top_key,
        }));

    harness.draft_into_field(field, "3 mm");
    harness.wait_until("the typed thickness is previewed", |harness| {
        draft_volume(harness, plate).is_some()
    });
    let removed = 16000.0 - draft_volume(&harness, plate).unwrap();
    assert!((removed - 34.0 * 34.0 * 7.0).abs() < 0.1 * 34.0 * 34.0 * 7.0);
    assert_eq!(harness.model.undo_label(), Some("Create Shell 1"));

    harness.draft_into_field(field, "30 mm");
    harness.wait_until("the typed thickness fails in the preview", |harness| {
        harness.model.draft_failure(shell).is_some()
    });
    harness.frame();
    assert!(harness.shows_containing(crate::feature_fields::DRAFT_FAILS));
    assert_eq!(harness.model.undo_label(), Some("Create Shell 1"));
}

#[test]
fn the_shell_button_needs_faces_of_a_body_and_opens_every_selected_one() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let built = harness.built();
    let side = built
        .picks
        .pickables()
        .find(|pickable| matches!(pickable, Pickable::Face { .. }) && *pickable != top)
        .expect("the plate has other faces");

    harness.select([]);
    harness.click("Shell");
    harness.settle();
    assert_eq!(harness.workspace.editing.solid(), None);
    assert_ne!(harness.model.undo_label(), Some("Create Shell 1"));

    harness.select([top, side]);
    harness.click("Shell");
    harness.settle();
    let shell = harness
        .workspace
        .editing
        .solid()
        .expect("the shell is open");
    assert_eq!(shell_of(&harness, shell).open.len(), 2);
    assert!(harness.body_volume(plate) < 16000.0 - 38.0 * 38.0 * 9.0);
}

#[test]
fn offset_face_moves_the_selected_face_and_follows_a_typed_distance_and_clicked_faces() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let Pickable::Face { face: top_key, .. } = top else {
        panic!("the top is a face");
    };

    harness.select([top]);
    harness.use_tool_with(Key::Q, Modifiers::ALT);
    harness.settle();
    let offset = harness
        .workspace
        .editing
        .solid()
        .expect("the offset face is open");
    assert_eq!(harness.model.undo_label(), Some("Create Offset face 1"));
    assert_eq!(offset_of(&harness, offset).faces.len(), 1);
    assert!(volume_about(&harness, plate, 40.0 * 40.0 * 11.0));
    assert!(harness.shows("Click faces to move them, and again to leave one out"));
    assert!(harness.shows("Distance"));

    harness.type_into_field(Id::new(("offset-face-distance", offset)), "3 mm");
    harness.settle();
    assert!(volume_about(&harness, plate, 40.0 * 40.0 * 13.0));

    harness.type_into_field(Id::new(("offset-face-distance", offset)), "-2 mm");
    harness.settle();
    assert!(volume_about(&harness, plate, 40.0 * 40.0 * 8.0));

    let bottom = pickable_described(
        &mut harness,
        "Extrude 1 start face: click to move it with Offset face 1 or leave it out",
    );
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), bottom);
    harness.settle();
    assert_eq!(offset_of(&harness, offset).faces.len(), 2);
    assert_eq!(
        harness.model.undo_label(),
        Some("Move a face with Offset face 1")
    );
    assert!(volume_about(&harness, plate, 40.0 * 40.0 * 6.0));

    harness.click_pickable(
        Plane::XY,
        Point2::new(20.0, 20.0),
        Pickable::ShellFace {
            feature: offset,
            face: top_key,
        },
    );
    harness.settle();
    assert_eq!(offset_of(&harness, offset).faces.len(), 1);
    assert_eq!(
        harness.model.undo_label(),
        Some("Leave a face out of Offset face 1")
    );
    assert!(volume_about(&harness, plate, 40.0 * 40.0 * 8.0));

    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }
    assert_eq!(harness.workspace.editing.solid(), None);
}

fn split_face_of(harness: &Harness, feature: FeatureId) -> &caditor_document::SplitFace {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.split_face())
        .unwrap()
}

fn face_count(harness: &Harness, body: FeatureId) -> usize {
    harness
        .model
        .evaluation()
        .body_result(body)
        .and_then(|result| result.solid())
        .map_or(0, |solid| solid.solid.faces().count())
}

#[test]
fn split_face_divides_the_selected_face_along_a_sketch_curve_and_takes_clicked_faces() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let mut line = Sketch::new(Plane::XY);
    let entity = line.add_line(Point2::new(20.0, -10.0), Point2::new(20.0, 50.0));
    let part_line = harness.add_sketch(line);

    harness.select([
        top,
        Pickable::SketchEntity {
            feature: part_line,
            entity,
        },
    ]);
    harness.use_tool_with(Key::K, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    let split = harness
        .workspace
        .editing
        .solid()
        .expect("the split face is open");
    assert_eq!(harness.model.undo_label(), Some("Create Split face 1"));
    assert_eq!(
        split_face_of(&harness, split).along,
        caditor_document::SplitAlong::Sketch(part_line)
    );
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert_eq!(face_count(&harness, plate), 7);
    assert!(volume_about(&harness, plate, 40.0 * 40.0 * 10.0));
    assert!(harness.shows("Click faces to split, and again to leave one out"));
    assert!(harness.shows("Faces to split"));
    assert!(harness.shows("Split along"));

    let bottom = pickable_described(
        &mut harness,
        "Extrude 1 start face: click to split it with Split face 1 or leave it out",
    );
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), bottom);
    harness.settle();
    assert_eq!(split_face_of(&harness, split).faces.len(), 2);
    assert_eq!(
        harness.model.undo_label(),
        Some("Split a face with Split face 1")
    );
    assert_eq!(face_count(&harness, plate), 8);

    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), bottom);
    harness.settle();
    assert_eq!(split_face_of(&harness, split).faces.len(), 1);
    assert_eq!(face_count(&harness, plate), 7);

    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }
    assert_eq!(harness.workspace.editing.solid(), None);
}

#[test]
fn split_face_carries_its_sketch_along_a_chosen_line_from_its_panel() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let mut line = Sketch::new(Plane::XY);
    let entity = line.add_line(Point2::new(20.0, -10.0), Point2::new(20.0, 50.0));
    let part_line = harness.add_sketch(line);
    let mut slanted = Sketch::new(Plane::XZ);
    let slope = slanted.add_line(Point2::new(0.0, 0.0), Point2::new(5.0, 10.0));
    let slant = harness.add_sketch(slanted);
    harness.select([
        top,
        Pickable::SketchEntity {
            feature: part_line,
            entity,
        },
    ]);
    harness.use_tool_with(Key::K, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    let split = harness
        .workspace
        .editing
        .solid()
        .expect("the split face is open");
    let on_top_at = |harness: &Harness, x: f64| {
        harness
            .model
            .evaluation()
            .body(plate)
            .unwrap()
            .vertices()
            .any(|(_, vertex)| {
                let point = vertex.point();
                (point.z - 10.0).abs() < 1e-6 && (point.x - x).abs() < 1e-6
            })
    };
    assert!(harness.shows("Carried"));
    assert!(on_top_at(&harness, 20.0));

    harness.select([Pickable::SketchEntity {
        feature: slant,
        entity: slope,
    }]);
    open_combo(&mut harness, "Carried");
    harness.click_lowest("Along an edge or axis");
    harness.settle();
    assert_eq!(
        split_face_of(&harness, split).direction(),
        Some(&caditor_document::AxisReference::Sketch {
            sketch: slant,
            entity: slope,
        })
    );
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert_eq!(face_count(&harness, plate), 7);
    assert!(on_top_at(&harness, 25.0));
    assert!(!on_top_at(&harness, 20.0));
    assert!(harness.shows("Along"));

    open_combo(&mut harness, "Carried");
    harness.click_lowest("Square to the sketch");
    harness.settle();
    assert_eq!(split_face_of(&harness, split).direction(), None);
    assert!(on_top_at(&harness, 20.0));
}

#[test]
fn split_face_wraps_its_sketch_round_a_cylinder_from_the_palette() {
    let mut harness = Harness::new();
    let mut disc = Sketch::new(Plane::XY);
    disc.add_circle(Point2::new(0.0, 0.0), 10.0);
    harness.add_sketch(disc);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let rod = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let tangent = Plane::from_frame(
        caditor_geometry::Point3::new(0.0, 10.0, 0.0),
        caditor_geometry::Vector3::Y,
        caditor_geometry::Vector3::Z,
    )
    .unwrap();
    let mut label = Sketch::new(tangent);
    let edge = label.add_line(Point2::new(2.0, -3.0), Point2::new(7.0, -3.0));
    label.add_line(Point2::new(7.0, -3.0), Point2::new(7.0, 3.0));
    label.add_line(Point2::new(7.0, 3.0), Point2::new(2.0, 3.0));
    label.add_line(Point2::new(2.0, 3.0), Point2::new(2.0, -3.0));
    let label = harness.add_sketch(label);
    let walls = |harness: &Harness| {
        harness
            .model
            .evaluation()
            .body(rod)
            .unwrap()
            .faces()
            .filter(|(_, face)| matches!(face.surface(), caditor_kernel::Surface::Cylinder(_)))
            .count()
    };
    let wall = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            let Pickable::Face { body, face } = pickable else {
                return false;
            };
            let result = harness
                .model
                .evaluation()
                .body_result(*body)
                .and_then(|result| result.solid())
                .unwrap();
            crate::bodies::find_face(result, *face)
                .and_then(|id| result.solid.face(id))
                .is_some_and(|face| matches!(face.surface(), caditor_kernel::Surface::Cylinder(_)))
        })
        .expect("the wall is pickable");
    harness.select([
        wall,
        Pickable::SketchEntity {
            feature: label,
            entity: edge,
        },
    ]);
    harness.use_tool_with(Key::K, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    let split = harness
        .workspace
        .editing
        .solid()
        .expect("the split face is open");
    assert_eq!(walls(&harness), 3);

    run_from_palette(&mut harness, "wrap the split");
    harness.settle();

    assert!(split_face_of(&harness, split).is_wrapped());
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert_eq!(walls(&harness), 2);
    assert!(harness.shows("Wrapped round the faces"));
}

#[test]
fn the_offset_face_command_needs_faces_of_a_body_and_takes_every_selected_one() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let built = harness.built();
    let side = built
        .picks
        .pickables()
        .find(|pickable| matches!(pickable, Pickable::Face { .. }) && *pickable != top)
        .expect("the plate has other faces");

    harness.select([]);
    harness.use_tool_with(Key::Q, Modifiers::ALT);
    harness.settle();
    assert_eq!(harness.workspace.editing.solid(), None);
    assert_ne!(harness.model.undo_label(), Some("Create Offset face 1"));

    harness.select([top, side]);
    harness.use_tool_with(Key::Q, Modifiers::ALT);
    harness.settle();
    let offset = harness
        .workspace
        .editing
        .solid()
        .expect("the offset face is open");
    assert_eq!(offset_of(&harness, offset).faces.len(), 2);
    assert!(harness.body_volume(plate) > 16000.0);
}
