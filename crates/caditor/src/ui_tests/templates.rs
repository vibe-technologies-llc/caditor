use std::{fs, path::Path};

use tempfile::TempDir;

use super::{Harness, run_from_palette, sample_document};
use crate::{
    files::FileCommand,
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand},
};

fn has_width(harness: &Harness) -> bool {
    harness.model.document().parameter_named("width").is_some()
}

fn notice_says(harness: &Harness, text: &str) -> bool {
    harness
        .model
        .notice()
        .is_some_and(|notice| notice.text.contains(text))
}

fn templates_folder(dir: &Path) -> std::path::PathBuf {
    dir.join("config").join("templates")
}

#[test]
fn a_model_saved_as_a_template_starts_untitled_copies() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let template = templates_folder(dir.path()).join("Bracket.caditor");

    harness.answer_dialog(Some(templates_folder(dir.path()).join("Bracket")));
    run_from_palette(&mut harness, "Save as template");
    harness.wait_until("the template is saved", |harness| {
        notice_says(harness, "as a template")
    });

    assert!(template.is_file());
    assert_eq!(
        harness.files.templates().listed(),
        std::slice::from_ref(&template)
    );
    assert_eq!(harness.model.path(), None);

    harness.command(FileCommand::NewEmpty);

    assert!(!has_width(&harness));

    harness.answer_dialog(Some(template.clone()));
    run_from_palette(&mut harness, "New from template");
    harness.wait_until("the template is copied", has_width);
    let saved = fs::read(&template).unwrap();

    assert_eq!(harness.model.path(), None);
    assert!(!harness.model.is_dirty());

    harness.edit_width("55 mm");

    assert_eq!(fs::read(&template).unwrap(), saved);
}

#[test]
fn new_model_starts_from_the_default_template_until_it_is_gone() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let folder = templates_folder(dir.path());
    let template = folder.join("Base.caditor");
    fs::create_dir_all(&folder).unwrap();
    caditor_file::save(&sample_document().unwrap(), &template, false).unwrap();

    harness.command(FileCommand::NewEmpty);
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::DefaultTemplate(Some("Base.caditor".to_owned())),
    )));
    harness.command(FileCommand::New);
    harness.wait_until("the default template is copied", has_width);

    assert_eq!(harness.model.path(), None);

    fs::remove_file(&template).unwrap();
    harness.command(FileCommand::New);
    harness.wait_until("the missing template is reported", |harness| {
        notice_says(harness, "Could not start from the template “Base.caditor”")
    });

    assert!(harness.model.is_empty_and_untitled());
    assert!(notice_says(&harness, "Preferences"));
}

#[test]
fn an_unreadable_template_leaves_an_empty_model_and_says_why() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let folder = templates_folder(dir.path());
    let template = folder.join("Broken.caditor");
    fs::create_dir_all(&folder).unwrap();
    fs::write(&template, b"not a model").unwrap();

    harness.command(FileCommand::NewFromTemplate(Some(template)));
    harness.wait_until("the unreadable template is reported", |harness| {
        notice_says(harness, "A new empty model was started instead")
    });

    assert!(harness.model.is_empty_and_untitled());
}

#[test]
fn preferences_choose_the_template_new_model_starts_from() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let folder = templates_folder(dir.path());
    fs::create_dir_all(&folder).unwrap();
    caditor_file::save(
        &sample_document().unwrap(),
        &folder.join("Base.caditor"),
        false,
    )
    .unwrap();

    harness.perform(Action::Preferences(PreferencesCommand::Show));
    harness.wait_until("the templates are listed", |harness| {
        !harness.files.templates().listed().is_empty()
    });
    harness.click("An empty model");
    harness.click("Base");

    assert_eq!(
        harness.workspace.preferences.default_template.as_deref(),
        Some("Base.caditor")
    );
    assert!(harness.shows("Base"));
}
