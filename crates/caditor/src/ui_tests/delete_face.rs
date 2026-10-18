use std::f64::consts::PI;

use caditor_document::FeatureId;
use caditor_geometry::{Plane, Point2};
use caditor_kernel::Surface;
use caditor_sketch::Sketch;
use egui::{Key, Modifiers};

use super::{Harness, rectangle, volume_about};
use crate::{bodies, selection::Pickable};

const PLATE: f64 = 40.0 * 40.0 * 10.0;

fn delete_face_of(harness: &Harness, feature: FeatureId) -> &caditor_document::DeleteFace {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.delete_face())
        .unwrap()
}

pub(super) fn holed_plate(harness: &mut Harness) -> (FeatureId, Pickable, Pickable) {
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    sketch.add_circle(Point2::new(20.0, 20.0), 5.0);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let plate = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let faces: Vec<Pickable> = harness
        .built()
        .picks
        .pickables()
        .filter(|pickable| matches!(pickable, Pickable::Face { .. }))
        .collect();
    let surface_of = |pickable: &Pickable| {
        let Pickable::Face { body, face } = *pickable else {
            return None;
        };
        let result = harness.model.evaluation().body_result(body)?.solid()?;
        let found = bodies::find_face(result, face)?;
        Some(result.solid.face(found)?.surface().clone())
    };
    let bore = faces
        .iter()
        .copied()
        .find(|pickable| matches!(surface_of(pickable), Some(Surface::Cylinder(_))))
        .expect("the bore is pickable");
    let top = faces
        .iter()
        .copied()
        .find(|pickable| {
            pickable.describe(harness.document(), harness.model.evaluation())
                == "Plate › Extrude 1 end face"
                || pickable.describe(harness.document(), harness.model.evaluation())
                    == "Extrude 1 › Extrude 1 end face"
        })
        .expect("the top is pickable");
    (plate, bore, top)
}

#[test]
fn delete_face_fills_a_bore_and_takes_or_keeps_faces_clicked_in_the_view() {
    let mut harness = Harness::new();
    let (plate, bore, top) = holed_plate(&mut harness);
    let (Pickable::Face { face: bore_key, .. }, Pickable::Face { face: top_key, .. }) = (bore, top)
    else {
        panic!("the bore and the top are faces");
    };
    assert!(volume_about(&harness, plate, PLATE - PI * 25.0 * 10.0));

    harness.select([bore]);
    harness.use_tool_with(Key::Q, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    let deletion = harness
        .workspace
        .editing
        .solid()
        .expect("the delete face is open");
    assert_eq!(harness.model.undo_label(), Some("Create Delete face 1"));
    assert_eq!(delete_face_of(&harness, deletion).faces.len(), 1);
    assert!(volume_about(&harness, plate, PLATE));
    assert!(harness.shows("Click faces to delete them, and again to keep one"));
    assert!(harness.shows("Faces to delete"));

    harness.click_pickable(
        Plane::XY,
        Point2::new(20.0, 20.0),
        Pickable::ShellFace {
            feature: deletion,
            face: top_key,
        },
    );
    harness.settle();
    assert_eq!(delete_face_of(&harness, deletion).faces.len(), 2);
    assert_eq!(
        harness.model.undo_label(),
        Some("Delete a face with Delete face 1")
    );
    assert_eq!(harness.model.evaluation().failed_count(), 1);

    harness.click_pickable(
        Plane::XY,
        Point2::new(20.0, 20.0),
        Pickable::ShellFace {
            feature: deletion,
            face: top_key,
        },
    );
    harness.settle();
    assert_eq!(delete_face_of(&harness, deletion).faces.len(), 1);
    assert_eq!(
        harness.model.undo_label(),
        Some("Keep a face out of Delete face 1")
    );
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(volume_about(&harness, plate, PLATE));

    harness.click_pickable(
        Plane::XY,
        Point2::new(20.0, 20.0),
        Pickable::ShellFace {
            feature: deletion,
            face: bore_key,
        },
    );
    harness.settle();
    assert!(delete_face_of(&harness, deletion).faces.is_empty());
    assert!(volume_about(&harness, plate, PLATE - PI * 25.0 * 10.0));

    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }
    assert_eq!(harness.workspace.editing.solid(), None);
}

#[test]
fn the_delete_face_command_needs_faces_of_a_body_and_undoes_in_one_step() {
    let mut harness = Harness::new();
    let (plate, bore, _) = holed_plate(&mut harness);

    harness.select([]);
    harness.use_tool_with(Key::Q, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    assert_eq!(harness.workspace.editing.solid(), None);
    assert_ne!(harness.model.undo_label(), Some("Create Delete face 1"));

    harness.select([bore]);
    harness.use_tool_with(Key::Q, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    assert!(volume_about(&harness, plate, PLATE));
    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();
    assert!(volume_about(&harness, plate, PLATE - PI * 25.0 * 10.0));
}
