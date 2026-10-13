use egui::{Key, Modifiers};
use tempfile::TempDir;

use super::{Harness, canonical, feature_named, run_from_palette, sample_document};
use crate::{
    commands::{Command, Offer, RecentSlot},
    editing::Tool,
    files::FileCommand,
    model::{Action, Notice, RecomputeStatus},
    palette::{Choice, State},
    panels::Focus,
    preferences::{self, PreferenceChange, PreferencesCommand},
    toggles::ToggleStates,
};

#[test]
fn the_command_palette_runs_what_fits_the_context_and_explains_the_rest() {
    let mut harness = Harness::new();
    harness.edit_width("50 mm");
    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.show_new_windows();
    assert!(harness.workspace.palette.is_open());

    harness.type_text("fillet");
    assert!(harness.shows(&format!(
        "Fillet is not available: {}.",
        crate::blend_tools::NOTHING_TO_BLEND
    )));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    harness.frame();
    assert!(!harness.workspace.palette.is_open());
    assert!(
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.starts_with("Fillet is not available"))
    );

    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.show_new_windows();
    harness.type_text("draw line");
    assert!(harness.shows("Draw line"));
    assert!(
        harness.shows("Draw line is not available here: it works only while a sketch is edited.")
    );
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.workspace.palette.is_open());
    assert_eq!(harness.tool(), None);

    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.show_new_windows();
    harness.type_text("zqzqzq");
    assert!(harness.shows("Nothing is called “zqzqzq”. Try another word, or fewer letters."));

    harness.replace_text("undo");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.workspace.palette.is_open());
    assert_eq!(harness.expression_text("width"), "40 mm");

    let base = harness.document().features().next().unwrap().id();
    harness.edit(base);
    harness.click("Search commands");
    assert!(harness.workspace.palette.is_open());
    harness.type_text("draw line");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    assert_eq!(harness.tool(), Some(Tool::Line));
}

#[test]
fn a_shortcut_recorded_in_the_editor_runs_its_command_and_is_remembered() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.edit_width("50 mm");
    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.show_new_windows();
    harness.frame();
    harness.draft_into_field(egui::Id::new(preferences::SEARCH_FIELD), "shortcuts");
    harness.click("Keyboard shortcuts…");
    assert!(harness.shows("Keyboard shortcuts"));
    harness.type_text("undo");
    assert_eq!(harness.count_shown("Redo"), 1);
    harness.click("Keyboard shortcuts");

    harness.click("Add…");
    assert!(harness.shows("Press the keys… (Esc cancels)"));
    harness.key(Key::F9, Modifiers::ALT);
    harness.show_new_windows();
    assert!(
        harness
            .workspace
            .preferences
            .keymap
            .shortcuts(Command::Undo)
            .contains(&egui::KeyboardShortcut::new(Modifiers::ALT, Key::F9))
    );
    assert!(harness.shows("F9"));

    let shown_before = harness.count_shown("F");
    harness.click("Add…");
    harness.key(Key::F, Modifiers::NONE);
    harness.show_new_windows();
    assert!(harness.shows("F is already used by Fit view. Use it for Undo instead?"));
    harness.click("Keep it where it is");
    assert_eq!(harness.count_shown("F"), shown_before);

    harness.click("Add…");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    assert!(harness.shows(
        "Esc, Enter and Tab keep their meaning everywhere (back out, confirm, move between \
         fields), so they cannot be shortcuts."
    ));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    assert!(harness.workspace.shortcut_editor.is_none());
    assert!(harness.workspace.preferences_open);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.workspace.preferences_open);

    harness.key(Key::F9, Modifiers::ALT);
    harness.frame();
    assert_eq!(harness.expression_text("width"), "40 mm");
    harness.wait_until("the shortcut is saved", |_| {
        caditor_file::Settings::load(&dir.path().join("config")).texts("keys.edit.undo")
            == Some(vec!["Ctrl+Z".to_owned(), "Alt+F9".to_owned()])
    });
    harness.hover("Redo");
    assert!(harness.shows("Redo Edit width (Ctrl+Shift+Z)"));
}

#[test]
fn resetting_all_shortcuts_asks_first_and_can_be_undone() {
    let mut harness = Harness::new();
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Bind(
            Command::Undo,
            egui::KeyboardShortcut::new(Modifiers::ALT, Key::F9),
        ),
    )));
    harness.perform(Action::Preferences(PreferencesCommand::ShowShortcuts));
    let bound = harness.workspace.preferences.keymap.clone();
    assert!(!bound.is_all_default());

    harness.click("Reset all shortcuts");
    assert!(harness.shows("Reset all shortcuts?"));
    assert_eq!(harness.workspace.preferences.keymap, bound);
    harness.click("Cancel");
    assert!(!harness.shows("Reset all shortcuts?"));
    assert_eq!(harness.workspace.preferences.keymap, bound);

    harness.click("Reset all shortcuts");
    harness.click("Reset all shortcuts");
    assert!(harness.workspace.preferences.keymap.is_all_default());
    assert!(harness.shows("Every shortcut is back to the one caditor starts with."));
    harness.click_lowest("Undo");
    assert_eq!(harness.workspace.preferences.keymap, bound);
    assert!(!harness.shows("Every shortcut is back to the one caditor starts with."));
    assert!(harness.workspace.shortcut_editor.is_some());
}

#[test]
fn the_palette_finds_features_and_parameters_and_goes_to_them() {
    let mut harness = Harness::new();
    let side = feature_named(&harness, "Side sketch");
    let height = harness.parameter("height");

    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.show_new_windows();
    harness.type_text("side sk");
    assert!(harness.shows("Features"));
    assert!(harness.shows("Press Enter to select it in the feature tree."));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.workspace.palette.is_open());
    assert_eq!(harness.workspace.panels.selected, Some(side));

    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.show_new_windows();
    harness.type_text("heig");
    assert!(harness.shows("Parameters"));
    assert!(harness.shows("width / 2"));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.workspace.palette.is_open());
    assert_eq!(
        harness.focused(),
        Some(Focus::ParameterValue(height).field_id())
    );
}

#[test]
fn the_palette_lists_what_does_not_fit_the_context_last_with_the_reason() {
    let mut harness = Harness::new();
    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.show_new_windows();
    harness.type_text("trim");
    assert!(harness.shows("Trim sketch curves"));
    let entries = harness.workspace.palette.entries(
        &harness.workspace.last_offers,
        &harness.workspace.preferences.keymap,
        harness.model.document(),
        &ToggleStates::of(&harness.workspace.viewport, false),
    );
    let trim = entries
        .iter()
        .position(|entry| entry.choice == Choice::Command(Command::SketchTool(Tool::Trim)))
        .unwrap();
    let last_ready = entries
        .iter()
        .rposition(|entry| entry.state == State::Ready)
        .unwrap_or_default();
    assert!(trim > last_ready, "{entries:#?}");
    let refusal =
        "Trim sketch curves is not available here: it works only while a sketch is edited.";
    for _ in 0..entries.len() {
        if harness.shows(refusal) {
            break;
        }
        harness.key(Key::ArrowDown, Modifiers::NONE);
        harness.frame();
    }
    assert!(harness.shows(refusal));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.workspace.palette.is_open());
    assert_eq!(harness.tool(), None);
}

#[test]
fn recent_models_notices_and_recompute_are_commands() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let path = root.as_path().join("kept.caditor");
    caditor_file::save(&sample_document().unwrap(), &path, false).unwrap();
    let mut harness = Harness::with_directories(Some(root.as_path()));
    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the file is open", |harness| harness.model.path().is_some());
    harness.command(FileCommand::New);
    harness.settle();
    assert_eq!(harness.model.path(), None);

    let recent = harness
        .workspace
        .last_offers
        .iter()
        .find(|offer| offer.command == Command::OpenRecent(RecentSlot::ALL[0]))
        .map(Offer::title);
    assert_eq!(
        recent.as_deref(),
        Some("Open the most recent model: kept.caditor")
    );
    run_from_palette(&mut harness, "kept");
    harness.wait_until("the recent model is open", |harness| {
        harness.model.path() == Some(path.as_path())
    });

    harness.perform(Action::Inform(Notice::info("Something to read.")));
    harness.frame();
    assert!(harness.model.notice().is_some());
    run_from_palette(&mut harness, "dismiss the notice");
    harness.frame();
    assert!(harness.model.notice().is_none());

    run_from_palette(&mut harness, "recompute the model");
    harness.settle();
    assert_eq!(harness.model.status(), RecomputeStatus::UpToDate);
}
