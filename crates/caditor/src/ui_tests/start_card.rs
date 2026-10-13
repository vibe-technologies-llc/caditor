use std::path::Path;

use caditor_document::Document;
use egui::{Key, Modifiers, Rect, accesskit::Role};
use tempfile::TempDir;

use super::{Harness, run_from_palette};
use crate::{
    app::Workspace,
    files::FileCommand,
    model::Action,
    preferences::{PreferenceChange, Preferences, PreferencesCommand, SHOW_START_CARD},
    start_card::{
        CLOSE, DONT_SHOW_AGAIN, START_WITH_BOX, START_WITH_IMPORT, START_WITH_SKETCH, TITLE,
    },
    view_cube,
};

fn empty_model() -> Harness {
    let mut harness = Harness::starting(None, Document::default(), Workspace::new());
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();
    harness
}

fn empty_model_in(dir: &Path) -> Harness {
    let mut preferences =
        Preferences::from_settings(caditor_file::Settings::load(&dir.join("config")));
    preferences.onboarding.welcomed = true;
    preferences.onboarding.hints = false;
    let workspace = Workspace::with_preferences(preferences);
    let mut harness = Harness::starting(Some(dir), Document::default(), workspace);
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();
    harness
}

fn card_parts(harness: &mut Harness) -> Vec<Rect> {
    let texts: Vec<Rect> = harness
        .texts
        .iter()
        .filter(|(text, _)| [TITLE, DONT_SHOW_AGAIN].contains(&text.as_str()))
        .map(|(_, rect)| *rect)
        .collect();
    let buttons: Vec<Rect> = [START_WITH_SKETCH, START_WITH_BOX, START_WITH_IMPORT, CLOSE]
        .into_iter()
        .map(|name| harness.button_rect(name))
        .collect();
    buttons.into_iter().chain(texts).collect()
}

fn set_scale(harness: &mut Harness, scale: f32) {
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(scale),
    )));
    for _ in 0..4 {
        harness.frame();
    }
}

#[test]
fn an_empty_model_shows_the_start_card_with_its_starts_close_and_checkbox() {
    let harness = empty_model();

    assert!(harness.shows(TITLE));
    assert!(harness.shows_containing("Sketch on a plane and extrude it"));
    assert!(harness.accessible_named(Role::Button, START_WITH_SKETCH));
    assert!(harness.accessible_named(Role::Button, START_WITH_BOX));
    assert!(harness.accessible_named(Role::Button, START_WITH_IMPORT));
    assert!(harness.accessible_named(Role::Button, CLOSE));
    assert!(harness.accessible_named(Role::CheckBox, DONT_SHOW_AGAIN));
}

#[test]
fn the_primary_start_opens_a_new_sketch_and_the_card_goes() {
    let mut harness = empty_model();

    harness.click_button(START_WITH_SKETCH);
    harness.frame();
    harness.frame();

    assert!(harness.workspace.editing.is_choosing_plane());
    assert!(!harness.shows(TITLE));
}

#[test]
fn closing_the_card_hides_it_for_this_model_only_and_leaves_the_model_unchanged() {
    let mut harness = empty_model();

    harness.click_button(CLOSE);
    harness.frame();

    assert!(!harness.shows(TITLE));
    assert!(!harness.model.is_dirty());
    assert!(harness.workspace.preferences.onboarding.start_card);

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    assert!(!harness.shows(TITLE));

    harness.command(FileCommand::New);
    harness.settle();

    assert!(harness.shows(TITLE));
}

#[test]
fn show_the_start_card_brings_it_back_for_the_current_model() {
    let mut harness = empty_model();
    harness.click_button(CLOSE);
    harness.frame();

    run_from_palette(&mut harness, "Show the start card");
    harness.frame();
    harness.frame();

    assert!(harness.shows(TITLE));
}

#[test]
fn dont_show_this_again_outlasts_a_restart_and_the_preference_brings_the_card_back() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config");
    let mut harness = empty_model_in(dir.path());
    assert!(harness.shows(TITLE));

    harness.click(DONT_SHOW_AGAIN);
    harness.frame();

    assert!(harness.shows(TITLE));

    harness.click_button(CLOSE);
    harness.frame();

    assert!(!harness.shows(TITLE));
    assert!(!harness.workspace.preferences.onboarding.start_card);
    assert!(!harness.model.is_dirty());
    harness.wait_until("the start card preference is saved", |_| {
        caditor_file::Settings::load(&config).flag("onboarding.start_card") == Some(false)
    });
    drop(harness);

    let mut restarted = empty_model_in(dir.path());

    assert!(!restarted.shows(TITLE));

    run_from_palette(&mut restarted, "Show the start card");
    restarted.frame();
    restarted.frame();

    assert!(restarted.shows(TITLE));
    assert!(!restarted.workspace.preferences.onboarding.start_card);

    restarted.click_button(CLOSE);
    restarted.frame();
    restarted.key(Key::Comma, Modifiers::COMMAND);
    restarted.frame();
    restarted.show_new_windows();
    restarted.click(SHOW_START_CARD);
    restarted.click("Close");
    restarted.frame();

    assert!(restarted.workspace.preferences.onboarding.start_card);
    assert!(restarted.shows(TITLE));
    restarted.wait_until("the start card preference is saved again", |_| {
        caditor_file::Settings::load(&config).flag("onboarding.start_card") == Some(true)
    });
}

#[test]
fn a_start_used_with_dont_show_this_again_ticked_stops_the_card_on_empty_models() {
    let mut harness = empty_model();

    harness.click(DONT_SHOW_AGAIN);
    harness.click_button(START_WITH_SKETCH);
    harness.frame();
    harness.frame();

    assert!(harness.workspace.editing.is_choosing_plane());
    assert!(!harness.workspace.preferences.onboarding.start_card);

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(!harness.workspace.editing.is_choosing_plane());
    assert!(!harness.shows(TITLE));
}

#[test]
fn the_card_fits_in_the_view_clear_of_the_view_cube_at_every_interface_size() {
    for scale in [0.75, 1.0, 1.5, 2.0] {
        let mut harness = empty_model();
        set_scale(&mut harness, scale);

        let view = harness.workspace.viewport.rect().unwrap();
        let cube = view_cube::area(view);
        let parts = card_parts(&mut harness);

        assert_eq!(parts.len(), 6, "at {scale}");
        for part in parts {
            assert!(
                view.contains_rect(part),
                "{part:?} outside {view:?} at {scale}"
            );
            assert!(!part.intersects(cube), "{part:?} over the cube at {scale}");
        }
    }
}
