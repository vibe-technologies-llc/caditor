use caditor_document::{
    BodyOperation, Edit, ExtrudeEnd, ExtrudeExtent, FeatureId, FeatureKind, HoleDepth,
    SolidFeature, Transaction,
};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::Sketch;

use super::{Harness, drag_value_arrow, extruded_plate, length_value, open_hole, rectangle};
use crate::{
    hole_tools,
    model::Action,
    move_manipulator::{Handle, Reach},
    value_gauges::Measured,
};

fn foot_of(harness: &Harness, measured: Measured) -> Point3 {
    harness
        .workspace
        .viewport
        .handle_foot(Handle::Value(measured))
        .unwrap_or_else(|| panic!("{measured:?} is shown"))
}

fn extrusion_up_to_next(harness: &mut Harness) -> FeatureId {
    let (plate, _) = extruded_plate(harness);
    let level = Plane::new(Point3::new(0.0, 0.0, 5.0), Vector3::Z).unwrap();
    let mut sketch = Sketch::new(level);
    rectangle(
        &mut sketch,
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 20.0),
    );
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let extrude = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    let FeatureKind::Solid(SolidFeature::Extrude(mut definition)) =
        harness.document().feature(extrude).unwrap().kind.clone()
    else {
        panic!("an extrusion is open");
    };
    definition.operation = BodyOperation::Remove(plate);
    definition.extent = ExtrudeExtent::OneSide {
        end: ExtrudeEnd::UpToNext { offset: None },
        reversed: true,
    };
    harness.perform(Action::Apply(Transaction::single(
        "Cut up to the next face",
        Edit::SetFeatureKind {
            id: extrude,
            kind: FeatureKind::Solid(SolidFeature::Extrude(definition)),
        },
    )));
    harness.settle();
    harness.workspace.viewport.advance(super::CAMERA_SETTLE);
    harness.frame();
    extrude
}

#[test]
fn an_extrusion_up_to_the_next_face_has_a_past_the_face_arrow_on_that_face() {
    let mut harness = Harness::new();
    let extrude = extrusion_up_to_next(&mut harness);
    let foot = foot_of(&harness, Measured::EndOffset(Reach::Only));

    assert!(
        (foot - Point3::new(15.0, 15.0, 0.0)).length() < 1e-9,
        "{foot:?}"
    );

    let step = drag_value_arrow(&mut harness, Measured::EndOffset(Reach::Only), 3.0);
    let FeatureKind::Solid(SolidFeature::Extrude(definition)) =
        &harness.document().feature(extrude).unwrap().kind
    else {
        panic!("the extrusion stays an extrusion");
    };
    let ExtrudeExtent::OneSide {
        end: ExtrudeEnd::UpToNext { offset },
        ..
    } = &definition.extent
    else {
        panic!("the extrusion stays up to the next face");
    };
    let found = length_value(&harness, offset.as_deref().expect("the drag set an offset"));

    assert_eq!(harness.model.undo_label(), Some("Edit Extrude 2"));
    assert!((found - 3.0).abs() <= step, "{found} with steps of {step}");
}

#[test]
fn a_hole_up_to_the_next_face_has_a_past_the_face_arrow_on_that_face() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Hole");
    harness.settle();
    harness.select([]);
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    let mut definition = open_hole(&harness, hole);
    definition.depth = HoleDepth::up_to_next();
    let change = hole_tools::edit(harness.document(), hole, definition).unwrap();
    harness.perform(Action::Apply(change));
    harness.settle();
    harness.workspace.viewport.advance(super::CAMERA_SETTLE);
    harness.frame();
    let foot = foot_of(&harness, Measured::HoleEndOffset);

    assert!(
        (foot - Point3::new(20.0, 20.0, 0.0)).length() < 1e-9,
        "{foot:?}"
    );

    let step = drag_value_arrow(&mut harness, Measured::HoleEndOffset, -2.0);
    let offset = open_hole(&harness, hole).depth.offset().cloned().unwrap();

    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert!((length_value(&harness, &offset) + 2.0).abs() <= step);
}
