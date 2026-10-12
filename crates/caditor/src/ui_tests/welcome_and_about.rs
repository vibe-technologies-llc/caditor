use caditor_geometry::Point2;
use egui::{Key, Modifiers, accesskit::Role};
use tempfile::TempDir;

use super::{
    FILE_TIMEOUT, Harness, assert_readable, logo_loaded, offer, run_from_palette, sample_document,
};
use crate::{
    about,
    commands::Command,
    editing::EditingCommand,
    files::FileCommand,
    model::Action,
    onboarding::Hint,
    preferences::{InputMode, PreferenceChange, PreferencesCommand},
};

#[test]
fn a_first_run_welcomes_opens_a_sample_and_offers_tips_until_they_are_hidden() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::first_run(dir.path());
    harness.frame();
    assert!(harness.shows("Welcome to caditor"));
    assert!(!harness.shows("Tip"));
    harness.click("Flanged spool");
    harness.settle();
    assert!(!harness.shows("Welcome to caditor"));
    assert!(
        harness
            .document()
            .features()
            .any(|feature| feature.name == "Spool")
    );
    assert!(!harness.model.is_dirty());
    assert_eq!(harness.model.undo_label(), None);
    assert!(
        harness.shows(
            &Hint::Navigate.text(&harness.workspace.preferences.keymap, InputMode::default())
        )
    );

    harness.click("Got it");
    assert!(
        harness.shows(
            &Hint::Palette.text(&harness.workspace.preferences.keymap, InputMode::default())
        )
    );
    harness.click("Hide tips");
    assert!(!harness.shows("Tip"));
    let config = dir.path().join("config");
    harness.wait_until("the tips are saved", |_| {
        let settings = caditor_file::Settings::load(&config);
        settings.flag("onboarding.hints") == Some(false)
            && settings.flag("onboarding.welcomed") == Some(true)
            && settings.texts("onboarding.dismissed_hints") == Some(vec!["navigate".to_owned()])
    });

    harness.command(FileCommand::New);
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::RestoreHints,
    )));
    harness.settle();
    let keymap = harness.workspace.preferences.keymap.clone();
    assert!(harness.shows(&Hint::Start.text(&keymap, InputMode::default())));
    harness.draw_on_new_sketch();
    assert!(harness.shows(&Hint::Draw.text(&keymap, InputMode::default())));
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 30.0));
    harness.settle();
    assert!(harness.shows(&Hint::Constrain.text(&keymap, InputMode::default())));
    harness.perform(Action::Editing(EditingCommand::Finish));
    harness.settle();
    assert!(harness.shows(&Hint::Sweep.text(&keymap, InputMode::default())));
    harness.click("Help");
    harness.click("Welcome and samples…");
    assert!(harness.shows("Welcome to caditor"));
    harness.click("Start with an empty model");
    assert!(!harness.shows("Welcome to caditor"));
    harness.click("Continue without saving");
    harness.settle();
    assert_eq!(harness.document().features().len(), 0);
    assert!(!harness.model.is_dirty());
}

#[test]
fn the_welcome_lists_recent_files_opens_one_and_clears_them() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("kept.caditor");
    caditor_file::save(&sample_document().unwrap(), &path, false).unwrap();
    let mut earlier = Harness::with_directories(Some(dir.path()));
    earlier.command(FileCommand::OpenPath(path.clone()));
    earlier.wait_until("the file is open", |harness| harness.model.path().is_some());
    assert!(earlier.files.wait_for_jobs(FILE_TIMEOUT));
    let closing = earlier.model.close().unwrap();
    assert!(closing.wait(FILE_TIMEOUT));
    drop(earlier);

    let mut harness = Harness::first_run(dir.path());
    harness.wait_until("the recent files are read", |harness| {
        !harness.files.recent().is_empty()
    });
    assert!(harness.shows("Welcome to caditor"));
    assert!(harness.shows("Recent files"));
    assert!(harness.shows("kept.caditor"));
    harness.click("kept.caditor");
    harness.wait_until("the recent file opens", |harness| {
        harness.model.path().is_some()
    });
    assert!(!harness.shows("Welcome to caditor"));

    harness.perform(Action::Preferences(PreferencesCommand::ShowWelcome));
    assert!(harness.shows("Recent files"));
    harness.click("Clear recent files");
    assert!(harness.files.recent().is_empty());
    assert!(harness.shows("Welcome to caditor"));
    assert!(!harness.shows("Recent files"));
}

#[test]
fn starting_with_an_empty_model_from_the_welcome_replaces_an_opened_one() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::first_run(dir.path());
    harness.frame();
    harness.click("Flanged spool");
    harness.settle();
    assert!(harness.document().features().len() > 0);

    harness.click("Help");
    harness.click("Welcome and samples…");
    harness.click("Start with an empty model");
    harness.settle();
    assert!(!harness.shows("Welcome to caditor"));
    assert!(!harness.shows("Save changes"));
    assert_eq!(harness.document().features().len(), 0);

    harness.click("Help");
    harness.click("Welcome and samples…");
    harness.click("Start with an empty model");
    harness.settle();
    assert!(!harness.shows("Welcome to caditor"));
    assert_eq!(harness.model.undo_label(), None);
}

#[test]
fn about_shows_the_version_from_the_help_menu_and_the_palette() {
    let mut harness = Harness::new();
    harness.click("Help");
    harness.click("About caditor");
    assert!(harness.workspace.about_open);
    assert!(harness.shows(crate::about::VERSION));
    harness.click("Close");
    assert!(!harness.workspace.about_open);

    run_from_palette(&mut harness, "about");
    assert!(harness.workspace.about_open);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.workspace.about_open);
}

#[test]
fn tips_are_dismissed_and_hidden_from_the_palette() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::first_run(dir.path());
    harness.frame();
    harness.click("Flanged spool");
    harness.settle();
    let keymap = harness.workspace.preferences.keymap.clone();
    assert!(harness.shows(&Hint::Navigate.text(&keymap, InputMode::default())));

    run_from_palette(&mut harness, "dismiss the tip");
    assert!(!harness.shows(&Hint::Navigate.text(&keymap, InputMode::default())));
    assert!(harness.shows(&Hint::Palette.text(&keymap, InputMode::default())));
    run_from_palette(&mut harness, "hide tips");
    assert!(!harness.shows("Tip"));
    assert!(!harness.workspace.preferences.onboarding.hints);
    assert!(offer(&harness, Command::DismissTip).availability.is_err());
}

#[test]
fn the_welcome_dialog_and_tips_are_readable_by_screen_readers() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::first_run(dir.path());
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();
    assert!(harness.shows("Welcome to caditor"));
    assert_readable(&harness, "The welcome dialog");
    assert!(harness.accessible_named(Role::Button, "Close (Esc)"));
    assert!(harness.accessible_named(Role::Button, "Open the Flanged spool sample"));

    harness.click("Flanged spool");
    harness.settle();
    assert!(harness.shows("Got it"));
    assert_readable(&harness, "A tip");
}

#[test]
fn the_about_dialog_shows_the_logo_beside_the_name_and_tagline() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();

    harness.perform(Action::Preferences(PreferencesCommand::ShowAbout));
    harness.show_new_windows();

    assert!(harness.shows("Parametric CAD"));
    assert!(harness.shows(about::VERSION));
    assert!(logo_loaded(&harness, 64));
    assert_readable(&harness, "The About dialog");
}
