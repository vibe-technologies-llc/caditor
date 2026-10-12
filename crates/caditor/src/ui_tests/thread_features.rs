use egui::{Key, Modifiers};

use super::{Harness, extruded_plate, extruded_shaft, thread_lines, thread_of};
use crate::model::Action;

#[test]
fn a_thread_on_a_shaft_names_its_designation_draws_its_lines_and_takes_the_hand() {
    let mut harness = Harness::new();
    harness.perform(Action::ShowThreadsAsCosmetic(true));
    let side = extruded_shaft(&mut harness);
    assert!(thread_lines(&mut harness).is_empty());

    harness.select([side]);
    harness.use_tool_with(Key::O, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    let thread = harness
        .workspace
        .editing
        .solid()
        .expect("the thread is open");
    assert!(
        harness
            .model
            .undo_label()
            .is_some_and(|label| label.starts_with("Create Thread on Extrude 1 side"))
    );
    assert!(harness.shows("M8-6g"));
    assert!(harness.shows("Choose the face to thread, its size and its length in the panel"));
    assert!(harness.shows("External, on a shaft or boss"));
    assert!(harness.shows(crate::thread_panel::DESCRIPTION));

    harness.click("Left");
    harness.settle();
    assert_eq!(
        thread_of(&harness, thread).hand,
        caditor_document::ThreadHand::Left
    );
    assert!(harness.shows("M8-6g-LH"));

    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }
    assert_eq!(harness.workspace.editing.solid(), None);
    let lines = thread_lines(&mut harness);
    assert!(!lines.is_empty());
    assert!(lines.iter().all(|line| line.stroke.dashes_where_hidden()));
    assert!(lines.iter().any(|line| line.stroke.is_dashed_where_seen()));
    assert!(lines.iter().any(|line| !line.stroke.is_dashed_where_seen()));
}

#[test]
fn the_thread_command_refuses_a_flat_face_and_several_faces() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    let no_thread = |harness: &Harness| {
        harness
            .document()
            .features()
            .all(|feature| feature.kind.thread().is_none())
    };

    harness.select([top]);
    harness.use_tool_with(Key::O, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    assert!(no_thread(&harness));
    assert_eq!(harness.workspace.editing.solid(), None);

    harness.select([]);
    harness.use_tool_with(Key::O, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    assert!(no_thread(&harness));
}
