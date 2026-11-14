use egui::{Key, Modifiers, accesskit::Role};

use super::{Harness, extruded_plate, feature_named, run_from_palette};
use crate::{
    model::{Action, Notice, NoticeAction, RecomputeStatus},
    panels::Focus,
    status_bar,
};

#[test]
fn recent_messages_keep_a_failed_save_after_a_later_notice_replaced_it() {
    let mut harness = Harness::new();
    harness.perform(Action::Inform(Notice::failure(
        "Saving “plate” failed: the disk is full.",
    )));
    harness.perform(Action::Inform(Notice::info("Exported 1 body.")));
    harness.perform(Action::Inform(Notice::info("Exported 1 body.")));
    harness.frame();
    assert!(harness.shows("Exported 1 body."));
    assert!(!harness.shows("Saving “plate” failed: the disk is full."));

    harness.click("Help");
    harness.click(crate::messages::TITLE);

    assert!(harness.workspace.messages_open);
    assert!(harness.shows("Saving “plate” failed: the disk is full."));
    assert_eq!(harness.count_shown("Exported 1 body."), 2);
    assert!(!harness.shows(crate::messages::EMPTY));
    harness.click(crate::messages::COPY_ALL);
    harness.frame();
    let copied = harness.clipboard.clone().unwrap_or_default();
    let lines: Vec<&str> = copied.lines().collect();
    assert_eq!(lines.len(), harness.model.recorded_notices().count());
    assert_eq!(lines[0], "Information: Exported 1 body. (just now)");
    assert_eq!(
        lines[1],
        "Error: Saving “plate” failed: the disk is full. (just now)"
    );
    harness.click_button(crate::widgets::CLOSE_DIALOG);
    assert!(!harness.workspace.messages_open);

    assert_eq!(
        harness
            .model
            .recorded_notices()
            .take(2)
            .map(|recorded| recorded.notice.text.as_str())
            .collect::<Vec<_>>(),
        [
            "Exported 1 body.",
            "Saving “plate” failed: the disk is full."
        ]
    );
}

#[test]
fn a_cancelled_or_stopped_recompute_is_announced() {
    let mut harness = Harness::new();
    harness.settle();
    harness.context.enable_accesskit();
    let live_of = |harness: &Harness, text: &str| {
        harness
            .accessible
            .iter()
            .find(|(_, node)| node.role() == Role::Label && node.value() == Some(text))
            .map(|(_, node)| node.live())
    };

    harness.model.set_status(RecomputeStatus::Cancelled);
    harness.frame();
    harness.frame();
    let cancelled = live_of(&harness, crate::status_bar::CANCELLED);
    harness.model.set_status(RecomputeStatus::Stopped);
    harness.frame();
    harness.frame();
    let stopped = live_of(&harness, crate::status_bar::STOPPED);

    assert_eq!(cancelled, Some(Some(egui::accesskit::Live::Polite)));
    assert_eq!(stopped, Some(Some(egui::accesskit::Live::Assertive)));
}

#[test]
fn notices_are_live_regions_so_a_screen_reader_announces_them() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    harness.frame();
    let live_of = |harness: &Harness, text: &str| {
        harness
            .accessible
            .iter()
            .find(|(_, node)| node.role() == Role::Label && node.value() == Some(text))
            .map(|(_, node)| node.live())
    };

    harness.perform(Action::Inform(Notice::info("Exported 1 body.")));
    harness.frame();
    harness.frame();
    assert_eq!(
        live_of(&harness, "Exported 1 body."),
        Some(Some(egui::accesskit::Live::Polite))
    );

    harness.perform(Action::Inform(Notice::failure(
        "Saving failed: the disk is full.",
    )));
    harness.frame();
    harness.frame();
    assert_eq!(
        live_of(&harness, "Saving failed: the disk is full."),
        Some(Some(egui::accesskit::Live::Assertive))
    );
}

#[test]
fn recent_messages_open_from_the_palette_and_close_with_escape() {
    let mut harness = Harness::new();

    run_from_palette(&mut harness, "recent messages");
    assert!(harness.workspace.messages_open);
    assert!(harness.shows(crate::messages::TITLE));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.workspace.messages_open);
}

#[test]
fn the_failed_pill_is_a_button_that_shows_the_first_failed_feature() {
    let mut harness = Harness::new();
    let height = harness.parameter("height");
    harness.type_into(
        Focus::ParameterValue(height),
        "400 mm * 1 mm / (width - 30 mm)",
    );
    let width = harness.parameter("width");
    harness.type_into(Focus::ParameterValue(width), "30 mm");
    harness.settle();

    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();
    assert!(
        harness.accessible.iter().any(|(_, node)| {
            node.role() == Role::Button
                && node.label() == Some("1 feature failed")
                && node.live() == Some(egui::accesskit::Live::Assertive)
        }),
        "the failed pill is a live region once accessibility is on"
    );

    harness.click_button("1 feature failed");
    harness.frame();
    harness.frame();
    assert_eq!(
        harness.workspace.panels.selected,
        Some(feature_named(&harness, "Side sketch"))
    );
}

#[test]
fn a_deleted_feature_is_named_in_a_notice_whose_undo_brings_it_back_while_it_is_the_last_change() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();
    let name = harness.document().feature(plate).unwrap().name.clone();
    let before = harness.document().features().count();

    harness.workspace.panels.selected = Some(plate);
    harness.key(Key::Delete, Modifiers::NONE);
    harness.frame();
    harness.settle();
    let told = harness.model.notice().map(|notice| notice.text.clone());
    let after = harness.document().features().count();
    harness.click_button(status_bar::UNDO_SWITCH);
    harness.settle();
    let restored = harness.document().features().count();

    harness.workspace.panels.selected = Some(plate);
    harness.key(Key::Delete, Modifiers::NONE);
    harness.frame();
    harness.settle();
    let offered = harness
        .model
        .notice()
        .and_then(|notice| notice.action.clone());
    harness.edit_width("61 mm");
    harness.perform(Action::Inform(Notice {
        action: offered.clone(),
        ..Notice::info("Deleted it.")
    }));
    harness.frame();
    let offers_a_stale_step = offered
        .as_ref()
        .is_some_and(|action| harness.model.offers(action));
    let Some(NoticeAction::UndoStep(stale)) = offered else {
        panic!("a delete offers to undo its step");
    };
    harness.perform(Action::UndoStep(stale));
    harness.frame();

    assert_eq!(told, Some(format!("Deleted {name}.")));
    assert_eq!((after, restored), (before - 1, before));
    assert!(!offers_a_stale_step);
    assert!(harness.shows(crate::model::STEP_NOT_LAST));
    assert_eq!(harness.document().features().count(), before - 1);
}

#[test]
fn undo_and_redo_name_the_step_they_took_without_filling_recent_messages_or_hiding_a_failure() {
    let mut harness = Harness::new();
    harness.edit_width("61 mm");
    harness.edit_width("62 mm");
    harness.perform(Action::DismissNotice);
    harness.frame();
    let label = harness.model.undo_label().unwrap().to_owned();
    let recorded = harness.model.recorded_notices().count();

    harness.perform(Action::Undo);
    harness.frame();
    let undone = harness.model.notice().map(|notice| notice.text.clone());
    harness.perform(Action::Redo);
    harness.frame();
    let redone = harness.model.notice().map(|notice| notice.text.clone());
    harness.perform_all(vec![Action::Undo, Action::Undo]);
    harness.frame();
    let jumped = harness.model.notice().map(|notice| notice.text.clone());
    let recorded_after = harness.model.recorded_notices().count();

    harness.perform(Action::Inform(Notice::failure(
        "Saving “plate” failed: the disk is full.",
    )));
    harness.perform(Action::Redo);
    harness.frame();

    assert_eq!(undone, Some(format!("Undid “{label}”.")));
    assert_eq!(redone, Some(format!("Redid “{label}”.")));
    assert!(
        jumped
            .as_deref()
            .is_some_and(|text| text.starts_with("Undid 2 steps, back to before “")),
        "{jumped:?}"
    );
    assert_eq!(recorded_after, recorded);
    assert!(harness.shows("Saving “plate” failed: the disk is full."));
}

#[test]
fn delete_with_nothing_to_delete_names_the_tools_that_take_faces_and_bodies_away() {
    let mut harness = Harness::new();
    harness.frame();

    harness.key(Key::Delete, Modifiers::NONE);
    harness.frame();
    harness.frame();

    let notice = harness.model.notice().map(|notice| notice.text.clone());
    assert!(
        notice
            .as_deref()
            .is_some_and(|text| text.contains("Delete face") && text.contains("Remove body")),
        "{notice:?}"
    );
}
