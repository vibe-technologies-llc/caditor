use caditor_document::{
    Datum, Edit, ExtrudeExtent, FeatureId, MatePair, PlaneReference, RevolveExtent, SolidFeature,
    SolidStart, Transaction,
};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::Sketch;
use egui::{Key, Modifiers};

use super::{
    Harness, add_peg, choose, datum_plane_of, extruded_plate, extrusion_above_plate, open_combo,
    open_hole, pickable_described, primitive_of, rectangle, removed_about, run_from_palette,
    start_of_solid, top_edge_along_x,
};
use crate::{
    model::Action,
    plane_selector,
    selection::{Pickable, PrincipalPlane},
};

fn end_target(harness: &Harness, feature: FeatureId) -> Option<PlaneReference> {
    let SolidFeature::Extrude(extrude) = harness.solid(feature) else {
        panic!("expected an extrusion");
    };
    let ExtrudeExtent::OneSide { end, .. } = &extrude.extent else {
        return None;
    };
    end.target().cloned()
}

fn datum_plane_made(harness: &mut Harness) -> FeatureId {
    harness.select([]);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the new plane is open");
    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }
    harness.perform(Action::Apply(Transaction::single(
        "Rename Plane 1",
        Edit::RenameFeature {
            id: plane,
            name: "Datum A".to_owned(),
        },
    )));
    harness.settle();
    plane
}

#[test]
fn an_extrusion_runs_up_to_a_plane_chosen_from_a_list_instead_of_clicked() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let tower = extrusion_above_plate(&mut harness, 30.0);
    choose(&mut harness, "New body", "Add to body");
    harness.click_lowest(crate::feature_fields::REVERSE_DIRECTION);
    harness.settle();

    open_combo(&mut harness, "End");
    harness.click_lowest("Up to face");
    harness.settle();
    let asks_in_the_view = harness.workspace.editing.picking().is_some();
    let offers_a_list = harness.shows(plane_selector::UNCHOSEN);

    open_combo(&mut harness, "Up to");
    harness.click_lowest("The XY plane");
    harness.settle();

    assert!(asks_in_the_view);
    assert!(offers_a_list);
    assert!(harness.workspace.editing.picking().is_none());
    assert_eq!(
        end_target(&harness, tower),
        Some(PlaneReference::Principal(PrincipalPlane::Xy))
    );
    assert!((harness.body_volume(plate) - 18_000.0).abs() < 1.0);
    assert_eq!(harness.model.undo_label(), Some("Edit Extrude 2"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);

    open_combo(&mut harness, "Up to");
    harness.click_lowest("The XZ plane");
    harness.settle();
    assert_eq!(
        end_target(&harness, tower),
        Some(PlaneReference::Principal(PrincipalPlane::Xz))
    );
}

#[test]
fn an_extrusion_starts_at_a_plane_chosen_from_a_list_and_goes_back_to_its_sketch_plane() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let tower = extrusion_above_plate(&mut harness, 30.0);
    choose(&mut harness, "New body", "Add to body");

    open_combo(&mut harness, "Start");
    harness.click_lowest("Face or plane");
    harness.settle();
    let asks_in_the_view = harness.workspace.editing.picking().is_some();

    open_combo(&mut harness, "Starts at");
    harness.click_lowest("The XY plane");
    harness.settle();
    let started = start_of_solid(&harness, tower);

    harness.click_button("Start at the sketch plane again");
    harness.settle();
    let cleared = start_of_solid(&harness, tower);

    assert!(asks_in_the_view);
    assert!(harness.workspace.editing.picking().is_none());
    assert_eq!(
        started,
        Some(SolidStart::Plane(PlaneReference::Principal(
            PrincipalPlane::Xy
        )))
    );
    assert_eq!(cleared, None);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn a_datum_plane_starts_from_a_plane_chosen_from_a_list() {
    let mut harness = Harness::new();
    harness.select([]);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the new plane is open");
    assert_eq!(
        datum_plane_of(&harness, plane).base,
        PlaneReference::Principal(PrincipalPlane::Xy)
    );

    open_combo(&mut harness, "Starts from");
    harness.click_lowest("The XZ plane");
    harness.settle();

    assert_eq!(
        datum_plane_of(&harness, plane).base,
        PlaneReference::Principal(PrincipalPlane::Xz)
    );
    assert_eq!(harness.model.undo_label(), Some("Edit Plane 1"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn a_new_primitive_is_placed_on_a_plane_chosen_from_a_list() {
    let mut harness = Harness::new();
    harness.select([]);
    run_from_palette(&mut harness, "Box");
    harness.settle();
    let made = harness.workspace.editing.solid().expect("the box is open");
    assert_eq!(
        primitive_of(&harness, made).plane,
        PlaneReference::Principal(PrincipalPlane::Xy)
    );

    open_combo(&mut harness, "Placed on");
    harness.click_lowest("The YZ plane");
    harness.settle();

    assert_eq!(
        primitive_of(&harness, made).plane,
        PlaneReference::Principal(PrincipalPlane::Yz)
    );
    assert!(harness.workspace.editing.picking().is_none());
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn a_mirror_and_a_new_sketch_list_the_datum_planes_made_before_them() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let datum = datum_plane_made(&mut harness);

    harness.select([Pickable::Edge {
        body: plate,
        edge: top_edge_along_x(&harness, plate, 0.0),
    }]);
    harness.click("Mirror body");
    harness.settle();
    let mirror = harness
        .workspace
        .editing
        .solid()
        .expect("the mirror is open");

    open_combo(&mut harness, "Mirror across");
    harness.click_lowest("Datum A");
    harness.settle();

    let mirrored = harness
        .document()
        .feature(mirror)
        .and_then(|feature| feature.kind.mirror())
        .map(|mirror| mirror.plane.clone());
    assert_eq!(mirrored, Some(PlaneReference::Datum(datum)));
    assert_eq!(harness.model.evaluation().failed_count(), 0);

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.select([]);
    harness.click("New sketch");
    harness.frame();
    open_combo(&mut harness, crate::new_sketch_panel::PLANE);
    let listed = harness
        .texts
        .iter()
        .filter(|(shown, _)| shown == "Datum A")
        .map(|(_, rect)| rect.center())
        .min_by(|a, b| a.y.total_cmp(&b.y))
        .expect("the list names the datum plane");
    harness.click_screen(listed);
    harness.show_new_windows();
    harness.settle();

    let sketch = harness
        .editing()
        .expect("the sketch on the datum plane is edited");
    assert!(harness.shows("Lies on Datum A"));
    assert_ne!(sketch, datum);
}

#[test]
fn a_section_plane_goes_through_a_plane_chosen_from_a_list() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    run_from_palette(&mut harness, "show or hide the section view");
    harness.frame();
    harness.frame();
    assert_eq!(
        harness.workspace.section.cuts[0].base,
        Pickable::Plane(PrincipalPlane::Xz)
    );

    open_combo(&mut harness, crate::section_panel::PLANE);
    harness.click_lowest("The YZ plane");
    harness.frame();
    harness.frame();

    assert_eq!(
        harness.workspace.section.cuts[0].base,
        Pickable::Plane(PrincipalPlane::Yz)
    );
}

#[test]
fn a_coordinate_system_lays_its_xy_plane_along_a_plane_chosen_from_a_list() {
    let mut harness = Harness::new();
    harness.select([]);
    run_from_palette(&mut harness, "Coordinate system");
    harness.settle();
    let frame = harness
        .workspace
        .editing
        .solid()
        .expect("the coordinate system is open");

    open_combo(&mut harness, "XY plane");
    harness.click_lowest("The XZ plane");
    harness.settle();

    let Some(Datum::Frame(held)) = harness
        .document()
        .feature(frame)
        .and_then(|feature| feature.kind.datum())
    else {
        panic!("a coordinate system was expected");
    };
    assert_eq!(held.plane, PlaneReference::Principal(PrincipalPlane::Xz));
    assert_eq!(harness.model.undo_label(), Some("Edit Coordinate system 1"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn a_hole_is_drilled_up_to_a_plane_chosen_from_a_list() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let top = Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap();
    let mut sketch = Sketch::new(top);
    sketch.add_point(Point2::new(20.0, 20.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    let bore = std::f64::consts::PI * 3.0 * 3.0;

    open_combo(&mut harness, "Depth");
    harness.click_lowest(crate::hole_panel::UP_TO_FACE);
    harness.settle();
    let asks_in_the_view = harness.workspace.editing.picking().is_some();

    open_combo(&mut harness, "Up to");
    harness.click_lowest("The XY plane");
    harness.settle();

    assert!(asks_in_the_view);
    assert!(harness.workspace.editing.picking().is_none());
    assert_eq!(
        open_hole(&harness, hole).depth.target(),
        Some(&PlaneReference::Principal(PrincipalPlane::Xy))
    );
    assert!(removed_about(&harness, plate, bore * 10.0));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn a_revolve_turns_up_to_a_plane_chosen_from_a_list() {
    let mut harness = Harness::new();
    let mut section = Sketch::new(Plane::XZ);
    rectangle(&mut section, Point2::new(2.0, 0.0), Point2::new(4.0, 3.0));
    harness.add_sketch(section);
    harness.select([]);
    harness.click("Revolve");
    harness.settle();
    let revolve = harness
        .workspace
        .editing
        .solid()
        .expect("the revolve is open");

    choose(&mut harness, "Full turn", crate::solid_panel::TURN_UP_TO);
    let asks_in_the_view = harness.workspace.editing.picking().is_some();
    open_combo(&mut harness, "Up to");
    harness.click_lowest("The YZ plane");
    harness.settle();

    let SolidFeature::Revolve(turned) = harness.solid(revolve) else {
        panic!("expected a revolve");
    };
    let quarter = std::f64::consts::PI * (16.0 - 4.0) * 3.0 / 4.0;
    let volume = harness.body_volume(revolve);
    assert!(asks_in_the_view);
    assert_eq!(
        turned.extent,
        RevolveExtent::up_to(PlaneReference::Principal(PrincipalPlane::Yz), false)
    );
    assert!((volume - quarter).abs() < 0.01 * quarter, "{volume}");
    assert!(harness.workspace.editing.picking().is_none());
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn a_mate_rests_on_a_plane_chosen_from_a_list() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    add_peg(&mut harness);
    let bottom = pickable_described(&mut harness, "Peg › Peg start face");

    harness.select([bottom, Pickable::Plane(PrincipalPlane::Xy)]);
    run_from_palette(&mut harness, "mate body");
    harness.settle();
    let mate = harness.workspace.editing.solid().expect("the mate is open");
    let target = |harness: &Harness| match harness
        .document()
        .feature(mate)
        .and_then(|feature| feature.kind.mate())
        .map(|mate| mate.pair.clone())
    {
        Some(MatePair::Faces(faces)) => faces.target,
        other => panic!("a faces mate was expected, found {other:?}"),
    };
    assert_eq!(
        target(&harness),
        PlaneReference::Principal(PrincipalPlane::Xy)
    );

    open_combo(&mut harness, "Onto");
    harness.click_lowest("The XZ plane");
    harness.settle();

    assert_eq!(
        target(&harness),
        PlaneReference::Principal(PrincipalPlane::Xz)
    );
    assert_eq!(harness.model.undo_label(), Some("Edit Mate 1"));
}
