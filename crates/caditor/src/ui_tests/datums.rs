use caditor_document::{Edit, FeatureId, FeatureKind, Transaction};
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{
    Harness, datum_of, datum_plane_of, extruded_plate, offer, open_solid, plane_height, rectangle,
    run_from_palette, top_edge_along_x,
};
use crate::{
    commands::Command,
    model::Action,
    selection::{Pickable, PrincipalPlane},
};

fn datum_plane(harness: &Harness, feature: FeatureId) -> Plane {
    crate::datum_tools::result(harness.model.evaluation(), feature)
        .and_then(|result| result.plane())
        .expect("the datum plane has a position")
}

#[test]
fn a_datum_plane_carries_a_sketch_that_follows_its_offset() {
    let mut harness = Harness::new();
    harness.select([]);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the new plane is open");
    assert_eq!(harness.model.undo_label(), Some("Create Plane 1"));
    assert!(datum_of(&harness, plane).is_plane());
    assert_eq!(datum_plane(&harness, plane).origin().z, 10.0);
    assert!(harness.shows("Starts from"));
    assert!(harness.shows("The XY plane"));
    assert!(harness.shows("Choose in the panel the plane or flat face the plane starts from"));

    harness.type_into_field(Id::unique(("datum-field", "offset", plane)), "25 mm");
    harness.settle();
    assert_eq!(datum_plane(&harness, plane).origin().z, 25.0);
    assert!(
        harness
            .built()
            .picks
            .pickables()
            .any(|pickable| pickable == Pickable::Datum(plane))
    );

    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }
    assert_eq!(harness.workspace.editing.solid(), None);
    harness.select([Pickable::Datum(plane)]);
    harness.click("New sketch");
    harness.settle();
    let sketch = harness.editing().expect("the new sketch is edited");
    assert_eq!(plane_height(&harness, sketch), 25.0);
    let attachment = harness
        .document()
        .feature(sketch)
        .and_then(|feature| feature.kind.attachment())
        .cloned();
    assert_eq!(
        attachment,
        Some(caditor_document::SketchAttachment::Datum(plane))
    );

    harness.use_tool(Key::P);
    harness.click_at(Point2::new(5.0, 5.0));
    harness.settle();
    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
    }
    assert_eq!(harness.editing(), None);
    assert_eq!(harness.sketch(sketch).entities().count(), 1);
    let Some(caditor_document::Datum::Plane(mut changed)) = harness
        .document()
        .feature(plane)
        .and_then(|feature| feature.kind.datum())
        .cloned()
    else {
        panic!("the plane is a datum plane");
    };
    changed.offset = Expression::Measure(-5.0, Unit::Millimetre);
    harness.perform(Action::Apply(Transaction::single(
        "Move the plane",
        Edit::SetFeatureKind {
            id: plane,
            kind: FeatureKind::Datum(caditor_document::Datum::Plane(changed)),
        },
    )));
    harness.settle();
    assert_eq!(plane_height(&harness, sketch), -5.0);
}

#[test]
fn an_axis_from_a_selected_edge_turns_a_plane_and_a_revolve() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let front = top_edge_along_x(&harness, plate, 0.0);
    let edge = Pickable::Edge {
        body: plate,
        edge: front,
    };

    harness.select([top, edge]);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the plane is open");
    let tilted = datum_plane(&harness, plane);
    let half = std::f64::consts::FRAC_1_SQRT_2;
    assert!((tilted.normal().z.abs() - half).abs() < 1e-9, "{tilted:?}");
    assert!(
        tilted
            .signed_distance(caditor_geometry::Point3::new(7.0, 0.0, 10.0))
            .abs()
            < 1e-9
    );
    assert!(harness.shows("Turned about"));
    assert!(harness.shows("Angle"));

    harness.select([edge]);
    harness.click("Axis");
    harness.settle();
    let axis = harness.workspace.editing.solid().expect("the axis is open");
    assert_eq!(harness.model.undo_label(), Some("Create Axis 1"));
    let line = crate::datum_tools::result(harness.model.evaluation(), axis)
        .and_then(|result| result.axis())
        .expect("the axis has a position");
    assert!((line.direction().x.abs() - 1.0).abs() < 1e-9);

    let mut section = Sketch::new(Plane::XZ);
    rectangle(
        &mut section,
        Point2::new(0.0, 20.0),
        Point2::new(10.0, 30.0),
    );
    let section = harness.add_sketch(section);
    harness.select([Pickable::Datum(axis)]);
    harness.click("Revolve");
    harness.settle();
    let revolve = harness
        .workspace
        .editing
        .solid()
        .expect("the revolution is open");
    let definition = harness.solid(revolve);
    assert_eq!(definition.sketch(), section);
    assert_eq!(
        definition
            .axis()
            .and_then(caditor_document::RevolveAxis::model),
        Some(&caditor_document::AxisReference::Datum(axis))
    );
    assert_eq!(
        harness
            .model
            .evaluation()
            .feature(revolve)
            .map(|status| &status.state),
        Some(&caditor_document::FeatureState::UpToDate)
    );
}

#[test]
fn a_revolve_and_a_datum_plane_take_the_selection_from_the_palette() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let edge = Pickable::Edge {
        body: plate,
        edge: top_edge_along_x(&harness, plate, 0.0),
    };

    let mut section = Sketch::new(Plane::XZ);
    rectangle(
        &mut section,
        Point2::new(0.0, 20.0),
        Point2::new(10.0, 30.0),
    );
    harness.add_sketch(section);
    harness.select([]);
    harness.click("Revolve");
    harness.settle();
    let revolve = harness
        .workspace
        .editing
        .solid()
        .expect("the revolve is open");
    assert_eq!(
        harness
            .solid(revolve)
            .axis()
            .and_then(caditor_document::RevolveAxis::model),
        None
    );
    harness.select([edge]);
    run_from_palette(&mut harness, "revolve about selected axis");
    harness.settle();
    assert!(
        harness
            .solid(revolve)
            .axis()
            .and_then(caditor_document::RevolveAxis::model)
            .is_some()
    );

    harness.select([]);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the plane is open");
    assert!(datum_plane(&harness, plane).normal().z.abs() > 0.999);
    assert_eq!(
        offer(&harness, Command::DatumUseSelected).availability,
        Err("Select a plane or flat face made before this plane".into())
    );
    harness.select([Pickable::Plane(PrincipalPlane::Xz)]);
    run_from_palette(&mut harness, "base datum on selection");
    harness.settle();
    assert!(datum_plane(&harness, plane).normal().y.abs() > 0.999);

    harness.select([edge]);
    run_from_palette(&mut harness, "turn datum plane about selected axis");
    harness.settle();
    let Some(caditor_document::Datum::Plane(turned)) = harness
        .document()
        .feature(plane)
        .and_then(|feature| feature.kind.datum())
    else {
        panic!("the plane is a datum plane");
    };
    assert!(turned.rotation.is_some());
}

#[test]
fn a_datum_takes_its_base_and_turn_from_the_selection_in_its_panel() {
    let mut harness = Harness::new();
    harness.select([]);
    harness.click("Plane");
    harness.settle();
    let plane = open_solid(&harness);
    harness.hold_still();

    harness.select([Pickable::Plane(PrincipalPlane::Xz)]);
    harness.click_beside("Use selected", "Starts from");
    harness.settle();
    let based = datum_plane(&harness, plane).normal();
    harness.select([Pickable::Axis(crate::selection::Axis::Z)]);
    harness.click_beside("Use selected", "Turned about");
    harness.settle();
    let turned = datum_plane_of(&harness, plane).rotation.is_some();
    harness.hold_still();
    harness.type_into_field(Id::unique(("datum-field", "angle", plane)), "90 deg");
    harness.settle();
    let quarter = datum_plane(&harness, plane).normal();
    harness.click_button("Stop turning the plane");
    harness.settle();
    let unturned = datum_plane_of(&harness, plane).rotation.is_none();

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.select([
        Pickable::Plane(PrincipalPlane::Xy),
        Pickable::Plane(PrincipalPlane::Xz),
    ]);
    harness.click("Axis");
    harness.settle();
    let axis = open_solid(&harness);
    harness.hold_still();
    let meeting = matches!(
        datum_of(&harness, axis),
        caditor_document::Datum::Axis(caditor_document::DatumAxis::Intersection(..))
    );
    harness.select([Pickable::Axis(crate::selection::Axis::Z)]);
    harness.click_beside("Use selected", "Defined by");
    harness.settle();

    assert!(based.y.abs() > 0.999, "{based}");
    assert!(turned);
    assert!(quarter.x.abs() > 0.999, "{quarter}");
    assert!(unturned);
    assert!(datum_plane(&harness, plane).normal().y.abs() > 0.999);
    assert!(meeting);
    assert!(matches!(
        datum_of(&harness, axis),
        caditor_document::Datum::Axis(caditor_document::DatumAxis::Along(_))
    ));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}
