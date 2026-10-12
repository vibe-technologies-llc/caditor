use caditor_document::{
    FeatureId, Hole, HoleFit, HoleShape, HoleStandard, HoleStyle, MetricSize, PlaneReference,
};
use caditor_expression::{Expression, Unit};
use caditor_geometry::Point3;
use caditor_kernel::Solid;

use super::{Harness, drag_value_arrow, extruded_plate, length_value, open_hole};
use crate::{
    hole_tools::{self, millimetres},
    model::Action,
    move_manipulator::Handle,
    selection::PrincipalPlane,
    units::LengthUnit,
    value_gauges::Measured,
};

fn open_plain_hole(harness: &mut Harness) -> FeatureId {
    let (_, top) = extruded_plate(harness);
    harness.select([top]);
    harness.click("Hole");
    harness.settle();
    harness.select([]);
    harness.workspace.editing.solid().expect("the hole is open")
}

fn reshape(harness: &mut Harness, hole: FeatureId, change: impl FnOnce(&mut Hole)) {
    let mut definition = open_hole(harness, hole);
    change(&mut definition);
    let transaction = hole_tools::edit(harness.document(), hole, definition).unwrap();
    harness.perform(Action::Apply(transaction));
    harness.settle();
    harness.workspace.viewport.advance(super::CAMERA_SETTLE);
    harness.frame();
}

fn foot_of(harness: &Harness, measured: Measured) -> Point3 {
    harness
        .workspace
        .viewport
        .handle_foot(Handle::Value(measured))
        .unwrap_or_else(|| panic!("{measured:?} is shown"))
}

fn on_an_edge(solid: &Solid, point: Point3) -> bool {
    solid.edges().any(|(_, edge)| {
        let parameter = edge.curve().closest_parameter(point, edge.interval());
        edge.curve().point(parameter).distance(point) < 1e-6
    })
}

fn drilled(harness: &Harness, hole: FeatureId) -> &Solid {
    harness
        .model
        .evaluation()
        .feature(hole)
        .and_then(|status| status.result.as_deref())
        .and_then(|result| result.solid())
        .map(|solid| &solid.solid)
        .expect("the hole has a body")
}

fn length_of(harness: &Harness, expression: &Expression) -> f64 {
    length_value(harness, expression)
}

fn angle_of(harness: &Harness, expression: &Expression) -> f64 {
    harness
        .model
        .parameters()
        .evaluate_expression(expression)
        .unwrap()
        .value
}

const ROOT_HALF: f64 = std::f64::consts::FRAC_1_SQRT_2;

fn counterbored(harness: &mut Harness) -> FeatureId {
    let hole = open_plain_hole(harness);
    reshape(harness, hole, |hole| {
        hole.style = HoleStyle::Counterbore {
            diameter: millimetres(12.0),
            depth: millimetres(4.0),
        };
    });
    hole
}

fn counterbore_of(harness: &Harness, hole: FeatureId) -> (f64, f64) {
    let HoleStyle::Counterbore { diameter, depth } = open_hole(harness, hole).style else {
        panic!("the hole stays counterbored");
    };
    (length_of(harness, &diameter), length_of(harness, &depth))
}

#[test]
fn a_counterbored_hole_shows_arrows_on_its_shoulder_and_mouth() {
    let mut harness = Harness::new();
    let hole = counterbored(&mut harness);
    let result = drilled(&harness, hole);
    let at = |radius: f64, level: f64, side: f64| {
        Point3::new(
            20.0 + radius * ROOT_HALF,
            20.0 + side * radius * ROOT_HALF,
            level,
        )
    };

    let mouth = foot_of(&harness, Measured::CounterboreDiameter);
    let shoulder = foot_of(&harness, Measured::CounterboreDepth);
    let bore = foot_of(&harness, Measured::HoleDiameter);

    assert!((mouth - at(6.0, 10.0, 1.0)).length() < 1e-9, "{mouth:?}");
    assert!(
        (shoulder - at(6.0, 6.0, -1.0)).length() < 1e-9,
        "{shoulder:?}"
    );
    assert!((bore - at(3.0, 6.0, 1.0)).length() < 1e-9, "{bore:?}");
    assert!(on_an_edge(result, mouth));
    assert!(on_an_edge(result, shoulder));
    assert!(on_an_edge(result, bore));
}

#[test]
fn dragging_a_counterbore_s_arrows_changes_its_diameter_and_depth_and_makes_the_size_custom() {
    let mut harness = Harness::new();
    let hole = open_plain_hole(&mut harness);
    let size = HoleStandard {
        size: MetricSize::M6,
        fit: HoleFit::Normal,
    };
    reshape(&mut harness, hole, |definition| {
        *definition = hole_tools::with_standard(definition, size, LengthUnit::Millimetre);
        definition.style = HoleStyle::Counterbore {
            diameter: millimetres(12.0),
            depth: millimetres(4.0),
        };
        definition.standard = Some(size);
    });
    let before = counterbore_of(&harness, hole);

    let step = drag_value_arrow(&mut harness, Measured::CounterboreDepth, 1.0);
    let (_, deeper) = counterbore_of(&harness, hole);

    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert!(
        (deeper - before.1 - 1.0).abs() <= step,
        "{before:?} to {deeper}"
    );
    assert_eq!(open_hole(&harness, hole).standard, None);

    let step = drag_value_arrow(&mut harness, Measured::CounterboreDiameter, 2.0);
    let (wider, same) = counterbore_of(&harness, hole);

    assert!(
        (wider - before.0 - 4.0).abs() <= 2.0 * step,
        "{before:?} to {wider}"
    );
    assert_eq!(same, deeper);
}

#[test]
fn a_countersunk_hole_shows_arrows_at_its_mouth_and_where_the_cone_meets_the_bore() {
    let mut harness = Harness::new();
    let hole = open_plain_hole(&mut harness);
    reshape(&mut harness, hole, |hole| {
        hole.style = HoleStyle::Countersink {
            diameter: millimetres(12.0),
            angle: Expression::measure(90.0, Unit::Degree),
        };
    });
    let result = drilled(&harness, hole);
    let mouth = foot_of(&harness, Measured::CountersinkDiameter);
    let flare = foot_of(&harness, Measured::CountersinkAngle);
    let bore = foot_of(&harness, Measured::HoleDiameter);
    let at = |radius: f64, level: f64, side: f64| {
        Point3::new(
            20.0 + radius * ROOT_HALF,
            20.0 + side * radius * ROOT_HALF,
            level,
        )
    };

    assert!((mouth - at(6.0, 10.0, 1.0)).length() < 1e-9, "{mouth:?}");
    assert!((flare - at(3.0, 7.0, -1.0)).length() < 1e-9, "{flare:?}");
    assert!((bore - at(3.0, 7.0, 1.0)).length() < 1e-9, "{bore:?}");
    assert!(on_an_edge(result, mouth));
    assert!(on_an_edge(result, flare));
    assert!(on_an_edge(result, bore));

    drag_value_arrow(&mut harness, Measured::CountersinkAngle, 1.0);
    let HoleStyle::Countersink { angle, diameter } = open_hole(&harness, hole).style else {
        panic!("the hole stays countersunk");
    };

    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert!(
        (angle_of(&harness, &angle) - 2.0 * (3.0_f64 / 4.0).atan().to_degrees()).abs() <= 5.0,
        "{}",
        angle_of(&harness, &angle)
    );
    assert_eq!(length_of(&harness, &diameter), 12.0);
}

#[test]
fn a_slotted_hole_shows_arrows_at_the_tips_of_its_slot() {
    let mut harness = Harness::new();
    let hole = open_plain_hole(&mut harness);
    reshape(&mut harness, hole, |hole| {
        hole.shape = HoleShape::Slot {
            length: millimetres(20.0),
            angle: Expression::measure(0.0, Unit::Degree),
        };
    });
    let result = drilled(&harness, hole);
    let length = foot_of(&harness, Measured::SlotLength);
    let turn = foot_of(&harness, Measured::SlotAngle);
    let width = foot_of(&harness, Measured::HoleDiameter);

    assert!(
        (length - Point3::new(33.0, 20.0, 10.0)).length() < 1e-9,
        "{length:?}"
    );
    assert!(
        (turn - Point3::new(7.0, 20.0, 10.0)).length() < 1e-9,
        "{turn:?}"
    );
    assert!(
        (width - Point3::new(25.0, 23.0, 10.0)).length() < 1e-9,
        "{width:?}"
    );
    assert!(on_an_edge(result, length));
    assert!(on_an_edge(result, turn));
    assert!(on_an_edge(result, width));

    let step = drag_value_arrow(&mut harness, Measured::SlotLength, 3.0);
    let HoleShape::Slot { length, angle } = open_hole(&harness, hole).shape else {
        panic!("the hole stays a slot");
    };

    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert!(
        (length_of(&harness, &length) - 26.0).abs() <= 2.0 * step,
        "{}",
        length_of(&harness, &length)
    );
    assert_eq!(angle_of(&harness, &angle), 0.0);

    drag_value_arrow(&mut harness, Measured::SlotAngle, 20.0);
    let HoleShape::Slot { angle, .. } = open_hole(&harness, hole).shape else {
        panic!("the hole stays a slot");
    };

    assert!(
        (angle_of(&harness, &angle) - 20.0).abs() <= 5.0,
        "{}",
        angle_of(&harness, &angle)
    );
}

#[test]
fn a_tapped_hole_s_thread_depth_has_an_arrow_on_the_bore_and_keeps_the_size() {
    let mut harness = Harness::new();
    harness.perform(Action::ShowThreadsAsCosmetic(true));
    let hole = open_plain_hole(&mut harness);
    let standard = HoleStandard {
        size: MetricSize::M6,
        fit: HoleFit::Tapped,
    };
    reshape(&mut harness, hole, |definition| {
        *definition = hole_tools::with_standard(definition, standard, LengthUnit::Millimetre);
        definition.thread.depth = Some(millimetres(4.0));
    });
    let radius = length_of(&harness, &open_hole(&harness, hole).diameter) / 2.0;
    let foot = foot_of(&harness, Measured::ThreadDepth);

    assert!(
        (foot - Point3::new(20.0 + radius * ROOT_HALF, 20.0 - radius * ROOT_HALF, 6.0)).length()
            < 1e-9,
        "{foot:?}"
    );

    let step = drag_value_arrow(&mut harness, Measured::ThreadDepth, 2.0);
    let depth = open_hole(&harness, hole).thread.depth.unwrap();

    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert!((length_of(&harness, &depth) - 6.0).abs() <= step);
    assert_eq!(open_hole(&harness, hole).standard, Some(standard));
}

#[test]
fn a_hole_up_to_a_face_has_a_past_the_face_arrow_where_its_axis_meets_the_face() {
    let mut harness = Harness::new();
    let hole = open_plain_hole(&mut harness);
    reshape(&mut harness, hole, |hole| {
        hole.depth =
            caditor_document::HoleDepth::up_to_face(PlaneReference::Principal(PrincipalPlane::Xy));
    });
    let foot = foot_of(&harness, Measured::HoleEndOffset);

    assert!(
        (foot - Point3::new(20.0, 20.0, 0.0)).length() < 1e-9,
        "{foot:?}"
    );

    let step = drag_value_arrow(&mut harness, Measured::HoleEndOffset, -3.0);
    let offset = open_hole(&harness, hole).depth.offset().cloned().unwrap();

    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert!((length_of(&harness, &offset) + 3.0).abs() <= step);
}
