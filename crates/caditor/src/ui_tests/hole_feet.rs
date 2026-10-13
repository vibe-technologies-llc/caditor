use caditor_document::{
    FeatureId, Hole, HoleFit, HoleShape, HoleSizing, HoleStandard, HoleStep, HoleStyle, MetricSize,
    PlaneReference,
};
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::Solid;
use caditor_sketch::Sketch;
use egui::{Event, Key, Modifiers};

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

const CIRCLE_DIAMETER: f64 = 12.0;
const HEAD_SCALE: f64 = CIRCLE_DIAMETER / 6.0;
const TYPED_WIDE: f64 = 8.0;
const TYPED_DEEP: f64 = 2.0;

fn open_circled_hole(harness: &mut Harness, sizing: HoleSizing) -> FeatureId {
    extruded_plate(harness);
    let top = Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap();
    let mut sketch = Sketch::new(top);
    sketch.add_circle(Point2::new(20.0, 20.0), CIRCLE_DIAMETER / 2.0);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    reshape(harness, hole, |hole| hole.sizing = sizing);
    hole
}

fn at(radius: f64, level: f64, side: f64) -> Point3 {
    Point3::new(
        20.0 + radius * ROOT_HALF,
        20.0 + side * radius * ROOT_HALF,
        level,
    )
}

fn steps_of(harness: &Harness, hole: FeatureId) -> Vec<(f64, f64)> {
    let HoleStyle::Stepped(steps) = open_hole(harness, hole).style else {
        panic!("the hole stays stepped");
    };
    steps
        .iter()
        .map(|step| {
            (
                length_of(harness, &step.diameter),
                length_of(harness, &step.depth),
            )
        })
        .collect()
}

fn two_steps() -> HoleStyle {
    HoleStyle::Stepped(vec![
        HoleStep {
            diameter: millimetres(26.0),
            depth: millimetres(2.0),
        },
        HoleStep {
            diameter: millimetres(16.0),
            depth: millimetres(3.0),
        },
    ])
}

fn stepped(harness: &mut Harness) -> FeatureId {
    let hole = open_plain_hole(harness);
    reshape(harness, hole, |hole| hole.style = two_steps());
    hole
}

fn counterbored_typed(
    harness: &mut Harness,
    sizing: HoleSizing,
    (wide, deep): (f64, f64),
) -> FeatureId {
    let hole = open_circled_hole(harness, sizing);
    reshape(harness, hole, |hole| {
        hole.style = HoleStyle::Counterbore {
            diameter: millimetres(wide),
            depth: millimetres(deep),
        };
    });
    hole
}

#[test]
fn a_stepped_hole_shows_a_diameter_and_a_depth_arrow_on_each_step() {
    let mut harness = Harness::new();
    let hole = stepped(&mut harness);
    let result = drilled(&harness, hole);

    let first_mouth = foot_of(&harness, Measured::StepDiameter(0));
    let first_floor = foot_of(&harness, Measured::StepDepth(0));
    let second_mouth = foot_of(&harness, Measured::StepDiameter(1));
    let second_floor = foot_of(&harness, Measured::StepDepth(1));
    let bore = foot_of(&harness, Measured::HoleDiameter);

    assert!(
        (first_mouth - at(13.0, 10.0, 1.0)).length() < 1e-9,
        "{first_mouth:?}"
    );
    assert!(
        (first_floor - at(13.0, 8.0, -1.0)).length() < 1e-9,
        "{first_floor:?}"
    );
    assert!(
        (second_mouth - at(8.0, 8.0, 1.0)).length() < 1e-9,
        "{second_mouth:?}"
    );
    assert!(
        (second_floor - at(8.0, 5.0, -1.0)).length() < 1e-9,
        "{second_floor:?}"
    );
    assert!((bore - at(3.0, 5.0, 1.0)).length() < 1e-9, "{bore:?}");
    for foot in [first_mouth, first_floor, second_mouth, second_floor] {
        assert!(on_an_edge(result, foot), "{foot:?}");
    }
}

#[test]
fn dragging_a_step_s_arrows_changes_only_that_step_and_makes_the_size_custom() {
    let mut harness = Harness::new();
    let hole = open_plain_hole(&mut harness);
    let size = HoleStandard {
        size: MetricSize::M6,
        fit: HoleFit::Normal,
    };
    reshape(&mut harness, hole, |definition| {
        *definition = hole_tools::with_standard(definition, size, LengthUnit::Millimetre);
        definition.style = two_steps();
        definition.standard = Some(size);
    });
    let before = steps_of(&harness, hole);

    let step = drag_value_arrow(&mut harness, Measured::StepDepth(1), 1.0);
    let after = steps_of(&harness, hole);

    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert_eq!(after.first(), before.first());
    assert_eq!(after[1].0, before[1].0);
    assert!(
        (after[1].1 - before[1].1 - 1.0).abs() <= step,
        "{before:?} to {after:?}"
    );
    assert_eq!(open_hole(&harness, hole).standard, None);

    let step = drag_value_arrow(&mut harness, Measured::StepDiameter(1), -1.0);
    let narrower = steps_of(&harness, hole);

    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert!(
        (narrower[1].0 - before[1].0 + 2.0).abs() <= 2.0 * step,
        "{before:?} to {narrower:?}"
    );
    assert_eq!(narrower[1].1, after[1].1);
    assert_eq!(narrower[0], before[0]);

    harness.perform(Action::Undo);
    harness.settle();

    assert_eq!(steps_of(&harness, hole), after);
}

#[test]
fn dragging_the_first_step_s_arrows_leaves_the_second_alone() {
    let mut harness = Harness::new();
    let hole = stepped(&mut harness);
    let before = steps_of(&harness, hole);

    let step = drag_value_arrow(&mut harness, Measured::StepDepth(0), 1.0);
    let deeper = steps_of(&harness, hole);

    assert!(
        (deeper[0].1 - before[0].1 - 1.0).abs() <= step,
        "{before:?} to {deeper:?}"
    );
    assert_eq!(deeper[0].0, before[0].0);
    assert_eq!(deeper[1], before[1]);

    let step = drag_value_arrow(&mut harness, Measured::StepDiameter(0), -1.0);
    let narrower = steps_of(&harness, hole);

    assert!(
        (narrower[0].0 - before[0].0 + 2.0).abs() <= 2.0 * step,
        "{before:?} to {narrower:?}"
    );
    assert_eq!(narrower[0].1, deeper[0].1);
    assert_eq!(narrower[1], before[1]);
}

#[test]
fn a_step_drag_never_goes_below_one_step_like_the_typed_field() {
    let mut harness = Harness::new();
    let hole = stepped(&mut harness);

    let step = drag_value_arrow(&mut harness, Measured::StepDepth(0), -20.0);
    let shrunk = steps_of(&harness, hole);

    assert!(shrunk[0].1 >= step - 1e-9, "{shrunk:?}");
}

#[test]
fn a_counterbore_in_a_hole_sized_by_circles_has_arrows_that_follow_the_circle() {
    let mut harness = Harness::new();
    let hole = counterbored_typed(
        &mut harness,
        HoleSizing::CirclesAndHeads,
        (TYPED_WIDE, TYPED_DEEP),
    );
    let result = drilled(&harness, hole);

    let mouth = foot_of(&harness, Measured::CounterboreDiameter);
    let shoulder = foot_of(&harness, Measured::CounterboreDepth);

    let wide = TYPED_WIDE * HEAD_SCALE / 2.0;
    let deep = TYPED_DEEP * HEAD_SCALE;
    assert!((mouth - at(wide, 10.0, 1.0)).length() < 1e-9, "{mouth:?}");
    assert!(
        (shoulder - at(wide, 10.0 - deep, -1.0)).length() < 1e-9,
        "{shoulder:?}"
    );
    assert!(on_an_edge(result, mouth));
    assert!(on_an_edge(result, shoulder));

    let step = drag_value_arrow(&mut harness, Measured::CounterboreDepth, 1.0);
    let (_, deeper) = counterbore_of(&harness, hole);

    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert!(
        (deeper - TYPED_DEEP - 1.0 / HEAD_SCALE).abs() <= step / HEAD_SCALE,
        "{deeper}"
    );
    assert_eq!(
        open_hole(&harness, hole).sizing,
        HoleSizing::CirclesAndHeads
    );

    let step = drag_value_arrow(&mut harness, Measured::CounterboreDiameter, 1.0);
    let (wider, _) = counterbore_of(&harness, hole);

    assert!(
        (wider - TYPED_WIDE - 2.0 / HEAD_SCALE).abs() <= 2.0 * step / HEAD_SCALE,
        "{wider}"
    );
}

#[test]
fn a_counterbore_in_a_hole_sized_by_circles_alone_keeps_its_typed_size_on_screen() {
    let mut harness = Harness::new();
    let hole = counterbored_typed(&mut harness, HoleSizing::Circles, (16.0, 3.0));
    let result = drilled(&harness, hole);

    let mouth = foot_of(&harness, Measured::CounterboreDiameter);
    let shoulder = foot_of(&harness, Measured::CounterboreDepth);

    assert!((mouth - at(8.0, 10.0, 1.0)).length() < 1e-9, "{mouth:?}");
    assert!(
        (shoulder - at(8.0, 7.0, -1.0)).length() < 1e-9,
        "{shoulder:?}"
    );
    assert!(on_an_edge(result, mouth));
    assert!(on_an_edge(result, shoulder));
    assert!(
        harness
            .workspace
            .viewport
            .handle_foot(Handle::Value(Measured::HoleDiameter))
            .is_none()
    );

    let step = drag_value_arrow(&mut harness, Measured::CounterboreDepth, 1.0);
    let (_, deeper) = counterbore_of(&harness, hole);

    assert!((deeper - 4.0).abs() <= step, "{deeper}");
}

#[test]
fn steps_in_a_hole_sized_by_circles_scale_with_the_circle() {
    let mut harness = Harness::new();
    let hole = open_circled_hole(&mut harness, HoleSizing::CirclesAndHeads);
    reshape(&mut harness, hole, |hole| {
        hole.style = HoleStyle::Stepped(vec![HoleStep {
            diameter: millimetres(TYPED_WIDE),
            depth: millimetres(TYPED_DEEP),
        }]);
    });
    let result = drilled(&harness, hole);

    let mouth = foot_of(&harness, Measured::StepDiameter(0));
    let floor = foot_of(&harness, Measured::StepDepth(0));

    let wide = TYPED_WIDE * HEAD_SCALE / 2.0;
    let deep = TYPED_DEEP * HEAD_SCALE;
    assert!((mouth - at(wide, 10.0, 1.0)).length() < 1e-9, "{mouth:?}");
    assert!(
        (floor - at(wide, 10.0 - deep, -1.0)).length() < 1e-9,
        "{floor:?}"
    );
    assert!(on_an_edge(result, mouth));
    assert!(on_an_edge(result, floor));

    let step = drag_value_arrow(&mut harness, Measured::StepDepth(0), 1.0);
    let after = steps_of(&harness, hole);

    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert!(
        (after[0].1 - TYPED_DEEP - 1.0 / HEAD_SCALE).abs() <= step / HEAD_SCALE,
        "{after:?}"
    );

    let step = drag_value_arrow(&mut harness, Measured::StepDiameter(0), 1.0);
    let wider = steps_of(&harness, hole);

    assert!(
        (wider[0].0 - TYPED_WIDE - 2.0 / HEAD_SCALE).abs() <= 2.0 * step / HEAD_SCALE,
        "{wider:?}"
    );
}

#[test]
fn a_countersink_in_a_hole_sized_by_circles_has_arrows_at_the_scaled_mouth_and_cone_foot() {
    let mut harness = Harness::new();
    let hole = open_circled_hole(&mut harness, HoleSizing::CirclesAndHeads);
    reshape(&mut harness, hole, |hole| {
        hole.style = HoleStyle::Countersink {
            diameter: millimetres(TYPED_WIDE + 1.0),
            angle: Expression::measure(90.0, Unit::Degree),
        };
    });
    let result = drilled(&harness, hole);

    let mouth = foot_of(&harness, Measured::CountersinkDiameter);
    let flare = foot_of(&harness, Measured::CountersinkAngle);

    let wide = (TYPED_WIDE + 1.0) * HEAD_SCALE / 2.0;
    let narrow = CIRCLE_DIAMETER / 2.0;
    assert!((mouth - at(wide, 10.0, 1.0)).length() < 1e-9, "{mouth:?}");
    assert!(
        (flare - at(narrow, 10.0 - (wide - narrow), -1.0)).length() < 1e-9,
        "{flare:?}"
    );
    assert!(on_an_edge(result, mouth));
    assert!(on_an_edge(result, flare));

    let step = drag_value_arrow(&mut harness, Measured::CountersinkDiameter, 1.0);
    let HoleStyle::Countersink { diameter, .. } = open_hole(&harness, hole).style else {
        panic!("the hole stays countersunk");
    };
    let typed = length_of(&harness, &diameter);

    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert!(
        (typed - TYPED_WIDE - 1.0 - 2.0 / HEAD_SCALE).abs() <= 2.0 * step / HEAD_SCALE,
        "{typed}"
    );

    drag_value_arrow(&mut harness, Measured::CountersinkAngle, 1.0);
    let HoleStyle::Countersink { angle, .. } = open_hole(&harness, hole).style else {
        panic!("the hole stays countersunk");
    };

    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert!(
        angle_of(&harness, &angle) < 90.0,
        "{}",
        angle_of(&harness, &angle)
    );
}

#[test]
fn a_slot_in_a_hole_sized_by_circles_is_as_wide_as_the_circle() {
    let mut harness = Harness::new();
    let hole = open_circled_hole(&mut harness, HoleSizing::Circles);
    reshape(&mut harness, hole, |hole| {
        hole.shape = HoleShape::Slot {
            length: millimetres(20.0),
            angle: Expression::measure(0.0, Unit::Degree),
        };
    });
    let result = drilled(&harness, hole);

    let length = foot_of(&harness, Measured::SlotLength);
    let turn = foot_of(&harness, Measured::SlotAngle);

    assert!(
        (length - Point3::new(36.0, 20.0, 10.0)).length() < 1e-9,
        "{length:?}"
    );
    assert!(
        (turn - Point3::new(4.0, 20.0, 10.0)).length() < 1e-9,
        "{turn:?}"
    );
    assert!(on_an_edge(result, length));
    assert!(on_an_edge(result, turn));

    let step = drag_value_arrow(&mut harness, Measured::SlotLength, -2.0);
    let HoleShape::Slot { length, .. } = open_hole(&harness, hole).shape else {
        panic!("the hole stays a slot");
    };

    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert!(
        (length_of(&harness, &length) - 16.0).abs() <= 2.0 * step,
        "{}",
        length_of(&harness, &length)
    );

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
fn a_tapped_hole_sized_by_circles_has_its_thread_depth_arrow_on_the_circle_s_wall() {
    let mut harness = Harness::new();
    harness.perform(Action::ShowThreadsAsCosmetic(true));
    let hole = open_circled_hole(&mut harness, HoleSizing::Circles);
    let standard = HoleStandard {
        size: MetricSize::M6,
        fit: HoleFit::Tapped,
    };
    reshape(&mut harness, hole, |definition| {
        *definition = hole_tools::with_standard(definition, standard, LengthUnit::Millimetre);
        definition.sizing = HoleSizing::Circles;
        definition.thread.depth = Some(millimetres(4.0));
    });

    let foot = foot_of(&harness, Measured::ThreadDepth);

    assert!(
        (foot - at(CIRCLE_DIAMETER / 2.0, 6.0, -1.0)).length() < 1e-9,
        "{foot:?}"
    );

    let step = drag_value_arrow(&mut harness, Measured::ThreadDepth, 2.0);
    let depth = open_hole(&harness, hole).thread.depth.unwrap();

    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert!((length_of(&harness, &depth) - 6.0).abs() <= step);
    assert_eq!(open_hole(&harness, hole).standard, Some(standard));
}

fn hover_value_arrow(harness: &mut Harness, measured: Measured) {
    harness.workspace.viewport.advance(super::CAMERA_SETTLE);
    harness.frame();
    let at = harness
        .workspace
        .viewport
        .handle_position(Handle::Value(measured), 0.0)
        .expect("the value arrow is shown");
    harness.events.push(Event::PointerMoved(at));
    harness.frame();
    harness.frame();
}

#[test]
fn a_value_typed_over_a_step_arrow_sets_that_step_and_is_refused_where_its_field_would_be() {
    let mut harness = Harness::new();
    let hole = stepped(&mut harness);
    let before = steps_of(&harness, hole);

    hover_value_arrow(&mut harness, Measured::StepDiameter(1));
    harness.type_text("12");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();
    let typed = steps_of(&harness, hole);

    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert_eq!(typed, vec![before[0], (12.0, before[1].1)]);

    hover_value_arrow(&mut harness, Measured::StepDepth(0));
    harness.type_text("0");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    assert!(harness.shows(crate::feature_fields::ABOVE_ZERO));
    assert_eq!(steps_of(&harness, hole), typed);
}
