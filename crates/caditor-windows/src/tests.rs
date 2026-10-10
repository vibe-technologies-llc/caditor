use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

use tempfile::TempDir;

use crate::{
    FileId, HIDDEN_ATTRIBUTE, boot_id,
    files::{verbatim_for_tests, without_verbatim_for_tests},
    final_path, machine_guid, move_file_durably, process_running, replace_file,
};

#[test]
fn a_replaced_file_takes_the_new_contents_and_keeps_its_attributes() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("model.caditor");
    let replacement = dir.path().join("model.tmp");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .attributes(HIDDEN_ATTRIBUTE)
        .open(&target)
        .unwrap()
        .write_all(b"old")
        .unwrap();
    fs::write(&replacement, "new").unwrap();

    replace_file(&replacement, &target).unwrap();

    let attributes = fs::metadata(&target).unwrap().file_attributes();
    assert_eq!(fs::read_to_string(&target).unwrap(), "new");
    assert_ne!(attributes & HIDDEN_ATTRIBUTE, 0);
    assert!(!replacement.exists());
}

#[test]
fn replacing_a_missing_file_fails_and_leaves_the_replacement() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("missing.caditor");
    let replacement = dir.path().join("model.tmp");
    fs::write(&replacement, "new").unwrap();

    assert!(replace_file(&replacement, &target).is_err());
    assert!(replacement.exists());
    assert!(!target.exists());
}

#[test]
fn a_durable_move_replaces_what_is_there() {
    let dir = TempDir::new().unwrap();
    let from = dir.path().join("from");
    let to = dir.path().join("to");
    fs::write(&from, "moved").unwrap();
    fs::write(&to, "old").unwrap();

    move_file_durably(&from, &to).unwrap();

    assert_eq!(fs::read_to_string(&to).unwrap(), "moved");
    assert!(!from.exists());
}

#[test]
fn a_file_keeps_its_identity_through_a_rename_and_differs_from_others() {
    let dir = TempDir::new().unwrap();
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    fs::write(&first, "a").unwrap();
    fs::write(&second, "b").unwrap();
    let open = fs::File::open(&first).unwrap();
    let renamed = dir.path().join("renamed");

    let before = FileId::of(&open).unwrap();
    fs::rename(&first, &renamed).unwrap();

    assert_eq!(FileId::of_path(&renamed).unwrap(), before);
    assert_ne!(FileId::of_path(&second).unwrap(), before);
    assert!(FileId::of_path(&first).is_err());
}

#[test]
fn this_process_is_running_and_the_idle_process_is_not_ours_to_see() {
    assert!(process_running(std::process::id()));
    assert!(!process_running(0));
}

#[test]
fn the_machine_has_an_identifier_that_stays_the_same() {
    let guid = machine_guid();

    assert!(
        guid.as_ref()
            .is_none_or(|guid| guid.chars().filter(char::is_ascii_hexdigit).count() >= 16)
    );
    assert_eq!(machine_guid(), guid);
    assert_eq!(boot_id(), boot_id());
}

#[test]
fn paths_are_made_verbatim_for_long_names() {
    assert_eq!(
        verbatim_for_tests(Path::new(r"C:\models\a.caditor")).unwrap(),
        r"\\?\C:\models\a.caditor"
    );
    assert_eq!(
        verbatim_for_tests(Path::new(r"\\server\share\a.caditor")).unwrap(),
        r"\\?\UNC\server\share\a.caditor"
    );
    assert_eq!(
        verbatim_for_tests(Path::new(r"\\?\C:\a.caditor")).unwrap(),
        r"\\?\C:\a.caditor"
    );
}

#[test]
fn verbatim_prefixes_are_dropped_from_final_paths() {
    assert_eq!(
        without_verbatim_for_tests(r"\\?\C:\models\a.caditor"),
        r"C:\models\a.caditor"
    );
    assert_eq!(
        without_verbatim_for_tests(r"\\?\UNC\server\share\a.caditor"),
        r"\\server\share\a.caditor"
    );
    assert_eq!(
        without_verbatim_for_tests(r"\\?\Volume{1}\a.caditor"),
        r"\\?\Volume{1}\a.caditor"
    );
}

#[test]
fn the_final_path_of_a_file_is_absolute_without_a_verbatim_prefix_and_in_its_real_case() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("Model.caditor");
    fs::write(&file, "a").unwrap();
    let shouted = dir.path().join("MODEL.CADITOR");

    let resolved = final_path(&shouted).unwrap();

    assert!(resolved.is_absolute());
    assert!(!resolved.to_string_lossy().starts_with(r"\\?\"));
    assert!(resolved.ends_with("Model.caditor"));
    assert!(final_path(&dir.path().join("missing")).is_err());
}
