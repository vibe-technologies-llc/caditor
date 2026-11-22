use std::{
    fs,
    path::{Path, PathBuf},
};

use caditor_document::Document;
use egui::{Event, Key, Modifiers};
use tempfile::TempDir;

use super::{FILE_TIMEOUT, Harness, canonical};
use crate::{
    commands::Command,
    files::on_disk::{DISMISS, RELOAD},
};

const CHANGED: &str = "“plate.caditor” was changed by another program";
const GONE: &str = "“plate.caditor” is no longer on disk";
const RELOAD_PROMPT: &str = "Reload “plate.caditor” and discard your changes?";

fn saved_model(dir: &TempDir) -> (Harness, PathBuf) {
    let path = canonical(dir).join("plate.caditor");
    let mut harness = Harness::with_directories(Some(dir.path()));

    harness.answer_dialog(Some(path.clone()));
    harness.key(Key::S, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    (harness, path)
}

fn saved_elsewhere(harness: &Harness, path: &Path) -> Document {
    let mut outside = harness.model.document().clone();

    let mut transaction = outside.transaction("Elsewhere");

    transaction.add_parameter("elsewhere", transaction.parse("3 mm").unwrap());
    outside.apply(transaction.finish()).unwrap();
    caditor_file::save(&outside, path, false).unwrap();
    outside
}

fn regain_focus(harness: &mut Harness) {
    harness.events.push(Event::WindowFocused(true));
    harness.frame();

    assert!(harness.files.wait_for_jobs(FILE_TIMEOUT));

    harness.show_new_windows();
}

fn wait_for_reload(harness: &mut Harness, path: &Path, outside: &Document) {
    harness.wait_until("the file is reloaded", |harness| {
        !harness.files.is_opening()
            && harness.model.path() == Some(path)
            && harness.model.document() == outside
    });
}

#[test]
fn an_unchanged_file_raises_no_notice_when_the_window_regains_focus() {
    let dir = TempDir::new().unwrap();
    let (mut harness, _) = saved_model(&dir);

    harness.edit_width("41 mm");
    regain_focus(&mut harness);

    assert_eq!(harness.files.outside_change(), None);
    assert!(!harness.shows_containing(CHANGED));
    assert!(harness.model.is_dirty());
}

#[test]
fn a_file_changed_outside_without_edits_here_reloads_in_place() {
    let dir = TempDir::new().unwrap();
    let (mut harness, path) = saved_model(&dir);

    let outside = saved_elsewhere(&harness, &path);

    regain_focus(&mut harness);

    assert!(harness.shows_containing(CHANGED));
    assert!(!harness.files.is_blocking());

    harness.click_button(RELOAD);
    wait_for_reload(&mut harness, &path, &outside);

    assert!(!harness.model.is_dirty());
    assert!(!harness.shows_containing(CHANGED));
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Reloaded “plate.caditor” as another program saved it."
    );

    harness.edit_width("43 mm");
    harness.key(Key::S, Modifiers::COMMAND);
    harness.wait_until("the edit is saved over the reloaded file", |harness| {
        !harness.model.is_dirty()
    });

    assert!(!harness.shows(CHANGED));
    assert_eq!(
        caditor_file::load(&path).unwrap().document,
        *harness.model.document()
    );
}

#[test]
fn reloading_over_unsaved_edits_asks_and_save_a_copy_keeps_them_before_reloading() {
    let dir = TempDir::new().unwrap();
    let (mut harness, path) = saved_model(&dir);

    let outside = saved_elsewhere(&harness, &path);

    harness.edit_width("41 mm");

    let edited = harness.model.document().clone();

    regain_focus(&mut harness);
    harness.click_button(RELOAD);

    assert!(harness.shows(RELOAD_PROMPT));

    harness.click("Cancel");

    assert!(!harness.shows(RELOAD_PROMPT));
    assert!(harness.model.is_dirty());
    assert_eq!(*harness.model.document(), edited);
    assert!(harness.shows_containing(CHANGED));

    let copy = canonical(&dir).join("plate copy.caditor");

    harness.answer_dialog(Some(copy.clone()));
    harness.click_button(RELOAD);
    harness.click_button("Save a copy…");
    wait_for_reload(&mut harness, &path, &outside);

    assert!(!harness.model.is_dirty());
    assert_eq!(caditor_file::load(&copy).unwrap().document, edited);
    assert_eq!(caditor_file::load(&path).unwrap().document, outside);
}

#[test]
fn reload_and_discard_replaces_unsaved_edits_with_the_file_on_disk() {
    let dir = TempDir::new().unwrap();
    let (mut harness, path) = saved_model(&dir);

    let outside = saved_elsewhere(&harness, &path);

    harness.edit_width("41 mm");
    regain_focus(&mut harness);
    harness.click_button(RELOAD);
    harness.click_button("Reload and discard");
    wait_for_reload(&mut harness, &path, &outside);

    assert!(!harness.model.is_dirty());
    assert!(!harness.shows_containing(CHANGED));
}

#[test]
fn a_dismissed_change_is_not_told_again_until_the_file_changes_again() {
    let dir = TempDir::new().unwrap();
    let (mut harness, path) = saved_model(&dir);

    let first = saved_elsewhere(&harness, &path);

    regain_focus(&mut harness);
    harness.click_button(DISMISS);

    assert!(!harness.shows_containing(CHANGED));

    regain_focus(&mut harness);

    assert!(!harness.shows_containing(CHANGED));
    assert_ne!(*harness.model.document(), first);

    let mut again = first.clone();

    let mut transaction = again.transaction("Elsewhere again");

    transaction.add_parameter("again", transaction.parse("4 mm").unwrap());
    again.apply(transaction.finish()).unwrap();
    caditor_file::save(&again, &path, false).unwrap();
    regain_focus(&mut harness);

    assert!(harness.shows_containing(CHANGED));

    harness
        .workspace
        .queue_command(Command::DismissOutsideChange);
    harness.show_new_windows();

    assert!(!harness.shows_containing(CHANGED));

    harness.edit_width("41 mm");
    harness.key(Key::S, Modifiers::COMMAND);
    harness.wait_until("the save meets the changed-on-disk guard", |harness| {
        harness.shows(CHANGED)
    });
}

#[test]
fn a_deleted_file_is_told_without_reload_and_saving_writes_it_again() {
    let dir = TempDir::new().unwrap();
    let (mut harness, path) = saved_model(&dir);

    fs::remove_file(&path).unwrap();
    regain_focus(&mut harness);

    assert!(harness.shows_containing(GONE));
    assert!(harness.find_button_rect(DISMISS).is_some());
    assert!(harness.find_button_rect(RELOAD).is_none());

    harness.edit_width("41 mm");
    harness.key(Key::S, Modifiers::COMMAND);
    harness.wait_until("the model is written again", |harness| {
        !harness.model.is_dirty()
    });

    assert!(!harness.shows_containing(GONE));
    assert_eq!(
        caditor_file::load(&path).unwrap().document,
        *harness.model.document()
    );
}
