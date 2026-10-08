use std::os::unix::fs::{MetadataExt, PermissionsExt};

use caditor_document::CancelToken;

use super::*;

#[test]
fn saving_over_a_file_keeps_its_permissions_and_leaves_no_temporary_files() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    save(&Document::default(), &path, false).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

    save(&sample(), &path, false).unwrap();

    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640);
    assert_eq!(files_in(dir.path()), ["model.caditor"]);
    assert_eq!(load(&path).unwrap().document, sample());
}

fn supplementary_group() -> Option<u32> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let primary = fs::metadata("/proc/self").ok()?.gid();
    status
        .lines()
        .find_map(|line| line.strip_prefix("Groups:"))?
        .split_whitespace()
        .filter_map(|group| group.parse().ok())
        .find(|group| *group != primary)
}

#[test]
fn saving_over_a_file_keeps_its_group_and_extended_attributes() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    fs::write(&path, "old").unwrap();
    let group = supplementary_group();
    if let Some(group) = group {
        std::os::unix::fs::chown(&path, None, Some(group)).unwrap();
    }
    let tagged = xattr::set(&path, "user.caditor.note", b"kept").is_ok();

    save(&sample(), &path, false).unwrap();

    if let Some(group) = group {
        assert_eq!(fs::metadata(&path).unwrap().gid(), group);
    }
    if tagged {
        assert_eq!(
            xattr::get(&path, "user.caditor.note").unwrap().as_deref(),
            Some(&b"kept"[..])
        );
    }
    assert_eq!(load(&path).unwrap().document, sample());
}

fn make_fifo(path: &Path) {
    rustix::fs::mknodat(
        rustix::fs::CWD,
        path,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        0,
    )
    .unwrap();
}

#[test]
fn pipes_devices_folders_and_huge_files_are_refused_without_reading_them() {
    let dir = TempDir::new().unwrap();
    let fifo = dir.path().join("pipe.caditor");
    make_fifo(&fifo);
    let huge = dir.path().join("huge.caditor");
    fs::File::create(&huge)
        .unwrap()
        .set_len(read::MAX_FILE_SIZE + 1)
        .unwrap();

    let refusal = |path: &Path| match load(path) {
        Err(LoadError::Unreadable(reason)) => reason.to_string(),
        other => panic!("expected a refusal, got {other:?}"),
    };
    assert_eq!(refusal(&fifo), "it is a device, pipe or socket, not a file");
    assert_eq!(
        refusal(Path::new("/dev/zero")),
        "it is a device, pipe or socket, not a file"
    );
    assert_eq!(refusal(dir.path()), "it is a folder, not a file");
    assert_eq!(refusal(&huge), "it is larger than the 2 GiB caditor reads");
    assert!(crate::history(&fifo).is_err());
    assert!(read_step_file(&fifo, &CancelToken::never()).is_err());
    assert!(read_dxf(&fifo, &CancelToken::never()).is_err());

    let journal = dir.path().join(".pipe.caditor.journal");
    make_fifo(&journal);
    assert!(matches!(journal_for(&fifo, None), FileJournal::None));

    let error = save(&sample(), &fifo, false).unwrap_err();
    assert_eq!(
        error.to_string(),
        "a device, pipe or socket with that name already exists"
    );
}

#[test]
fn restoring_keeps_the_recovered_journal_until_a_new_one_is_written() {
    let dir = TempDir::new().unwrap();
    let base = sample();
    let storage = Storage::spawn(config(&dir), untitled(&base), || {}).unwrap();
    let mut editor = Editor::new(base);
    record_session(&storage, &mut editor);
    crash(storage);
    let recovery = dir.path().join("recovery");
    let recovered = scan(Some(&recovery), &[]).remove(0);

    fs::set_permissions(&recovery, fs::Permissions::from_mode(0o500)).unwrap();
    let start = Start {
        file: None,
        on_disk: None,
        loaded_with_problems: false,
        base: recovered.base.clone(),
        folded: recovered.folded,
        entries: recovered.entries.clone(),
        replaces: Some(recovered.journal.clone()),
        after: None,
    };
    let storage = Storage::spawn(config(&dir), start, || {}).unwrap();
    assert!(matches!(
        wait_for_report(&storage),
        Report::JournalFailed { .. }
    ));
    assert!(recovered.journal.exists());

    fs::set_permissions(&recovery, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(storage.flusher().flush(WAIT));
    assert_eq!(wait_for_report(&storage), Report::JournalRestored);
    assert!(!recovered.journal.exists());
    crash(storage);
    let again = scan(Some(&recovery), &[]);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].editor.document(), editor.document());
}

#[test]
fn saving_through_a_symbolic_link_writes_the_file_it_points_to() {
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("real.caditor");
    let link = dir.path().join("link.caditor");
    save(&Document::default(), &real, false).unwrap();
    std::os::unix::fs::symlink("real.caditor", &link).unwrap();

    save(&sample(), &link, false).unwrap();

    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(load(&real).unwrap().document, sample());
    assert_eq!(crate::history(&real).unwrap().versions.len(), 1);
}

#[test]
fn a_save_that_cannot_read_the_earlier_versions_fails_and_changes_nothing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    save(&Document::default(), &path, false).unwrap();
    let before = fs::read(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o200)).unwrap();

    let error = save(&sample(), &path, false).unwrap_err();

    assert!(
        error.to_string().contains("earlier versions"),
        "{}",
        error.to_string()
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn journals_take_the_permissions_of_their_model() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let base = sample();
    save(&base, &path, false).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let start = Start {
        file: Some(path.clone()),
        on_disk: None,
        ..untitled(&base)
    };
    let storage = Storage::spawn(config(&dir), start, || {}).unwrap();
    assert!(storage.flusher().flush(WAIT));
    let mode = |file: &Path| fs::metadata(file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir.path().join(".model.caditor.journal")), 0o640);
    crash(storage);

    let storage = Storage::spawn(config(&dir), untitled(&base), || {}).unwrap();
    assert!(storage.flusher().flush(WAIT));
    let recovery = dir.path().join("recovery");
    let untitled_journal = fs::read_dir(&recovery)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|journal| journal.to_string_lossy().contains("untitled"))
        .unwrap();
    assert_eq!(mode(&untitled_journal), 0o600);
    crash(storage);
}
