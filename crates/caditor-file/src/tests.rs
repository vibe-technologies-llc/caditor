use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use caditor_document::{
    Document, Edit, Editor, FeatureId, FeatureKind, RollbackBar, SketchAttachment, Transaction,
};
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, Sketch};
use tempfile::TempDir;

use super::{
    binary::testing::{
        corrupt_chunk, current_model_from_json, model_from_json, records_as_json, rewrite_journal,
        sharing_from, with_unknown_codec,
    },
    save, *,
};

const WAIT: Duration = Duration::from_secs(10);

fn dimensioned_line(plane: Plane, length: f64, value: Expression) -> Sketch {
    let mut sketch = Sketch::new(plane);
    let line = sketch.add_line(Point2::ZERO, Point2::new(length, 0.0));
    let Some(Entity::Line { start, end }) = sketch.entity(line).cloned() else {
        panic!("expected a line");
    };
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    sketch
        .add_constraint(Constraint::Distance {
            from: start,
            to: end,
            value,
        })
        .unwrap();
    sketch
}

fn sample() -> Document {
    let mut document = Document::default();
    let mut transaction = document.transaction("Sample");
    let width = transaction.add_parameter("width", transaction.parse("40 mm").unwrap());
    transaction.add_parameter("height", transaction.parse("width / 2 + 0.1 mm").unwrap());
    transaction.add_parameter("scrap", Expression::Number(1.0));
    let tilted = Plane::with_x_axis(
        Point3::new(1.5, -2.25, 1e-9),
        Vector3::new(1.0, 2.0, 3.0),
        Vector3::X,
    )
    .unwrap();
    transaction.add_feature(
        "Base sketch",
        FeatureKind::from(dimensioned_line(
            Plane::XY,
            40.0,
            Expression::Parameter(width),
        )),
    );
    let side = dimensioned_line(tilted, 1.0 / 3.0, transaction.parse("height * 2").unwrap());
    transaction.add_feature("Side sketch", FeatureKind::from(side));
    let scrap_feature = transaction.add_feature("Scrap", FeatureKind::from(Sketch::new(Plane::YZ)));
    document.apply(transaction.finish()).unwrap();

    let scrap = document.parameter_named("scrap").unwrap().id();
    document
        .apply(Transaction::new(
            "Remove scraps",
            vec![
                Edit::RemoveParameter { id: scrap },
                Edit::RemoveFeature { id: scrap_feature },
            ],
        ))
        .unwrap();
    document
}

fn edit_width(document: &Document, text: &str) -> Transaction {
    Transaction::single(
        "Edit width",
        Edit::SetParameterExpression {
            id: document.parameter_named("width").unwrap().id(),
            expression: document.parse(text).unwrap(),
        },
    )
}

fn lines_of(document: &Document) -> Vec<String> {
    records_as_json(&crate::encode(document).unwrap())
}

fn encode(document: &Document) -> Result<String, SaveError> {
    Ok(lines_of(document).join("\n"))
}

fn decode_lines(lines: &[String]) -> Loaded {
    decode(&current_model_from_json(lines)).unwrap()
}

fn decode_text(text: &str) -> Loaded {
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    decode_lines(&lines)
}

fn through_binary<T: serde::de::DeserializeOwned>(json: &str) -> T {
    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    binary::value::from_bytes(&binary::value::to_bytes(&value).unwrap()).unwrap()
}

fn issues_mention(loaded: &Loaded, text: &str) -> bool {
    loaded.issues.iter().any(|issue| issue.contains(text))
}

fn wait_for_report(storage: &Storage) -> Report {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(report) = storage.poll().unwrap().into_iter().next() {
            return report;
        }
        assert!(
            Instant::now() < deadline,
            "the storage worker never reported"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn config(dir: &TempDir) -> StorageConfig {
    StorageConfig {
        recovery_dir: Some(dir.path().join("recovery")),
    }
}

fn untitled(base: &Document) -> Start {
    Start {
        file: None,
        on_disk: None,
        loaded_with_problems: false,
        base: base.clone(),
        entries: Vec::new(),
        replaces: None,
        after: None,
    }
}

fn crash(storage: Storage) {
    assert!(storage.close(false).wait(WAIT));
}

fn record_session(storage: &Storage, editor: &mut Editor) {
    let first = edit_width(editor.document(), "50 mm");
    editor.apply(first.clone()).unwrap();
    storage.record(JournalEntry::Apply(first)).unwrap();
    let second = edit_width(editor.document(), "60 mm");
    editor.apply(second.clone()).unwrap();
    storage.record(JournalEntry::Apply(second)).unwrap();
    let undo = editor.next_undo().cloned().unwrap();
    editor.undo().unwrap();
    storage.record(JournalEntry::Undo(undo)).unwrap();
    assert!(storage.flusher().flush(WAIT));
}

#[test]
fn a_saved_model_loads_back_exactly() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let document = sample();

    let saved = save(&document, &path, false).unwrap();
    assert_eq!((saved.backup, saved.dropped_for_size), (None, 0));
    let loaded = load(&path).unwrap();

    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(loaded.document.next_parameter_id(), 3);
    assert_eq!(loaded.document.next_feature_id(), 3);
    let bytes = fs::read(&path).unwrap();
    assert!(bytes.starts_with(&binary::MODEL_MAGIC));
    let records = records_as_json(&bytes);
    assert!(records[1].contains("\"expression\":\"$0 / 2 + 0.1 mm\""));
    assert_eq!(records.len(), 2 + 2 + 1);
    assert_eq!(files_in(dir.path()), ["model.caditor"]);
}

#[test]
fn saves_sharing_earlier_versions_with_the_file_they_replace_keep_every_version() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let copy = dir.path().join("copy.caditor");
    let mut editor = Editor::new(sample());
    let mut bulk = editor.document().transaction("Bulk");
    for index in 0..400_u64 {
        let text = format!("{} mm", index.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 40);
        bulk.add_parameter(format!("extra{index}"), bulk.parse(&text).unwrap());
    }
    editor.apply(bulk.finish()).unwrap();
    let mut saved = Vec::new();

    sharing_from(1, || {
        for step in 0..10 {
            let edit = edit_width(editor.document(), &format!("{} mm", 40 + step));
            editor.apply(edit).unwrap();
            save(editor.document(), &path, false).unwrap();
            saved.push(editor.document().clone());
        }
        let options = SaveOptions {
            history_from: Some(&path),
            ..SaveOptions::default()
        };
        editor
            .apply(edit_width(editor.document(), "99 mm"))
            .unwrap();
        save_with(editor.document(), &copy, &options).unwrap();
        saved.push(editor.document().clone());
    });

    for (file, versions) in [(&path, 9), (&copy, 10)] {
        let listed = crate::history(file).unwrap();
        assert_eq!(listed.versions.len(), versions);
        for version in &listed.versions {
            assert!(version.available);
            let loaded = load_version(file, version.index).unwrap();
            assert_eq!(loaded.document, saved[versions - 1 - version.index]);
        }
        assert_eq!(load(file).unwrap().document, saved[versions]);
        let bytes = fs::read(file).unwrap();
        let container = binary::parse(&bytes, &binary::MODEL_MAGIC).unwrap();
        assert!(
            container
                .chunks()
                .any(|chunk| chunk.kind == Some(binary::ChunkKind::Padding))
        );
    }
    assert_eq!(files_in(dir.path()), ["copy.caditor", "model.caditor"]);
}

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

#[test]
fn a_failed_save_reports_a_plain_reason_and_leaves_the_folder_clean() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("missing").join("model.caditor");
    let error = save(&sample(), &path, false).unwrap_err();
    assert_eq!(error.to_string(), "its folder no longer exists");
    assert_eq!(files_in(dir.path()), Vec::<String>::new());
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
        Err(LoadError::Unreadable(reason)) => reason,
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
    assert!(read_step_file(&fifo).is_err());
    assert!(read_dxf(&fifo).is_err());

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
fn overwriting_a_damaged_file_keeps_the_original_as_a_backup() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    fs::write(&path, "damaged original").unwrap();
    fs::write(dir.path().join("model.damaged.caditor"), "earlier backup").unwrap();

    let backup = save(&sample(), &path, true).unwrap().backup.unwrap();

    assert_eq!(backup, dir.path().join("model.damaged-2.caditor"));
    assert_eq!(fs::read_to_string(&backup).unwrap(), "damaged original");
    assert_eq!(load(&path).unwrap().document, sample());
}

#[test]
fn a_record_stored_in_an_unknown_way_reads_like_something_from_a_newer_version() {
    let bytes = with_unknown_codec(&crate::encode(&sample()).unwrap(), 1);

    let loaded = decode(&bytes).unwrap();

    assert!(loaded.issues.contains(
        &"Record 2 is stored in a way this version of caditor does not know, so it was left out. \
          It may come from a newer version."
            .to_owned()
    ));
    assert!(
        !loaded
            .issues
            .iter()
            .any(|issue| issue.contains("ends early"))
    );
}

fn saved_twice_then_damaged(path: &Path) -> Vec<u8> {
    save(&Document::default(), path, false).unwrap();
    save(&sample(), path, false).unwrap();
    let damaged = corrupt_chunk(&fs::read(path).unwrap(), &binary::MODEL_MAGIC, 2);
    fs::write(path, &damaged).unwrap();
    damaged
}

#[test]
fn saving_over_a_file_that_went_bad_since_it_was_loaded_keeps_a_backup() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let damaged = saved_twice_then_damaged(&path);

    let backup = save(&sample(), &path, false).unwrap().backup.unwrap();

    assert_eq!(backup, dir.path().join("model.damaged.caditor"));
    assert_eq!(fs::read(&backup).unwrap(), damaged);
    assert!(load(&path).unwrap().issues.is_empty());
    assert_eq!(load(&path).unwrap().document, sample());
}

#[test]
fn a_file_that_is_no_longer_a_model_is_kept_when_saved_over() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    save(&sample(), &path, false).unwrap();
    fs::write(&path, "the sync client wrote this").unwrap();

    let backup = save(&sample(), &path, false).unwrap().backup.unwrap();

    assert_eq!(
        fs::read_to_string(backup).unwrap(),
        "the sync client wrote this"
    );
}

#[test]
fn intact_and_empty_files_get_no_backup_and_a_save_as_leaves_the_damaged_source_alone() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let copy = dir.path().join("copy.caditor");
    save(&sample(), &path, false).unwrap();
    assert_eq!(save(&sample(), &path, false).unwrap().backup, None);
    fs::write(&copy, "").unwrap();
    assert_eq!(save(&sample(), &copy, false).unwrap().backup, None);

    let damaged = saved_twice_then_damaged(&path);
    let options = SaveOptions {
        history_from: Some(&path),
        ..SaveOptions::default()
    };
    let other = dir.path().join("other.caditor");
    assert_eq!(save_with(&sample(), &other, &options).unwrap().backup, None);

    assert_eq!(fs::read(&path).unwrap(), damaged);
    assert_eq!(
        files_in(dir.path()),
        ["copy.caditor", "model.caditor", "other.caditor"]
    );
}

#[test]
fn a_damaged_line_loses_only_that_record() {
    let document = sample();
    let mut lines = lines_of(&document);
    let side = lines
        .iter_mut()
        .find(|line| line.contains("Side sketch"))
        .unwrap();
    side.truncate(side.len() / 2);

    let loaded = decode_lines(&lines);

    assert_eq!(loaded.issues, ["Record 4 is damaged and was left out."]);
    let names: Vec<&str> = loaded
        .document
        .features()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, ["Base sketch"]);
    assert_eq!(loaded.document.parameters(), document.parameters());
}

#[test]
fn a_lost_parameter_is_replaced_by_a_stand_in_that_keeps_its_name_when_known() {
    let document = sample();
    let mut lines = lines_of(&document);
    lines[0] = lines[0].replace("\"expression\":\"40 mm\"", "\"expression\":40");
    let loaded = decode_lines(&lines);

    assert!(issues_mention(
        &loaded,
        "The parameter “width” is damaged and was left out."
    ));
    assert!(issues_mention(&loaded, "It was replaced by “width” = 0"));
    let width = loaded.document.parameter_named("width").unwrap();
    assert_eq!(width.expression, Expression::Number(0.0));
    assert_eq!(loaded.document.features().len(), 2);

    lines.remove(0);
    let loaded = decode_lines(&lines);
    assert!(issues_mention(&loaded, "It was replaced by “lost_0” = 0"));
    let height = loaded.document.parameter_named("height").unwrap();
    assert_eq!(
        loaded.document.expression_text(&height.expression),
        "lost_0 / 2 + 0.1 mm"
    );
}

#[test]
fn stored_id_counters_and_ids_beyond_the_storable_range_are_clamped_or_left_out() {
    let document = sample();
    let mut lines = lines_of(&document);
    let next_ids = lines
        .iter_mut()
        .find(|line| line.contains("\"next_ids\""))
        .unwrap();
    *next_ids = format!(
        r#"{{"next_ids":{{"parameter":{max},"feature":{max}}}}}"#,
        max = u64::MAX
    );
    lines.insert(
        0,
        format!(
            r#"{{"parameter":{{"expression":"1 mm","id":{},"name":"far"}}}}"#,
            u64::MAX
        ),
    );

    let loaded = decode_lines(&lines);

    assert!(issues_mention(
        &loaded,
        &format!("Its ID {} is beyond the range caditor stores", u64::MAX)
    ));
    assert!(loaded.document.parameter_named("far").is_none());
    assert_eq!(loaded.document.parameters().len(), 2);
    let limit = caditor_document::FIRST_UNSTORABLE_ID;
    assert_eq!(loaded.document.next_parameter_id(), limit);
    assert_eq!(loaded.document.next_feature_id(), limit);
}

#[test]
fn content_from_a_newer_version_is_reported_and_the_rest_is_kept() {
    let document = sample();
    let mut lines = lines_of(&document);
    let base = lines
        .iter()
        .position(|line| line.contains("Base sketch"))
        .unwrap();
    lines[base] = lines[base].replacen(
        "\"entities\":[",
        "\"entities\":[{\"id\":90,\"ellipse\":{\"center\":0}},",
        1,
    );
    let side = lines
        .iter()
        .position(|line| line.contains("Side sketch"))
        .unwrap();
    lines[side] =
        "{\"feature\":{\"id\":1,\"name\":\"Pad\",\"loft\":{\"distance\":\"5\"}}}".to_owned();
    lines.push("{\"assembly\":{\"parts\":[]}}".to_owned());

    let loaded = decode(&model_from_json(FORMAT_VERSION + 1, &lines)).unwrap();

    assert_eq!(
        loaded.issues,
        [
            &format!(
                "This model was made by a newer version of caditor (format {}). Anything this \
                 version does not understand was left out.",
                FORMAT_VERSION + 1
            ),
            "The feature “Pad” is a kind this version of caditor does not know (loft), so it \
             was left out. It may come from a newer version.",
            "Record 6 holds something this version of caditor does not know (assembly), so it was \
             left out. It may come from a newer version.",
            "In “Base sketch”, an entity of a kind this version of caditor does not know \
             (ellipse) was left out. It may come from a newer version.",
        ]
    );
    let base_sketch = loaded.document.features().next().unwrap();
    assert_eq!(**document.features().next().as_ref().unwrap(), *base_sketch);
}

#[test]
fn cycles_invalid_names_and_broken_dimensions_are_repaired_and_reported() {
    let text = r#"{"parameter":{"id":0,"name":"a","expression":"$1 + 1"}}
{"parameter":{"id":1,"name":"b","expression":"$0 * 2"}}
{"parameter":{"id":2,"name":"mm","expression":"3"}}
{"parameter":{"id":3,"name":"a","expression":"4"}}
{"parameter":{"id":3,"name":"twin","expression":"5"}}
{"parameter":{"id":4,"name":"typo","expression":"4 $$ 2"}}
{"feature":{"id":0,"name":" ","sketch":{"plane":{"origin":[0,0,0],"normal":[0,0,2],"x_axis":[0,0,1]},"entities":[{"id":0,"point":[0,0]},{"id":1,"point":[3,4]},{"id":2,"line":{"start":0,"end":9}}],"constraints":[{"id":3,"distance":{"from":0,"to":1,"value":"?"}},{"id":4,"vertical":2}],"next_id":5}}}
"#;
    let loaded = decode_text(text);

    assert_eq!(
        loaded.issues,
        [
            "A parameter was renamed to “parameter_2” because “mm” is not a usable name ('mm' \
             is a unit, so it cannot be used as a name).",
            "A parameter was renamed to “parameter_3” because two parameters are named “a”.",
            "Two parameters share the ID 3, so “twin” was left out.",
            "The value of “typo” could not be read, so it was set to 0. Enter its value again.",
            "A feature had no name, so it was named “Feature 0”.",
            "The plane of “Feature 0” could not be read, so the sketch was placed on the XY \
             plane.",
            "In “Feature 0”, Line 2 was left out because it uses an entity that does not exist.",
            "In “Feature 0”, the value of a distance could not be read, so it was set to its \
             drawn length, 5 mm.",
            "In “Feature 0”, a constraint was left out because it uses an entity that does not \
             exist.",
            "“b” depended on itself (b → a → b), so it was set to 0. Its value was a * 2.",
        ]
    );
    let document = &loaded.document;
    assert_eq!(document.parameters().len(), 5);
    let sketch = document.features().next().unwrap().kind.sketch().unwrap();
    assert_eq!(sketch.entities().len(), 2);
    assert_eq!(sketch.constraints().len(), 1);
    assert_eq!(sketch.plane(), Plane::XY);
}

#[test]
fn a_damaged_head_keeps_every_record() {
    let bytes = corrupt_chunk(&crate::encode(&sample()).unwrap(), &binary::MODEL_MAGIC, 0);
    let loaded = decode(&bytes).unwrap();
    assert_eq!(
        loaded.issues,
        ["A damaged part of the file was skipped; anything it held was left out."]
    );
    assert_eq!(loaded.document, sample());
    let missing = load(Path::new("/nonexistent/model.caditor")).unwrap_err();
    assert_eq!(missing.to_string(), "it no longer exists");
}

#[test]
fn a_crashed_session_is_recovered_with_its_undo_history() {
    let dir = TempDir::new().unwrap();
    let base = sample();
    let storage = Storage::spawn(config(&dir), untitled(&base), || {}).unwrap();
    let mut editor = Editor::new(base.clone());
    record_session(&storage, &mut editor);
    crash(storage);

    let recovery = dir.path().join("recovery");
    let recovered = scan(Some(&recovery), &[]);

    assert_eq!(recovered.len(), 1);
    let recovered = &recovered[0];
    assert_eq!(recovered.changes(), 3);
    assert_eq!(recovered.file, None);
    assert_eq!(recovered.base, base);
    assert_eq!(recovered.editor.document(), editor.document());
    assert_eq!(recovered.editor.undo_label(), Some("Edit width"));
    assert_eq!(recovered.editor.next_redo(), editor.next_redo());
    assert!(recovered.issues.is_empty());
}

#[test]
fn a_torn_or_corrupted_tail_loses_only_the_changes_after_it() {
    let dir = TempDir::new().unwrap();
    let base = sample();
    let storage = Storage::spawn(config(&dir), untitled(&base), || {}).unwrap();
    let mut editor = Editor::new(base.clone());
    record_session(&storage, &mut editor);
    crash(storage);
    let recovery = dir.path().join("recovery");
    let journal = fs::read_dir(&recovery)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let original = fs::read(&journal).unwrap();

    let mut torn = original.clone();
    torn.extend_from_slice(b"CDCK\x07\x01\0\0\x40");
    fs::write(&journal, torn).unwrap();
    let Inspection::Recoverable(recovered) = inspect(&journal).unwrap() else {
        panic!("the journal should still be recoverable");
    };
    assert_eq!(recovered.changes(), 3);
    assert_eq!(recovered.issues.len(), 1);

    fs::write(
        &journal,
        corrupt_chunk(&original, &binary::JOURNAL_MAGIC, 3),
    )
    .unwrap();
    let Inspection::Recoverable(recovered) = inspect(&journal).unwrap() else {
        panic!("the journal should still be recoverable");
    };
    assert_eq!(recovered.changes(), 1);
    let width = recovered
        .editor
        .document()
        .parameter_named("width")
        .unwrap();
    assert_eq!(
        recovered
            .editor
            .document()
            .expression_text(&width.expression),
        "50 mm"
    );
    assert!(recovered.issues[0].contains("most recent changes were damaged"));
}

#[test]
fn a_crashed_models_journal_is_offered_even_when_no_recent_file_names_it() {
    let dir = TempDir::new().unwrap();
    let recovery = dir.path().join("recovery");
    let path = dir.path().join("model.caditor");
    let base = sample();
    save(&base, &path, false).unwrap();
    let start = Start {
        file: Some(path.clone()),
        on_disk: None,
        ..untitled(&base)
    };
    let storage = Storage::spawn(config(&dir), start, || {}).unwrap();
    let mut editor = Editor::new(base);
    record_session(&storage, &mut editor);
    crash(storage);

    let recovered = scan(Some(&recovery), &[]);
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].file.as_deref(), Some(path.as_path()));
    assert_eq!(recovered[0].editor.document(), editor.document());

    discard(&recovered[0].journal).unwrap();
    assert!(scan(Some(&recovery), &[]).is_empty());
    assert_eq!(files_in(&recovery), Vec::<String>::new());
}

fn save_request(ticket: u64, document: &Document, path: &Path, replace: bool) -> SaveRequest {
    SaveRequest {
        ticket,
        document: document.clone(),
        path: path.to_path_buf(),
        keep_original: false,
        label: None,
        replace_outside_changes: replace,
    }
}

#[test]
fn a_save_over_a_file_changed_since_it_was_opened_is_refused_unless_replacing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    save(&Document::default(), &path, false).unwrap();
    let opened = load(&path).unwrap().digest.unwrap();
    save(&sample(), &path, false).unwrap();
    let outside = fs::read(&path).unwrap();

    let options = SaveOptions {
        history_from: Some(&path),
        unless_changed_from: Some(&opened),
        ..SaveOptions::default()
    };
    let refused = save_with(&Document::default(), &path, &options);

    assert_eq!(refused, Err(SaveError::ChangedOnDisk));
    assert_eq!(fs::read(&path).unwrap(), outside);

    let current = load(&path).unwrap().digest.unwrap();
    let options = SaveOptions {
        unless_changed_from: Some(&current),
        ..options
    };
    save_with(&Document::default(), &path, &options).unwrap();
    let listed = crate::history(&path).unwrap();
    let kept = load_version(&path, listed.versions[0].index).unwrap();

    assert_eq!(load(&path).unwrap().document, Document::default());
    assert_eq!(kept.document, sample());
}

#[test]
fn the_storage_worker_reports_an_outside_change_and_replaces_it_when_asked() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let base = Document::default();
    save(&base, &path, false).unwrap();
    let start = Start {
        file: Some(path.clone()),
        on_disk: load(&path).unwrap().digest,
        ..untitled(&base)
    };
    let storage = Storage::spawn(config(&dir), start, || {}).unwrap();
    save(&sample(), &path, false).unwrap();
    let mut edited = base.clone();
    let mut transaction = edited.transaction("Width");
    transaction.add_parameter("width", transaction.parse("5 mm").unwrap());
    edited.apply(transaction.finish()).unwrap();

    storage
        .save(save_request(1, &edited, &path, false))
        .unwrap();
    let refused = wait_for_report(&storage);
    storage.save(save_request(2, &edited, &path, true)).unwrap();
    let replaced = wait_for_report(&storage);
    storage
        .save(save_request(3, &edited, &path, false))
        .unwrap();
    let again = wait_for_report(&storage);

    assert_eq!(
        refused,
        Report::ChangedOnDisk {
            ticket: 1,
            path: path.clone(),
        }
    );
    assert!(matches!(replaced, Report::Saved { ticket: 2, .. }));
    assert!(matches!(again, Report::Saved { ticket: 3, .. }));
    assert_eq!(load(&path).unwrap().document, edited);
    assert!(storage.close(true).wait(WAIT));
}

#[test]
fn a_recovered_session_remembers_what_its_file_held() {
    let dir = TempDir::new().unwrap();
    let recovery = dir.path().join("recovery");
    let path = dir.path().join("model.caditor");
    let base = sample();
    save(&base, &path, false).unwrap();
    let on_disk = load(&path).unwrap().digest;
    let start = Start {
        file: Some(path.clone()),
        on_disk: on_disk.clone(),
        ..untitled(&base)
    };
    let storage = Storage::spawn(config(&dir), start, || {}).unwrap();
    let mut editor = Editor::new(base);
    record_session(&storage, &mut editor);
    crash(storage);

    let recovered = scan(Some(&recovery), &[]);

    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].on_disk, on_disk);
}

#[test]
fn saving_moves_the_journal_next_to_the_file_and_closing_removes_it() {
    let dir = TempDir::new().unwrap();
    let base = sample();
    let storage = Storage::spawn(config(&dir), untitled(&base), || {}).unwrap();
    let mut editor = Editor::new(base);
    record_session(&storage, &mut editor);
    let recovery = dir.path().join("recovery");
    assert_eq!(files_in(&recovery).len(), 1);

    let path = dir.path().join("model.caditor");
    storage
        .save(SaveRequest {
            ticket: 7,
            document: editor.document().clone(),
            path: path.clone(),
            keep_original: false,
            label: None,
            replace_outside_changes: false,
        })
        .unwrap();
    let report = wait_for_report(&storage);
    let digest = load(&path).unwrap().digest.unwrap();

    assert_eq!(
        report,
        Report::Saved {
            ticket: 7,
            path: path.clone(),
            backup: None,
            dropped_for_size: 0,
            digest,
        }
    );
    let markers = files_in(&recovery);
    assert_eq!(markers.len(), 1);
    assert!(markers[0].ends_with(".location"), "{markers:?}");
    assert_eq!(
        files_in(dir.path()),
        [".model.caditor.journal", "model.caditor", "recovery"]
    );
    assert!(matches!(
        journal_for(&path, Some(&recovery)),
        FileJournal::InUse
    ));

    assert!(storage.close(true).wait(WAIT));
    assert_eq!(files_in(dir.path()), ["model.caditor", "recovery"]);
    assert_eq!(files_in(&recovery), Vec::<String>::new());
    assert!(matches!(
        journal_for(&path, Some(&recovery)),
        FileJournal::None
    ));
}

#[test]
fn set_aside_journals_are_pruned_once_they_are_old() {
    let dir = TempDir::new().unwrap();
    let recovery = dir.path().join("recovery");
    fs::create_dir(&recovery).unwrap();
    let model = dir.path().join("model.caditor");
    let now = paths::now_seconds();
    let old = now - paths::SET_ASIDE_KEPT_SECONDS - 60;
    let kept = [
        recovery.join(format!("untitled-1-2.journal.{now}.unreadable")),
        recovery.join(format!("untitled-1-3.journal.{old}.notes")),
        recovery.join("untitled-1-4.journal.unreadable"),
        dir.path()
            .join(format!(".other.caditor.journal.{old}.unreadable")),
        dir.path()
            .join(format!(".model.caditor.journal.{now}.unreadable")),
    ];
    let pruned = [
        recovery.join(format!("untitled-1-5.journal.{old}.unreadable")),
        recovery.join(format!("file-1.journal.{old}-3.unreadable")),
        dir.path()
            .join(format!(".model.caditor.journal.{old}.unreadable")),
    ];
    for file in kept.iter().chain(&pruned) {
        fs::write(file, "set aside").unwrap();
    }

    assert!(scan(Some(&recovery), std::slice::from_ref(&model)).is_empty());

    assert!(kept.iter().all(|file| file.exists()));
    assert!(pruned.iter().all(|file| !file.exists()));
}

#[test]
fn an_unreadable_journal_is_set_aside_before_a_new_one_replaces_it() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let base = sample();
    save(&base, &path, false).unwrap();
    let start = Start {
        file: Some(path.clone()),
        on_disk: None,
        ..untitled(&base)
    };
    let storage = Storage::spawn(config(&dir), start, || {}).unwrap();
    let mut editor = Editor::new(base.clone());
    record_session(&storage, &mut editor);
    crash(storage);
    let journal = dir.path().join(".model.caditor.journal");
    let mut newer = fs::read(&journal).unwrap();
    newer[8] = 99;
    fs::write(&journal, &newer).unwrap();
    let recovery = dir.path().join("recovery");

    let FileJournal::SetAside(kept) = journal_for(&path, Some(&recovery)) else {
        panic!("the unreadable journal should be set aside");
    };
    assert_eq!(kept.len(), 1);
    assert_eq!(fs::read(&kept[0]).unwrap(), newer);
    assert!(!journal.exists());
    let name = kept[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with(".model.caditor.journal."));
    assert!(name.ends_with(".unreadable"));
    assert!(describe_set_aside(&kept[0]).contains(&name));

    let start = Start {
        file: Some(path.clone()),
        on_disk: None,
        ..untitled(&base)
    };
    let storage = Storage::spawn(config(&dir), start, || {}).unwrap();
    assert!(storage.flusher().flush(WAIT));
    assert!(journal.exists());
    assert_eq!(fs::read(&kept[0]).unwrap(), newer);
    crash(storage);
    assert!(matches!(
        journal_for(&path, Some(&recovery)),
        FileJournal::None
    ));
    assert!(scan(Some(&recovery), &[path]).is_empty());
    assert!(kept[0].exists());
}

#[test]
fn a_journal_locked_by_another_window_is_never_replaced() {
    let dir = TempDir::new().unwrap();
    let journal = dir.path().join(".model.caditor.journal");
    fs::write(&journal, b"theirs").unwrap();
    let theirs = crate::lock::lock_existing(&journal).unwrap().unwrap();
    let temporary = dir.path().join("ours.tmp");
    fs::write(&temporary, b"ours").unwrap();

    let refused = crate::lock::install(&temporary, &journal, None).unwrap_err();
    assert_eq!(refused.kind(), std::io::ErrorKind::ResourceBusy);
    assert_eq!(fs::read(&journal).unwrap(), b"theirs");
    assert!(crate::lock::lock_existing(&journal).unwrap().is_none());

    drop(theirs);
    crate::lock::install(&temporary, &journal, None).unwrap();
    assert_eq!(fs::read(&journal).unwrap(), b"ours");
    assert!(!temporary.exists());
}

#[test]
fn a_window_whose_journal_is_replaced_stops_claiming_protection_and_retakes_it() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let base = sample();
    save(&base, &path, false).unwrap();
    let start = Start {
        file: Some(path.clone()),
        on_disk: None,
        ..untitled(&base)
    };
    let storage = Storage::spawn(config(&dir), start, || {}).unwrap();
    assert!(storage.flusher().flush(WAIT));
    let journal = dir.path().join(".model.caditor.journal");
    let intruder = dir.path().join("intruder");
    fs::write(&intruder, b"not ours").unwrap();
    fs::rename(&intruder, &journal).unwrap();

    let mut editor = Editor::new(base);
    let change = edit_width(editor.document(), "70 mm");
    editor.apply(change.clone()).unwrap();
    storage.record(JournalEntry::Apply(change)).unwrap();
    assert!(storage.flusher().flush(WAIT));
    let reports = storage.poll().unwrap();
    assert!(
        matches!(
            reports.as_slice(),
            [Report::JournalFailed { .. }, Report::JournalRestored]
        ),
        "{reports:?}"
    );
    crash(storage);
    let FileJournal::Recoverable(recovered) =
        journal_for(&path, Some(&dir.path().join("recovery")))
    else {
        panic!("the change should be recoverable from the retaken journal");
    };
    assert_eq!(recovered.changes(), 1);
}

#[test]
fn a_journal_whose_changes_reached_the_file_is_tidied_away() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let base = sample();
    save(&base, &path, false).unwrap();
    let start = Start {
        file: Some(path.clone()),
        on_disk: None,
        ..untitled(&base)
    };
    let storage = Storage::spawn(config(&dir), start, || {}).unwrap();
    let mut editor = Editor::new(base);
    record_session(&storage, &mut editor);
    crash(storage);
    let recovery = dir.path().join("recovery");

    let FileJournal::Recoverable(recovered) = journal_for(&path, Some(&recovery)) else {
        panic!("unsaved changes should be offered");
    };
    assert_eq!(recovered.file.as_deref(), Some(path.as_path()));
    assert_eq!(scan(Some(&recovery), std::slice::from_ref(&path)).len(), 1);

    save(editor.document(), &path, false).unwrap();
    assert!(scan(Some(&recovery), &[path]).is_empty());
    assert_eq!(files_in(dir.path()), ["model.caditor", "recovery"]);
    assert_eq!(files_in(&recovery), Vec::<String>::new());
}

#[test]
fn a_journal_with_no_net_change_is_removed_and_one_can_be_discarded() {
    let dir = TempDir::new().unwrap();
    let base = sample();
    let storage = Storage::spawn(config(&dir), untitled(&base), || {}).unwrap();
    assert!(storage.flusher().flush(WAIT));
    crash(storage);
    let recovery = dir.path().join("recovery");
    assert!(scan(Some(&recovery), &[]).is_empty());
    assert_eq!(files_in(&recovery), Vec::<String>::new());

    let storage = Storage::spawn(config(&dir), untitled(&base), || {}).unwrap();
    let mut editor = Editor::new(base);
    record_session(&storage, &mut editor);
    crash(storage);
    let recovered = scan(Some(&recovery), &[]);
    assert_eq!(recovered.len(), 1);
    discard(&recovered[0].journal).unwrap();
    assert_eq!(files_in(&recovery), Vec::<String>::new());
}

#[test]
fn restoring_rewrites_the_journal_in_place() {
    let dir = TempDir::new().unwrap();
    let base = sample();
    let storage = Storage::spawn(config(&dir), untitled(&base), || {}).unwrap();
    let mut editor = Editor::new(base);
    record_session(&storage, &mut editor);
    crash(storage);
    let recovery = dir.path().join("recovery");
    let recovered = scan(Some(&recovery), &[]).remove(0);

    let start = Start {
        file: None,
        on_disk: None,
        loaded_with_problems: false,
        base: recovered.base.clone(),
        entries: recovered.entries.clone(),
        replaces: Some(recovered.journal.clone()),
        after: None,
    };
    let storage = Storage::spawn(config(&dir), start, || {}).unwrap();
    assert!(storage.flusher().flush(WAIT));
    assert_eq!(files_in(&recovery).len(), 1);
    assert!(!recovered.journal.exists());
    crash(storage);

    let again = scan(Some(&recovery), &[]);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].editor.document(), editor.document());
}

#[test]
fn recent_files_are_deduplicated_limited_and_persisted() {
    let dir = TempDir::new().unwrap();
    let mut recent = RecentFiles::default();
    for index in 0..12 {
        recent.add(PathBuf::from(format!("/models/{index}.caditor")));
    }
    recent.add(PathBuf::from("/models/5.caditor"));
    assert_eq!(recent.paths().len(), RECENT_LIMIT);
    assert_eq!(recent.paths()[0], Path::new("/models/5.caditor"));
    assert_eq!(recent.paths()[1], Path::new("/models/11.caditor"));

    recent.save(dir.path()).unwrap();
    assert_eq!(RecentFiles::load(dir.path()), recent);
    recent.remove(Path::new("/models/5.caditor"));
    assert_eq!(recent.paths().len(), RECENT_LIMIT - 1);
    assert_eq!(
        RecentFiles::load(&dir.path().join("missing")),
        RecentFiles::default()
    );
}

struct EveryKind {
    document: Document,
    angle: ConstraintId,
    radius: ConstraintId,
}

fn every_kind() -> EveryKind {
    let mut document = Document::default();
    let mut transaction = document.transaction("Every kind");
    let width = transaction.add_parameter("width", transaction.parse("4 mm").unwrap());
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 0.0));
    let Some(Entity::Line { start, end }) = sketch.entity(line).cloned() else {
        panic!("expected a line");
    };
    let other = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(30.0, 25.0));
    let circle = sketch.add_circle(Point2::new(50.0, 5.0), 5.0);
    let arc = sketch.add_arc(
        Point2::new(80.0, 0.0),
        Point2::new(90.0, 0.0),
        Point2::new(80.0, 10.0),
    );
    sketch.add_spline(&[
        Point2::new(0.0, -10.0),
        Point2::new(10.0, -20.0),
        Point2::new(20.0, -5.0),
        Point2::new(35.5, -12.25),
    ]);
    let Some(Entity::Circle { center, .. }) = sketch.entity(circle).cloned() else {
        panic!("expected a circle");
    };
    let mut add = |constraint| sketch.add_constraint(constraint).unwrap();
    add(Constraint::Coincident(start, EntityId::ORIGIN));
    add(Constraint::Coincident(end, other));
    add(Constraint::Horizontal(line));
    add(Constraint::Vertical(other));
    add(Constraint::Parallel(other, EntityId::HORIZONTAL_AXIS));
    add(Constraint::Perpendicular(line, EntityId::VERTICAL_AXIS));
    add(Constraint::Tangent(other, circle));
    add(Constraint::Equal(circle, arc));
    add(Constraint::Distance {
        from: EntityId::VERTICAL_AXIS,
        to: center,
        value: Expression::Measure(40.0, Unit::Millimetre),
    });
    let angle = add(Constraint::Angle {
        from: EntityId::HORIZONTAL_AXIS,
        to: other,
        reversed: false,
        value: Expression::Measure(30.0, Unit::Degree),
    });
    add(Constraint::Angle {
        from: line,
        to: other,
        reversed: true,
        value: Expression::Measure(45.0, Unit::Degree),
    });
    let radius = add(Constraint::Radius {
        entity: arc,
        value: Expression::Parameter(width),
    });
    transaction.add_feature("Everything", FeatureKind::from(sketch));
    document.apply(transaction.finish()).unwrap();
    EveryKind {
        document,
        angle,
        radius,
    }
}

#[test]
fn every_entity_and_constraint_kind_round_trips() {
    let EveryKind { document, .. } = every_kind();
    let text = encode(&document).unwrap();
    for record in [
        "\"circle\":{\"center\":",
        "\"arc\":{\"center\":",
        "\"spline\":{\"control_points\":[",
        "\"parallel\":[",
        "\"perpendicular\":[",
        "\"tangent\":[",
        "\"equal\":[",
        "\"angle\":{\"from\":18446744073709551614,",
        "\"angle\":{\"from\":2,\"reversed\":true,\"to\":5,\"value\":\"45 deg\"}",
        "\"radius\":{\"entity\":",
        "\"coincident\":[0,18446744073709551615]",
        "\"value\":\"30 deg\"",
    ] {
        assert!(text.contains(record), "{record} is missing from {text}");
    }

    let loaded = decode_text(&text);
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
}

#[test]
fn unreadable_angles_and_radii_take_their_drawn_values() {
    let EveryKind {
        document,
        angle,
        radius,
    } = every_kind();
    let text = encode(&document)
        .unwrap()
        .replace("\"value\":\"30 deg\"", "\"value\":\"30 ((\"")
        .replace("\"value\":\"$0\"", "\"value\":\"$$\"");

    let loaded = decode_text(&text);

    assert_eq!(
        loaded.issues,
        [
            "In “Everything”, the value of an angle could not be read, so it was set to its drawn \
         angle, 26.565051°.",
            "In “Everything”, the value of a radius could not be read, so it was set to its drawn \
         radius, 10 mm.",
        ]
    );
    let sketch = loaded
        .document
        .features()
        .next()
        .unwrap()
        .kind
        .sketch()
        .unwrap();
    assert!(matches!(
        sketch.constraint(angle).and_then(Constraint::dimension),
        Some(Expression::Measure(degrees, Unit::Degree)) if (degrees - 26.565_051_177_078).abs() < 1e-9
    ));
    assert_eq!(
        sketch.constraint(radius).and_then(Constraint::dimension),
        Some(&Expression::Measure(10.0, Unit::Millimetre))
    );
}

struct AddedKinds {
    horizontal: ConstraintId,
    vertical: ConstraintId,
    diameter: ConstraintId,
    around: ConstraintId,
    spacing: ConstraintId,
}

fn with_added_kinds(mut document: Document) -> (Document, AddedKinds) {
    let mut transaction = document.transaction("Added kinds");
    let width = transaction.add_parameter("gap", transaction.parse("6 mm").unwrap());
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 0.0));
    let Some(Entity::Line { start, end }) = sketch.entity(line).cloned() else {
        panic!("expected a line");
    };
    let other = sketch.add_line(Point2::new(0.0, 6.0), Point2::new(30.0, 6.0));
    let circle = sketch.add_circle(Point2::new(50.0, 5.0), 5.0);
    let arc = sketch.add_arc(
        Point2::new(50.0, 5.0),
        Point2::new(58.0, 5.0),
        Point2::new(50.0, 13.0),
    );
    let middle = sketch.add_point(Point2::new(15.0, 0.0));
    let mirrored = sketch.add_point(Point2::new(-5.0, 20.0));
    let lone = sketch.add_point(Point2::new(5.0, 20.0));
    let spline = sketch.add_spline(&[
        Point2::new(0.0, 30.0),
        Point2::new(10.0, 40.0),
        Point2::new(20.0, 30.0),
    ]);
    let rider = sketch.add_point(Point2::new(10.0, 35.0));
    let mut add = |constraint| sketch.add_constraint(constraint).unwrap();
    add(Constraint::HorizontalPoints(start, end));
    add(Constraint::VerticalPoints(start, EntityId::ORIGIN));
    add(Constraint::Midpoint {
        point: middle,
        line,
    });
    add(Constraint::Concentric(arc, circle));
    add(Constraint::Collinear(line, EntityId::HORIZONTAL_AXIS));
    add(Constraint::Symmetric {
        first: mirrored,
        second: lone,
        about: EntityId::VERTICAL_AXIS,
    });
    add(Constraint::Fix {
        point: lone,
        at: Point2::new(5.0, 20.0),
    });
    let horizontal = add(Constraint::HorizontalDistance {
        from: start,
        to: end,
        value: Expression::Measure(30.0, Unit::Millimetre),
    });
    let vertical = add(Constraint::VerticalDistance {
        from: end,
        to: lone,
        value: Expression::Measure(20.0, Unit::Millimetre),
    });
    let diameter = add(Constraint::Diameter {
        entity: circle,
        value: Expression::Measure(10.0, Unit::Millimetre),
    });
    let around = add(Constraint::Distance {
        from: lone,
        to: circle,
        value: Expression::Measure(45.0, Unit::Millimetre),
    });
    let spacing = add(Constraint::Distance {
        from: line,
        to: other,
        value: Expression::Parameter(width),
    });
    add(Constraint::Coincident(rider, spline));
    add(Constraint::Tangent(other, spline));
    let centreline = sketch.add_line(Point2::new(-10.0, 0.0), Point2::new(-10.0, 30.0));
    sketch.set_construction(centreline, true).unwrap();
    transaction.add_feature("Added kinds", FeatureKind::from(sketch));
    document.apply(transaction.finish()).unwrap();
    (
        document,
        AddedKinds {
            horizontal,
            vertical,
            diameter,
            around,
            spacing,
        },
    )
}

#[test]
fn added_constraint_kinds_round_trip() {
    let (document, _) = with_added_kinds(Document::default());
    let text = encode(&document).unwrap();
    for record in [
        "\"horizontal_points\":[0,1]",
        "\"vertical_points\":[0,18446744073709551615]",
        "\"midpoint\":{\"line\":2,\"point\":12}",
        "\"concentric\":[11,7]",
        "\"collinear\":[2,18446744073709551614]",
        "\"symmetric\":{\"about\":18446744073709551613,\"first\":13,\"second\":14}",
        "\"fix\":{\"at\":[5.0,20.0],\"point\":14}",
        "\"horizontal_distance\":{\"from\":0,\"to\":1,\"value\":\"30 mm\"}",
        "\"vertical_distance\":{\"from\":1,\"to\":14,\"value\":\"20 mm\"}",
        "\"diameter\":{\"entity\":7,\"value\":\"10 mm\"}",
        "\"distance\":{\"from\":14,\"to\":7,",
        "\"distance\":{\"from\":2,\"to\":5,\"value\":\"$0\"}",
        "\"coincident\":[19,18]",
        "\"tangent\":[5,18]",
    ] {
        assert!(text.contains(record), "{record} is missing from {text}");
    }

    let loaded = decode_text(&text);
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
}

#[test]
fn unreadable_added_dimensions_take_their_drawn_values() {
    let (document, kinds) = with_added_kinds(Document::default());
    let text = encode(&document)
        .unwrap()
        .replace("\"value\":\"30 mm\"", "\"value\":\"30 ((\"")
        .replace("\"value\":\"20 mm\"", "\"value\":\"20 ((\"")
        .replace("\"value\":\"10 mm\"", "\"value\":\"10 ((\"")
        .replace("\"value\":\"45 mm\"", "\"value\":\"45 ((\"")
        .replace("\"value\":\"$0\"", "\"value\":\"$$\"");

    let loaded = decode_text(&text);

    assert_eq!(
        loaded.issues,
        [
            "In “Added kinds”, the value of a horizontal distance could not be read, so it was \
             set to its drawn length, 30 mm.",
            "In “Added kinds”, the value of a vertical distance could not be read, so it was set \
             to its drawn length, 20 mm.",
            "In “Added kinds”, the value of a diameter could not be read, so it was set to its \
             drawn diameter, 10 mm.",
            "In “Added kinds”, the value of a distance could not be read, so it was set to its \
             drawn length, 42.434165 mm.",
            "In “Added kinds”, the value of a distance could not be read, so it was set to its \
             drawn length, 6 mm.",
        ]
    );
    let sketch = loaded
        .document
        .features()
        .next()
        .unwrap()
        .kind
        .sketch()
        .unwrap();
    let value = |constraint| {
        sketch
            .constraint(constraint)
            .and_then(Constraint::dimension)
    };
    let millimetres = |value| Expression::Measure(value, Unit::Millimetre);
    assert_eq!(value(kinds.horizontal), Some(&millimetres(30.0)));
    assert_eq!(value(kinds.vertical), Some(&millimetres(20.0)));
    assert_eq!(value(kinds.diameter), Some(&millimetres(10.0)));
    assert_eq!(value(kinds.spacing), Some(&millimetres(6.0)));
    assert!(matches!(
        value(kinds.around),
        Some(Expression::Measure(length, Unit::Millimetre)) if (length - 42.434_165).abs() < 1e-6
    ));
}

struct SketchSession {
    base: Document,
    plate: FeatureId,
}

fn sketch_session() -> SketchSession {
    let mut base = sample();
    let mut transaction = base.transaction("New sketch");
    let plate = transaction.add_feature("Plate", FeatureKind::from(Sketch::new(Plane::XY)));
    base.apply(transaction.finish()).unwrap();
    SketchSession { base, plate }
}

fn every_sketch_edit(document: &Document, plate: FeatureId) -> Transaction {
    let width = document.parameter_named("width").unwrap().id();
    let mut transaction = document.transaction("Every sketch edit");
    let center = transaction.add_sketch_entity(plate, Entity::Point(Point2::new(1.0 / 3.0, -2.5)));
    let circle = transaction.add_sketch_entity(
        plate,
        Entity::Circle {
            center,
            radius: 0.1,
        },
    );
    let radius = transaction.add_sketch_constraint(
        plate,
        Constraint::Radius {
            entity: circle,
            value: transaction.parse("width / 3").unwrap(),
        },
    );
    transaction.edit(Edit::SetSketchEntity {
        feature: plate,
        id: center,
        entity: Entity::Point(Point2::new(f64::MIN_POSITIVE, 1e300)),
    });
    transaction.edit(Edit::SetDimension {
        feature: plate,
        constraint: radius,
        value: Expression::Parameter(width),
    });
    transaction.edit(Edit::RemoveSketchConstraint {
        feature: plate,
        id: radius,
    });
    transaction.edit(Edit::RemoveSketchEntity {
        feature: plate,
        id: circle,
    });
    let start = transaction.add_sketch_entity(plate, Entity::Point(Point2::ZERO));
    let end = transaction.add_sketch_entity(plate, Entity::Point(Point2::X));
    let centreline = transaction.add_sketch_entity_as(plate, Entity::Line { start, end }, true);
    transaction.edit(Edit::SetSketchConstruction {
        feature: plate,
        id: centreline,
        construction: false,
    });
    transaction.finish()
}

#[test]
fn every_sketch_edit_record_round_trips() {
    let SketchSession { base, plate } = sketch_session();
    let transaction = every_sketch_edit(&base, plate);
    assert!(base.check(&transaction).is_ok());

    let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
    for record in [
        "{\"add_sketch_entity\":{\"feature\":3,\"entity\":{\"id\":0,\"point\":[0.3333333333333333,-2.5]}}}",
        "{\"add_sketch_entity\":{\"feature\":3,\"entity\":{\"id\":1,\"circle\":{\"center\":0,\"radius\":0.1}}}}",
        "{\"add_sketch_constraint\":{\"feature\":3,\"constraint\":{\"id\":2,\"radius\":{\"entity\":1,\"value\":\"$0 / 3\"}}}}",
        "{\"set_sketch_entity\":{\"feature\":3,\"entity\":{\"id\":0,\"point\":[2.2250738585072014e-308,1e+300]}}}",
        "{\"remove_sketch_constraint\":{\"feature\":3,\"id\":2}}",
        "{\"remove_sketch_entity\":{\"feature\":3,\"id\":1}}",
        "{\"add_sketch_entity\":{\"feature\":3,\"entity\":{\"id\":5,\"construction\":true,\"line\":{\"start\":3,\"end\":4}}}}",
        "{\"set_sketch_construction\":{\"feature\":3,\"id\":5,\"construction\":false}}",
    ] {
        assert!(text.contains(record), "{record} is missing from {text}");
    }

    let record: format::TransactionRecord = through_binary(&text);
    assert_eq!(format::restore_transaction(record), Some(transaction));

    let damaged: format::TransactionRecord =
        through_binary(&text.replace("\"$0 / 3\"", "\"$0 //\""));
    assert_eq!(format::restore_transaction(damaged), None);
}

fn record(storage: &Storage, entry: JournalEntry) {
    storage.record(entry).unwrap();
}

fn apply_and_record(storage: &Storage, editor: &mut Editor, transaction: Transaction) {
    editor.apply(transaction.clone()).unwrap();
    record(storage, JournalEntry::Apply(transaction));
}

fn undo_and_record(storage: &Storage, editor: &mut Editor) {
    let undone = editor.next_undo().cloned().unwrap();
    editor.undo().unwrap();
    record(storage, JournalEntry::Undo(undone));
}

fn redo_and_record(storage: &Storage, editor: &mut Editor) {
    let redone = editor.next_redo().cloned().unwrap();
    editor.redo().unwrap();
    record(storage, JournalEntry::Redo(redone));
}

fn history(editor: &Editor) -> Vec<(Document, Option<Transaction>, Option<Transaction>)> {
    let mut walker = editor.clone();
    while walker.undo().unwrap().is_some() {}
    let mut states = Vec::new();
    loop {
        states.push((
            walker.document().clone(),
            walker.next_undo().cloned(),
            walker.next_redo().cloned(),
        ));
        if walker.redo().unwrap().is_none() {
            return states;
        }
    }
}

fn plate_sketch(document: &Document, plate: FeatureId) -> &Sketch {
    document.feature(plate).unwrap().kind.sketch().unwrap()
}

#[test]
fn a_crashed_sketching_session_is_recovered_with_its_undo_history() {
    let dir = TempDir::new().unwrap();
    let SketchSession { base, plate } = sketch_session();
    let storage = Storage::spawn(config(&dir), untitled(&base), || {}).unwrap();
    let mut editor = Editor::new(base.clone());

    let mut transaction = editor.document().transaction("Draw line");
    let start = transaction.add_sketch_entity(plate, Entity::Point(Point2::new(0.5, 0.25)));
    let end = transaction.add_sketch_entity(plate, Entity::Point(Point2::new(38.0, 1.0)));
    let line = transaction.add_sketch_entity(plate, Entity::Line { start, end });
    let transaction = transaction.finish();
    apply_and_record(&storage, &mut editor, transaction);

    let mut transaction = editor.document().transaction("Add constraints");
    transaction.add_sketch_constraint(plate, Constraint::Horizontal(line));
    let distance = transaction.add_sketch_constraint(
        plate,
        Constraint::Distance {
            from: start,
            to: end,
            value: Expression::Measure(35.0, Unit::Millimetre),
        },
    );
    let transaction = transaction.finish();
    apply_and_record(&storage, &mut editor, transaction);

    let width = editor.document().parse("width * 2").unwrap();
    apply_and_record(
        &storage,
        &mut editor,
        Transaction::single(
            "Edit distance",
            Edit::SetDimension {
                feature: plate,
                constraint: distance,
                value: width,
            },
        ),
    );
    apply_and_record(
        &storage,
        &mut editor,
        Transaction::single(
            "Move point",
            Edit::SetSketchEntity {
                feature: plate,
                id: end,
                entity: Entity::Point(Point2::new(80.0, -3.0)),
            },
        ),
    );
    let mut transaction = editor.document().transaction("Delete point");
    transaction.remove_sketch_items(plate, [start], []);
    let transaction = transaction.finish();
    apply_and_record(&storage, &mut editor, transaction);
    assert_eq!(plate_sketch(editor.document(), plate).entities().len(), 1);

    undo_and_record(&storage, &mut editor);
    undo_and_record(&storage, &mut editor);
    redo_and_record(&storage, &mut editor);
    assert!(storage.flusher().flush(WAIT));
    crash(storage);

    let recovery = dir.path().join("recovery");
    let recovered = scan(Some(&recovery), &[]);

    assert_eq!(recovered.len(), 1);
    let recovered = &recovered[0];
    assert_eq!(recovered.changes(), 8);
    assert!(recovered.issues.is_empty(), "{:?}", recovered.issues);
    assert_eq!(recovered.base, base);
    assert_eq!(recovered.editor.document(), editor.document());
    assert_eq!(recovered.editor.undo_label(), Some("Move point"));
    assert_eq!(recovered.editor.redo_label(), Some("Delete point"));
    assert_eq!(history(&recovered.editor), history(&editor));
    assert_eq!(
        plate_sketch(recovered.editor.document(), plate).point(end),
        Some(Point2::new(80.0, -3.0))
    );
}

#[test]
fn an_edit_kind_this_version_does_not_know_stops_replay_at_its_line() {
    let dir = TempDir::new().unwrap();
    let storage = Storage::spawn(config(&dir), untitled(&sample()), || {}).unwrap();
    let mut editor = Editor::new(sample());
    record_session(&storage, &mut editor);
    crash(storage);
    let journal = fs::read_dir(dir.path().join("recovery"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let original = fs::read(&journal).unwrap();
    let rewritten = rewrite_journal(&original, |index, json| match index {
        3 => json.replace("set_parameter_expression", "bend_sheet"),
        _ => json,
    });
    fs::write(&journal, rewritten).unwrap();

    let Inspection::Recoverable(recovered) = inspect(&journal).unwrap() else {
        panic!("the journal should still be recoverable");
    };
    assert_eq!(recovered.changes(), 1);
    assert_eq!(recovered.issues.len(), 1);
}

fn solid_model() -> (Document, FeatureId, FeatureId) {
    use caditor_document::{
        BodyOperation, Extrude, ExtrudeExtent, RegionChoice, Revolve, RevolveAxis, RevolveExtent,
        SolidFeature,
    };
    let mut document = Document::default();
    let mut transaction = document.transaction("Solids");
    let depth = transaction.add_parameter("depth", transaction.parse("3 mm").unwrap());
    let mut outline = Sketch::new(Plane::XY);
    let corners = [(0.0, 0.0), (6.0, 0.0), (6.0, 4.0), (0.0, 4.0)];
    for index in 0..4 {
        let (a, b) = (corners[index], corners[(index + 1) % 4]);
        outline.add_line(Point2::new(a.0, a.1), Point2::new(b.0, b.1));
    }
    let sketch = transaction.add_feature("Outline", FeatureKind::from(outline));
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch,
            regions: RegionChoice::Chosen(vec![caditor_kernel::RegionKey::from_digest(
                0x0123_4567_89ab_cdef_0011_2233_4455_6677,
            )]),
            extent: ExtrudeExtent::two_sides(
                Expression::Parameter(depth),
                transaction.parse("1 mm").unwrap(),
            ),
            operation: BodyOperation::NewBody,
        })),
    );
    let turned = transaction.add_feature(
        "Turned",
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch,
            regions: RegionChoice::All,
            axis: RevolveAxis::Sketch(EntityId::HORIZONTAL_AXIS),
            extent: RevolveExtent::OneSide {
                angle: transaction.parse("90 deg").unwrap(),
                reversed: true,
            },
            operation: BodyOperation::Remove(base),
        })),
    );
    document.apply(transaction.finish()).unwrap();
    (document, base, turned)
}

#[test]
fn solid_features_are_saved_and_loaded() {
    let (document, _, _) = solid_model();
    let text = encode(&document).unwrap();
    assert!(text.contains("\"extrude\""));
    assert!(text.contains("\"remove\":1"));
    let loaded = decode_text(&text);
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
}

#[test]
fn a_damaged_solid_value_falls_back_and_is_reported() {
    let (document, _, _) = solid_model();
    let text = encode(&document)
        .unwrap()
        .replace("\"angle\":\"90 deg\"", "\"angle\":\"90 ((\"");
    let loaded = decode_text(&text);
    assert_eq!(
        loaded.issues,
        ["The angle of “Turned” could not be read, so it was set to 360 deg."]
    );
    assert_eq!(loaded.document.features().count(), 3);
}

#[test]
fn a_changed_solid_feature_round_trips_through_the_journal() {
    let (document, base, _) = solid_model();
    let kind = document.feature(base).unwrap().kind.clone();
    let transaction = Transaction::single("Edit Base", Edit::SetFeatureKind { id: base, kind });
    let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
    let record = through_binary(&text);
    assert_eq!(format::restore_transaction(record), Some(transaction));
}

#[test]
fn construction_geometry_stays_construction_through_saving() {
    let mut sketch = Sketch::new(Plane::XY);
    let centreline = sketch.add_line(Point2::ZERO, Point2::Y);
    let circle = sketch.add_circle(Point2::X, 0.5);
    sketch.set_construction(centreline, true).unwrap();
    let mut document = Document::default();
    let mut transaction = document.transaction("New sketch");
    let feature = transaction.add_feature("Profile", FeatureKind::from(sketch));
    document.apply(transaction.finish()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let restored = loaded
        .document
        .feature(feature)
        .unwrap()
        .kind
        .sketch()
        .unwrap();

    assert!(text.contains(&format!(
        "\"construction\":true,\"id\":{}",
        centreline.raw()
    )));
    assert_eq!(text.matches("construction").count(), 1);
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert!(restored.is_construction(centreline));
    assert!(!restored.is_construction(circle));
    assert_eq!(loaded.document, document);
}

#[test]
fn a_point_stored_as_construction_geometry_loads_as_an_ordinary_point() {
    let mut document = Document::default();
    let mut transaction = document.transaction("New sketch");
    let mut sketch = Sketch::new(Plane::XY);
    let point = sketch.add_point(Point2::X);
    let feature = transaction.add_feature("Points", FeatureKind::from(sketch));
    document.apply(transaction.finish()).unwrap();
    let stored = format!("{{\"id\":{},\"point\"", point.raw());
    let text = encode(&document).unwrap().replace(
        &stored,
        &format!("{{\"construction\":true,\"id\":{},\"point\"", point.raw()),
    );

    let loaded = decode_text(&text);
    let restored = loaded
        .document
        .feature(feature)
        .unwrap()
        .kind
        .sketch()
        .unwrap();

    assert!(issues_mention(&loaded, "was kept as ordinary geometry"));
    assert_eq!(restored.entity(point), Some(&Entity::Point(Point2::X)));
    assert!(!restored.is_construction(point));
}

#[test]
fn a_hidden_feature_stays_hidden_through_saving_and_the_journal() {
    let (mut document, base, _) = solid_model();
    let visible = encode(&document).unwrap();
    let hide = Transaction::single(
        "Hide Base",
        Edit::SetFeatureHidden {
            id: base,
            hidden: true,
        },
    );
    let shown = document.apply(hide.clone()).unwrap();
    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&hide)).unwrap());

    assert!(!visible.contains("hidden"));
    assert!(text.contains("\"hidden\":true"));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert!(loaded.document.feature(base).unwrap().hidden);
    assert_eq!(loaded.document, document);
    assert_eq!(format::restore_transaction(journaled), Some(hide));
    assert_eq!(
        shown.edits(),
        [Edit::SetFeatureHidden {
            id: base,
            hidden: false
        }]
    );
}

#[test]
fn hidden_principal_geometry_stays_hidden_through_saving_the_journal_and_its_snapshot() {
    use caditor_document::{PrincipalAxis, PrincipalGeometry, PrincipalPlane};

    let (mut document, _, _) = solid_model();
    let visible = encode(&document).unwrap();
    let hide = Transaction::new(
        "Hide",
        [
            PrincipalGeometry::Origin,
            PrincipalGeometry::Axis(PrincipalAxis::Z),
            PrincipalGeometry::Plane(PrincipalPlane::Xz),
        ]
        .into_iter()
        .map(|geometry| Edit::SetPrincipalHidden {
            geometry,
            hidden: true,
        })
        .collect(),
    );
    document.apply(hide.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&hide)).unwrap());
    let recovered = journal::decode_journal(
        &journal::encode_journal(None, None, false, &document, &[]).unwrap(),
    )
    .unwrap();

    assert!(!visible.contains("principal"));
    assert!(text.contains(r#"{"principal":{"hidden":["origin",{"axis":"z"},{"plane":"xz"}]}}"#));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(format::restore_transaction(journaled), Some(hide));
    assert_eq!(recovered.issues, Vec::<String>::new());
    assert_eq!(recovered.base, document);
}

fn attached_model() -> (Document, FeatureId, FeatureId) {
    use caditor_document::{FaceAttachment, SketchFeature};
    use caditor_kernel::{FaceName, FaceOrigin, FaceReference};
    let (mut document, base, _) = solid_model();
    let top = Plane::from_frame(Point3::new(0.0, 0.0, 3.0), Vector3::Z, Vector3::X).unwrap();
    let attachment = FaceAttachment {
        body: base,
        face: FaceReference::new(
            FaceName::from_digest(0xfeed_0000_0000_0000_0000_0000_0000_0001),
            Some(FaceOrigin::EndCap {
                feature: base.raw(),
            }),
            [FaceName::from_digest(3), FaceName::from_digest(u128::MAX)],
        ),
    };
    let mut transaction = document.transaction("Sketch on top");
    let sketch = transaction.add_feature(
        "Top",
        FeatureKind::Sketch(SketchFeature::on_face(
            dimensioned_line(top, 2.0, Expression::Number(2.0)),
            attachment,
        )),
    );
    document.apply(transaction.finish()).unwrap();
    (document, base, sketch)
}

#[test]
fn a_sketch_on_a_face_is_saved_and_loaded() {
    let (document, _, _) = attached_model();
    let text = encode(&document).unwrap();
    assert!(text.contains(
        "\"attachment\":{\"body\":1,\"face\":\"feed0000000000000000000000000001\",\"neighbours\":\
         [\"00000000000000000000000000000003\",\"ffffffffffffffffffffffffffffffff\"],\"origin\":\
         {\"end_cap\":{\"feature\":1}}}"
    ));
    let loaded = decode_text(&text);
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
}

#[test]
fn a_sketch_whose_face_cannot_be_read_stays_where_it_was() {
    let (document, _, sketch) = attached_model();
    let text = encode(&document)
        .unwrap()
        .replace("feed0000000000000000000000000001", "not a digest");
    let loaded = decode_text(&text);
    assert_eq!(
        loaded.issues,
        ["The face that “Top” lies on could not be read, so the sketch stays where it was."]
    );
    let restored = loaded.document.feature(sketch).unwrap();
    assert!(restored.kind.attachment().is_none());
    assert_eq!(
        restored.kind.sketch().unwrap().plane(),
        document
            .feature(sketch)
            .unwrap()
            .kind
            .sketch()
            .unwrap()
            .plane()
    );
}

#[test]
fn a_sketch_on_a_body_that_is_gone_stays_on_it_as_saved() {
    let (document, base, sketch) = attached_model();
    let lines: Vec<String> = encode(&document)
        .unwrap()
        .lines()
        .filter(|line| !line.contains("\"name\":\"Base\""))
        .map(str::to_owned)
        .collect();
    let loaded = decode_lines(&lines);
    assert!(loaded.document.feature(base).is_none());
    let restored = loaded.document.feature(sketch).unwrap();
    assert_eq!(
        restored.kind.attachment().and_then(SketchAttachment::body),
        Some(base)
    );
    assert!(loaded.document.next_feature_id() > base.raw());
}

#[test]
fn a_placement_change_round_trips_through_the_journal() {
    let (document, _, sketch) = attached_model();
    let attachment = document.feature(sketch).unwrap().kind.attachment().cloned();
    for attachment in [attachment, None] {
        let transaction = Transaction::single(
            "Place",
            Edit::SetSketchPlacement {
                feature: sketch,
                plane: Plane::YZ,
                attachment,
            },
        );
        assert!(document.check(&transaction).is_ok());
        let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
        let record = through_binary(&text);
        assert_eq!(format::restore_transaction(record), Some(transaction));
    }
}

fn blended_model() -> (Document, FeatureId, FeatureId) {
    use caditor_document::{Blend, BlendKind, FaceAttachment, SketchFeature};
    use caditor_kernel::{
        EdgeName, EdgeReference, FaceName, FaceOrigin, FaceReference, VertexName,
    };
    let (mut document, base, _) = solid_model();
    let edge = EdgeReference::new(
        EdgeName::from_digest(0xabcd),
        [FaceName::from_digest(1), FaceName::from_digest(2)],
        [VertexName::from_digest(3), VertexName::from_digest(4)],
    );
    let mut transaction = document.transaction("Blends");
    let fillet = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: base,
            edges: vec![edge],
            size: transaction.parse("depth / 3").unwrap(),
        }),
    );
    transaction.add_feature(
        "Chamfer 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Chamfer,
            body: base,
            edges: vec![
                edge,
                edge.with_origins([
                    Some(FaceOrigin::EndCap { feature: 1 }),
                    Some(FaceOrigin::Side {
                        feature: 1,
                        entity: 4,
                    }),
                ]),
            ],
            size: transaction.parse("0.5 mm").unwrap(),
        }),
    );
    let top = Plane::from_frame(Point3::new(0.0, 0.0, 3.0), Vector3::Z, Vector3::X).unwrap();
    let sketch = transaction.add_feature(
        "On chamfer",
        FeatureKind::Sketch(SketchFeature::on_face(
            dimensioned_line(top, 2.0, Expression::Number(2.0)),
            FaceAttachment {
                body: base,
                face: FaceReference::new(
                    FaceName::from_digest(5),
                    Some(FaceOrigin::Chamfer { feature: 7 }),
                    [FaceName::from_digest(6)],
                ),
            },
        )),
    );
    document.apply(transaction.finish()).unwrap();
    (document, fillet, sketch)
}

#[test]
fn fillets_and_chamfers_are_saved_and_loaded() {
    let (document, _, _) = blended_model();
    let text = encode(&document).unwrap();
    assert!(text.contains(
        "\"fillet\":{\"body\":1,\"edges\":[{\"ends\":[\"00000000000000000000000000000003\",\
         \"00000000000000000000000000000004\"],\"faces\":[\"00000000000000000000000000000001\",\
         \"00000000000000000000000000000002\"],\"name\":\"0000000000000000000000000000abcd\"}],\
         \"size\":\"$0 / 3\"}"
    ));
    assert!(text.contains("\"chamfer\":{\"body\":1"));
    assert!(text.contains("\"origin\":{\"chamfer\":{\"feature\":7}}"));
    assert!(text.contains(
        "\"origins\":[{\"end_cap\":{\"feature\":1}},{\"side\":{\"entity\":4,\"feature\":1}}]"
    ));
    let loaded = decode_text(&text);
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
}

#[test]
fn an_unreadable_blend_edge_is_left_out_and_reported() {
    let (document, fillet, _) = blended_model();
    let text =
        encode(&document)
            .unwrap()
            .replacen("0000000000000000000000000000abcd", "not a digest", 1);
    let loaded = decode_text(&text);
    assert_eq!(
        loaded.issues,
        ["Some edges chosen for “Fillet 1” could not be read and were left out."]
    );
    let restored = loaded.document.feature(fillet).unwrap();
    assert!(restored.kind.blend().unwrap().edges.is_empty());
}

#[test]
fn a_changed_blend_round_trips_through_the_journal() {
    let (document, fillet, _) = blended_model();
    let kind = document.feature(fillet).unwrap().kind.clone();
    let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id: fillet, kind });
    assert!(document.check(&transaction).is_ok());
    let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
    let record = through_binary(&text);
    assert_eq!(format::restore_transaction(record), Some(transaction));
}

fn shelled_model() -> (Document, FeatureId) {
    use caditor_document::Shell;
    use caditor_kernel::{FaceName, FaceOrigin, FaceReference};
    let (mut document, base, _) = solid_model();
    let top = FaceReference::new(
        FaceName::from_digest(0xbeef),
        Some(FaceOrigin::EndCap { feature: 1 }),
        [FaceName::from_digest(2)],
    );
    let mut transaction = document.transaction("Shell");
    let shell = transaction.add_feature(
        "Shell 1",
        FeatureKind::Shell(Shell {
            body: base,
            open: vec![top],
            thickness: transaction.parse("depth / 4").unwrap(),
        }),
    );
    document.apply(transaction.finish()).unwrap();
    (document, shell)
}

#[test]
fn shells_are_saved_and_loaded() {
    let (document, shell) = shelled_model();
    let text = encode(&document).unwrap();
    assert!(text.contains(
        "\"shell\":{\"body\":1,\"open\":[{\"face\":\"0000000000000000000000000000beef\",\
         \"neighbours\":[\"00000000000000000000000000000002\"],\"origin\":{\"end_cap\":\
         {\"feature\":1}}}],\"thickness\":\"$0 / 4\"}"
    ));
    let loaded = decode_text(&text);
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);

    let kind = document.feature(shell).unwrap().kind.clone();
    let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id: shell, kind });
    let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
    let record = through_binary(&text);
    assert_eq!(format::restore_transaction(record), Some(transaction));
}

#[test]
fn an_unreadable_opened_face_is_left_closed_and_reported() {
    let (document, shell) = shelled_model();
    let text =
        encode(&document)
            .unwrap()
            .replacen("0000000000000000000000000000beef", "not a digest", 1);
    let loaded = decode_text(&text);
    assert_eq!(
        loaded.issues,
        ["Some faces opened by “Shell 1” could not be read and were left closed."]
    );
    let restored = loaded.document.feature(shell).unwrap();
    assert!(restored.kind.shell().unwrap().open.is_empty());
}

fn patterned_model() -> (Document, FeatureId, FeatureId) {
    use caditor_document::{
        AxisReference, CircularPattern, LinearDirection, Pattern, PatternKind, PrincipalAxis,
    };
    use caditor_kernel::{
        EdgeName, EdgeReference, FaceName, FaceOrigin, FaceReference, VertexName,
    };
    let (mut document, base, _) = solid_model();
    let mut transaction = document.transaction("Patterns");
    let linear = transaction.add_feature(
        "Linear pattern 1",
        FeatureKind::from(Pattern {
            body: base,
            kind: PatternKind::Linear {
                first: LinearDirection {
                    axis: AxisReference::Edge {
                        body: base,
                        edge: Box::new(EdgeReference::new(
                            EdgeName::from_digest(0xed),
                            [FaceName::from_digest(1), FaceName::from_digest(2)],
                            [VertexName::from_digest(3), VertexName::from_digest(4)],
                        )),
                    },
                    count: transaction.parse("3").unwrap(),
                    spacing: transaction.parse("depth * 4").unwrap(),
                    reversed: false,
                },
                second: Some(LinearDirection {
                    axis: AxisReference::Principal(PrincipalAxis::Y),
                    count: transaction.parse("2").unwrap(),
                    spacing: transaction.parse("12 mm").unwrap(),
                    reversed: true,
                }),
            },
        }),
    );
    let circular = transaction.add_feature(
        "Circular pattern 1",
        FeatureKind::from(Pattern {
            body: base,
            kind: PatternKind::Circular(CircularPattern {
                axis: AxisReference::Face {
                    body: base,
                    face: FaceReference::new(
                        FaceName::from_digest(0xfa),
                        Some(FaceOrigin::Side {
                            feature: 1,
                            entity: 2,
                        }),
                        [FaceName::from_digest(3)],
                    ),
                },
                count: transaction.parse("4").unwrap(),
                angle: transaction.parse("180 deg").unwrap(),
                reversed: false,
            }),
        }),
    );
    document.apply(transaction.finish()).unwrap();
    (document, linear, circular)
}

#[test]
fn patterns_are_saved_and_loaded() {
    let (document, linear, circular) = patterned_model();
    let text = encode(&document).unwrap();
    assert!(
        text.contains("\"linear_pattern\":{\"body\":1,\"first\":{\"axis\":{\"edge\":{\"body\":1,")
    );
    assert!(text.contains(
        "\"second\":{\"axis\":{\"principal\":\"y\"},\"count\":\"2\",\"reversed\":true,\
         \"spacing\":\"12 mm\"}"
    ));
    assert!(text.contains("\"spacing\":\"$0 * 4\""));
    assert!(
        text.contains(
            "\"circular_pattern\":{\"angle\":\"180 deg\",\"axis\":{\"face\":{\"body\":1,"
        )
    );
    let loaded = decode_text(&text);
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);

    for pattern in [linear, circular] {
        let kind = document.feature(pattern).unwrap().kind.clone();
        let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id: pattern, kind });
        let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
        let record = through_binary(&text);
        assert_eq!(format::restore_transaction(record), Some(transaction));
    }
}

#[test]
fn a_pattern_whose_axis_cannot_be_read_falls_back_and_is_reported() {
    use caditor_document::{AxisReference, PatternKind, PrincipalAxis};
    let (document, linear, circular) = patterned_model();
    let text = encode(&document)
        .unwrap()
        .replacen("000000000000000000000000000000ed", "not a digest", 1)
        .replacen("000000000000000000000000000000fa", "not a digest", 1);
    let loaded = decode_text(&text);
    assert_eq!(
        loaded.issues,
        [
            "The direction of “Linear pattern 1” could not be read, so it runs along the X axis.",
            "The axis of “Circular pattern 1” could not be read, so it turns about the Z axis.",
        ]
    );
    let restored = loaded
        .document
        .feature(linear)
        .unwrap()
        .kind
        .pattern()
        .unwrap();
    let PatternKind::Linear { first, second } = &restored.kind else {
        panic!("the pattern stays linear");
    };
    assert_eq!(first.axis, AxisReference::Principal(PrincipalAxis::X));
    assert!(second.is_some());
    let restored = loaded
        .document
        .feature(circular)
        .unwrap()
        .kind
        .pattern()
        .unwrap();
    let PatternKind::Circular(turned) = &restored.kind else {
        panic!("the pattern stays circular");
    };
    assert_eq!(turned.axis, AxisReference::Principal(PrincipalAxis::Z));
}

#[test]
fn a_model_saved_in_format_1_before_patterns_loads_unchanged() {
    let (document, _, _) = datum_model();
    let lines = lines_of(&document);
    let loaded = decode(&model_from_json(1, &lines)).unwrap();
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);

    let seed = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds/model/datums.caditor"),
    )
    .unwrap();
    assert_eq!(seed.get(8..12), Some(1u32.to_le_bytes().as_slice()));
    assert_eq!(decode(&seed).unwrap().issues, Vec::<String>::new());
}

fn datum_model() -> (Document, FeatureId, FeatureId) {
    use caditor_document::{
        AxisReference, BodyOperation, Datum, DatumAxis, DatumPlane, PlaneReference, PlaneRotation,
        PrincipalAxis, PrincipalPlane, RegionChoice, Revolve, RevolveAxis, RevolveExtent,
        SketchFeature, SolidFeature,
    };
    use caditor_kernel::{EdgeName, EdgeReference, FaceName, FaceReference, VertexName};
    let (mut document, base, _) = solid_model();
    let mut transaction = document.transaction("Datums");
    let plane = transaction.add_feature(
        "Plane 1",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Xz),
            rotation: Some(PlaneRotation {
                axis: AxisReference::Edge {
                    body: base,
                    edge: Box::new(EdgeReference::new(
                        EdgeName::from_digest(0xed),
                        [FaceName::from_digest(1), FaceName::from_digest(2)],
                        [VertexName::from_digest(3), VertexName::from_digest(4)],
                    )),
                },
                angle: transaction.parse("depth * 10 deg / 1 mm").unwrap(),
            }),
            offset: transaction.parse("2 mm").unwrap(),
        })),
    );
    let axis = transaction.add_feature(
        "Axis 1",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Intersection(
            PlaneReference::Datum(plane),
            PlaneReference::Principal(PrincipalPlane::Xy),
        ))),
    );
    transaction.add_feature(
        "Axis 2",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Along(AxisReference::Face {
            body: base,
            face: FaceReference::new(FaceName::from_digest(9), None, []),
        }))),
    );
    transaction.add_feature(
        "Axis 3",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Along(AxisReference::Principal(
            PrincipalAxis::Y,
        )))),
    );
    let sketch = transaction.add_feature(
        "On plane",
        FeatureKind::Sketch(SketchFeature::on_datum(
            dimensioned_line(Plane::XZ, 2.0, Expression::Number(2.0)),
            plane,
        )),
    );
    transaction.add_feature(
        "Spun",
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch,
            regions: RegionChoice::All,
            axis: RevolveAxis::Model(AxisReference::Datum(axis)),
            extent: RevolveExtent::Full,
            operation: BodyOperation::NewBody,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    (document, plane, sketch)
}

#[test]
fn datum_planes_and_axes_are_saved_and_loaded() {
    let (document, plane, sketch) = datum_model();
    let text = encode(&document).unwrap();
    assert!(text.contains("\"plane\":{\"base\":{\"principal\":\"xz\"},\"offset\":"));
    assert!(text.contains(
        "\"rotation\":{\"angle\":\"$0 * 10 deg / 1 mm\",\"axis\":{\"edge\":{\"body\":1,"
    ));
    assert!(text.contains("\"axis\":{\"intersection\":[{\"datum\":"));
    assert!(text.contains("\"axis\":{\"along\":{\"principal\":\"y\"}}"));
    assert!(text.contains(&format!("\"datum\":{}", plane.raw())));
    assert!(text.contains("\"axis\":{\"datum\":"));
    let loaded = decode_text(&text);
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);

    let kind = document.feature(sketch).unwrap().kind.clone();
    let FeatureKind::Sketch(placed) = kind else {
        panic!("the sketch is a sketch");
    };
    let transaction = Transaction::single(
        "Place",
        Edit::SetSketchPlacement {
            feature: sketch,
            plane: Plane::XY,
            attachment: placed.attachment,
        },
    );
    assert!(document.check(&transaction).is_ok());
    let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
    let record = through_binary(&text);
    assert_eq!(format::restore_transaction(record), Some(transaction));
}

#[test]
fn a_sketch_on_a_plane_that_is_gone_stays_on_it_as_saved() {
    let (document, plane, sketch) = datum_model();
    let text = encode(&document).unwrap();
    let without_plane: String = text
        .lines()
        .filter(|line| !line.contains("\"name\":\"Plane 1\""))
        .map(|line| format!("{line}\n"))
        .collect();
    let loaded = decode_text(&without_plane);
    assert!(loaded.document.feature(plane).is_none());
    let restored = loaded.document.feature(sketch).unwrap();
    assert_eq!(
        restored.kind.attachment().and_then(SketchAttachment::datum),
        Some(plane)
    );
}

fn only_file_in(dir: &Path) -> PathBuf {
    let mut entries = fs::read_dir(dir).unwrap();
    let path = entries.next().unwrap().unwrap().path();
    assert!(entries.next().is_none());
    path
}

#[test]
fn a_journal_whose_first_change_cannot_be_read_is_kept() {
    let dir = TempDir::new().unwrap();
    let storage = Storage::spawn(config(&dir), untitled(&sample()), || {}).unwrap();
    let mut editor = Editor::new(sample());
    record_session(&storage, &mut editor);
    crash(storage);
    let journal = only_file_in(&dir.path().join("recovery"));
    let original = fs::read(&journal).unwrap();
    let rewritten = rewrite_journal(&original, |index, json| match index {
        2 => json.replace("set_parameter_expression", "bend_sheet"),
        _ => json,
    });
    fs::write(&journal, rewritten).unwrap();

    let Inspection::Recoverable(recovered) = inspect(&journal).unwrap() else {
        panic!("a journal with unread changes should be offered, not removed");
    };
    assert_eq!(recovered.changes(), 0);
    assert!(recovered.issues[0].contains("newer version of caditor"));
    assert!(journal.exists());
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
fn a_model_holding_an_undefined_number_is_refused_rather_than_saved_damaged() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::ZERO, Point2::new(f64::NAN, 1.0));
    let mut document = Document::default();
    let mut transaction = document.transaction("Undefined");
    transaction.add_feature("Broken sketch", FeatureKind::from(sketch));
    document.apply(transaction.finish()).unwrap();

    let error = save(&document, &path, false).unwrap_err();

    assert!(
        error.to_string().contains("infinite or undefined"),
        "{}",
        error.to_string()
    );
    assert!(!path.exists());
}

#[test]
fn a_read_only_model_is_not_replaced_and_says_why() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    save(&Document::default(), &path, false).unwrap();
    let before = fs::read(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();

    let error = save(&sample(), &path, false).unwrap_err();

    assert_eq!(error.to_string(), "the file is read-only");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(files_in(dir.path()), ["model.caditor"]);
}

#[test]
fn orphaned_temporary_files_are_removed_by_the_next_save() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let tag = save::boot_tag();
    let orphan = dir
        .path()
        .join(format!(".model.caditor.{tag}-4294967295-3.tmp"));
    let ours = dir.path().join(format!(
        ".model.caditor.{tag}-{}-999.tmp",
        std::process::id()
    ));
    let other_host = dir
        .path()
        .join(".model.caditor.0123456789abcdef-4294967295-3.tmp");
    let unrelated = dir.path().join(".model.caditor.notes.tmp");
    for file in [&orphan, &ours, &other_host, &unrelated] {
        fs::write(file, "partial").unwrap();
    }

    save(&sample(), &path, false).unwrap();

    assert_eq!(!orphan.exists(), tag != "unknown");
    assert!(ours.exists());
    assert!(other_host.exists());
    assert!(unrelated.exists());
}

#[test]
fn the_scan_sweeps_temporaries_left_by_earlier_boots_of_this_machine() {
    let dir = TempDir::new().unwrap();
    let recovery = dir.path().join("recovery");
    fs::create_dir(&recovery).unwrap();
    let (machine, boot) = (save::machine_tag(), save::boot_tag());
    let earlier_boot = recovery.join(format!(
        ".untitled-1-2.journal.{machine}-0123456789abcdef-4294967295-3.tmp"
    ));
    let ended_run = recovery.join(format!(
        ".untitled-1-2.journal.{machine}-{boot}-4294967295-4.tmp"
    ));
    let running = recovery.join(format!(
        ".untitled-1-2.journal.{machine}-{boot}-{}-5.tmp",
        std::process::id()
    ));
    let other_machine = recovery.join(format!(
        ".untitled-1-2.journal.0123456789abcdef-{boot}-4294967295-6.tmp"
    ));
    let older_format = recovery.join(".untitled-1-2.journal.0123456789abcdef-4294967295-7.tmp");
    let unrelated = recovery.join(".notes.tmp");
    let all = [
        &earlier_boot,
        &ended_run,
        &running,
        &other_machine,
        &older_format,
        &unrelated,
    ];
    for file in all {
        fs::write(file, "partial").unwrap();
    }

    assert!(scan(Some(&recovery), &[]).is_empty());

    let identified = machine != "unknown" && boot != "unknown";
    assert_eq!(!earlier_boot.exists(), identified);
    assert_eq!(!ended_run.exists(), boot != "unknown");
    assert!(running.exists());
    assert!(other_machine.exists());
    assert!(older_format.exists());
    assert!(unrelated.exists());
}

#[test]
fn a_model_whose_name_fills_the_limit_is_saved_backed_up_and_journaled() {
    let dir = TempDir::new().unwrap();
    let name = format!("{}.caditor", "é".repeat(123));
    assert_eq!(name.len(), 254);
    let path = dir.path().join(&name);
    fs::write(&path, "damaged original").unwrap();

    let backup = save(&sample(), &path, true).unwrap().backup.unwrap();
    assert_eq!(fs::read_to_string(&backup).unwrap(), "damaged original");
    assert!(backup.file_name().unwrap().len() <= 255);
    assert!(backup.to_str().unwrap().ends_with(".damaged.caditor"));
    save(&sample(), &path, false).unwrap();
    assert_eq!(load(&path).unwrap().document, sample());

    let start = Start {
        file: Some(path.clone()),
        on_disk: None,
        ..untitled(&sample())
    };
    let storage = Storage::spawn(config(&dir), start, || {}).unwrap();
    let mut editor = Editor::new(sample());
    record_session(&storage, &mut editor);
    assert!(storage.poll().unwrap().is_empty());
    crash(storage);
    let FileJournal::Recoverable(recovered) =
        journal_for(&path, Some(&dir.path().join("recovery")))
    else {
        panic!("the journal should be kept in the recovery folder");
    };
    assert_eq!(recovered.editor.document(), editor.document());
}

#[test]
fn a_recovered_file_remembers_that_it_loaded_with_problems() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let base = sample();
    save(&base, &path, false).unwrap();
    let start = Start {
        file: Some(path.clone()),
        on_disk: None,
        loaded_with_problems: true,
        ..untitled(&base)
    };
    let storage = Storage::spawn(config(&dir), start, || {}).unwrap();
    let mut editor = Editor::new(base);
    record_session(&storage, &mut editor);
    crash(storage);

    let FileJournal::Recoverable(recovered) =
        journal_for(&path, Some(&dir.path().join("recovery")))
    else {
        panic!("unsaved changes should be offered");
    };
    assert!(recovered.loaded_with_problems);
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

#[test]
fn saving_over_a_model_open_in_another_window_is_refused() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let base = sample();
    save(&base, &path, false).unwrap();
    let other = Storage::spawn(
        config(&dir),
        Start {
            file: Some(path.clone()),
            on_disk: None,
            ..untitled(&base)
        },
        || {},
    )
    .unwrap();
    assert!(other.flusher().flush(WAIT));
    let journal = dir.path().join(".model.caditor.journal");
    let before = fs::read(&journal).unwrap();

    let storage = Storage::spawn(config(&dir), untitled(&Document::default()), || {}).unwrap();
    storage
        .save(SaveRequest {
            ticket: 1,
            document: Document::default(),
            path: path.clone(),
            keep_original: false,
            label: None,
            replace_outside_changes: false,
        })
        .unwrap();
    let Report::SaveFailed { reason, .. } = wait_for_report(&storage) else {
        panic!("the save should be refused");
    };
    assert_eq!(reason, "it is open in another caditor window");
    assert_eq!(load(&path).unwrap().document, base);
    assert_eq!(fs::read(&journal).unwrap(), before);
    crash(storage);
    crash(other);
}

#[test]
fn features_that_share_a_name_are_loaded_under_distinct_names() {
    let mut lines = lines_of(&sample());
    let side = lines
        .iter_mut()
        .find(|line| line.contains("\"Side sketch\""))
        .unwrap();
    *side = side.replace("\"Side sketch\"", "\"Base sketch\"");
    let loaded = decode_lines(&lines);
    let names: Vec<&str> = loaded
        .document
        .features()
        .map(|feature| feature.name.as_str())
        .collect();
    assert_eq!(names, ["Base sketch", "Base sketch 2"]);
    assert_eq!(
        loaded.issues,
        ["Two features were named “Base sketch”, so one of them is now “Base sketch 2”."]
    );
}

#[test]
fn feature_names_padded_with_spaces_are_loaded_trimmed() {
    let mut lines = lines_of(&sample());

    let side = lines
        .iter_mut()
        .find(|line| line.contains("\"Side sketch\""))
        .unwrap();
    *side = side.replace("\"Side sketch\"", "\" Base sketch \"");
    let loaded = decode_lines(&lines);
    let names: Vec<&str> = loaded
        .document
        .features()
        .map(|feature| feature.name.as_str())
        .collect();

    assert_eq!(names, ["Base sketch", "Base sketch 2"]);
}

#[test]
fn a_revolve_whose_axis_line_is_gone_loads_turning_about_the_vertical_axis() {
    let mut document = Document::default();

    let mut section = Sketch::new(Plane::XZ);
    let pivot = section.add_line(Point2::new(0.0, 0.0), Point2::new(0.0, 3.0));
    section.add_line(Point2::new(2.0, 0.0), Point2::new(4.0, 3.0));
    let mut transaction = document.transaction("Build");
    let section = transaction.add_feature("Section", FeatureKind::from(section));
    let ring = transaction.add_feature(
        "Ring",
        FeatureKind::Solid(caditor_document::SolidFeature::Revolve(
            caditor_document::Revolve {
                sketch: section,
                regions: caditor_document::RegionChoice::All,
                axis: caditor_document::RevolveAxis::Sketch(pivot),
                extent: caditor_document::RevolveExtent::Full,
                operation: caditor_document::BodyOperation::NewBody,
            },
        )),
    );
    document.apply(transaction.finish()).unwrap();
    let mut lines = lines_of(&document);
    let axis = format!("\"axis\":{},", pivot.raw());
    let revolve = lines.iter_mut().find(|line| line.contains(&axis)).unwrap();
    *revolve = revolve.replace(&axis, "\"axis\":999,");
    let loaded = decode_lines(&lines);
    let axis = loaded
        .document
        .feature(ring)
        .and_then(|feature| feature.kind.solid())
        .and_then(|solid| solid.axis_line());

    assert_eq!(axis, Some(EntityId::VERTICAL_AXIS));
    assert_eq!(
        loaded.issues,
        [
            "“Ring” turned about a line of its sketch that could not be restored, so it now turns \
             about the sketch's vertical axis."
        ]
    );
}

mod seeds;

fn parameter_line(id: usize, expression: &str) -> String {
    format!(r#"{{"parameter":{{"expression":"{expression}","id":{id},"name":"p"}}}}"#)
}

fn empty_sketch_line(id: usize, name: &str) -> String {
    format!(
        r#"{{"feature":{{"id":{id},"name":"{name}","sketch":{{"constraints":[],"entities":[],"next_id":0,"plane":{{"normal":[0.0,0.0,1.0],"origin":[0.0,0.0,0.0],"x_axis":[1.0,0.0,0.0]}}}}}}}}"#
    )
}

#[test]
fn many_repeated_names_cycles_and_duplicate_ids_load_quickly() {
    let half = MAX_RECORDS / 2;
    let mut lines: Vec<String> = (0..half)
        .map(|id| parameter_line(id, &format!("${id} + 1 mm")))
        .collect();
    lines.extend((0..half).map(|index| empty_sketch_line(index % 2, "F")));

    let started = Instant::now();
    let loaded = decode_lines(&lines);

    assert!(
        started.elapsed() < Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
    let document = &loaded.document;
    assert_eq!(document.parameters().len(), half);
    assert!(document.parameter_named("parameter_9").is_some());
    let names: Vec<&str> = document.features().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["F", "F 2"]);
    assert!(issues_mention(
        &loaded,
        "“parameter_7” depended on itself (parameter_7 → parameter_7)"
    ));
    assert!(issues_mention(&loaded, "Two features share the ID 1"));
}

#[test]
fn records_beyond_the_limit_are_left_out_and_reported() {
    let lines: Vec<String> = (0..MAX_RECORDS + 3)
        .map(|id| parameter_line(id, "1 mm"))
        .collect();
    let loaded = decode_lines(&lines);
    assert_eq!(loaded.document.parameters().len(), MAX_RECORDS);
    assert!(issues_mention(
        &loaded,
        "The model holds more parameters and features than caditor loads (10000), so the last 3 \
         were left out."
    ));
}

#[test]
fn a_journal_larger_than_a_chunk_replays_whole() {
    let base = sample();
    let change = edit_width(&base, "50 mm");
    let entries = [JournalEntry::Apply(change)];
    let bytes = binary::testing::with_slices_of(64, || {
        journal::encode_journal(None, None, false, &base, &entries).unwrap()
    });

    let contents = journal::decode_journal(&bytes).unwrap();
    assert_eq!(contents.issues, Vec::<String>::new());
    assert_eq!(contents.base, base);
    assert_eq!(contents.entries, entries);
    assert_eq!(contents.unreadable_entries, 0);
}

fn suppressed_and_rolled_back() -> (Document, FeatureId, FeatureId, Transaction) {
    let (mut document, base, turned) = solid_model();
    let change = Transaction::new(
        "Suppress and roll back",
        vec![
            Edit::SetFeatureSuppressed {
                id: base,
                suppressed: true,
            },
            Edit::SetRollbackBar {
                bar: RollbackBar::Before(turned),
            },
        ],
    );
    document.apply(change.clone()).unwrap();
    (document, base, turned, change)
}

#[test]
fn suppressed_features_and_the_rollback_bar_stay_through_saving_the_journal_and_its_snapshot() {
    let (plain, _, _) = solid_model();
    let (document, base, turned, change) = suppressed_and_rolled_back();
    let plain_text = encode(&plain).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&change)).unwrap());
    let recovered = journal::decode_journal(
        &journal::encode_journal(None, None, false, &document, &[]).unwrap(),
    )
    .unwrap();

    assert!(!plain_text.contains("suppressed"));
    assert!(!plain_text.contains("rollback"));
    assert!(text.contains(&format!(
        r#"{{"suppressed":{{"features":[{}]}}}}"#,
        base.raw()
    )));
    assert!(text.contains(&format!(r#"{{"rollback":{{"before":{}}}}}"#, turned.raw())));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(format::restore_transaction(journaled), Some(change));
    assert_eq!(recovered.issues, Vec::<String>::new());
    assert_eq!(recovered.base, document);
}

#[test]
fn a_deleted_suppressed_feature_comes_back_suppressed_from_its_journaled_undo() {
    let (document, base, _, _) = suppressed_and_rolled_back();
    let mut deleted = document.clone();

    let undo = deleted
        .apply(document.deletion(&[base], "Delete Base"))
        .unwrap();
    let record = format::transaction_record(&undo);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&record).unwrap());
    let restored = format::restore_transaction(journaled).unwrap();
    deleted.apply(restored.clone()).unwrap();

    assert_eq!(restored, undo);
    assert!(deleted.feature(base).unwrap().suppressed);
    assert!(deleted.same_content(&document));
}

#[test]
fn suppressing_and_rolling_back_are_recovered_from_the_journal_after_a_crash() {
    let dir = TempDir::new().unwrap();
    let (base_document, base, turned, _) = suppressed_and_rolled_back();
    let storage = Storage::spawn(config(&dir), untitled(&base_document), || {}).unwrap();
    let mut editor = Editor::new(base_document.clone());
    let changes = [
        base_document.suppression(&[base], false, "Unsuppress Base"),
        base_document.roll_to(RollbackBar::AtEnd, "Roll to end"),
    ];
    for change in changes {
        editor.apply(change.clone()).unwrap();
        storage.record(JournalEntry::Apply(change)).unwrap();
    }
    let delete = editor.document().deletion(&[turned], "Delete Turned");
    editor.apply(delete.clone()).unwrap();
    storage.record(JournalEntry::Apply(delete)).unwrap();
    let undo = editor.next_undo().cloned().unwrap();
    editor.undo().unwrap();
    storage.record(JournalEntry::Undo(undo)).unwrap();
    assert!(storage.flusher().flush(WAIT));
    crash(storage);

    let recovered = scan(Some(&dir.path().join("recovery")), &[]).remove(0);

    assert_eq!(recovered.base, base_document);
    assert_eq!(recovered.editor.document(), editor.document());
    assert_eq!(
        recovered.editor.document().rollback_bar(),
        RollbackBar::AtEnd
    );
    assert!(
        !recovered
            .editor
            .document()
            .feature(base)
            .unwrap()
            .suppressed
    );
}

#[test]
fn models_saved_before_format_three_load_with_nothing_suppressed_or_rolled_back() {
    let (document, _, _) = solid_model();
    let lines = lines_of(&document);

    for version in [1, 2] {
        let loaded = decode(&model_from_json(version, &lines)).unwrap();

        assert_eq!(loaded.issues, Vec::<String>::new());
        assert_eq!(loaded.document, document);
        assert_eq!(loaded.document.rollback_bar(), RollbackBar::AtEnd);
        assert!(
            loaded
                .document
                .features()
                .all(|feature| !feature.suppressed)
        );
    }
    let bytes = crate::encode(&document).unwrap();
    assert_eq!(bytes.get(8..12), Some(&3_u32.to_le_bytes()[..]));
}

#[test]
fn a_rollback_bar_above_a_feature_that_is_gone_is_reported_and_left_at_the_end() {
    let (document, _, _) = solid_model();
    let mut lines = lines_of(&document);
    lines.push(r#"{"rollback":{"before":77}}"#.to_owned());
    lines.push(r#"{"suppressed":{"features":[78]}}"#.to_owned());

    let loaded = decode_lines(&lines);

    assert_eq!(
        loaded.issues,
        [
            "The rollback bar stood above a feature that could not be restored, so it is at the \
             end of the tree and every feature is computed."
        ]
    );
    assert_eq!(loaded.document.rollback_bar(), RollbackBar::AtEnd);
    assert!(loaded.document.same_content(&document));
}

fn extents_model() -> (Document, [FeatureId; 4]) {
    use caditor_document::{
        BodyOperation, Datum, DatumPlane, Extrude, ExtrudeEnd, ExtrudeExtent, FaceAttachment,
        PlaneReference, PrincipalPlane, RegionChoice, Revolve, RevolveAxis, RevolveExtent,
        SolidFeature,
    };
    use caditor_kernel::{FaceName, FaceOrigin, FaceReference};
    let (mut document, base, _) = solid_model();
    let sketch = document.features().next().unwrap().id();
    let mut transaction = document.transaction("Extents");
    let level = transaction.add_feature(
        "Level",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Xy),
            rotation: None,
            offset: transaction.parse("20 mm").unwrap(),
        })),
    );
    let extrude = |extent| {
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch,
            regions: RegionChoice::All,
            extent,
            operation: BodyOperation::Remove(base),
        }))
    };
    let through = transaction.add_feature(
        "Through",
        extrude(ExtrudeExtent::OneSide {
            end: ExtrudeEnd::ThroughAll,
            reversed: true,
        }),
    );
    let between = transaction.add_feature(
        "Between",
        extrude(ExtrudeExtent::TwoSides {
            forward: ExtrudeEnd::UpToFace(PlaneReference::Face(FaceAttachment {
                body: base,
                face: FaceReference::new(
                    FaceName::from_digest(0xface),
                    Some(FaceOrigin::EndCap {
                        feature: base.raw(),
                    }),
                    [FaceName::from_digest(7)],
                ),
            })),
            backward: ExtrudeEnd::UpToFace(PlaneReference::Datum(level)),
        }),
    );
    let next = transaction.add_feature(
        "Next",
        extrude(ExtrudeExtent::TwoSides {
            forward: ExtrudeEnd::UpToNext,
            backward: ExtrudeEnd::Distance(transaction.parse("depth / 2").unwrap()),
        }),
    );
    let turned = transaction.add_feature(
        "Two angles",
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch,
            regions: RegionChoice::All,
            axis: RevolveAxis::Sketch(EntityId::HORIZONTAL_AXIS),
            extent: RevolveExtent::TwoSides {
                forward: transaction.parse("30 deg").unwrap(),
                backward: transaction.parse("45 deg").unwrap(),
            },
            operation: BodyOperation::Remove(base),
        })),
    );
    document.apply(transaction.finish()).unwrap();
    (document, [through, between, next, turned])
}

#[test]
fn extents_to_faces_planes_and_the_next_face_and_two_angles_are_saved_and_loaded() {
    let (document, features) = extents_model();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);

    assert!(text.contains("\"extrude\":{\"extent\":{\"two_sides\""));
    assert!(text.contains("\"extrude_to\":{\"extent\":{\"one_side\":{\"end\":\"through_all\""));
    assert!(text.contains("\"forward\":{\"up_to_face\":{\"face\":{\"body\":1,"));
    assert!(text.contains("\"backward\":{\"up_to_face\":{\"datum\":"));
    assert!(text.contains("\"forward\":\"up_to_next\""));
    assert!(text.contains("\"backward\":{\"distance\":\"$0 / 2\"}"));
    assert!(text.contains("\"revolve_two_angles\":{\"axis\":"));
    assert!(text.contains("\"forward\":\"30 deg\""));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    for feature in features {
        let kind = document.feature(feature).unwrap().kind.clone();
        let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id: feature, kind });
        let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
        let record = through_binary(&text);
        assert_eq!(format::restore_transaction(record), Some(transaction));
    }
}

#[test]
fn unreadable_ends_and_angles_fall_back_and_are_reported() {
    let (document, [_, between, next, turned]) = extents_model();
    let text = encode(&document)
        .unwrap()
        .replacen("0000000000000000000000000000face", "not a digest", 1)
        .replacen(
            "\"forward\":\"up_to_next\"",
            "\"forward\":\"up_to_vertex\"",
            1,
        )
        .replacen("\"forward\":\"30 deg\"", "\"forward\":\"30 ((\"", 1);

    let loaded = decode_text(&text);

    assert_eq!(
        loaded.issues,
        [
            "The face or plane that the forward end of “Between” runs up to could not be read, \
             so that end was set to 10 mm.",
            "The forward end of “Next” could not be read, so it was set to 10 mm.",
            "The forward angle of “Two angles” could not be read, so it was set to 180 deg.",
        ]
    );
    let ten = caditor_document::ExtrudeEnd::Distance(Expression::Measure(10.0, Unit::Millimetre));
    for feature in [between, next] {
        let caditor_document::SolidFeature::Extrude(extrude) = loaded
            .document
            .feature(feature)
            .unwrap()
            .kind
            .solid()
            .unwrap()
        else {
            panic!("an extrusion stays an extrusion");
        };
        let caditor_document::ExtrudeExtent::TwoSides { forward, .. } = &extrude.extent else {
            panic!("two sides stay two sides");
        };
        assert_eq!(forward, &ten);
    }
    assert!(loaded.document.feature(turned).is_some());
}

#[test]
fn a_model_saved_in_format_2_loads_unchanged() {
    let (document, _, _) = patterned_model();
    let lines = lines_of(&document);
    let loaded = decode(&model_from_json(2, &lines)).unwrap();
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert!(lines.iter().all(|line| !line.contains("extrude_to")));
}
