use caditor_document::{Datum, DatumAxis, Edit, PatternKind, Transaction};
use caditor_geometry::{Plane, Point2};
use egui::{Id, Key, Modifiers};

use super::{CAMERA_SETTLE, Harness, datum_of, datum_plane_of, extruded_plate, pattern_of};
use crate::{
    feature_fields::{
        self, ABOVE_ZERO, ABOVE_ZERO_OR_REVERSE, CHOOSE_IN_VIEW, MISSING_BODY, REVERSE_DIRECTION,
        TURN,
    },
    model::Action,
    reference_picking::Slot,
    selection::{Axis, Pickable, PrincipalPlane},
};

fn click_in_view(harness: &mut Harness, pickable: Pickable) {
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(0.0, 0.0), pickable);
    harness.settle();
}

fn picking_slot(harness: &Harness) -> Option<Slot> {
    harness
        .workspace
        .editing
        .picking()
        .map(|picking| picking.slot)
}

#[test]
fn a_pattern_takes_its_second_direction_clicked_in_the_view_after_choose_in_the_view() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    harness.select([]);
    harness.click("Linear pattern");
    harness.settle();
    let pattern = harness
        .workspace
        .editing
        .solid()
        .expect("the pattern is open");
    harness.hold_still();

    let offered = harness.shows(CHOOSE_IN_VIEW);
    harness.click_beside(CHOOSE_IN_VIEW, "Second direction");
    harness.settle();
    let asked = picking_slot(&harness);
    let prompted =
        harness.shows("Click an axis, straight edge or round face to also repeat along.");
    click_in_view(&mut harness, Pickable::Axis(Axis::Y));
    let PatternKind::Linear { second, .. } = &pattern_of(&harness, pattern).kind else {
        panic!("the pattern stays linear");
    };
    let second = second.clone();

    assert!(offered);
    assert_eq!(asked, Some(Slot::PatternSecond));
    assert!(prompted);
    assert!(second.is_some());
    assert_eq!(picking_slot(&harness), None);
    assert_eq!(harness.workspace.editing.solid(), Some(pattern));
    assert_eq!(pattern_of(&harness, pattern).body, plate);
    assert_eq!(harness.model.undo_label(), Some("Edit Linear pattern 1"));
}

#[test]
fn a_datum_takes_its_references_clicked_in_the_view_and_escape_stops_choosing() {
    let mut harness = Harness::new();
    harness.select([]);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the plane is open");
    harness.hold_still();

    harness.click_beside(CHOOSE_IN_VIEW, "Turned about");
    harness.settle();
    let asked = picking_slot(&harness);
    click_in_view(&mut harness, Pickable::Axis(Axis::Z));
    let turned = datum_plane_of(&harness, plane).rotation.is_some();

    harness.click_beside(CHOOSE_IN_VIEW, "Starts from");
    harness.settle();
    let asked_again = picking_slot(&harness);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let stopped = picking_slot(&harness);
    let still_open = harness.workspace.editing.solid();

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.select([Pickable::Axis(Axis::X)]);
    harness.click("Axis");
    harness.settle();
    let axis = harness.workspace.editing.solid().expect("the axis is open");
    harness.hold_still();
    harness.select([]);
    harness.frame();
    harness.click_beside(CHOOSE_IN_VIEW, "Runs along");
    harness.settle();
    click_in_view(&mut harness, Pickable::Plane(PrincipalPlane::Xy));
    let waits_for_a_second =
        harness.shows("Click a second plane or flat face that crosses the first.");
    click_in_view(&mut harness, Pickable::Plane(PrincipalPlane::Xz));
    let meeting = matches!(
        datum_of(&harness, axis),
        Datum::Axis(DatumAxis::Intersection(..))
    );

    assert_eq!(asked, Some(Slot::DatumRotation));
    assert!(turned);
    assert_eq!(asked_again, Some(Slot::DatumBase));
    assert_eq!(stopped, None);
    assert_eq!(still_open, Some(plane));
    assert!(waits_for_a_second);
    assert!(meeting);
    assert_eq!(picking_slot(&harness), None);
    assert!(harness.shows("Defined by"));
}

#[test]
fn every_panel_refuses_a_value_with_the_shared_wording() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.select([]);
    harness.click("Linear pattern");
    harness.settle();
    let pattern = harness
        .workspace
        .editing
        .solid()
        .expect("the pattern is open");
    harness.hold_still();

    harness.type_into_field(Id::new(("pattern-field", "spacing", pattern)), "0 mm");
    let spacing = harness.shows(ABOVE_ZERO_OR_REVERSE);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.click_lowest("Circular");
    harness.settle();
    harness.type_into_field(Id::new(("pattern-field", "angle", pattern)), "400 deg");
    let angle = harness.shows(TURN);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    assert!(spacing);
    assert!(angle);
    assert!(harness.shows(REVERSE_DIRECTION));
    assert!(!harness.shows("Reversed"));
    assert!(harness.shows(crate::pattern_panel::CIRCULAR_HINT));
    assert_eq!(
        crate::feature_fields::Rule::AboveZero.check(-1.0),
        Err(ABOVE_ZERO.to_owned())
    );
}

#[test]
fn a_missing_body_is_marked_and_a_refused_change_names_the_feature() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    harness.select([]);
    harness.click("Linear pattern");
    harness.settle();
    harness.hold_still();
    harness.perform(Action::Apply(Transaction::single(
        "Delete Extrude 1",
        Edit::RemoveFeature { id: plate },
    )));
    harness.settle();
    harness.hold_still();

    let refused = feature_fields::applied("Linear pattern 1", Err("It is in use".to_owned()));

    assert!(harness.shows(MISSING_BODY));
    assert!(!harness.shows("a missing body"));
    assert!(matches!(
        refused,
        Action::Inform(notice) if notice.text == "Linear pattern 1 was not changed: It is in use"
    ));
}
