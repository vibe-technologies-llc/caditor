use egui::{Key, Modifiers, accesskit::Role};

use super::{Harness, feature_named, run_from_palette};
use crate::{
    model::{Action, Notice, RecomputeStatus},
    panels::Focus,
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
