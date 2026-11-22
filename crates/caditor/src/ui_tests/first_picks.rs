use caditor_document::{
    FeatureId, FeatureState, MissingInput, PlaneReference, PrincipalPlane, SketchAttachment,
    SplitAlong,
};
use caditor_geometry::{Plane, Point2};
use egui::{Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, add_peg, extruded_plate, extruded_shaft, offer, run_from_palette,
    thread_of,
};
use crate::{
    body_selection,
    commands::Command,
    first_pick::{self, FaceTool},
    reference_picking::Slot,
    selection::Pickable,
};

const FIRST_PICK_TOOLS: [Command; 5] = [
    Command::Hole,
    Command::Thread,
    Command::Draft,
    Command::SplitFace,
    Command::ReplaceFace,
];

fn waiting_for(harness: &Harness, feature: FeatureId) -> Option<MissingInput> {
    match harness.model.evaluation().feature(feature)?.state {
        FeatureState::Waiting(missing) => Some(missing),
        _ => None,
    }
}

fn face_described(harness: &mut Harness, part: &str) -> Pickable {
    harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            matches!(pickable, Pickable::Face { .. })
                && pickable
                    .describe(harness.document(), harness.model.evaluation())
                    .contains(part)
        })
        .unwrap_or_else(|| panic!("no face is described with {part}"))
}

fn in_open_feature(harness: &Harness, face: Pickable) -> Pickable {
    let Pickable::Face { face, .. } = face else {
        panic!("{face:?} is not a face");
    };
    let feature = harness
        .workspace
        .editing
        .solid()
        .expect("a feature is open");
    Pickable::ShellFace { feature, face }
}

fn start_from_nothing(harness: &mut Harness, command: Command) {
    harness.select([]);
    run_from_palette(harness, &command.title());
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
}

fn refusal(harness: &Harness, command: Command) -> String {
    offer(harness, command)
        .availability
        .expect_err("the command is refused")
        .into_owned()
}

#[test]
fn a_hole_started_with_nothing_selected_waits_for_the_face_clicked_after() {
    let mut harness = Harness::new();

    let (plate, top) = extruded_plate(&mut harness);

    harness.select([]);

    let features = harness.document().features().count();

    harness.use_tool_with(Key::O, Modifiers::ALT);
    harness.settle();

    let awaited = harness.workspace.editing.awaited_face();
    let counted_while_waiting = harness.document().features().count();
    let prompted = harness.shows(first_pick::HOLE_PROMPT);
    let carded = harness.shows(first_pick::WAITING_FOR_A_FACE);

    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    harness.settle();

    let hole = harness.workspace.editing.solid().expect("the hole is open");
    let definition = harness
        .document()
        .feature(hole)
        .and_then(|feature| feature.kind.hole())
        .expect("the open feature is a hole")
        .clone();
    let on_face = harness
        .document()
        .feature(definition.sketch)
        .and_then(|sketch| sketch.kind.attachment())
        .is_some_and(|attachment| matches!(attachment, SketchAttachment::Face(_)));

    assert_eq!(awaited, Some(FaceTool::Hole));
    assert_eq!(counted_while_waiting, features);
    assert!(prompted);
    assert!(carded);
    assert_eq!(definition.body, plate);
    assert!(on_face);
    assert_eq!(harness.model.undo_label(), Some("Create Hole 1"));
    assert_eq!(harness.workspace.editing.awaited_face(), None);
    assert!(harness.body_volume(plate) < 16000.0);
}

#[test]
fn escape_or_cancel_stops_waiting_for_a_face_without_making_anything() {
    let mut harness = Harness::new();

    extruded_plate(&mut harness);

    let features = harness.document().features().count();

    start_from_nothing(&mut harness, Command::Hole);

    let waited = harness.workspace.editing.awaited_face();

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    let after_escape = harness.workspace.editing.awaited_face();

    start_from_nothing(&mut harness, Command::Thread);

    let waited_again = harness.workspace.editing.awaited_face();

    harness.click(first_pick::CANCEL);
    harness.frame();

    assert_eq!(waited, Some(FaceTool::Hole));
    assert_eq!(after_escape, None);
    assert_eq!(waited_again, Some(FaceTool::Thread));
    assert_eq!(harness.workspace.editing.awaited_face(), None);
    assert_eq!(harness.document().features().count(), features);
}

#[test]
fn a_thread_started_with_nothing_selected_refuses_a_flat_face_and_takes_the_round_one() {
    let mut harness = Harness::new();

    let side = extruded_shaft(&mut harness);
    let top = face_described(&mut harness, "end face");

    harness.select([]);
    harness.use_tool_with(Key::O, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();

    let awaited = harness.workspace.editing.awaited_face();

    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    harness.settle();

    let refused = harness
        .model
        .notice()
        .map(|notice| notice.text.clone())
        .unwrap_or_default();
    let still_waiting = harness.workspace.editing.awaited_face();

    harness.click_pickable(Plane::XY, Point2::new(24.0, 20.0), side);
    harness.settle();

    let thread = harness
        .workspace
        .editing
        .solid()
        .expect("the thread is open");
    let name = harness
        .document()
        .feature(thread)
        .map(|feature| feature.name.clone())
        .unwrap_or_default();

    assert_eq!(awaited, Some(FaceTool::Thread));
    assert!(
        refused.starts_with("Thread: Select a cylindrical face"),
        "{refused}"
    );
    assert_eq!(still_waiting, Some(FaceTool::Thread));
    assert!(name.starts_with("Thread on "), "{name}");
    assert_eq!(thread_of(&harness, thread).size.label(), "M8");
    assert_eq!(harness.workspace.editing.awaited_face(), None);
}

#[test]
fn a_draft_started_with_nothing_selected_takes_its_pull_from_the_first_face_clicked() {
    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);
    let wall = face_described(&mut harness, " side ");

    start_from_nothing(&mut harness, Command::Draft);

    let draft = harness
        .workspace
        .editing
        .solid()
        .expect("the draft is open");
    let waiting = waiting_for(&harness, draft);
    let before = harness
        .document()
        .feature(draft)
        .and_then(|feature| feature.kind.draft())
        .cloned()
        .expect("the open feature is a draft");

    let clicked = in_open_feature(&harness, wall);

    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), clicked);
    harness.settle();

    let after = harness
        .document()
        .feature(draft)
        .and_then(|feature| feature.kind.draft())
        .cloned()
        .expect("the open feature is a draft");

    assert_eq!(waiting, Some(MissingInput::FacesToDraft { body: plate }));
    assert!(before.faces.is_empty());
    assert_eq!(before.body, plate);
    assert_eq!(
        before.neutral,
        PlaneReference::Principal(PrincipalPlane::Xy)
    );
    assert_eq!(after.faces.len(), 1);
    assert!(
        matches!(after.neutral, PlaneReference::Face(_)),
        "{:?}",
        after.neutral
    );
    assert_eq!(waiting_for(&harness, draft), None);
}

#[test]
fn a_split_face_started_with_nothing_selected_takes_a_face_then_chooses_its_plane() {
    let mut harness = Harness::new();

    let (plate, top) = extruded_plate(&mut harness);

    start_from_nothing(&mut harness, Command::SplitFace);

    let split = harness
        .workspace
        .editing
        .solid()
        .expect("the split is open");
    let waiting = waiting_for(&harness, split);
    let choosing_first = harness.workspace.editing.picking();

    let clicked = in_open_feature(&harness, top);

    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), clicked);
    harness.settle();

    let definition = harness
        .document()
        .feature(split)
        .and_then(|feature| feature.kind.split_face())
        .cloned()
        .expect("the open feature splits faces");
    let choosing = harness
        .workspace
        .editing
        .picking()
        .map(|picking| picking.slot);

    assert_eq!(waiting, Some(MissingInput::FacesToSplit { body: plate }));
    assert_eq!(choosing_first, None);
    assert_eq!(definition.faces.len(), 1);
    assert_eq!(definition.along, SplitAlong::Unchosen);
    assert_eq!(choosing, Some(Slot::SplitPlane));
    assert_eq!(
        waiting_for(&harness, split),
        Some(MissingInput::SplitAlong { body: plate })
    );
}

#[test]
fn a_replace_face_started_with_nothing_selected_takes_the_faces_clicked_then_waits_for_a_target() {
    let mut harness = Harness::new();

    let (plate, top) = extruded_plate(&mut harness);

    start_from_nothing(&mut harness, Command::ReplaceFace);

    let replace = harness
        .workspace
        .editing
        .solid()
        .expect("the replace face is open");
    let waiting = waiting_for(&harness, replace);

    let clicked = in_open_feature(&harness, top);

    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), clicked);
    harness.settle();

    let definition = harness
        .document()
        .feature(replace)
        .and_then(|feature| feature.kind.replace_face())
        .cloned()
        .expect("the open feature replaces faces");

    assert_eq!(waiting, Some(MissingInput::FacesToReplace { body: plate }));
    assert_eq!(definition.faces.len(), 1);
    assert_eq!(definition.target, None);
    assert_eq!(
        waiting_for(&harness, replace),
        Some(MissingInput::TargetPlane { body: plate })
    );
}

#[test]
fn the_first_pick_tools_refuse_in_words_with_no_body_or_several_shown() {
    let mut harness = Harness::new();

    harness.frame();

    let without_bodies: Vec<String> = FIRST_PICK_TOOLS
        .iter()
        .map(|command| refusal(&harness, *command))
        .collect();

    extruded_plate(&mut harness);
    add_peg(&mut harness);
    harness.select([]);
    harness.frame();

    let with_two: Vec<String> = FIRST_PICK_TOOLS
        .iter()
        .map(|command| refusal(&harness, *command))
        .collect();

    for reason in &without_bodies {
        assert_eq!(reason, body_selection::NO_BODY_SHOWN);
    }
    for reason in &with_two {
        assert!(
            reason.contains("with one body shown"),
            "{reason} does not say a lone body can be started first"
        );
    }
}
