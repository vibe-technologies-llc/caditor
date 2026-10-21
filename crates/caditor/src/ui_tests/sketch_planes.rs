use caditor_document::{
    BodyOperation, Extrude, ExtrudeExtent, FeatureKind, RegionChoice, SolidFeature,
};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, attached_body, extruded_plate, offer, open_combo, plane_height,
    rectangle, run_from_palette,
};
use crate::{
    commands::Command,
    editing::EditingCommand,
    model::Action,
    selection::{Pickable, PrincipalPlane},
};

#[test]
fn an_empty_new_sketch_that_something_uses_is_kept_and_the_notice_says_why() {
    let mut harness = Harness::new();
    harness.select([Pickable::Plane(PrincipalPlane::Xz)]);
    harness.click("New sketch");
    let feature = harness.editing().expect("the new sketch is being edited");
    let mut transaction = harness.document().transaction("Extrude the empty sketch");
    transaction.add_feature(
        "Empty extrusion",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: feature,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("2 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    harness.perform(Action::Apply(transaction.finish()));
    let features = harness.document().features().len();

    harness.click("Finish sketch");

    assert_eq!(harness.editing(), None);
    assert_eq!(harness.document().features().len(), features);
    assert!(harness.document().feature(feature).is_some());
    assert!(harness.shows("Kept the empty Sketch 1: Empty extrusion uses it."));
}

#[test]
fn a_new_sketch_on_the_selected_plane_is_edited_until_finished() {
    let mut harness = Harness::new();
    harness.select([Pickable::Plane(PrincipalPlane::Xz)]);
    harness.click("New sketch");

    let feature = harness.editing().expect("the new sketch is being edited");
    let created = harness.document().feature(feature).unwrap();
    assert_eq!(created.name, "Sketch 1");
    assert_eq!(harness.sketch(feature).plane(), Plane::XZ);
    assert_eq!(harness.model.undo_label(), Some("Create Sketch 1"));
    assert!(harness.workspace.viewport.selection().is_empty());
    harness.settle();
    assert!(harness.shows("Editing Sketch 1"));
    assert!(harness.shows("Fully constrained"));

    harness.click("Finish sketch");
    assert_eq!(harness.editing(), None);
    assert!(!harness.shows("Editing Sketch 1"));
    assert_eq!(harness.document().features().len(), 2);
    assert!(harness.shows("Removed the empty Sketch 1; Redo brings it back."));

    harness.perform(Action::Redo);
    assert_eq!(harness.document().features().len(), 3);
    assert_eq!(harness.editing(), None);

    harness.click("New sketch");
    assert!(harness.workspace.editing.is_choosing_plane());
    assert!(harness.shows("Click a plane or a flat face to sketch on"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(!harness.workspace.editing.is_choosing_plane());
    assert!(!harness.shows("Click a plane or a flat face to sketch on"));

    harness.perform(Action::Editing(EditingCommand::NewSketch(Some(
        PrincipalPlane::Yz,
    ))));
    let second = harness.editing().unwrap();
    assert_eq!(harness.document().feature(second).unwrap().name, "Sketch 2");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.editing(), None);
    assert!(harness.document().feature(second).is_none());
    assert_eq!(harness.document().features().len(), 3);
}

#[test]
fn a_new_sketch_s_plane_is_chosen_from_its_card_as_well_as_in_the_view() {
    let mut harness = Harness::new();
    harness.select([]);
    harness.click("New sketch");
    harness.frame();

    assert!(harness.workspace.editing.is_choosing_plane());
    assert!(harness.shows(crate::plane_selector::UNCHOSEN));

    harness.click(crate::new_sketch_panel::CANCEL);
    harness.frame();

    assert!(!harness.workspace.editing.is_choosing_plane());
    assert!(!harness.shows(crate::plane_selector::UNCHOSEN));

    harness.click("New sketch");
    harness.frame();
    open_combo(&mut harness, crate::new_sketch_panel::PLANE);
    harness.click_lowest("The XZ plane");
    harness.frame();

    let feature = harness.editing().expect("the chosen plane is sketched on");
    assert_eq!(harness.sketch(feature).plane(), Plane::XZ);
    assert_eq!(harness.model.undo_label(), Some("Create Sketch 1"));
    assert!(!harness.workspace.editing.is_choosing_plane());
}

#[test]
fn undoing_the_creation_of_the_edited_sketch_ends_editing() {
    let mut harness = Harness::new();
    harness.select([Pickable::Plane(PrincipalPlane::Xy)]);
    harness.click("New sketch");
    assert!(harness.editing().is_some());

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.frame();
    assert_eq!(harness.editing(), None);
    assert_eq!(harness.document().features().len(), 2);
    assert!(!harness.shows("Finish sketch"));
}

#[test]
fn a_sketch_started_on_a_selected_face_follows_it_when_the_body_changes() {
    let mut harness = Harness::new();
    let (extrude, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.hover("New sketch");
    assert!(harness.shows_containing(
        "Start a sketch on the selected face; it follows the face when the model changes"
    ));
    harness.click("New sketch");
    harness.settle();

    let sketch = harness.editing().expect("the new sketch is edited");
    assert_eq!(harness.model.undo_label(), Some("Create Sketch 1"));
    assert_eq!(attached_body(&harness, sketch), Some(extrude));
    assert_eq!(plane_height(&harness, sketch), 10.0);
    assert!(harness.shows("Lies on Extrude 1 end face"));

    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(20.0, 30.0));
    harness.settle();
    harness.click("Extrude");
    harness.settle();
    let boss = harness
        .workspace
        .editing
        .solid()
        .expect("the second extrusion is open");
    assert_eq!(
        harness.solid(boss).operation(),
        BodyOperation::Remove(extrude)
    );
    assert!((harness.body_volume(extrude) - (16000.0 - 2000.0)).abs() < 1.0);

    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.type_into_field(Id::unique(("solid-field", "distance", extrude)), "25 mm");
    harness.settle();
    assert_eq!(plane_height(&harness, sketch), 25.0);
    assert!((harness.body_volume(extrude) - (40000.0 - 2000.0)).abs() < 1.0);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn clicking_a_flat_face_while_choosing_a_plane_starts_a_sketch_on_it() {
    let mut harness = Harness::new();
    let (extrude, top) = extruded_plate(&mut harness);
    harness.select([]);
    harness.click("New sketch");
    assert!(harness.workspace.editing.is_choosing_plane());
    assert!(harness.shows("Click a plane or a flat face to sketch on"));

    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    harness.settle();

    let sketch = harness.editing().expect("the new sketch is edited");
    assert!(!harness.workspace.editing.is_choosing_plane());
    assert_eq!(attached_body(&harness, sketch), Some(extrude));
    assert_eq!(plane_height(&harness, sketch), 10.0);
}

#[test]
fn the_tree_places_a_sketch_on_the_selected_face_and_detaches_it() {
    let mut harness = Harness::new();
    let (extrude, top) = extruded_plate(&mut harness);
    let mut transaction = harness.document().transaction("Add sketch");
    let mut loose = Sketch::new(Plane::XY);
    rectangle(&mut loose, Point2::new(5.0, 5.0), Point2::new(15.0, 15.0));
    let sketch = transaction.add_feature("Loose", FeatureKind::from(loose));
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();

    harness.select([top]);
    harness.click_button("Show details of Loose");
    harness.click("Place on selected face");
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Place Loose on a face"));
    assert_eq!(attached_body(&harness, sketch), Some(extrude));
    assert_eq!(plane_height(&harness, sketch), 10.0);
    assert!(harness.shows("Lies on Extrude 1 end face"));

    harness.click("Detach");
    harness.settle();
    assert_eq!(
        harness.model.undo_label(),
        Some("Detach Loose from its face")
    );
    assert_eq!(attached_body(&harness, sketch), None);
    assert_eq!(harness.sketch(sketch).plane().origin().z, 10.0);
    assert!(!harness.shows("Lies on Extrude 1 end face"));

    harness.perform(Action::Undo);
    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(attached_body(&harness, sketch), None);
    assert_eq!(plane_height(&harness, sketch), 0.0);
}

#[test]
fn a_sketch_is_placed_on_the_selected_face_and_detached_from_the_palette() {
    let mut harness = Harness::new();
    let (extrude, top) = extruded_plate(&mut harness);
    let mut transaction = harness.document().transaction("Add sketch");
    let mut loose = Sketch::new(Plane::XY);
    rectangle(&mut loose, Point2::new(5.0, 5.0), Point2::new(15.0, 15.0));
    let sketch = transaction.add_feature("Loose", FeatureKind::from(loose));
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();

    harness.click("Loose");
    assert_eq!(
        offer(&harness, Command::PlaceSketch).availability,
        Err("Select a datum plane or a flat face to place the sketch on".into())
    );
    assert_eq!(
        offer(&harness, Command::DetachSketch).availability,
        Err("Loose does not lie on a face or datum plane".into())
    );

    harness.select([top]);
    run_from_palette(&mut harness, "place sketch");
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Place Loose on a face"));
    assert_eq!(attached_body(&harness, sketch), Some(extrude));
    assert_eq!(plane_height(&harness, sketch), 10.0);

    run_from_palette(&mut harness, "detach sketch");
    harness.settle();
    assert_eq!(
        harness.model.undo_label(),
        Some("Detach Loose from its face")
    );
    assert_eq!(attached_body(&harness, sketch), None);
    assert_eq!(harness.sketch(sketch).plane().origin().z, 10.0);
}

#[test]
fn leaving_a_sketch_for_a_feature_built_on_it_is_quiet_and_starting_another_says_what_closed() {
    let mut harness = Harness::new();
    let sketch = harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 30.0));
    harness.settle();

    harness.click("Extrude");
    harness.settle();
    let announced = |harness: &Harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.starts_with("Finished"))
    };
    assert!(harness.workspace.editing.solid().is_some());
    assert!(!announced(&harness));

    harness.perform(Action::Editing(EditingCommand::Enter(sketch)));
    harness.perform(Action::Editing(EditingCommand::NewSketch(None)));
    assert!(harness.workspace.editing.is_choosing_plane());
    assert_eq!(
        harness.model.notice().map(|notice| notice.text.as_str()),
        Some("Finished Sketch 1 to start a new sketch.")
    );
}
