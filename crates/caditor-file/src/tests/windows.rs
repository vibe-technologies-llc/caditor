use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

use caditor_windows::HIDDEN_ATTRIBUTE;

use super::*;

fn hidden(path: &Path) -> bool {
    fs::metadata(path).unwrap().file_attributes() & HIDDEN_ATTRIBUTE != 0
}

#[test]
fn folders_and_huge_files_are_refused_without_reading_them() {
    let dir = TempDir::new().unwrap();
    let huge = dir.path().join("huge.caditor");
    fs::File::create(&huge)
        .unwrap()
        .set_len(read::MAX_FILE_SIZE + 1)
        .unwrap();

    let refusal = |path: &Path| match load(path) {
        Err(LoadError::Unreadable(reason)) => reason.to_string(),
        other => panic!("expected a refusal, got {other:?}"),
    };
    assert_eq!(refusal(dir.path()), "it is a folder, not a file");
    assert_eq!(refusal(&huge), "it is larger than the 2 GiB caditor reads");
}

#[test]
fn a_journal_beside_its_model_is_hidden_and_never_read_only() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let base = sample();
    save(&base, &path, false).unwrap();
    let mut read_only = fs::metadata(&path).unwrap().permissions();
    read_only.set_readonly(true);
    fs::set_permissions(&path, read_only).unwrap();
    let start = Start {
        file: Some(path.clone()),
        on_disk: None,
        ..untitled(&base)
    };

    let storage = Storage::spawn(config(&dir), start, || {}).unwrap();
    let mut editor = Editor::new(base);
    record_session(&storage, &mut editor);
    assert!(storage.flusher().flush(WAIT));
    assert!(storage.flusher().flush(WAIT));

    let journal = dir.path().join(".model.caditor.journal");
    assert!(hidden(&journal));
    assert!(!fs::metadata(&journal).unwrap().permissions().readonly());
    crash(storage);
    let recovered = scan(Some(&dir.path().join("recovery")), &[path]);
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].editor.document(), editor.document());
}

#[test]
fn saving_over_a_hidden_model_keeps_it_hidden_and_leaves_no_temporary_files() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    save(&Document::default(), &path, false).unwrap();
    let previous = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .attributes(HIDDEN_ATTRIBUTE)
        .open(&path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, &previous))
        .unwrap();

    save(&sample(), &path, false).unwrap();

    assert!(hidden(&path));
    assert_eq!(files_in(dir.path()), ["model.caditor"]);
    assert_eq!(load(&path).unwrap().document, sample());
    assert_eq!(crate::history(&path).unwrap().versions.len(), 1);
}

#[test]
fn a_new_model_is_not_hidden() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");

    save(&sample(), &path, false).unwrap();

    assert!(!hidden(&path));
}

#[test]
fn the_state_and_configuration_folders_are_the_local_and_roaming_application_data() {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let roaming = std::env::var_os("APPDATA").map(PathBuf::from);

    assert_eq!(state_dir(), local.map(|base| base.join("caditor")));
    assert_eq!(config_dir(), roaming.map(|base| base.join("caditor")));
}

#[test]
fn every_spelling_of_a_model_shares_a_fallback_journal_and_one_written_under_the_old_hash_is_found()
{
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("Model.caditor");
    let recovery = dir.path().join("recovery");
    let shouted = dir.path().join("MODEL.CADITOR");
    save(&sample(), &path, false).unwrap();
    crash_journal(&dir, &path, &sample());
    let journal = dir.path().join(".Model.caditor.journal");
    fs::create_dir_all(&recovery).unwrap();
    let legacy = paths::fallback_named(paths::path_hash(&shouted), &recovery);
    fs::rename(&journal, &legacy).unwrap();

    let found = journal_for(&shouted, Some(&recovery));

    assert_eq!(
        paths::fallback_journal(&path, &recovery),
        paths::fallback_journal(&shouted, &recovery)
    );
    assert!(matches!(found, FileJournal::Recoverable(_)), "{found:?}");
}

fn crash_journal(dir: &TempDir, path: &Path, base: &Document) {
    let start = Start {
        file: Some(path.to_path_buf()),
        on_disk: None,
        ..untitled(base)
    };
    let storage = Storage::spawn(config(dir), start, || {}).unwrap();
    let mut editor = Editor::new(base.clone());
    record_session(&storage, &mut editor);
    crash(storage);
}
