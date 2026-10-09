use caditor_document::{
    Combine, Extrude, ExtrudeEnd, ExtrudeExtent, FeatureId, FeatureKind, Move, Scale, SolidFeature,
    Transaction,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Event, Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, add_peg, draft_volume, drag_screen, extruded_plate, pickable_described,
    rectangle, run_from_palette,
};
use crate::{
    combine_panel,
    model::Action,
    move_manipulator::{Handle, Reach},
    reversing::NOTHING_TO_REVERSE,
    scale_panel,
    selection::Pickable,
};

const ONLY: Handle = Handle::Reach(Reach::Only);

fn open_extrusion(harness: &mut Harness) -> FeatureId {
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open")
}

fn distance_field(extrude: FeatureId) -> Id {
    Id::new(("solid-field", "distance", extrude))
}

fn extrude_of(harness: &Harness, feature: FeatureId) -> Extrude {
    match harness.solid(feature) {
        SolidFeature::Extrude(extrude) => extrude.clone(),
        SolidFeature::Revolve(_) => panic!("the feature is an extrusion"),
    }
}

fn end_distance(harness: &Harness, feature: FeatureId) -> Expression {
    match extrude_of(harness, feature).extent {
        ExtrudeExtent::OneSide {
            end: ExtrudeEnd::Distance(distance),
            ..
        } => distance,
        other => panic!("the extrusion runs a distance one way, not {other:?}"),
    }
}

fn millimetres(harness: &Harness, expression: &Expression) -> f64 {
    let parameters = harness.model.parameters();
    expression
        .evaluate_as(Dimension::LENGTH, &|id| parameters.value(id))
        .unwrap()
}

fn drag_arrow(harness: &mut Harness, along: f64) {
    let from = harness
        .workspace
        .viewport
        .handle_position(ONLY, 0.0)
        .expect("the distance arrow is shown");
    let to = harness
        .workspace
        .viewport
        .handle_position(ONLY, along)
        .unwrap();
    drag_screen(harness, from, to);
    harness.frame();
    harness.settle();
}

#[test]
fn a_named_distance_being_typed_previews_the_extrusion_and_its_arrow_follows() {
    let mut harness = Harness::new();
    let extrude = open_extrusion(&mut harness);
    let before = harness
        .workspace
        .viewport
        .handle_position(ONLY, 0.0)
        .expect("the distance arrow is shown");

    harness.draft_into_field(distance_field(extrude), "h = 30 mm");
    harness.wait_until("the typed distance is previewed", |harness| {
        draft_volume(harness, extrude).is_some()
    });
    harness.frame();
    let after = harness
        .workspace
        .viewport
        .handle_position(ONLY, 0.0)
        .expect("the arrow is still shown");

    assert!((draft_volume(&harness, extrude).unwrap() - 40.0 * 40.0 * 30.0).abs() < 1.0);
    assert_eq!(harness.model.undo_label(), Some("Create Extrude 1"));
    assert!(before.distance(after) > 5.0, "{before:?} {after:?}");
}

#[test]
fn a_datum_plane_offset_being_typed_is_drawn_before_it_is_entered() {
    let mut harness = Harness::new();
    harness.select([]);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the new plane is open");
    let at_height =
        |harness: &mut Harness, height: f64| {
            harness.built().scene.lines().any(|line| {
                (line.start.z - height).abs() < 1e-6 && (line.end.z - height).abs() < 1e-6
            })
        };

    harness.draft_into_field(Id::new(("datum-field", "offset", plane)), "25 mm");
    harness.wait_until("the typed offset is computed", |harness| {
        harness.model.draft_evaluation().is_some()
    });
    harness.frame();

    assert_eq!(harness.model.undo_label(), Some("Create Plane 1"));
    assert!(at_height(&mut harness, 25.0));
    assert!(!at_height(&mut harness, 10.0));
}

#[test]
fn a_hole_diameter_being_typed_previews_its_drills() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");

    harness.draft_into_field(Id::new(("hole-field", "diameter", hole)), "9 mm");
    harness.wait_until("the typed diameter is drilled", |harness| {
        harness
            .model
            .draft_cuts()
            .is_some_and(|cuts| !cuts.is_empty())
    });

    assert!(
        harness
            .model
            .undo_label()
            .is_some_and(|label| label.starts_with("Create"))
    );
}

#[test]
fn dragging_the_arrow_of_a_named_distance_changes_the_name_and_keeps_it() {
    let mut harness = Harness::new();
    let extrude = open_extrusion(&mut harness);
    harness.type_into_field(distance_field(extrude), "h = 20 mm");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let h = harness.parameter("h");
    let step = harness.workspace.viewport.manipulator_step().unwrap();

    drag_arrow(&mut harness, -6.0);

    let held = harness.document().parameter(h).unwrap().expression.clone();
    let value = millimetres(&harness, &held);
    assert_eq!(
        end_distance(&harness, extrude),
        Expression::Parameter(h),
        "{:?}",
        harness.model.undo_label()
    );
    assert!((value - 14.0).abs() <= step, "{value} with steps of {step}");
    assert_eq!(harness.model.undo_label(), Some("Edit Extrude 1"));

    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(harness.expression_text("h"), "20 mm");
    assert_eq!(end_distance(&harness, extrude), Expression::Parameter(h));
}

#[test]
fn an_arrow_whose_distance_follows_another_parameter_says_so_and_does_not_drag() {
    let mut harness = Harness::new();
    let mut transaction = harness.document().transaction("Add depth");
    let depth = transaction.parse("20 mm").unwrap();
    transaction.add_parameter("depth", depth);
    let transaction: Transaction = transaction.finish();
    harness.perform(Action::Apply(transaction));
    harness.settle();
    let extrude = open_extrusion(&mut harness);
    harness.type_into_field(distance_field(extrude), "depth");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let revision = harness.model.revision();
    let from = harness
        .workspace
        .viewport
        .handle_position(ONLY, 0.0)
        .expect("the distance arrow is shown");

    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    let explained = harness.shows("Distance follows depth; change depth in Parameters");
    drag_arrow(&mut harness, -6.0);

    assert!(explained);
    assert_eq!(harness.model.revision(), revision);
    assert_eq!(harness.expression_text("depth"), "20 mm");
    assert_eq!(
        end_distance(&harness, extrude),
        Expression::Parameter(harness.parameter("depth"))
    );
}

#[test]
fn dragging_a_move_arrow_of_a_named_distance_keeps_the_name() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Move body");
    harness.settle();
    let movement = harness.workspace.editing.solid().expect("the move is open");
    harness.type_into_field(Id::new(("move-field", "offset", 0, movement)), "dx = 5 mm");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let dx = harness.parameter("dx");
    let along_x = Handle::Along(caditor_document::MoveAxis::X);
    let from = harness
        .workspace
        .viewport
        .handle_position(along_x, 0.0)
        .expect("the X arrow is shown");
    let to = harness
        .workspace
        .viewport
        .handle_position(along_x, 10.0)
        .unwrap();

    drag_screen(&mut harness, from, to);
    harness.frame();
    harness.settle();

    let moved: Move = match &harness.document().feature(movement).unwrap().kind {
        FeatureKind::Move(moved) => moved.clone(),
        _ => panic!("the move is still a move"),
    };
    let held = harness.document().parameter(dx).unwrap().expression.clone();
    assert_eq!(moved.offset[0], Expression::Parameter(dx));
    assert!(millimetres(&harness, &held) > 10.0);
    assert_eq!(harness.model.undo_label(), Some("Edit Move body 1"));
}

#[test]
fn a_value_field_selects_its_text_when_focus_arrives_so_typing_replaces_it() {
    let mut harness = Harness::new();
    let extrude = open_extrusion(&mut harness);
    let field = distance_field(extrude);

    harness
        .context
        .memory_mut(|memory| memory.request_focus(field));
    harness.frame();
    harness.frame();
    harness.events.push(Event::Text("30 mm".to_owned()));
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    let typed = end_distance(&harness, extrude);
    assert!(
        (millimetres(&harness, &typed) - 30.0).abs() < 1e-9,
        "{}",
        harness.document().expression_text(&typed)
    );
}

fn other_plate_face(harness: &mut Harness, plate: FeatureId, top: Pickable) -> Pickable {
    pickable_described(harness, "Extrude 1 › Extrude 1 start face");
    let built = harness.built();
    let faces: Vec<Pickable> = built
        .picks
        .pickables()
        .filter(|pickable| {
            matches!(pickable, Pickable::Face { body, .. } if *body == plate) && *pickable != top
        })
        .collect();
    faces
        .into_iter()
        .find(|pickable| {
            pickable
                .describe(harness.document(), harness.model.evaluation())
                .contains("side")
        })
        .expect("a side face of the plate is pickable")
}

#[test]
fn choosing_shell_faces_in_the_view_first_takes_the_selected_ones() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let side = other_plate_face(&mut harness, plate, top);
    harness.select([top]);
    harness.click("Shell");
    harness.settle();
    let shell = harness
        .workspace
        .editing
        .solid()
        .expect("the shell is open");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness.settle();

    harness.select([top, side]);
    harness.click_button("Show details of Shell 1");
    harness.frame();
    harness.click("Choose in the view");
    harness.settle();

    let opened = harness
        .document()
        .feature(shell)
        .and_then(|feature| feature.kind.shell())
        .map(|shell| shell.open.len());
    assert_eq!(harness.workspace.editing.solid(), Some(shell));
    assert_eq!(opened, Some(2));
    assert_eq!(
        harness.model.undo_label(),
        Some("Open the selected faces of Shell 1")
    );
}

#[test]
fn choosing_offset_faces_in_the_view_first_takes_the_selected_ones() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let side = other_plate_face(&mut harness, plate, top);
    harness.select([top]);
    harness.key(Key::Q, Modifiers::ALT);
    harness.settle();
    let offset = harness
        .workspace
        .editing
        .solid()
        .expect("the offset face is open");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness.settle();

    harness.select([top, side]);
    harness.click_button("Show details of Offset face 1");
    harness.frame();
    harness.click("Choose in the view");
    harness.settle();

    let moved = harness
        .document()
        .feature(offset)
        .and_then(|feature| feature.kind.offset_face())
        .map(|offset| offset.faces.len());
    assert_eq!(moved, Some(2));
    assert_eq!(
        harness.model.undo_label(),
        Some("Move the selected faces with Offset face 1")
    );
}

fn combine_of(harness: &Harness, feature: FeatureId) -> Combine {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.combine())
        .unwrap()
        .clone()
}

#[test]
fn a_combine_takes_its_target_in_pick_order_and_swaps_it_with_the_tool() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);
    let peg_top = pickable_described(&mut harness, "Peg › Peg end face");

    harness.select([peg_top, top]);
    harness.click("Combine");
    harness.settle();
    let combine = harness
        .workspace
        .editing
        .solid()
        .expect("the combine is open");
    let made = combine_of(&harness, combine);

    harness.click(combine_panel::SWAP);
    harness.settle();
    let swapped = combine_of(&harness, combine);

    assert_eq!((made.body, made.tool), (peg, plate));
    assert_eq!((swapped.body, swapped.tool), (plate, peg));
    assert_eq!(harness.model.undo_label(), Some("Edit Combine 1"));
}

#[test]
fn reverse_the_direction_flips_the_open_feature_and_refuses_one_without_a_direction() {
    let mut harness = Harness::new();
    let extrude = open_extrusion(&mut harness);

    run_from_palette(&mut harness, "Reverse the direction");
    harness.settle();
    let reversed = matches!(
        extrude_of(&harness, extrude).extent,
        ExtrudeExtent::OneSide { reversed: true, .. }
    );
    assert!(reversed);
    assert_eq!(harness.model.undo_label(), Some("Reverse Extrude 1"));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Shell");
    harness.settle();
    let revision = harness.model.revision();
    run_from_palette(&mut harness, "Reverse the direction");
    harness.settle();

    assert_eq!(harness.model.revision(), revision);
    assert!(harness.shows_containing(NOTHING_TO_REVERSE));
}

fn scale_of(harness: &Harness, feature: FeatureId) -> Scale {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| match &feature.kind {
            FeatureKind::Scale(scale) => Some(scale.clone()),
            _ => None,
        })
        .unwrap()
}

fn centre_of(harness: &Harness, feature: FeatureId) -> [f64; 3] {
    scale_of(harness, feature)
        .center
        .map(|value| millimetres(harness, &value))
}

#[test]
fn a_scale_centre_is_taken_from_a_selected_corner_or_the_body_centre() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.key(Key::S, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    let scale = harness
        .workspace
        .editing
        .solid()
        .expect("the scale is open");
    harness.type_into_field(Id::new(("scale-field", ("factor", 0), scale)), "2");
    harness.settle();
    let corner = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| matches!(pickable, Pickable::Vertex { body, .. } if *body == plate))
        .expect("a corner of the plate is pickable");

    harness.click_button(scale_panel::BODY_CENTRE);
    harness.settle();
    let middle = centre_of(&harness, scale);
    harness.select([corner]);
    harness.frame();
    harness.click("Use selected");
    harness.settle();
    let at_corner = centre_of(&harness, scale);

    assert_eq!(middle, [20.0, 20.0, 5.0]);
    assert!(
        at_corner
            .iter()
            .zip([40.0, 40.0, 10.0])
            .all(|(found, most)| *found == 0.0 || *found == most),
        "{at_corner:?}"
    );
    assert_eq!(harness.model.undo_label(), Some("Edit Scale body 1"));
}

#[test]
fn a_thread_is_moved_to_another_round_face_with_use_selected() {
    let mut harness = Harness::new();
    harness.select([]);
    run_from_palette(&mut harness, "Cylinder");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.select([]);
    run_from_palette(&mut harness, "Cylinder");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();
    let first = pickable_described(&mut harness, "Cylinder 1 › Cylinder 1 wall");
    let second = pickable_described(&mut harness, "Cylinder 2 › Cylinder 2 wall");
    let Pickable::Face {
        body: second_body, ..
    } = second
    else {
        panic!("the wall is a face");
    };

    harness.select([first]);
    harness.key(Key::O, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    let thread = harness
        .workspace
        .editing
        .solid()
        .expect("the thread is open");
    harness.select([second]);
    harness.frame();
    harness.click("Use selected");
    harness.settle();

    let moved = harness
        .document()
        .feature(thread)
        .and_then(|feature| feature.kind.thread())
        .map(|thread| thread.body);
    assert_eq!(moved, Some(second_body));
    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Edit {}", harness.document().feature(thread).unwrap().name).as_str())
    );
}
