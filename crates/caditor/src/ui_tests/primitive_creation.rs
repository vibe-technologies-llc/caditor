use std::f64::consts::PI;

use caditor_document::BodyOperation;
use caditor_geometry::{Plane, Point2, Point3};
use egui::{Id, Key, Modifiers};

use super::{Harness, body_centre, extruded_plate, pickable_described, primitive_of, volume_about};
use crate::selection::{Pickable, PrincipalPlane};

#[test]
fn a_box_starts_on_the_xy_plane_and_a_click_places_it_where_it_goes() {
    let mut harness = Harness::new();
    harness.select([]);
    harness.use_tool_with(Key::B, Modifiers::ALT);
    harness.settle();
    let feature = harness.workspace.editing.solid().expect("the box is open");
    assert_eq!(harness.model.undo_label(), Some("Create Box 1"));
    assert_eq!(
        harness
            .workspace
            .editing
            .picking()
            .map(|picking| picking.slot),
        Some(crate::reference_picking::Slot::PrimitivePlace)
    );
    assert!(harness.shows(
        "Click a plane or flat face where the box goes, or press Escape to leave it where it is."
    ));
    assert!(volume_about(&harness, feature, 20.0 * 20.0 * 10.0));
    assert!(body_centre(&harness, feature).distance(Point3::new(0.0, 0.0, 5.0)) < 1e-6);

    harness.click_pickable(
        Plane::XY,
        Point2::new(6.0, -4.0),
        Pickable::Plane(PrincipalPlane::Xy),
    );
    harness.settle();
    assert_eq!(harness.workspace.editing.picking(), None);
    assert_eq!(harness.model.undo_label(), Some("Edit Box 1"));
    assert!(body_centre(&harness, feature).distance(Point3::new(6.0, -4.0, 5.0)) < 0.01);

    harness.type_into_field(Id::new(("primitive-field", feature, ("size", 0))), "40 mm");
    harness.settle();
    assert!(volume_about(&harness, feature, 40.0 * 20.0 * 10.0));
    assert!(harness.shows("New body"));
    assert!(harness.document().feature(feature).unwrap().makes_body());
}

#[test]
fn a_cylinder_on_a_selected_face_joins_its_body_and_cuts_into_it_when_switched() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);

    harness.select([top]);
    harness.use_tool_with(Key::Y, Modifiers::ALT);
    harness.settle();
    let cylinder = harness
        .workspace
        .editing
        .solid()
        .expect("the cylinder is open");
    assert_eq!(harness.model.undo_label(), Some("Create Cylinder 1"));
    assert_eq!(harness.workspace.editing.picking(), None);
    assert_eq!(
        primitive_of(&harness, cylinder).operation,
        BodyOperation::Add(plate)
    );
    assert!(volume_about(&harness, plate, 16000.0 + PI * 25.0 * 20.0));
    assert!(body_centre(&harness, plate).distance(Point3::new(20.0, 20.0, 15.0)) < 0.01);

    harness.click("Add to body");
    harness.click("Remove from body");
    harness.settle();
    let cut = primitive_of(&harness, cylinder);
    assert_eq!(cut.operation, BodyOperation::Remove(plate));
    assert!(cut.reversed);
    assert!(volume_about(&harness, plate, 16000.0 - PI * 25.0 * 10.0));
}

#[test]
fn activating_the_new_cylinder_s_highlighted_face_selects_it_and_stops_choosing_its_place() {
    let mut harness = Harness::new();
    harness.select([]);
    harness.use_tool_with(Key::Y, Modifiers::ALT);
    harness.settle();
    assert!(harness.workspace.editing.picking().is_some());

    let top = pickable_described(&mut harness, "Cylinder 1 › Cylinder 1 top face");
    for _ in 0..40 {
        if harness.workspace.viewport.keyboard_highlight() == Some(top) {
            break;
        }
        harness.key(Key::N, Modifiers::NONE);
        harness.frame();
    }
    assert_eq!(harness.workspace.viewport.keyboard_highlight(), Some(top));

    harness.key(Key::Space, Modifiers::NONE);
    harness.settle();

    assert_eq!(harness.workspace.editing.picking(), None);
    assert!(harness.workspace.viewport.selection().contains(top));
}
