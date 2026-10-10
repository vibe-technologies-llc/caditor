use egui::{Key, KeyboardShortcut, Modifiers};
use tempfile::TempDir;

use super::{Harness, fail_a_frame, run_from_palette};
use crate::{
    app::Workspace,
    commands::Command,
    model::{Action, Notice, RecomputeStatus},
    preferences::{PreferenceChange, Preferences, PreferencesCommand},
    shortcut_editor, status_bar,
    widgets::Tone,
};

fn notice_text(harness: &Harness) -> Option<String> {
    harness.model.notice().map(|notice| notice.text.clone())
}

fn open_palette(harness: &mut Harness, query: &str) {
    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.show_new_windows();
    harness.type_text(query);
}

fn close_palette(harness: &mut Harness) {
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
}

#[test]
fn notices_show_their_tone_and_lead_to_recent_messages() {
    let mut harness = Harness::new();

    harness.perform(Action::Inform(Notice::warning("Nothing was pasted.")));
    let warned = harness.shows(Tone::Warning.icon()) && harness.shows("Nothing was pasted.");
    harness.perform(Action::Inform(Notice::success("Exported 1 body.")));
    let succeeded = harness.shows(Tone::Success.icon()) && harness.shows("Exported 1 body.");
    harness.click_button(status_bar::SHOW_MESSAGES);
    harness.show_new_windows();
    let listed = harness.count_shown("Nothing was pasted.");

    assert!(warned);
    assert!(succeeded);
    assert!(harness.workspace.messages_open);
    assert_eq!(listed, 1);
    assert!(harness.describes("Warning"));
    assert!(harness.describes("Done"));
}

#[test]
fn the_palette_shows_whether_a_toggle_is_on_and_quiet_toggles_say_what_changed() {
    let mut harness = Harness::new();

    open_palette(&mut harness, "turn snapping");
    let on_at_first = harness.shows("On");
    close_palette(&mut harness);
    run_from_palette(&mut harness, "turn snapping");
    harness.frame();
    let told = notice_text(&harness);
    open_palette(&mut harness, "turn snapping");
    let off_now = harness.shows("Off") && !harness.shows("On");
    close_palette(&mut harness);

    harness.perform(Action::DismissNotice);
    harness.click("View");
    harness.click("Turn snapping on or off");
    harness.frame();

    assert!(on_at_first);
    assert_eq!(
        told.as_deref(),
        Some("Snapping off. Hold Alt to snap a point.")
    );
    assert!(off_now);
    assert!(harness.workspace.viewport.snapping());
    assert_eq!(notice_text(&harness), None);
}

#[test]
fn a_refused_palette_command_closes_the_palette_and_says_why() {
    let mut harness = Harness::new();

    open_palette(&mut harness, "finish sketch");
    let open_before = harness.workspace.palette.is_open();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    harness.frame();
    let told = notice_text(&harness);

    assert!(open_before);
    assert!(!harness.workspace.palette.is_open());
    assert!(told.is_some_and(|text| text.starts_with("Finish sketch is not available")));
}

#[test]
fn recent_palette_commands_outlast_a_failed_frame_and_a_restart() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));

    run_from_palette(&mut harness, "zoom extents");
    harness.frame();
    let recent = harness.workspace.palette.recent().to_vec();
    fail_a_frame(&mut harness);
    let after_failure = harness.workspace.palette.recent().to_vec();
    harness.wait_until("the recent commands are saved", |_| {
        caditor_file::Settings::load(&dir.path().join("config")).texts("palette.recent")
            == Some(vec![Command::FitView.id().to_owned()])
    });
    let settings = caditor_file::Settings::load(&dir.path().join("config"));
    let restarted = Workspace::with_preferences(Preferences::from_settings(settings));

    assert_eq!(recent, [Command::FitView]);
    assert_eq!(after_failure, recent);
    assert_eq!(restarted.palette.recent(), recent.as_slice());
}

#[test]
fn resetting_a_shortcut_asks_before_taking_its_keys_back_from_another_command() {
    let mut harness = Harness::new();
    let f = KeyboardShortcut::new(Modifiers::NONE, Key::F);
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Bind(Command::Undo, f),
    )));
    harness.perform(Action::Preferences(PreferencesCommand::ShowShortcuts));
    let reset_fit = "Reset Fit view to the shortcut caditor starts with";

    harness.click(shortcut_editor::CHANGED_ONLY);
    let only_changed = !harness.shows("Save") && harness.shows("Fit view");
    let changed = harness.count_shown(shortcut_editor::CHANGED);

    harness.click_button(reset_fit);
    let asked = harness.shows("F is used by Undo. Reset Fit view and move it there?");
    let untouched = harness
        .workspace
        .preferences
        .keymap
        .shortcuts(Command::Undo);
    harness.click("Keep it where it is");
    let kept = harness
        .workspace
        .preferences
        .keymap
        .shortcuts(Command::Undo);
    harness.click_button(reset_fit);
    harness.click("Reset and move it");
    let keymap = harness.workspace.preferences.keymap.clone();

    assert_eq!(changed, 2);
    assert!(only_changed);
    assert!(asked);
    assert!(untouched.contains(&f));
    assert_eq!(kept, untouched);
    assert!(keymap.is_default(Command::FitView));
    assert!(!keymap.shortcuts(Command::Undo).contains(&f));
}

#[test]
fn the_shortcut_filter_matches_the_starts_of_words() {
    let mut harness = Harness::new();
    harness.perform(Action::Preferences(PreferencesCommand::ShowShortcuts));

    harness.type_text("fit vi");
    let by_word_starts = harness.shows("Fit view");
    harness.replace_text("it vie");
    let inside_words = harness.shows("Fit view");

    assert!(by_word_starts);
    assert!(!inside_words);
}

#[test]
fn the_status_bar_buttons_name_their_keys() {
    let mut harness = Harness::new();
    harness.model.set_status(RecomputeStatus::Cancelled);
    harness.frame();

    harness.hover("Recompute");

    assert!(harness.shows("Bring the outdated features up to date (F5)"));
}
