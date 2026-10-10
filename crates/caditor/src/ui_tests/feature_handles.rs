use caditor_document::{
    Datum, Edit, ExtrudeEnd, ExtrudeExtent, FeatureId, FeatureKind, ParameterOwner, PatternKind,
    PlaneReference, PrimitiveShape, SolidFeature, Transaction,
};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3};
use caditor_sketch::Sketch;
use egui::{Event, Id, Key, Modifiers, PointerButton, Pos2};

use super::{
    CAMERA_SETTLE, Harness, blend_of, drag_value_arrow, extruded_plate, length_value, primitive_of,
    rectangle, run_from_palette, top_edge_along_x,
};
use crate::{
    field,
    model::Action,
    move_manipulator::{Handle, Reach},
    selection::{Axis, Pickable, PrincipalPlane},
    value_gauges::Measured,
};

fn open_feature(harness: &Harness) -> FeatureId {
    harness
        .workspace
        .editing
        .solid()
        .expect("a feature is open")
}

fn kind_of(harness: &Harness, feature: FeatureId) -> &FeatureKind {
    &harness.document().feature(feature).unwrap().kind
}

fn value_of(harness: &Harness, expression: &Expression) -> f64 {
    length_value(harness, expression)
}

fn grip(harness: &mut Harness, handle: Handle, along: f64) -> Pos2 {
    harness
        .workspace
        .viewport
        .handle_position(handle, along)
        .unwrap_or_else(|| panic!("{handle:?} is shown"))
}

fn hold_at(harness: &mut Harness, from: Pos2, to: Pos2) {
    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    for step in 1..=4 {
        let position = from + (to - from) * (step as f32 / 4.0);
        harness.events.push(Event::PointerMoved(position));
        harness.frame();
    }
    harness.frame();
}

fn release_at(harness: &mut Harness, to: Pos2) {
    harness.events.push(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.settle();
}

fn filleted_plate(harness: &mut Harness) -> FeatureId {
    let (plate, _) = extruded_plate(harness);
    let front = top_edge_along_x(harness, plate, 0.0);
    harness.select([Pickable::Edge {
        body: plate,
        edge: front,
    }]);
    harness.click("Fillet");
    harness.settle();
    harness.select([]);
    open_feature(harness)
}

#[test]
fn dragging_a_fillet_s_arrow_changes_its_radius_and_its_field_follows_the_drag() {
    let mut harness = Harness::new();
    let fillet = filleted_plate(&mut harness);
    let before = value_of(&harness, &blend_of(&harness, fillet).size);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let handle = Handle::Value(Measured::BlendSize);
    let from = grip(&mut harness, handle, 0.0);
    let to = grip(&mut harness, handle, 2.0);
    let step = harness.workspace.viewport.manipulator_step().unwrap();
    harness.events.push(Event::PointerMoved(from));
    harness.frame();

    assert!(harness.shows("Drag to change the fillet's radius"));

    hold_at(&mut harness, from, to);
    let FeatureKind::Blend(drafted) = harness.model.draft_kind(fillet).unwrap().clone() else {
        panic!("the draft stays a fillet");
    };
    let owner = ParameterOwner::Feature {
        feature: fillet,
        value: "Radius".to_owned(),
    };
    let shown = field::value_text(harness.document(), &owner, &drafted.size);

    assert!(harness.shows(&shown), "the Radius field shows {shown}");
    assert!(harness.shows_containing("Radius "));

    release_at(&mut harness, to);
    let after = value_of(&harness, &blend_of(&harness, fillet).size);
    let per_radius = std::f64::consts::SQRT_2 - 1.0;

    assert_eq!(harness.model.undo_label(), Some("Edit Fillet 1"));
    assert!(
        (after - before - 2.0 / per_radius).abs() <= step,
        "{before} to {after} with steps of {step}"
    );
}

#[test]
fn dragging_a_shell_s_arrow_changes_its_thickness() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Shell");
    harness.settle();
    harness.select([]);
    let shell = open_feature(&harness);
    let thickness = |harness: &Harness| {
        let FeatureKind::Shell(shell) = kind_of(harness, shell) else {
            panic!("the shell stays a shell");
        };
        value_of(harness, &shell.thickness)
    };
    let before = thickness(&harness);

    let step = drag_value_arrow(&mut harness, Measured::ShellThickness, 3.0);
    let after = thickness(&harness);

    assert_eq!(harness.model.undo_label(), Some("Edit Shell 1"));
    assert!(
        (after - before - 3.0).abs() <= step,
        "{before} to {after} with steps of {step}"
    );
}

#[test]
fn dragging_a_thin_wall_s_arrow_changes_its_thickness() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let extrude = open_feature(&harness);
    harness.click("Thin wall");
    harness.settle();
    let thickness = |harness: &Harness| {
        let FeatureKind::Solid(solid) = kind_of(harness, extrude) else {
            panic!("the extrusion stays an extrusion");
        };
        value_of(harness, &solid.wall().unwrap().thickness)
    };
    let before = thickness(&harness);

    let step = drag_value_arrow(&mut harness, Measured::WallThickness, 1.0);
    let after = thickness(&harness);

    assert_eq!(harness.model.undo_label(), Some("Edit Extrude 1"));
    assert!(
        (after - before - 2.0).abs() <= 2.0 * step,
        "{before} to {after} with steps of {step}"
    );
}

#[test]
fn dragging_an_extrusion_s_taper_arrow_sets_a_taper_from_none() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let extrude = open_feature(&harness);
    let taper = |harness: &Harness| {
        let FeatureKind::Solid(SolidFeature::Extrude(extrude)) = kind_of(harness, extrude) else {
            panic!("the extrusion stays an extrusion");
        };
        extrude.taper.as_deref().map(|taper| {
            harness
                .model
                .parameters()
                .evaluate_expression(taper)
                .unwrap()
                .value
        })
    };

    assert_eq!(taper(&harness), None);

    drag_value_arrow(&mut harness, Measured::Taper, 2.0);
    let found = taper(&harness).expect("the drag set a taper");

    assert_eq!(harness.model.undo_label(), Some("Edit Extrude 1"));
    assert!(found > 0.0 && found < 45.0, "a taper of {found}");
}

#[test]
fn dragging_a_hole_s_diameter_arrow_makes_it_a_custom_size() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Hole");
    harness.settle();
    harness.select([]);
    let hole = open_feature(&harness);
    let diameter = |harness: &Harness| {
        let hole = kind_of(harness, hole).hole().unwrap();
        (value_of(harness, &hole.diameter), hole.standard.is_none())
    };
    let (before, _) = diameter(&harness);

    let step = drag_value_arrow(&mut harness, Measured::HoleDiameter, 2.0);
    let (after, custom) = diameter(&harness);

    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert!(custom);
    assert!(
        (after - before - 4.0).abs() <= step,
        "{before} to {after} with steps of {step}"
    );
}

#[test]
fn dragging_a_box_s_arrows_changes_its_length_and_height() {
    let mut harness = Harness::new();
    harness.select([]);
    harness.use_tool_with(Key::B, Modifiers::ALT);
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let feature = open_feature(&harness);
    let sizes = |harness: &Harness| -> Vec<f64> {
        primitive_of(harness, feature)
            .shape
            .sizes()
            .into_iter()
            .map(|(_, size)| value_of(harness, size))
            .collect()
    };
    let before = sizes(&harness);

    let step = drag_value_arrow(&mut harness, Measured::PrimitiveSize(2), 5.0);
    let taller = sizes(&harness);
    drag_value_arrow(&mut harness, Measured::PrimitiveSize(0), 3.0);
    let longer = sizes(&harness);

    assert_eq!(harness.model.undo_label(), Some("Edit Box 1"));
    assert!(
        (taller[2] - before[2] - 5.0).abs() <= step,
        "{before:?} to {taller:?} with steps of {step}"
    );
    assert_eq!(taller[0], before[0]);
    assert!(
        (longer[0] - before[0] - 6.0).abs() <= 2.0 * step,
        "{taller:?} to {longer:?} with steps of {step}"
    );
}

#[test]
fn dragging_a_linear_pattern_s_arrows_removes_a_copy_and_changes_its_spacing() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.select([]);
    harness.click("Linear pattern");
    harness.settle();
    let pattern = open_feature(&harness);
    run_from_palette(&mut harness, "Fit view");
    let first = |harness: &Harness| {
        let FeatureKind::Pattern(pattern) = kind_of(harness, pattern) else {
            panic!("the pattern stays a pattern");
        };
        let PatternKind::Linear { first, .. } = &pattern.kind else {
            panic!("the pattern stays linear");
        };
        (
            harness
                .model
                .parameters()
                .evaluate_expression(&first.count)
                .unwrap()
                .value,
            value_of(harness, &first.spacing),
        )
    };
    let (count, spacing) = first(&harness);

    drag_value_arrow(&mut harness, Measured::PatternCount(0), -0.6 * spacing);
    let (fewer, same) = first(&harness);

    assert_eq!(harness.model.undo_label(), Some("Edit Linear pattern 1"));
    assert_eq!(fewer, count - 1.0);
    assert_eq!(same, spacing);

    let step = drag_value_arrow(
        &mut harness,
        Measured::PatternSpacing(0),
        -4.0 * (fewer - 1.0),
    );
    let (_, narrower) = first(&harness);

    assert!(
        (spacing - narrower - 4.0).abs() <= step,
        "{spacing} to {narrower} with steps of {step}"
    );
}

#[test]
fn hovering_an_arrow_and_typing_a_value_sets_it_and_previews_it_first() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let extrude = open_feature(&harness);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let only = Handle::Reach(Reach::Only);
    let at = grip(&mut harness, only, 0.0);
    harness.events.push(Event::PointerMoved(at));
    harness.frame();
    harness.frame();

    harness.type_text("25");

    assert!(harness.shows("Distance"));
    let Some(FeatureKind::Solid(SolidFeature::Extrude(drafted))) =
        harness.model.draft_kind(extrude).cloned()
    else {
        panic!("typing previews the extrusion");
    };
    let ExtrudeExtent::OneSide {
        end: ExtrudeEnd::Distance(previewed),
        ..
    } = &drafted.extent
    else {
        panic!("the preview keeps one distance");
    };
    assert_eq!(value_of(&harness, previewed), 25.0);

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();
    let FeatureKind::Solid(SolidFeature::Extrude(entered)) = kind_of(&harness, extrude) else {
        panic!("the extrusion stays an extrusion");
    };
    let ExtrudeExtent::OneSide {
        end: ExtrudeEnd::Distance(distance),
        ..
    } = &entered.extent
    else {
        panic!("the extrusion keeps one distance");
    };

    assert_eq!(harness.model.undo_label(), Some("Edit Extrude 1"));
    assert_eq!(value_of(&harness, distance), 25.0);
}

#[test]
fn a_dragged_extrusion_keeps_its_last_preview_drawn_while_the_next_one_computes() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let extrude = open_feature(&harness);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let only = Handle::Reach(Reach::Only);
    let from = grip(&mut harness, only, 0.0);
    let pulled = grip(&mut harness, only, 5.0);
    let further = grip(&mut harness, only, 10.0);

    hold_at(&mut harness, from, pulled);
    harness.wait_until("the first preview is drawn", |harness| {
        harness.model.draft_body_result().is_some()
    });
    let first = harness.model.draft_kind(extrude).cloned();
    harness.events.push(Event::PointerMoved(further));
    harness.frame();

    assert_ne!(harness.model.draft_kind(extrude).cloned(), first);
    assert!(harness.model.draft_body_result().is_some());

    release_at(&mut harness, further);

    assert_eq!(harness.model.undo_label(), Some("Edit Extrude 1"));
}

#[test]
fn an_extrusion_arrow_dropped_on_a_corner_of_another_body_stops_level_with_it() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(60.0, 0.0), Point2::new(80.0, 20.0));
    let second = harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let extrude = open_feature(&harness);
    harness.type_into_field(Id::new(("solid-field", "distance", extrude)), "25");
    harness.settle();
    run_from_palette(&mut harness, "Fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let only = Handle::Reach(Reach::Only);
    let from = grip(&mut harness, only, 0.0);
    let corner = harness
        .workspace
        .viewport
        .screen_of(Point3::new(40.0, 0.0, 10.0))
        .expect("the plate's corner is in view");

    hold_at(&mut harness, from, corner);

    assert!(harness.shows_containing("to a corner"));

    release_at(&mut harness, corner);
    let FeatureKind::Solid(SolidFeature::Extrude(dragged)) = kind_of(&harness, extrude) else {
        panic!("the extrusion stays an extrusion");
    };
    let ExtrudeExtent::OneSide {
        end: ExtrudeEnd::Distance(distance),
        ..
    } = &dragged.extent
    else {
        panic!("the extrusion keeps one distance");
    };

    assert_eq!(dragged.sketch, second);
    assert!((value_of(&harness, distance) - 10.0).abs() < 1e-6);
}

#[test]
fn dragging_a_rotated_plane_s_turn_arrow_changes_its_angle() {
    let mut harness = Harness::new();
    harness.select([Pickable::Plane(PrincipalPlane::Xy), Pickable::Axis(Axis::X)]);
    harness.click("Plane");
    harness.settle();
    let plane = open_feature(&harness);
    let angle = |harness: &Harness| {
        let Some(Datum::Plane(definition)) = kind_of(harness, plane).datum() else {
            panic!("the plane stays a plane");
        };
        definition.rotation.as_ref().map(|rotation| {
            harness
                .model
                .parameters()
                .evaluate_expression(&rotation.angle)
                .unwrap()
                .value
        })
    };
    let before = angle(&harness).expect("the plane turns about X");

    drag_value_arrow(&mut harness, Measured::PlaneAngle, 20.0);
    let after = angle(&harness).unwrap();

    assert_eq!(harness.model.undo_label(), Some("Edit Plane 1"));
    assert!((after - before - 20.0).abs() <= 5.0, "{before} to {after}");
}

#[test]
fn a_cylinder_s_arrow_drags_its_diameter_and_keeps_its_height() {
    let mut harness = Harness::new();
    harness.select([]);
    harness.use_tool_with(Key::Y, Modifiers::ALT);
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let feature = open_feature(&harness);
    let sizes = |harness: &Harness| {
        let PrimitiveShape::Cylinder { diameter, height } = &primitive_of(harness, feature).shape
        else {
            panic!("the cylinder stays a cylinder");
        };
        (value_of(harness, diameter), value_of(harness, height))
    };
    let (diameter, height) = sizes(&harness);

    let step = drag_value_arrow(&mut harness, Measured::PrimitiveSize(0), 4.0);
    let (wider, same) = sizes(&harness);

    assert_eq!(same, height);
    assert!(
        (wider - diameter - 8.0).abs() <= 2.0 * step,
        "{diameter} to {wider} with steps of {step}"
    );
}

#[test]
fn dragging_a_circular_pattern_s_turn_arrows_adds_copies_and_changes_its_angle() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.select([]);
    harness.click("Circular pattern");
    harness.settle();
    let pattern = open_feature(&harness);
    let read = |harness: &Harness| {
        let FeatureKind::Pattern(pattern) = kind_of(harness, pattern) else {
            panic!("the pattern stays a pattern");
        };
        let PatternKind::Circular(circular) = &pattern.kind else {
            panic!("the pattern stays circular");
        };
        let parameters = harness.model.parameters();
        (
            parameters
                .evaluate_expression(&circular.count)
                .unwrap()
                .value,
            parameters
                .evaluate_expression(&circular.angle)
                .unwrap()
                .value,
        )
    };
    let (count, angle) = read(&harness);

    drag_value_arrow(&mut harness, Measured::CircularCount, 0.7 * angle / count);
    let (more, same) = read(&harness);

    assert_eq!(harness.model.undo_label(), Some("Edit Circular pattern 1"));
    assert_eq!(more, count + 1.0);
    assert_eq!(same, angle);

    drag_value_arrow(&mut harness, Measured::CircularAngle, -30.0);
    let (_, narrower) = read(&harness);

    assert!((narrower - 330.0).abs() <= 5.0, "{angle} to {narrower}");
}

#[test]
fn dragging_the_arrow_past_an_up_to_face_end_sets_its_offset() {
    let mut harness = Harness::new();
    harness.select([]);
    harness.click("Plane");
    harness.settle();
    let plane = open_feature(&harness);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let extrude = open_feature(&harness);
    let FeatureKind::Solid(SolidFeature::Extrude(mut definition)) =
        kind_of(&harness, extrude).clone()
    else {
        panic!("an extrusion is open");
    };
    definition.extent = ExtrudeExtent::OneSide {
        end: ExtrudeEnd::up_to_face(PlaneReference::Datum(plane)),
        reversed: false,
    };
    harness.perform(Action::Apply(Transaction::single(
        "Up to the plane",
        Edit::SetFeatureKind {
            id: extrude,
            kind: FeatureKind::Solid(SolidFeature::Extrude(definition)),
        },
    )));
    harness.settle();
    let offset = |harness: &Harness| {
        let FeatureKind::Solid(SolidFeature::Extrude(extrude)) = kind_of(harness, extrude) else {
            panic!("the extrusion stays an extrusion");
        };
        let ExtrudeExtent::OneSide {
            end: ExtrudeEnd::UpToFace { offset, .. },
            ..
        } = &extrude.extent
        else {
            panic!("the extrusion stays up to the plane");
        };
        offset.as_deref().map(|offset| value_of(harness, offset))
    };

    assert_eq!(offset(&harness), None);

    let step = drag_value_arrow(&mut harness, Measured::EndOffset(Reach::Only), 3.0);
    let found = offset(&harness).expect("the drag set an offset");

    assert_eq!(harness.model.undo_label(), Some("Edit Extrude 1"));
    assert!((found - 3.0).abs() <= step, "{found} with steps of {step}");
}
