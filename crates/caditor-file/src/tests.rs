use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use caditor_document::{
    CancelToken, Document, Edit, Editor, FaceAttachment, FeatureId, FeatureKind, PlaneReference,
    PrincipalPlane, RollbackBar, SketchAttachment, Transaction,
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
        ..StorageConfig::default()
    }
}

fn untitled(base: &Document) -> Start {
    Start {
        file: None,
        on_disk: None,
        loaded_with_problems: false,
        base: base.clone(),
        folded: 0,
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
fn a_cancelled_load_stops_with_nothing_and_an_uncancelled_one_loads_the_model() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let document = sample();
    save(&document, &path, false).unwrap();

    let cancelled = load_cancellable(&path, &CancelToken::new(|| true));
    let loaded = load_cancellable(&path, &CancelToken::never()).unwrap();

    assert_eq!(cancelled, Err(LoadError::Cancelled));
    assert_eq!(loaded.document, document);
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
fn a_failed_save_reports_a_plain_reason_and_leaves_the_folder_clean() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("missing").join("model.caditor");
    let error = save(&sample(), &path, false).unwrap_err();
    assert_eq!(error.to_string(), "its folder no longer exists");
    assert_eq!(files_in(dir.path()), Vec::<String>::new());
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

const DAMAGED_START: &str = "The start of the file is damaged, so it no longer reads as a \
                             caditor model. Its parts still pass their checks and were loaded \
                             from them; saving writes a sound start again.";

#[test]
fn a_model_whose_magic_is_damaged_loads_from_its_checked_parts() {
    let bytes = crate::encode(&sample()).unwrap();

    for index in 0..binary::MODEL_MAGIC.len() {
        let mut flipped = bytes.clone();
        flipped[index] ^= 0x20;
        let loaded = decode(&flipped).unwrap();

        assert_eq!(loaded.issues, [DAMAGED_START], "byte {index}");
        assert_eq!(loaded.document, sample());
    }

    let mut shortened = bytes.clone();
    shortened.remove(3);
    let mut lengthened = bytes.clone();
    lengthened.insert(5, b'?');

    assert_eq!(decode(&shortened).unwrap().document, sample());
    assert_eq!(decode(&lengthened).unwrap().document, sample());
}

#[test]
fn a_damaged_magic_is_not_salvaged_without_an_intact_head_after_it() {
    let model = crate::encode(&sample()).unwrap();
    let mut journal = binary::start_file(&binary::JOURNAL_MAGIC, 1);
    binary::push_packed(&mut journal, binary::ChunkKind::JournalHeader, b"header").unwrap();
    journal[2] = b'X';
    let mut garbled = corrupt_chunk(&model, &binary::MODEL_MAGIC, 0);
    garbled[0] = b'#';

    assert_eq!(decode(&journal), Err(LoadError::NotAModel));
    assert_eq!(decode(&garbled), Err(LoadError::NotAModel));
    assert_eq!(
        decode(b"CDCK is how caditor marks its chunks"),
        Err(LoadError::NotAModel)
    );
}

#[test]
fn saving_over_a_model_with_a_damaged_magic_keeps_it_and_its_versions() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    save(&Document::default(), &path, false).unwrap();
    save(&sample(), &path, false).unwrap();
    let mut damaged = fs::read(&path).unwrap();
    damaged[1] = b'X';
    fs::write(&path, &damaged).unwrap();

    let backup = save(&sample(), &path, false).unwrap().backup.unwrap();

    assert_eq!(fs::read(&backup).unwrap(), damaged);
    assert!(load(&path).unwrap().issues.is_empty());
    assert_eq!(crate::history(&path).unwrap().versions.len(), 1);
    assert_eq!(
        load_version(&path, 0).unwrap().document,
        Document::default()
    );
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

fn crashed_then_set_aside(dir: &TempDir, path: &Path, base: &Document) -> PathBuf {
    save(base, path, false).unwrap();
    let start = Start {
        file: Some(path.to_path_buf()),
        on_disk: None,
        ..untitled(base)
    };
    let storage = Storage::spawn(config(dir), start, || {}).unwrap();
    let mut editor = Editor::new(base.clone());
    record_session(&storage, &mut editor);
    crash(storage);
    let journal = dir.path().join(".model.caditor.journal");
    let set_aside = paths::unreadable_journal(&journal);
    fs::rename(&journal, &set_aside).unwrap();
    set_aside
}

#[test]
fn a_set_aside_journal_this_version_reads_is_offered_for_restoring() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let recovery = dir.path().join("recovery");
    let set_aside = crashed_then_set_aside(&dir, &path, &sample());

    let scanned = scan(Some(&recovery), std::slice::from_ref(&path));
    let FileJournal::Recoverable(opened) = journal_for(&path, Some(&recovery)) else {
        panic!("the set-aside journal reads, so it should be offered");
    };

    assert_eq!(scanned.len(), 1);
    assert_eq!(scanned[0].journal, set_aside);
    assert_eq!(scanned[0].file.as_deref(), Some(path.as_path()));
    assert_eq!(scanned[0].changes(), 3);
    assert_eq!(opened.journal, set_aside);
    assert_eq!(opened.editor.document(), scanned[0].editor.document());
}

#[test]
fn restoring_a_set_aside_journal_moves_it_back_beside_its_model() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let recovery = dir.path().join("recovery");
    let set_aside = crashed_then_set_aside(&dir, &path, &sample());
    let recovered = scan(Some(&recovery), std::slice::from_ref(&path)).remove(0);

    let start = Start {
        file: recovered.file.clone(),
        on_disk: recovered.on_disk.clone(),
        loaded_with_problems: recovered.loaded_with_problems,
        base: recovered.base.clone(),
        folded: recovered.folded,
        entries: recovered.entries.clone(),
        replaces: Some(recovered.journal.clone()),
        after: None,
    };
    let storage = Storage::spawn(config(&dir), start, || {}).unwrap();
    assert!(storage.flusher().flush(WAIT));
    crash(storage);

    assert!(!set_aside.exists());
    let again = scan(Some(&recovery), std::slice::from_ref(&path));
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].journal, dir.path().join(".model.caditor.journal"));
    assert_eq!(again[0].editor.document(), recovered.editor.document());
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
    assert_eq!(read::read_open(&theirs).unwrap(), b"theirs");
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
        folded: recovered.folded,
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
        curve: line,
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
fn a_curvature_between_joined_splines_round_trips_and_older_readers_report_it() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Splines");
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_spline(&[
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 0.0),
    ]);
    let second = sketch.add_spline(&[
        Point2::new(20.0, 0.0),
        Point2::new(25.0, -5.0),
        Point2::new(30.0, -15.0),
    ]);
    let ends = [first, second].map(|spline| match sketch.entity(spline) {
        Some(caditor_sketch::Entity::Spline { control_points }) => control_points.clone(),
        _ => panic!("expected a spline"),
    });
    sketch
        .add_constraint(Constraint::Coincident(ends[1][0], ends[0][2]))
        .unwrap();
    sketch
        .add_constraint(Constraint::Tangent(first, second))
        .unwrap();
    sketch
        .add_constraint(Constraint::Curvature(first, second))
        .unwrap();
    transaction.add_feature("Smooth", FeatureKind::from(sketch));
    document.apply(transaction.finish()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let older = decode_text(&text.replace("\"curvature\"", "\"curling\""));

    assert!(
        text.contains(&format!("\"curvature\":[{},{}]", first.raw(), second.raw())),
        "{text}"
    );
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert!(!older.issues.is_empty());
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
fn an_import_keeps_the_path_it_was_read_from_only_when_known() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Import");
    let kept = caditor_document::Import::new("cube.step", caditor_kernel::Solid::default(), "")
        .from_file(std::path::PathBuf::from("/models/cube.step"));
    let unknown = caditor_document::Import::new("old.step", caditor_kernel::Solid::default(), "");
    let first = transaction.add_feature("Cube", FeatureKind::Import(kept));
    let second = transaction.add_feature("Old", FeatureKind::Import(unknown));
    document.apply(transaction.finish()).unwrap();

    let text = encode(&document).unwrap();
    assert!(text.contains("\"path\":\"/models/cube.step\""), "{text}");
    assert_eq!(text.matches("\"path\"").count(), 1);

    let loaded = decode_text(&text);
    let path_of = |feature| {
        loaded
            .document
            .feature(feature)
            .and_then(|feature| feature.kind.import())
            .and_then(|import| import.path.clone())
    };
    assert_eq!(
        path_of(first),
        Some(std::path::PathBuf::from("/models/cube.step"))
    );
    assert_eq!(path_of(second), None);
}

#[test]
fn placed_copies_of_one_part_keep_its_step_text_once_and_share_it_when_loaded() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Import");
    let text: std::sync::Arc<str> = std::sync::Arc::from("ISO-10303-21; part");
    let copy = |x: f64| {
        let mut placement = caditor_document::BodyPlacement::default();
        placement.offset[0] = Expression::Measure(x, Unit::Millimetre);
        caditor_document::Import::new("pin.step", caditor_kernel::Solid::default(), text.clone())
            .placed(placement)
    };
    let at_origin =
        caditor_document::Import::new("pin.step", caditor_kernel::Solid::default(), text.clone());
    transaction.add_feature("Pin", FeatureKind::Import(copy(10.0)));
    transaction.add_feature("Pin 2", FeatureKind::Import(copy(20.0)));
    transaction.add_feature("Pin 3", FeatureKind::Import(at_origin));
    transaction.add_feature("Pin 4", FeatureKind::Import(copy(30.0)));
    document.apply(transaction.finish()).unwrap();

    let encoded = encode(&document).unwrap();
    let loaded = decode_text(&encoded);
    let lost = decode_text(&encoded.replacen("\"step\":\"ISO-10303-21; part\",", "", 1));
    let held: Vec<_> = loaded
        .document
        .features()
        .filter_map(|feature| feature.kind.import())
        .collect();

    assert_eq!(
        encoded.matches("ISO-10303-21; part").count(),
        2,
        "{encoded}"
    );
    assert_eq!(encoded.matches("\"shares\":").count(), 2, "{encoded}");
    assert_eq!(loaded.document, document);
    assert!(
        held.windows(2)
            .all(|pair| std::sync::Arc::ptr_eq(&pair[0].step, &pair[1].step))
    );
    assert!(
        lost.issues
            .iter()
            .any(|issue| issue.contains("“Pin 2”, imported from “pin.step”, was kept with another")),
        "{:?}",
        lost.issues
    );
}

#[test]
fn a_placed_import_is_a_record_of_its_own_and_an_unplaced_one_stays_readable_by_older_versions() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Import");
    let shift = transaction.add_parameter("shift", transaction.parse("4 mm").unwrap());
    let mut placement = caditor_document::BodyPlacement::default();
    placement.offset[2] = Expression::Parameter(shift);
    placement.turn[0] = Expression::Measure(30.0, Unit::Degree);
    let placed = caditor_document::Import::new("cube.step", caditor_kernel::Solid::default(), "")
        .placed(placement);
    let unplaced = caditor_document::Import::new("old.step", caditor_kernel::Solid::default(), "");
    let first = transaction.add_feature("Cube", FeatureKind::Import(placed.clone()));
    transaction.add_feature("Old", FeatureKind::Import(unplaced));
    document.apply(transaction.finish()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let damaged = decode_text(
        &text
            .replace("\"30 deg\"", "\"30 ((\"")
            .replace("\"$0\"", "\"$0 ((\""),
    );
    let placement_of = |loaded: &Loaded| {
        loaded
            .document
            .feature(first)
            .and_then(|feature| feature.kind.import())
            .map(|import| import.placement.clone())
    };

    assert!(
        text.contains(
            "\"placed_import\":{\"offset\":[\"0 mm\",\"0 mm\",\"$0\"],\"source\":\"cube.step\",\
             \"step\":\"\",\"turn\":[\"30 deg\",\"0 deg\",\"0 deg\"]}"
        ),
        "{text}"
    );
    assert!(
        text.contains("\"import\":{\"source\":\"old.step\""),
        "{text}"
    );
    assert!(
        loaded
            .issues
            .iter()
            .all(|issue| issue.contains("it is not a STEP file")),
        "{:?}",
        loaded.issues
    );
    assert_eq!(loaded.document, document);
    assert_eq!(
        placement_of(&damaged),
        Some(caditor_document::BodyPlacement::default())
    );
    assert_eq!(damaged.issues.len(), loaded.issues.len() + 2);
    assert!(
        damaged.issues.iter().any(|issue| issue
            == "The placement distance of “Cube” could not be read, so it was set to 0 mm."),
        "{:?}",
        damaged.issues
    );
    assert!(
        damaged.issues.iter().any(|issue| issue
            == "The placement turn of “Cube” could not be read, so it was set to 0 deg."),
        "{:?}",
        damaged.issues
    );
}

#[test]
fn arc_dimensions_round_trip_and_fall_back_to_their_drawn_values() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Arc dimensions");
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_arc(
        Point2::new(50.0, 5.0),
        Point2::new(58.0, 5.0),
        Point2::new(50.0, 13.0),
    );
    sketch
        .add_constraint(Constraint::ArcLength {
            arc,
            value: Expression::Measure(12.0, Unit::Millimetre),
        })
        .unwrap();
    sketch
        .add_constraint(Constraint::Sweep {
            arc,
            value: Expression::Measure(80.0, Unit::Degree),
        })
        .unwrap();
    transaction.add_feature("Arc dimensions", FeatureKind::from(sketch));
    document.apply(transaction.finish()).unwrap();

    let text = encode(&document).unwrap();
    for record in [
        "\"arc_length\":{\"arc\":3,\"value\":\"12 mm\"}",
        "\"sweep\":{\"arc\":3,\"value\":\"80 deg\"}",
    ] {
        assert!(text.contains(record), "{record} is missing from {text}");
    }
    let loaded = decode_text(&text);
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);

    let damaged = text
        .replace("\"value\":\"12 mm\"", "\"value\":\"12 ((\"")
        .replace("\"value\":\"80 deg\"", "\"value\":\"80 ((\"");
    let loaded = decode_text(&damaged);
    assert_eq!(
        loaded.issues,
        [
            "In “Arc dimensions”, the value of an arc length could not be read, so it was set to \
             its drawn arc length, 12.566371 mm.",
            "In “Arc dimensions”, the value of a sweep could not be read, so it was set to its \
             drawn sweep, 90°.",
        ]
    );
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
            regions: RegionChoice::Chosen(vec![caditor_kernel::RegionReference::new(
                caditor_kernel::RegionKey::from_digest(0x0123_4567_89ab_cdef_0011_2233_4455_6677),
                BTreeSet::from([caditor_kernel::BoundaryPiece {
                    entity: 3,
                    side: caditor_kernel::Side::Right,
                    piece: 0x7766_5544_3322_1100_fedc_ba98_7654_3210,
                }]),
                Some(Point2::new(2.5, 1.25)),
            )]),
            extent: ExtrudeExtent::two_sides(
                Expression::Parameter(depth),
                transaction.parse("1 mm").unwrap(),
            ),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
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
            start: None,
            other_bodies: Vec::new(),
            side: None,
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
fn a_cut_through_several_bodies_is_saved_as_its_own_record_kind_and_loaded() {
    use caditor_document::{BodyOperation, Extrude, ExtrudeExtent, RegionChoice, SolidFeature};
    let (mut document, base, _) = solid_model();
    let sketch = document
        .features()
        .find(|feature| feature.name == "Outline")
        .map(caditor_document::Feature::id)
        .unwrap();
    let mut transaction = document.transaction("Cut both");
    let extrusion = |operation, other_bodies| {
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("2 mm").unwrap(), false),
            operation,
            start: None,
            other_bodies,
        }))
    };
    let second = transaction.add_feature("Second", extrusion(BodyOperation::NewBody, Vec::new()));
    let cut = transaction.add_feature("Cut", extrusion(BodyOperation::Remove(base), vec![second]));
    document.apply(transaction.finish()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);

    assert!(text.contains(&format!(
        "\"cut_several\":{{\"bodies\":[{}],\"feature\":{{\"extrude\":",
        second.raw()
    )));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);

    let kind = document.feature(cut).unwrap().kind.clone();
    let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id: cut, kind });
    let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
    let record = through_binary(&text);
    assert_eq!(format::restore_transaction(record), Some(transaction));
}

fn without_region_references(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            fields.remove("region_references");
            fields.values_mut().for_each(without_region_references);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(without_region_references),
        _ => {}
    }
}

#[test]
fn chosen_regions_saved_without_references_load_by_their_keys() {
    let (document, base, _) = solid_model();
    let text = encode(&document).unwrap();
    let stripped: Vec<String> = lines_of(&document)
        .iter()
        .map(|line| {
            let mut record: serde_json::Value = serde_json::from_str(line).unwrap();
            without_region_references(&mut record);
            record.to_string()
        })
        .collect();

    let loaded = decode_lines(&stripped);

    assert!(text.contains("\"region_references\""));
    assert!(text.contains("\"anchor\":[2.5,1.25]"));
    assert_eq!(loaded.issues, Vec::<String>::new());
    let FeatureKind::Solid(solid) = &loaded.document.feature(base).unwrap().kind else {
        panic!("Base should load as a solid feature");
    };
    assert_eq!(
        solid.regions(),
        &caditor_document::RegionChoice::Chosen(vec![caditor_kernel::RegionReference::of_key(
            caditor_kernel::RegionKey::from_digest(0x0123_4567_89ab_cdef_0011_2233_4455_6677),
        )])
    );
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
fn a_parameter_note_and_order_survive_saving_and_the_journal() {
    let mut document = sample();
    let plain = encode(&document).unwrap();
    let height = document.parameter_named("height").unwrap().id();
    let change = Transaction::new(
        "Note and move height",
        vec![
            Edit::SetParameterNote {
                id: height,
                note: "Half the width, plus clearance".to_owned(),
            },
            Edit::MoveParameter {
                id: height,
                index: 0,
            },
        ],
    );
    let undo = document.apply(change.clone()).unwrap();
    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&change)).unwrap());
    let undone: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&undo)).unwrap());
    let order: Vec<&str> = loaded
        .document
        .parameters()
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect();

    assert!(!plain.contains("note"));
    assert!(
        text.contains("\"note\":\"Half the width, plus clearance\""),
        "{text}"
    );
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(order, ["height", "width"]);
    assert_eq!(loaded.document, document);
    assert_eq!(format::restore_transaction(journaled), Some(change));
    assert_eq!(format::restore_transaction(undone), Some(undo));
}

#[test]
fn an_overlong_parameter_note_is_cut_and_reported() {
    let document = sample();
    let long = "n".repeat(caditor_document::MAX_PARAMETER_NOTE_CHARS + 5);
    let text = encode(&document).unwrap().replacen(
        "\"expression\":\"40 mm\"",
        &format!("\"expression\":\"40 mm\",\"note\":\"{long}\""),
        1,
    );

    let loaded = decode_text(&text);

    assert!(issues_mention(
        &loaded,
        "The note on “width” was longer than"
    ));
    assert_eq!(
        loaded
            .document
            .parameter_named("width")
            .unwrap()
            .note
            .chars()
            .count(),
        caditor_document::MAX_PARAMETER_NOTE_CHARS
    );
}

fn bracket_properties() -> caditor_document::ModelProperties {
    caditor_document::ModelProperties {
        title: "Wall bracket".to_owned(),
        part_number: "BR-100".to_owned(),
        revision: "C".to_owned(),
        organisation: "Workshop".to_owned(),
        notes: "Print with 40% infill.\nCountersink by hand.".to_owned(),
        ..caditor_document::ModelProperties::default()
    }
}

fn named_dimension(document: &Document) -> Transaction {
    let base = document.features().next().unwrap();
    let sketch = base.kind.sketch().unwrap();
    let (constraint, _) = sketch
        .constraints()
        .find(|(_, constraint)| constraint.dimension().is_some())
        .unwrap();
    let mut transaction = document.transaction("Name length");
    let length = transaction.add_owned_parameter(
        "length",
        document.parse("width - 1 mm").unwrap(),
        caditor_document::ParameterOwner::Dimension {
            sketch: base.id(),
            constraint,
        },
    );
    transaction.edit(Edit::SetDimension {
        feature: base.id(),
        constraint,
        value: Expression::Parameter(length),
    });
    transaction.finish()
}

#[test]
fn named_values_survive_saving_the_journal_and_its_snapshot_in_a_record_of_their_own() {
    let mut document = sample();
    let plain = encode(&document).unwrap();
    let naming = named_dimension(&document);
    let undo = document.apply(naming.clone()).unwrap();
    let length = document.parameter_named("length").unwrap().id();
    let release = Transaction::single(
        "Release",
        Edit::SetParameterOwner {
            id: length,
            owner: None,
        },
    );

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&naming)).unwrap());
    let released: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&release)).unwrap());
    let recovered = journal::decode_journal(
        &journal::encode_journal(
            &journal::JournalHead {
                file: None,
                on_disk: None,
                loaded_with_problems: false,
                folded: 0,
            },
            &document,
            &[],
        )
        .unwrap(),
    )
    .unwrap();
    let parameter_line = text
        .lines()
        .find(|line| line.contains("\"length\""))
        .unwrap();

    assert!(!plain.contains("named_values"));
    assert!(!parameter_line.contains("owner"), "{parameter_line}");
    assert!(
        text.contains(r#"{"named_values":{"values":[{"owner":{"dimension":{"constraint":"#),
        "{text}"
    );
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(format::restore_transaction(journaled), Some(naming));
    assert_eq!(format::restore_transaction(released), Some(release));
    assert!(
        undo.edits()
            .iter()
            .any(|edit| matches!(edit, Edit::RemoveParameter { .. }))
    );
    assert_eq!(recovered.issues, Vec::<String>::new());
    assert_eq!(recovered.base, document);
}

#[test]
fn an_unreadable_named_value_leaves_its_parameter_listed_with_the_others() {
    let document = sample();
    let width = document.parameter_named("width").unwrap().id();
    let long = "v".repeat(caditor_document::MAX_VALUE_LABEL_CHARS + 5);
    let text = format!(
        "{}\n{{\"named_values\":{{\"values\":[{{\"parameter\":{width},\"owner\":{{\"feature\":{{\"feature\":0,\"value\":\"{long}\"}}}}}},{{\"parameter\":{width},\"owner\":\"lost\"}},{{\"parameter\":999,\"owner\":{{\"feature\":{{\"feature\":0,\"value\":\"Gone\"}}}}}}]}}}}",
        encode(&document).unwrap()
    );

    let loaded = decode_text(&text);
    let owner = loaded.document.parameter(width).unwrap().owner.clone();

    assert_eq!(
        owner,
        Some(caditor_document::ParameterOwner::Feature {
            feature: FeatureId::from_raw(0),
            value: "v".repeat(caditor_document::MAX_VALUE_LABEL_CHARS),
        })
    );
    assert!(issues_mention(
        &loaded,
        "was longer than this version keeps"
    ));
    assert!(issues_mention(
        &loaded,
        "Which dimension or feature value a model parameter names could not be read"
    ));
    assert_eq!(loaded.issues.len(), 2, "{:?}", loaded.issues);
}

#[test]
fn model_properties_survive_saving_the_journal_and_its_snapshot() {
    let mut document = sample();
    let plain = encode(&document).unwrap();
    let change = Transaction::single(
        "Model properties",
        Edit::SetModelProperties {
            properties: Box::new(bracket_properties()),
        },
    );
    let undo = document.apply(change.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&change)).unwrap());
    let undone: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&undo)).unwrap());
    let recovered = journal::decode_journal(
        &journal::encode_journal(
            &journal::JournalHead {
                file: None,
                on_disk: None,
                loaded_with_problems: false,
                folded: 0,
            },
            &document,
            &[],
        )
        .unwrap(),
    )
    .unwrap();

    assert!(!plain.contains("properties"));
    assert!(
        text.contains(r#"{"properties":{"notes":"Print with 40% infill.\nCountersink by hand.","organisation":"Workshop","part_number":"BR-100","revision":"C","title":"Wall bracket"}}"#),
        "{text}"
    );
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(format::restore_transaction(journaled), Some(change));
    assert_eq!(format::restore_transaction(undone), Some(undo));
    assert_eq!(recovered.issues, Vec::<String>::new());
    assert_eq!(recovered.base, document);
}

#[test]
fn an_overlong_model_property_is_cut_and_reported() {
    let document = sample();
    let long = "t".repeat(caditor_document::MAX_PROPERTY_CHARS + 3);
    let text = format!(
        "{}\n{{\"properties\":{{\"title\":\"{long}\",\"revision\":\"B\"}}}}",
        encode(&document).unwrap()
    );

    let loaded = decode_text(&text);

    assert!(issues_mention(
        &loaded,
        "The model's title was longer than 200 characters"
    ));
    assert_eq!(
        loaded.document.properties().title.chars().count(),
        caditor_document::MAX_PROPERTY_CHARS
    );
    assert_eq!(loaded.document.properties().revision, "B");
}

#[test]
fn a_removal_is_saved_loaded_and_journaled() {
    let (mut document, base, _) = solid_model();
    let mut transaction = document.transaction("Remove");
    transaction.add_feature(
        "Remove 1",
        FeatureKind::Remove(caditor_document::Remove { body: base }),
    );
    let add = transaction.finish();
    document.apply(add.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&add)).unwrap());

    assert!(
        text.contains(&format!("\"remove\":{{\"body\":{}}}", base.raw())),
        "{text}"
    );
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(format::restore_transaction(journaled), Some(add));
}

fn bore_views() -> caditor_document::SavedViews {
    let view = |distance: f64| caditor_document::SavedView {
        target: caditor_geometry::Point3::new(12.5, -3.0, 40.0),
        orientation: caditor_geometry::Rotation3::from_axis_angle(
            caditor_geometry::Vector3::Z,
            0.75,
        ),
        distance,
    };
    caditor_document::SavedViews {
        named: vec![
            caditor_document::NamedView {
                name: "Hidden bore".to_owned(),
                view: view(80.0),
            },
            caditor_document::NamedView {
                name: "As drawn".to_owned(),
                view: view(260.0),
            },
        ],
        home: Some(view(150.0)),
    }
}

#[test]
fn saved_views_survive_saving_the_journal_and_its_snapshot() {
    let mut document = sample();
    let plain = encode(&document).unwrap();
    let change = Transaction::single(
        "Saved views",
        Edit::SetSavedViews {
            views: Box::new(bore_views()),
        },
    );
    let undo = document.apply(change.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&change)).unwrap());
    let undone: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&undo)).unwrap());
    let recovered = journal::decode_journal(
        &journal::encode_journal(
            &journal::JournalHead {
                file: None,
                on_disk: None,
                loaded_with_problems: false,
                folded: 0,
            },
            &document,
            &[],
        )
        .unwrap(),
    )
    .unwrap();

    assert!(!plain.contains("\"views\""));
    assert!(text.contains("\"views\""), "{text}");
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(format::restore_transaction(journaled), Some(change));
    assert_eq!(format::restore_transaction(undone), Some(undo));
    assert_eq!(recovered.issues, Vec::<String>::new());
    assert_eq!(recovered.base, document);
}

#[test]
fn a_model_saved_before_views_existed_loads_with_none() {
    let document = sample();

    let loaded = decode_text(&encode(&document).unwrap());

    assert!(loaded.document.saved_views().is_empty());
    assert_eq!(loaded.issues, Vec::<String>::new());
}

#[test]
fn saved_views_that_cannot_be_used_are_repaired_and_reported() {
    let document = sample();
    let good = r#"{"target":[1.0,2.0,3.0],"orientation":[0.0,0.0,0.0,1.0],"distance":50.0}"#;
    let flat = r#"{"target":[1.0,2.0,3.0],"orientation":[0.0,0.0,0.0,1.0],"distance":0.0}"#;
    let views = format!(
        r#"{{"views":{{"named":[{{"name":"Bore","view":{good}}},{{"name":"bore","view":{good}}},{{"name":"  ","view":{good}}},{{"name":"Flat","view":{flat}}},{{"name":"Broken"}}],"home":{{"target":"here"}}}}}}"#
    );
    let text = format!("{}\n{views}", encode(&document).unwrap());

    let loaded = decode_text(&text);

    let names: Vec<&str> = loaded
        .document
        .saved_views()
        .named
        .iter()
        .map(|named| named.name.as_str())
        .collect();
    assert_eq!(names, ["Bore", "bore 2", "View 3"]);
    assert_eq!(loaded.document.saved_views().home, None);
    assert!(issues_mention(
        &loaded,
        "Two saved views were called “bore”"
    ));
    assert!(issues_mention(&loaded, "A saved view had no name"));
    assert!(issues_mention(
        &loaded,
        "“Flat” is not a view the camera can show"
    ));
    assert!(issues_mention(&loaded, "A saved view could not be read"));
    assert!(issues_mention(
        &loaded,
        "The Isometric view could not be read"
    ));
}

fn steel_appearance(document: &Document) -> caditor_document::BodyAppearance {
    caditor_document::BodyAppearance {
        colour: Some(caditor_document::Rgb::new(70, 130, 180)),
        material: Some("Steel".to_owned()),
        density: Some(document.parse("depth / 1 mm * 2.5").unwrap()),
        name: Some("Base plate".to_owned()),
        opacity: Some(40),
        faces: Vec::new(),
    }
}

#[test]
fn a_body_appearance_survives_saving_and_the_journal() {
    let (mut document, base, _) = solid_model();
    let plain = encode(&document).unwrap();
    let paint = Transaction::single(
        "Paint Base",
        Edit::SetBodyAppearance {
            id: base,
            appearance: steel_appearance(&document),
        },
    );
    let unpainted = document.apply(paint.clone()).unwrap();
    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&paint)).unwrap());
    let undone: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&unpainted)).unwrap());

    assert!(!plain.contains("appearance"));
    assert!(text.contains("\"colour\":\"#4682b4\""), "{text}");
    assert!(text.contains("\"material\":\"Steel\""), "{text}");
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(
        loaded.document.feature(base).unwrap().appearance,
        steel_appearance(&document)
    );
    assert_eq!(loaded.document, document);
    assert_eq!(format::restore_transaction(journaled), Some(paint));
    assert_eq!(format::restore_transaction(undone), Some(unpainted));
}

#[test]
fn face_colours_are_saved_and_a_damaged_one_is_left_out_in_words() {
    use caditor_document::{FaceColour, Rgb};
    use caditor_kernel::{FaceName, FaceReference};
    let (mut document, base, _) = solid_model();
    let face = |digest: u128| FaceReference::new(FaceName::from_digest(digest), None, []);
    let appearance = caditor_document::BodyAppearance {
        faces: vec![
            FaceColour {
                face: face(0xface),
                colour: Rgb::new(200, 64, 52),
            },
            FaceColour {
                face: face(0xbeef),
                colour: Rgb::new(38, 150, 150),
            },
        ],
        ..Default::default()
    };
    document
        .apply(Transaction::single(
            "Paint faces",
            Edit::SetBodyAppearance {
                id: base,
                appearance,
            },
        ))
        .unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let damaged = decode_text(&text.replace("#269696", "teal"));

    assert!(text.contains("\"colour\":\"#c84034\""), "{text}");
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(
        damaged.issues,
        ["The colour of a face of “Base” could not be read, so it shows in the body's colour."]
    );
    assert_eq!(
        damaged
            .document
            .feature(base)
            .unwrap()
            .appearance
            .faces
            .len(),
        1
    );
}

#[test]
fn an_unreadable_colour_or_density_loads_without_it_and_says_so() {
    let (mut document, base, _) = solid_model();
    document
        .apply(Transaction::single(
            "Paint Base",
            Edit::SetBodyAppearance {
                id: base,
                appearance: steel_appearance(&document),
            },
        ))
        .unwrap();
    let text = encode(&document)
        .unwrap()
        .replace("#4682b4", "steel blue")
        .replace(
            &document
                .feature(base)
                .unwrap()
                .appearance
                .density
                .as_ref()
                .unwrap()
                .to_stored_text(),
            "2 +",
        );

    let loaded = decode_text(&text);
    let appearance = &loaded.document.feature(base).unwrap().appearance;

    assert_eq!(
        loaded.issues,
        [
            "The colour of “Base” could not be read, so it shows in the default colour.",
            "The density of “Base” could not be read, so it was left out. Enter it again to see \
             the body's mass.",
        ]
    );
    assert_eq!(appearance.colour, None);
    assert_eq!(appearance.density, None);
    assert_eq!(appearance.material.as_deref(), Some("Steel"));
}

#[test]
fn a_damaged_hidden_feature_is_called_damaged_rather_than_of_an_unknown_kind() {
    let (document, _, _) = solid_model();
    let mut lines = lines_of(&document);
    let base = lines
        .iter()
        .position(|line| line.contains("\"name\":\"Base\""))
        .unwrap();
    lines[base] =
        "{\"feature\":{\"appearance\":{},\"hidden\":true,\"id\":1,\"name\":\"Base\",\"extrude\":7}}"
            .to_owned();

    let loaded = decode(&current_model_from_json(&lines)).unwrap();

    assert!(
        issues_mention(&loaded, "The feature “Base” is damaged and was left out."),
        "{:?}",
        loaded.issues
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
        &journal::encode_journal(
            &journal::JournalHead {
                file: None,
                on_disk: None,
                loaded_with_problems: false,
                folded: 0,
            },
            &document,
            &[],
        )
        .unwrap(),
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
        EdgeName, EdgeReference, FaceCopy, FaceName, FaceOrigin, FaceReference, VertexName,
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
                    Some(
                        FaceOrigin::Side {
                            feature: 1,
                            entity: 4,
                        }
                        .copied(FaceCopy {
                            pattern: 9,
                            index: [2, 1],
                        }),
                    ),
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
                    Some(FaceOrigin::Chamfer { feature: 7 }.copied(FaceCopy {
                        pattern: 9,
                        index: [1, 0],
                    })),
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
    assert!(text.contains("\"copies\":[null,{\"index\":[2,1],\"pattern\":9}]"));
    assert!(text.contains("\"copy\":{\"index\":[1,0],\"pattern\":9}"));
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

fn offset_model() -> (Document, FeatureId) {
    use caditor_document::OffsetFace;
    use caditor_kernel::{FaceName, FaceOrigin, FaceReference};
    let (mut document, base, _) = solid_model();
    let top = FaceReference::new(
        FaceName::from_digest(0xbeef),
        Some(FaceOrigin::EndCap { feature: 1 }),
        [FaceName::from_digest(2)],
    );
    let mut transaction = document.transaction("Offset face");
    let offset = transaction.add_feature(
        "Offset face 1",
        FeatureKind::OffsetFace(OffsetFace {
            body: base,
            faces: vec![top],
            distance: transaction.parse("depth / 4").unwrap(),
            tangent: true,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    (document, offset)
}

#[test]
fn moved_faces_are_saved_and_loaded_as_a_record_of_their_own() {
    let (document, offset) = offset_model();
    let text = encode(&document).unwrap();
    assert!(text.contains(
        "\"offset_face\":{\"body\":1,\"distance\":\"$0 / 4\",\"faces\":[{\"face\":\
         \"0000000000000000000000000000beef\",\"neighbours\":[\"00000000000000000000000000000002\"],\
         \"origin\":{\"end_cap\":{\"feature\":1}}}],\"tangent\":true}"
    ));
    let loaded = decode_text(&text);
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);

    let kind = document.feature(offset).unwrap().kind.clone();
    let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id: offset, kind });
    let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
    let record = through_binary(&text);
    assert_eq!(format::restore_transaction(record), Some(transaction));
}

#[test]
fn an_unreadable_moved_face_is_left_where_it_is_and_reported() {
    let (document, offset) = offset_model();
    let text =
        encode(&document)
            .unwrap()
            .replacen("0000000000000000000000000000beef", "not a digest", 1);
    let loaded = decode_text(&text);
    assert_eq!(
        loaded.issues,
        ["Some faces moved by “Offset face 1” could not be read and were left where they are."]
    );
    let restored = loaded.document.feature(offset).unwrap();
    assert!(restored.kind.offset_face().unwrap().faces.is_empty());
}

fn primitive_model() -> (Document, FeatureId, FeatureId) {
    use caditor_document::{BodyOperation, Primitive, PrimitiveAnchor, PrimitiveShape};
    use caditor_kernel::{FaceName, FaceOrigin, FaceReference};
    let mut document = Document::default();
    let mut transaction = document.transaction("Primitives");
    let side = transaction.add_parameter("side", transaction.parse("8 mm").unwrap());
    let block = transaction.add_feature(
        "Box 1",
        FeatureKind::Primitive(Primitive {
            shape: PrimitiveShape::Box {
                length: Expression::Parameter(side),
                width: transaction.parse("side / 2").unwrap(),
                height: transaction.parse("3 mm").unwrap(),
            },
            plane: PlaneReference::Principal(PrincipalPlane::Xz),
            at: [
                transaction.parse("1 mm").unwrap(),
                transaction.parse("-2 mm").unwrap(),
            ],
            anchor: PrimitiveAnchor::Corner,
            reversed: false,
            operation: BodyOperation::NewBody,
        }),
    );
    let top = FaceReference::new(
        FaceName::from_digest(0xcafe),
        Some(FaceOrigin::EndCap {
            feature: block.raw(),
        }),
        [FaceName::from_digest(3)],
    );
    let bore = transaction.add_feature(
        "Cylinder 1",
        FeatureKind::Primitive(Primitive {
            shape: PrimitiveShape::Cylinder {
                diameter: transaction.parse("2 mm").unwrap(),
                height: transaction.parse("1 mm").unwrap(),
            },
            plane: PlaneReference::Face(FaceAttachment {
                body: block,
                face: top,
            }),
            at: [
                transaction.parse("4 mm").unwrap(),
                transaction.parse("2 mm").unwrap(),
            ],
            anchor: PrimitiveAnchor::BaseCentre,
            reversed: true,
            operation: BodyOperation::Remove(block),
        }),
    );
    document.apply(transaction.finish()).unwrap();
    (document, block, bore)
}

#[test]
fn primitives_are_saved_and_loaded_as_a_record_of_their_own() {
    let (document, block, bore) = primitive_model();
    let text = encode(&document).unwrap();
    assert!(text.contains(
        "\"primitive\":{\"anchor\":\"corner\",\"at\":[\"1 mm\",\"-2 mm\"],\"operation\":\
         \"new_body\",\"plane\":{\"principal\":\"xz\"},\"shape\":{\"box\":{\"height\":\"3 mm\",\
         \"length\":\"$0\",\"width\":\"$0 / 2\"}}}"
    ));
    assert!(text.contains("\"reversed\":true"));
    let loaded = decode_text(&text);
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);

    for feature in [block, bore] {
        let kind = document.feature(feature).unwrap().kind.clone();
        let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id: feature, kind });
        let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
        let record = through_binary(&text);
        assert_eq!(format::restore_transaction(record), Some(transaction));
    }
}

#[test]
fn an_unreadable_primitive_plane_and_size_are_reported_and_replaced() {
    let (document, block, _) = primitive_model();
    let text = encode(&document)
        .unwrap()
        .replacen("{\"principal\":\"xz\"}", "{\"principal\":\"sideways\"}", 1)
        .replacen("\"height\":\"3 mm\"", "\"height\":\"3 (\"", 1);
    let loaded = decode_text(&text);
    assert_eq!(
        loaded.issues,
        [
            "The height of “Box 1” could not be read, so it was set to 10 mm.",
            "The plane or face “Box 1” stands on could not be read, so it stands on the XY plane.",
        ]
    );
    let restored = loaded.document.feature(block).unwrap();
    assert_eq!(
        restored.kind.primitive().unwrap().plane,
        PlaneReference::Principal(PrincipalPlane::Xy)
    );
}

fn patterned_model() -> (Document, FeatureId, FeatureId) {
    use caditor_document::{
        AxisReference, CircularPattern, LinearDirection, LinearSpacing, Pattern, PatternKind,
        PrincipalAxis,
    };
    use caditor_kernel::{
        EdgeName, EdgeReference, FaceName, FaceOrigin, FaceReference, VertexName,
    };
    let (mut document, base, _) = solid_model();
    let mut transaction = document.transaction("Patterns");
    let linear = transaction.add_feature(
        "Linear pattern 1",
        FeatureKind::from(Pattern::new(
            base,
            PatternKind::Linear {
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
                    measured: LinearSpacing::BetweenCopies,
                    reversed: false,
                },
                second: Some(LinearDirection {
                    axis: AxisReference::Principal(PrincipalAxis::Y),
                    count: transaction.parse("2").unwrap(),
                    spacing: transaction.parse("12 mm").unwrap(),
                    measured: LinearSpacing::BetweenCopies,
                    reversed: true,
                }),
            },
        )),
    );
    let circular = transaction.add_feature(
        "Circular pattern 1",
        FeatureKind::from(Pattern::new(
            base,
            PatternKind::Circular(CircularPattern {
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
        )),
    );
    document.apply(transaction.finish()).unwrap();
    (document, linear, circular)
}

fn spread_and_skipping(document: &mut Document, linear: FeatureId, circular: FeatureId) {
    use caditor_document::{LinearSpacing, PatternKind};
    let mut spread = document
        .feature(linear)
        .unwrap()
        .kind
        .pattern()
        .unwrap()
        .clone();
    if let PatternKind::Linear { first, .. } = &mut spread.kind {
        first.measured = LinearSpacing::Total;
    }
    spread.skipped.insert([1, 1]);
    let skipping = document
        .feature(circular)
        .unwrap()
        .kind
        .pattern()
        .unwrap()
        .toggled([2, 0])
        .unwrap();
    let transaction = Transaction::new(
        "Edit",
        vec![
            Edit::SetFeatureKind {
                id: linear,
                kind: FeatureKind::from(spread),
            },
            Edit::SetFeatureKind {
                id: circular,
                kind: FeatureKind::from(skipping),
            },
        ],
    );
    document.apply(transaction).unwrap();
}

#[test]
fn a_pattern_with_a_total_length_or_instances_left_out_is_a_record_kind_of_its_own() {
    let (mut document, linear, circular) = patterned_model();
    spread_and_skipping(&mut document, linear, circular);

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);

    assert!(!text.contains("\"linear_pattern\""));
    assert!(!text.contains("\"circular_pattern\""));
    assert!(text.contains("\"pattern\":{\"shape\":{\"linear\":{\"body\":1,"));
    assert!(text.contains("\"skipped\":[[1,1]]"));
    assert!(text.contains("\"skipped\":[[2,0]]"));
    assert!(text.contains("\"total\":true"));
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
fn a_pattern_repeating_features_is_a_kind_older_readers_report_and_reads_back() {
    use caditor_document::HoleStyle;
    let (mut document, hole) = holed_model(HoleStyle::Plain, true);
    let (patterned, linear, _) = patterned_model();
    let mut repeating = patterned
        .feature(linear)
        .unwrap()
        .kind
        .pattern()
        .unwrap()
        .clone()
        .repeating(vec![hole]);
    repeating.body = document.feature(hole).unwrap().kind.hole().unwrap().body;
    let mut transaction = document.transaction("Pattern the hole");
    let pattern = transaction.add_feature("Linear pattern 1", FeatureKind::from(repeating));
    document.apply(transaction.finish()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let older = decode_text(&text.replace("feature_pattern", "pattern_of_features"));
    let kind = document.feature(pattern).unwrap().kind.clone();
    let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id: pattern, kind });
    let journaled = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();

    assert!(text.contains("\"feature_pattern\":{\"feature\":{\"linear_pattern\":"));
    assert!(text.contains(&format!("\"repeated\":[{}]", hole.raw())));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert!(older.document.feature(pattern).is_none());
    assert!(!older.issues.is_empty());
    assert_eq!(
        format::restore_transaction(through_binary(&journaled)),
        Some(transaction)
    );
}

#[test]
fn a_mirror_of_features_is_a_kind_older_readers_report_and_reads_back() {
    use caditor_document::{HoleStyle, Mirror};
    let (mut document, hole) = holed_model(HoleStyle::Plain, true);
    let body = document.feature(hole).unwrap().kind.hole().unwrap().body;
    let mut transaction = document.transaction("Mirror the hole");
    let mirror = transaction.add_feature(
        "Mirror 1",
        FeatureKind::Mirror(
            Mirror::new(body, PlaneReference::Principal(PrincipalPlane::Xz)).mirroring(vec![hole]),
        ),
    );
    document.apply(transaction.finish()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let older = decode_text(&text.replace("feature_mirror", "mirror_of_features"));
    let not_a_mirror = decode_text(&text.replace(
        "\"feature_mirror\":{\"feature\":{\"mirror\":",
        "\"feature_mirror\":{\"feature\":{\"remove\":",
    ));
    let kind = document.feature(mirror).unwrap().kind.clone();
    let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id: mirror, kind });
    let journaled = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();

    assert!(text.contains("\"feature_mirror\":{\"feature\":{\"mirror\":{\"body\":"));
    assert!(text.contains(&format!("\"mirrored\":[{}]", hole.raw())));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert!(older.document.feature(mirror).is_none());
    assert!(!older.issues.is_empty());
    assert!(
        not_a_mirror
            .issues
            .iter()
            .any(|issue| issue.contains("listed features to mirror, but it is not a mirror"))
    );
    assert_eq!(
        format::restore_transaction(through_binary(&journaled)),
        Some(transaction)
    );
}

#[test]
fn a_move_turning_about_its_body_centre_is_a_kind_older_readers_report_and_reads_back() {
    use caditor_document::{Move, TurnCentre};
    let (mut document, base, _) = solid_model();
    let mut transaction = document.transaction("Move");
    let movement = transaction.add_feature(
        "Move body 1",
        FeatureKind::Move(Move {
            body: base,
            offset: std::array::from_fn(|_| transaction.parse("0 mm").unwrap()),
            turn: std::array::from_fn(|_| transaction.parse("30 deg").unwrap()),
            copy: true,
            about: TurnCentre::Body,
            frame: None,
        }),
    );
    document.apply(transaction.finish()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let older = decode_text(&text.replace("move_about_centre", "move_about_middle"));

    assert!(text.contains("\"move_about_centre\":{\"feature\":{\"copy\":"));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert!(older.document.feature(movement).is_none());
    assert!(!older.issues.is_empty());
}

#[test]
fn a_move_turning_about_an_axis_is_a_kind_older_readers_report_and_reads_back() {
    use caditor_document::{AxisReference, AxisTurn, Move, PrincipalAxis, TurnCentre};
    let (mut document, base, _) = solid_model();
    let mut transaction = document.transaction("Move");
    let movement = transaction.add_feature(
        "Move body 1",
        FeatureKind::Move(Move {
            body: base,
            offset: std::array::from_fn(|_| transaction.parse("0 mm").unwrap()),
            turn: std::array::from_fn(|_| transaction.parse("0 deg").unwrap()),
            copy: false,
            about: TurnCentre::Axis(Box::new(AxisTurn {
                axis: AxisReference::Principal(PrincipalAxis::Y),
                angle: transaction.parse("45 deg").unwrap(),
            })),
            frame: None,
        }),
    );
    document.apply(transaction.finish()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let older = decode_text(&text.replace("move_about_axis", "move_about_line"));

    assert!(
        text.contains("\"move_about_axis\":{\"angle\":\"45 deg\""),
        "{text}"
    );
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert!(older.document.feature(movement).is_none());
    assert!(!older.issues.is_empty());
}

#[test]
fn instances_left_out_that_no_pattern_can_make_are_dropped_on_loading() {
    let (mut document, linear, circular) = patterned_model();
    spread_and_skipping(&mut document, linear, circular);
    let text = encode(&document).unwrap().replacen(
        "\"skipped\":[[2,0]]",
        "\"skipped\":[[0,0],[2,0],[4000,0]]",
        1,
    );

    let loaded = decode_text(&text);
    let restored = loaded
        .document
        .feature(circular)
        .unwrap()
        .kind
        .pattern()
        .unwrap();

    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(restored.skipped.iter().collect::<Vec<_>>(), [&[2, 0]]);
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
            start: None,
            other_bodies: Vec::new(),
            side: None,
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
fn datum_points_and_planes_and_axes_through_points_are_saved_loaded_and_journaled() {
    use caditor_document::{
        AxisReference, Datum, DatumAxis, DatumPoint, PlaneReference, PlaneThrough, PointReference,
        PrincipalAxis, PrincipalPlane,
    };
    use caditor_kernel::{EdgeName, EdgeReference, FaceName, VertexName};
    use caditor_sketch::EntityId;
    let (mut document, base, _) = solid_model();
    let corner = PointReference::Vertex {
        body: base,
        vertex: VertexName::from_digest(0xc0),
    };
    let centre = PointReference::Centre {
        body: base,
        edge: Box::new(EdgeReference::new(
            EdgeName::from_digest(0xed),
            [FaceName::from_digest(1), FaceName::from_digest(2)],
            [VertexName::from_digest(3), VertexName::from_digest(4)],
        )),
    };
    let mut transaction = document.transaction("Datums");
    let point = transaction.add_feature(
        "Point 1",
        FeatureKind::Datum(Datum::Point(DatumPoint {
            base: corner.clone(),
            offset: [
                transaction.parse("depth").unwrap(),
                transaction.parse("0 mm").unwrap(),
                transaction.parse("-1 mm").unwrap(),
            ],
        })),
    );
    let through = Datum::PlaneThrough(PlaneThrough::Points([
        PointReference::Origin,
        PointReference::Datum(point),
        centre.clone(),
    ]));
    transaction.add_feature("Plane 1", FeatureKind::Datum(through));
    transaction.add_feature(
        "Plane 2",
        FeatureKind::Datum(Datum::PlaneThrough(PlaneThrough::Midway(
            PlaneReference::Principal(PrincipalPlane::Xy),
            PlaneReference::Principal(PrincipalPlane::Xz),
        ))),
    );
    transaction.add_feature(
        "Plane 3",
        FeatureKind::Datum(Datum::PlaneThrough(PlaneThrough::AxisAndPoint(
            AxisReference::Principal(PrincipalAxis::Z),
            corner.clone(),
        ))),
    );
    transaction.add_feature(
        "Plane 4",
        FeatureKind::Datum(Datum::PlaneThrough(PlaneThrough::NormalTo(
            AxisReference::Principal(PrincipalAxis::X),
            PointReference::Sketch {
                sketch: FeatureId::from_raw(0),
                entity: EntityId::from_raw(1),
            },
        ))),
    );
    transaction.add_feature(
        "Axis 1",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Points(
            PointReference::Datum(point),
            centre,
        ))),
    );
    transaction.add_feature(
        "Axis 2",
        FeatureKind::Datum(Datum::Axis(DatumAxis::NormalTo(
            PlaneReference::Principal(PrincipalPlane::Yz),
            corner,
        ))),
    );
    transaction.add_feature(
        "Axis 3",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Along(AxisReference::Sketch {
            sketch: FeatureId::from_raw(0),
            entity: EntityId::from_raw(2),
        }))),
    );
    let add = transaction.finish();
    document.apply(add.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&add)).unwrap());

    assert!(
        text.contains("\"point\":{\"base\":{\"vertex\":{\"body\":1,"),
        "{text}"
    );
    assert!(text.contains("\"plane_through\":{\"points\":[\"origin\",{\"datum\":"));
    assert!(text.contains("{\"centre\":{\"body\":1,\"edge\":"));
    assert!(text.contains("\"plane_through\":{\"midway\":[{\"principal\":\"xy\"},"));
    assert!(text.contains("\"plane_through\":{\"axis_and_point\":{\"axis\":{\"principal\":\"z\"}"));
    assert!(text.contains("{\"sketch\":{\"entity\":1,\"sketch\":0}}"));
    assert!(text.contains("\"axis_through\":{\"points\":[{\"datum\":"));
    assert!(text.contains("\"axis_through\":{\"normal_to\":{\"plane\":{\"principal\":\"yz\"}"));
    assert!(text.contains("\"axis\":{\"along\":{\"sketch_line\":{\"entity\":2,\"sketch\":0}}}"));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(format::restore_transaction(journaled), Some(add));
}

#[test]
fn an_unreadable_point_reference_falls_back_and_is_reported() {
    use caditor_document::{Datum, PlaneThrough, PointReference};
    let (mut document, _, _) = solid_model();
    let mut transaction = document.transaction("Datums");
    transaction.add_feature(
        "Plane 1",
        FeatureKind::Datum(Datum::PlaneThrough(PlaneThrough::Points([
            PointReference::Origin,
            PointReference::Origin,
            PointReference::Origin,
        ]))),
    );
    document.apply(transaction.finish()).unwrap();
    let text = encode(&document).unwrap().replacen(
        "[\"origin\",\"origin\",\"origin\"]",
        "[\"origin\",\"origin\",{\"vertex\":{\"body\":1,\"vertex\":\"zz\"}}]",
        1,
    );

    let loaded = decode_text(&text);

    assert!(issues_mention(
        &loaded,
        "What “Plane 1” passes through could not be read, so it is the XY plane."
    ));
    let plane = loaded
        .document
        .features()
        .find(|feature| feature.name == "Plane 1")
        .unwrap();
    assert!(matches!(plane.kind, FeatureKind::Datum(Datum::Plane(_))));
}

fn constructed_datums_model() -> (Document, Transaction) {
    use caditor_document::{
        AxisReference, CurveStation, Datum, DatumPoint, FaceTangent, PlaneReference, PlaneThrough,
        PointBy, PointReference, PrincipalAxis, PrincipalPlane,
    };
    use caditor_kernel::{EdgeName, EdgeReference, FaceName, FaceReference, VertexName};
    let (mut document, base, _) = solid_model();
    let edge = || {
        Box::new(EdgeReference::new(
            EdgeName::from_digest(0xed),
            [FaceName::from_digest(1), FaceName::from_digest(2)],
            [VertexName::from_digest(3), VertexName::from_digest(4)],
        ))
    };
    let face = || FaceReference::new(FaceName::from_digest(0xfa), None, Vec::new());
    let station = |distance: &str| CurveStation {
        body: base,
        edge: edge(),
        distance: Expression::parse_stored(distance).unwrap(),
    };
    let mut transaction = document.transaction("Constructed datums");
    transaction.add_feature(
        "Plane 1",
        FeatureKind::Datum(Datum::PlaneThrough(PlaneThrough::Tangent(Box::new(
            FaceTangent {
                body: base,
                face: face(),
                toward: PointReference::Origin,
            },
        )))),
    );
    transaction.add_feature(
        "Plane 2",
        FeatureKind::Datum(Datum::PlaneThrough(PlaneThrough::SquareToCurve(Box::new(
            station("3 mm"),
        )))),
    );
    transaction.add_feature(
        "Plane 3",
        FeatureKind::Datum(Datum::PlaneThrough(PlaneThrough::Lines(
            AxisReference::Principal(PrincipalAxis::X),
            AxisReference::Edge {
                body: base,
                edge: edge(),
            },
        ))),
    );
    transaction.add_feature(
        "Point 1",
        FeatureKind::Datum(Datum::PointBy(PointBy::LinesCross(
            AxisReference::Principal(PrincipalAxis::X),
            AxisReference::Principal(PrincipalAxis::Y),
        ))),
    );
    transaction.add_feature(
        "Point 2",
        FeatureKind::Datum(Datum::PointBy(PointBy::AxisAndPlane(
            AxisReference::Principal(PrincipalAxis::Z),
            PlaneReference::Principal(PrincipalPlane::Xy),
        ))),
    );
    transaction.add_feature(
        "Point 3",
        FeatureKind::Datum(Datum::PointBy(PointBy::ThreePlanes([
            PlaneReference::Principal(PrincipalPlane::Xy),
            PlaneReference::Principal(PrincipalPlane::Xz),
            PlaneReference::Principal(PrincipalPlane::Yz),
        ]))),
    );
    transaction.add_feature(
        "Point 4",
        FeatureKind::Datum(Datum::PointBy(PointBy::Along(Box::new(station("-2 mm"))))),
    );
    transaction.add_feature(
        "Point 5",
        FeatureKind::Datum(Datum::Point(DatumPoint {
            base: PointReference::SurfaceCentre {
                body: base,
                face: face(),
            },
            offset: [0, 1, 2].map(|_| Expression::parse_stored("0 mm").unwrap()),
        })),
    );
    let add = transaction.finish();
    document.apply(add.clone()).unwrap();
    (document, add)
}

#[test]
fn constructed_planes_and_points_are_kinds_older_readers_report_and_read_back() {
    let (document, add) = constructed_datums_model();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let older = decode_text(&text.replace("plane_construction", "plane_built"));
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&add)).unwrap());

    assert!(
        text.contains("\"plane_construction\":{\"tangent\":{"),
        "{text}"
    );
    assert!(text.contains("\"plane_construction\":{\"square_to_curve\":{"));
    assert!(text.contains("\"plane_construction\":{\"lines\":[{\"principal\":\"x\"}"));
    assert!(text.contains("\"point_construction\":{\"lines_cross\":["));
    assert!(text.contains("\"point_construction\":{\"axis_and_plane\":{"));
    assert!(text.contains("\"point_construction\":{\"three_planes\":["));
    assert!(text.contains("\"point_construction\":{\"along\":{"));
    assert!(text.contains("{\"surface_centre\":{\"body\":"));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert!(
        older
            .document
            .features()
            .all(|feature| !feature.name.starts_with("Plane"))
    );
    assert!(!older.issues.is_empty());
    assert_eq!(format::restore_transaction(journaled), Some(add));
}

#[test]
fn an_unreadable_constructed_datum_falls_back_and_is_reported() {
    use caditor_document::Datum;
    let (document, _) = constructed_datums_model();
    let text = encode(&document).unwrap();

    let station = decode_text(&text.replacen("\"distance\":\"3 mm\"", "\"distance\":\"((\"", 1));
    let tangent = decode_text(&text.replacen(
        "\"toward\":\"origin\"",
        "\"toward\":{\"vertex\":{\"body\":1,\"vertex\":\"zz\"}}",
        1,
    ));

    assert!(issues_mention(
        &station,
        "The distance of “Plane 2” could not be read, so it was set to 0 mm."
    ));
    assert!(issues_mention(
        &tangent,
        "What “Plane 1” is placed by could not be read, so it is the XY plane."
    ));
    assert!(matches!(
        tangent
            .document
            .features()
            .find(|feature| feature.name == "Plane 1")
            .unwrap()
            .kind,
        FeatureKind::Datum(Datum::Plane(_))
    ));
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
    let mut read_only = fs::metadata(&path).unwrap().permissions();
    read_only.set_readonly(true);
    fs::set_permissions(&path, read_only).unwrap();

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
    let stamp = |journal: &Path| {
        let metadata = fs::metadata(journal).unwrap();
        (metadata.len(), metadata.modified().unwrap())
    };
    let before = stamp(&journal);

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
    let Report::SaveFailed { error, .. } = wait_for_report(&storage) else {
        panic!("the save should be refused");
    };
    assert_eq!(error, SaveError::OpenInAnotherWindow);
    assert_eq!(load(&path).unwrap().document, base);
    assert_eq!(stamp(&journal), before);
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
                start: None,
                other_bodies: Vec::new(),
                side: None,
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
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

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
    let entries = [journal::Logged::Entry(JournalEntry::Apply(change))];
    let head = journal::JournalHead {
        file: None,
        on_disk: None,
        loaded_with_problems: false,
        folded: 0,
    };
    let bytes = binary::testing::with_slices_of(64, || {
        journal::encode_journal(&head, &base, &entries).unwrap()
    });

    let contents = journal::decode_journal(&bytes).unwrap();
    assert_eq!(contents.issues, Vec::<String>::new());
    assert_eq!(contents.base, base);
    assert_eq!(contents.entries, entries);
    assert_eq!(contents.unreadable_entries, 0);
}

fn journal_kinds(journal: &Path) -> Vec<binary::ChunkKind> {
    let bytes = fs::read(journal).unwrap();
    binary::parse(&bytes, &binary::JOURNAL_MAGIC)
        .unwrap()
        .chunks()
        .map(|chunk| chunk.kind.unwrap())
        .collect()
}

fn undo_and_redo(storage: &Storage, editor: &mut Editor) {
    let undone = editor.next_undo().cloned().unwrap();
    editor.undo().unwrap();
    storage.record(JournalEntry::Undo(undone)).unwrap();
    let redone = editor.next_redo().cloned().unwrap();
    editor.redo().unwrap();
    storage.record(JournalEntry::Redo(redone)).unwrap();
    assert!(storage.flusher().flush(WAIT));
}

#[test]
fn undo_and_redo_are_journaled_by_reference_and_replay_with_their_history() {
    use binary::ChunkKind::{Apply, JournalHeader, RedoNext, Snapshot, UndoLast};
    let dir = TempDir::new().unwrap();
    let storage = Storage::spawn(config(&dir), untitled(&sample()), || {}).unwrap();
    let mut editor = Editor::new(sample());
    let change = edit_width(editor.document(), "50 mm");
    editor.apply(change.clone()).unwrap();
    storage.record(JournalEntry::Apply(change)).unwrap();
    undo_and_redo(&storage, &mut editor);
    crash(storage);
    let journal = only_file_in(&dir.path().join("recovery"));

    let kinds = journal_kinds(&journal);
    let Inspection::Recoverable(recovered) = inspect(&journal).unwrap() else {
        panic!("the session should be recoverable");
    };

    assert_eq!(kinds, [JournalHeader, Snapshot, Apply, UndoLast, RedoNext]);
    assert_eq!(recovered.changes(), 3);
    assert_eq!(recovered.editor.document(), editor.document());
    assert_eq!(recovered.editor.next_undo(), editor.next_undo());
    assert!(matches!(recovered.entries[1], JournalEntry::Undo(_)));
}

#[test]
fn an_undo_reaching_past_the_journal_snapshot_is_journaled_whole() {
    use binary::ChunkKind::{JournalHeader, Redo, Snapshot, Undo, UndoLast};
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let start = Start {
        file: Some(path.clone()),
        ..untitled(&sample())
    };
    let storage = Storage::spawn(config(&dir), start, || {}).unwrap();
    let mut editor = Editor::new(sample());
    let change = edit_width(editor.document(), "50 mm");
    editor.apply(change.clone()).unwrap();
    storage.record(JournalEntry::Apply(change)).unwrap();
    storage
        .save(SaveRequest {
            ticket: 1,
            document: editor.document().clone(),
            path: path.clone(),
            keep_original: false,
            label: None,
            replace_outside_changes: false,
        })
        .unwrap();
    undo_and_redo(&storage, &mut editor);
    let undone = editor.next_undo().cloned().unwrap();
    editor.undo().unwrap();
    storage.record(JournalEntry::Undo(undone)).unwrap();
    assert!(storage.flusher().flush(WAIT));
    crash(storage);
    let journal = dir.path().join(".model.caditor.journal");

    let kinds = journal_kinds(&journal);
    let FileJournal::Recoverable(recovered) = journal_for(&path, None) else {
        panic!("the undone change should be recoverable");
    };

    assert_eq!(kinds, [JournalHeader, Snapshot, Undo, Redo, UndoLast]);
    assert_eq!(recovered.editor.document(), editor.document());
    assert_eq!(recovered.changes(), 3);
}

fn rebasing(dir: &TempDir) -> StorageConfig {
    StorageConfig {
        rebase_journal_after: 0,
        ..config(dir)
    }
}

struct Rebased {
    folded: usize,
    recorded: usize,
}

fn edit_until_rebased(storage: &Storage, editor: &mut Editor) -> Rebased {
    let mut states = vec![editor.document().clone()];
    for step in 0..400 {
        let change = edit_width(editor.document(), &format!("{} mm", 40 + step % 7));
        editor.apply(change.clone()).unwrap();
        states.push(editor.document().clone());
        storage.record(JournalEntry::Apply(change)).unwrap();
        assert!(storage.flusher().flush(WAIT));
        let rebased = storage
            .poll()
            .unwrap()
            .into_iter()
            .find_map(|report| match report {
                Report::Rebased { entries, base } => Some((entries, base)),
                _ => None,
            });
        if let Some((folded, base)) = rebased {
            assert_eq!(base, states[folded]);
            return Rebased {
                folded,
                recorded: states.len() - 1,
            };
        }
    }
    panic!("the journal never rebased");
}

fn later_kinds_are_applies(kinds: &[binary::ChunkKind]) -> bool {
    use binary::ChunkKind::{Apply, JournalHeader, RebasedSnapshot};
    kinds.starts_with(&[JournalHeader, RebasedSnapshot])
        && kinds.iter().skip(2).all(|kind| *kind == Apply)
}

#[test]
fn a_journal_past_its_size_rebases_onto_the_model_as_edited() {
    let dir = TempDir::new().unwrap();
    let storage = Storage::spawn(rebasing(&dir), untitled(&sample()), || {}).unwrap();
    let mut editor = Editor::new(sample());

    let rebased = edit_until_rebased(&storage, &mut editor);
    let change = edit_width(editor.document(), "99 mm");
    editor.apply(change.clone()).unwrap();
    storage.record(JournalEntry::Apply(change)).unwrap();
    assert!(storage.flusher().flush(WAIT));
    crash(storage);
    let journal = only_file_in(&dir.path().join("recovery"));

    let recovered = scan(Some(&dir.path().join("recovery")), &[]).remove(0);

    assert!(rebased.folded > 1);
    assert!(later_kinds_are_applies(&journal_kinds(&journal)));
    assert!(recovered.folded >= rebased.folded);
    assert_eq!(recovered.changes(), rebased.recorded + 1);
    assert_eq!(recovered.editor.document(), editor.document());
}

#[test]
fn a_rebased_journal_is_offered_and_restored_with_its_folded_changes() {
    let dir = TempDir::new().unwrap();
    let recovery = dir.path().join("recovery");
    let storage = Storage::spawn(rebasing(&dir), untitled(&sample()), || {}).unwrap();
    let mut editor = Editor::new(sample());
    let rebased = edit_until_rebased(&storage, &mut editor);
    crash(storage);
    let recovered = scan(Some(&recovery), &[]).remove(0);

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
    assert!(storage.flusher().flush(WAIT));
    crash(storage);
    let journal = only_file_in(&recovery);
    let again = scan(Some(&recovery), &[]).remove(0);

    assert!(recovered.folded > 0);
    assert_eq!(recovered.changes(), rebased.recorded);
    assert_eq!(recovered.editor.document(), editor.document());
    assert!(later_kinds_are_applies(&journal_kinds(&journal)));
    assert_eq!(again.changes(), rebased.recorded);
    assert_eq!(again.editor.document(), editor.document());
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
        &journal::encode_journal(
            &journal::JournalHead {
                file: None,
                on_disk: None,
                loaded_with_problems: false,
                folded: 0,
            },
            &document,
            &[],
        )
        .unwrap(),
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
            start: None,
            other_bodies: Vec::new(),
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
            start: None,
            other_bodies: Vec::new(),
            side: None,
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

fn starts_model() -> (Document, [FeatureId; 3]) {
    use caditor_document::{
        BodyOperation, Datum, DatumPlane, Extrude, ExtrudeExtent, PlaneReference, PrincipalPlane,
        RegionChoice, Revolve, RevolveAxis, RevolveExtent, SolidFeature, SolidStart,
    };
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let sketch = transaction.add_feature("Outline", FeatureKind::from(Sketch::new(Plane::XY)));
    let level = transaction.add_feature(
        "Level",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Xy),
            rotation: None,
            offset: transaction.parse("5 mm").unwrap(),
        })),
    );
    let raised = transaction.add_feature(
        "Raised",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::Symmetric {
                distance: transaction.parse("4 mm").unwrap(),
            },
            operation: BodyOperation::NewBody,
            start: Some(SolidStart::Plane(PlaneReference::Datum(level))),
            other_bodies: Vec::new(),
        })),
    );
    let lifted = transaction.add_feature(
        "Lifted",
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch,
            regions: RegionChoice::All,
            axis: RevolveAxis::Sketch(EntityId::VERTICAL_AXIS),
            extent: RevolveExtent::Full,
            operation: BodyOperation::NewBody,
            start: Some(SolidStart::Distance(transaction.parse("3 mm").unwrap())),
            other_bodies: Vec::new(),
            side: None,
        })),
    );
    let placed = transaction.add_feature(
        "Placed",
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch,
            regions: RegionChoice::All,
            axis: RevolveAxis::Sketch(EntityId::HORIZONTAL_AXIS),
            extent: RevolveExtent::TwoSides {
                forward: transaction.parse("30 deg").unwrap(),
                backward: transaction.parse("45 deg").unwrap(),
            },
            operation: BodyOperation::NewBody,
            start: Some(SolidStart::Plane(PlaneReference::Datum(level))),
            other_bodies: Vec::new(),
            side: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    (document, [raised, lifted, placed])
}

#[test]
fn starts_at_a_plane_or_off_a_revolution_are_saved_in_kinds_older_readers_report() {
    let (document, features) = starts_model();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);

    assert!(text.contains("\"extrude_from\":{"));
    assert!(text.contains("\"revolve_from\":{"));
    assert!(text.contains("\"start\":{\"plane\":{\"datum\":"));
    assert!(text.contains("\"start\":{\"distance\":\"3 mm\"}"));
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
fn a_revolution_keeping_one_side_of_its_axis_is_a_kind_older_readers_report() {
    use caditor_document::{
        AxisSide, BodyOperation, RegionChoice, Revolve, RevolveAxis, RevolveExtent, SolidFeature,
        SolidStart,
    };
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let sketch = transaction.add_feature("Outline", FeatureKind::from(Sketch::new(Plane::XY)));
    let left = transaction.add_feature(
        "Left",
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch,
            regions: RegionChoice::All,
            axis: RevolveAxis::Sketch(EntityId::VERTICAL_AXIS),
            extent: RevolveExtent::Full,
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            side: Some(AxisSide::Left),
        })),
    );
    let right = transaction.add_feature(
        "Right",
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch,
            regions: RegionChoice::All,
            axis: RevolveAxis::Sketch(EntityId::HORIZONTAL_AXIS),
            extent: RevolveExtent::TwoSides {
                forward: transaction.parse("30 deg").unwrap(),
                backward: transaction.parse("45 deg").unwrap(),
            },
            operation: BodyOperation::Remove(left),
            start: Some(SolidStart::Distance(transaction.parse("3 mm").unwrap())),
            other_bodies: Vec::new(),
            side: Some(AxisSide::Right),
        })),
    );
    document.apply(transaction.finish()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let older = decode_text(&text.replace("revolve_one_side", "revolve_other_side"));

    assert!(text.contains("\"revolve_one_side\":{\"feature\":{\"revolve\":"));
    assert!(text.contains("\"revolve_one_side\":{\"feature\":{\"revolve_from\":"));
    assert!(text.contains("\"side\":\"left\""));
    assert!(text.contains("\"side\":\"right\""));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert!(older.document.feature(left).is_none());
    assert!(older.document.feature(right).is_none());
    assert!(!older.issues.is_empty());
    for feature in [left, right] {
        let kind = document.feature(feature).unwrap().kind.clone();
        let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id: feature, kind });
        let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
        let record = through_binary(&text);
        assert_eq!(format::restore_transaction(record), Some(transaction));
    }
}

#[test]
fn an_unreadable_start_falls_back_to_the_sketch_plane_and_is_reported() {
    let (document, [raised, lifted, _]) = starts_model();
    let text = encode(&document)
        .unwrap()
        .replacen("\"start\":{\"plane\":", "\"start\":{\"plain\":", 1)
        .replacen(
            "\"start\":{\"distance\":\"3 mm\"}",
            "\"start\":{\"distance\":\"3 ((\"}",
            1,
        );

    let loaded = decode_text(&text);

    assert_eq!(
        loaded.issues,
        [
            "Where “Raised” starts could not be read, so it starts at its sketch plane.",
            "The start offset of “Lifted” could not be read, so it was set to 0 mm.",
        ]
    );
    for (feature, expected) in [
        (raised, None),
        (
            lifted,
            Some(caditor_document::SolidStart::Distance(Expression::Measure(
                0.0,
                Unit::Millimetre,
            ))),
        ),
    ] {
        match &loaded.document.feature(feature).unwrap().kind {
            FeatureKind::Solid(caditor_document::SolidFeature::Revolve(revolve)) => {
                assert_eq!(revolve.start, expected);
            }
            FeatureKind::Solid(caditor_document::SolidFeature::Extrude(extrude)) => {
                assert_eq!(extrude.start, expected);
            }
            other => panic!("expected a solid, found {other:?}"),
        }
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

fn fake_log(state: &Path, seconds: u64, process: u32, last: &str) -> PathBuf {
    let dir = state.join("logs");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("caditor-{seconds:020}-{process}.log"));
    fs::write(&path, format!("[INFO] started\n{last}\n")).unwrap();
    path
}

const NO_SUCH_PROCESS: u32 = 99_999_999;

#[test]
fn a_session_log_ends_with_a_marker_and_one_that_does_not_is_reported_once() {
    let dir = TempDir::new().unwrap();
    let log = SessionLog::create(dir.path()).unwrap();
    log.write(b"[INFO] working\n").unwrap();
    let crashed = fake_log(dir.path(), 5, NO_SUCH_PROCESS, "[ERROR] caditor panicked");
    let ended = fake_log(
        dir.path(),
        6,
        NO_SUCH_PROCESS,
        "caditor ended this session.",
    );
    let running = fake_log(dir.path(), 7, std::process::id(), "[INFO] still going");

    let stopped = ended_unexpectedly(dir.path(), log.path());
    mark_reported(&crashed).unwrap();
    log.end();

    assert_eq!(stopped, std::slice::from_ref(&crashed));
    assert!(ended_unexpectedly(dir.path(), log.path()).is_empty());
    assert!(ended.exists() && running.exists());
    assert!(
        fs::read_to_string(log.path())
            .unwrap()
            .ends_with("[INFO] working\ncaditor ended this session.\n")
    );
    let other = SessionLog::create(&dir.path().join("elsewhere")).unwrap();
    assert!(ended_unexpectedly(dir.path(), other.path()).is_empty());
}

#[test]
fn a_session_log_stops_at_its_size_limit_and_old_logs_are_pruned() {
    let dir = TempDir::new().unwrap();
    let log = SessionLog::create(dir.path()).unwrap();
    let line = vec![b'x'; 1 << 20];
    for _ in 0..MAX_LOG_SIZE / (1 << 20) + 3 {
        log.write(&line).unwrap();
    }
    log.end();
    let old: Vec<PathBuf> = (0..LOGS_KEPT as u64 + 3)
        .map(|seconds| {
            fake_log(
                dir.path(),
                seconds,
                NO_SUCH_PROCESS,
                "caditor ended this session.",
            )
        })
        .collect();

    prune_logs(dir.path(), log.path());

    let size = fs::metadata(log.path()).unwrap().len();
    assert!(size <= MAX_LOG_SIZE + 200, "{size}");
    assert!(log.path().exists());
    assert_eq!(
        old.iter().filter(|path| path.exists()).count(),
        LOGS_KEPT - 1
    );
    assert!(!old[0].exists());
    assert!(old.last().unwrap().exists());
}

fn fillet_saved_before_origins() -> (Document, FeatureId) {
    use caditor_document::{
        Blend, BlendKind, BodyOperation, CancelToken, Extrude, ExtrudeExtent, ModelEvaluator,
        Recompute, RegionChoice, SolidFeature,
    };
    use caditor_kernel::{EdgeReference, FaceOrigin};
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let mut outline = Sketch::new(Plane::XY);
    let corners = [(0.0, 0.0), (10.0, 0.0), (10.0, 8.0), (0.0, 8.0)];
    for index in 0..4 {
        let (a, b) = (corners[index], corners[(index + 1) % 4]);
        outline.add_line(Point2::new(a.0, a.1), Point2::new(b.0, b.1));
    }
    let sketch = transaction.add_feature("Outline", FeatureKind::from(outline));
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(transaction.parse("4 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = Recompute::default().run(
        &document,
        &ModelEvaluator,
        &CancelToken::never(),
        &|_, _| {},
    );
    let solid = evaluation.body(base).unwrap();

    let cap_edge = solid
        .edges()
        .map(|(id, _)| EdgeReference::capture(solid, id).unwrap())
        .find(|edge| {
            edge.origins().contains(&Some(FaceOrigin::EndCap {
                feature: base.raw(),
            }))
        })
        .unwrap();
    let saved = EdgeReference::new(cap_edge.name(), cap_edge.faces(), cap_edge.ends());

    let mut transaction = document.transaction("Fillet");
    let fillet = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: base,
            edges: vec![saved],
            size: transaction.parse("1 mm").unwrap(),
        }),
    );
    document.apply(transaction.finish()).unwrap();
    (document, fillet)
}

fn fillet_origins(document: &Document, fillet: FeatureId) -> Vec<bool> {
    document
        .feature(fillet)
        .unwrap()
        .kind
        .blend()
        .unwrap()
        .edges
        .iter()
        .map(|edge| edge.origins().iter().all(Option::is_some))
        .collect()
}

#[test]
fn opening_a_model_saved_before_edges_kept_origins_completes_them() {
    let (document, fillet) = fillet_saved_before_origins();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("old.caditor");
    save(&document, &path, false).unwrap();
    let mut changed = document.clone();
    let mut transaction = changed.transaction("Parameter");
    transaction.add_parameter("width", transaction.parse("2 mm").unwrap());
    changed.apply(transaction.finish()).unwrap();
    save(&changed, &path, false).unwrap();

    let decoded = decode(&fs::read(&path).unwrap()).unwrap();
    let loaded = load(&path).unwrap();
    let version = load_version(&path, 0).unwrap();

    assert_eq!(fillet_origins(&decoded.document, fillet), [false]);
    assert_eq!(fillet_origins(&loaded.document, fillet), [true]);
    assert_eq!(fillet_origins(&version.document, fillet), [true]);
    assert!(loaded.issues.is_empty());

    save(&loaded.document, &path, false).unwrap();
    let saved_again = decode(&fs::read(&path).unwrap()).unwrap();

    assert_eq!(fillet_origins(&saved_again.document, fillet), [true]);
}

#[test]
fn inactive_constraints_stay_inactive_through_saving_and_the_journal() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let (start, end) = match sketch.entity(line) {
        Some(Entity::Line { start, end }) => (*start, *end),
        other => panic!("expected a line, found {other:?}"),
    };
    let level = sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    let measured = sketch
        .add_constraint(Constraint::Distance {
            from: start,
            to: end,
            value: Expression::Measure(40.0, Unit::Millimetre),
        })
        .unwrap();
    sketch.set_active(measured, false).unwrap();
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

    assert!(text.contains(&format!("\"id\":{},\"inactive\":true", measured.raw())));
    assert_eq!(text.matches("inactive").count(), 1);
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert!(restored.is_active(level));
    assert!(!restored.is_active(measured));
    assert_eq!(loaded.document, document);

    let mut transaction = document.transaction("Edit constraints");
    transaction.set_sketch_constraint_active(feature, measured, true);
    transaction.edit(Edit::AddSketchConstraint {
        feature,
        id: ConstraintId::from_raw(90),
        constraint: Constraint::Vertical(line),
        inactive: true,
    });
    let transaction = transaction.finish();
    let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
    assert!(text.contains(&format!(
        "{{\"set_sketch_constraint_active\":{{\"feature\":{},\"id\":{},\"active\":true}}}}",
        feature.raw(),
        measured.raw()
    )));
    let record: format::TransactionRecord = through_binary(&text);
    assert_eq!(format::restore_transaction(record), Some(transaction));
}

#[test]
fn diameters_across_an_axis_are_saved_as_flagged_radii_that_older_readers_hold_as_distances() {
    let mut document = Document::default();
    let mut transaction = document.transaction("New sketch");
    let bore = transaction.add_parameter("bore", transaction.parse("30 mm").unwrap());
    let mut sketch = Sketch::new(Plane::XY);
    let axis = sketch.add_line(Point2::new(0.0, -10.0), Point2::new(0.0, 10.0));
    sketch.set_construction(axis, true).unwrap();
    let rim = sketch.add_point(Point2::new(12.0, 0.0));
    let hole = sketch.add_point(Point2::new(15.0, 5.0));
    let literal = sketch
        .add_constraint(Constraint::AxisDiameter {
            point: rim,
            axis,
            value: Expression::Measure(24.0, Unit::Millimetre),
        })
        .unwrap();
    let driven = sketch
        .add_constraint(Constraint::AxisDiameter {
            point: hole,
            axis,
            value: Expression::Parameter(bore),
        })
        .unwrap();
    let feature = transaction.add_feature("Profile", FeatureKind::from(sketch));
    document.apply(transaction.finish()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);

    assert_eq!(text.matches("\"diameter\":true").count(), 2);
    assert!(text.contains(&format!(
        "{{\"distance\":{{\"diameter\":true,\"from\":{},\"to\":{},\"value\":\"12 mm\"}}",
        rim.raw(),
        axis.raw()
    )));
    assert!(text.contains(&format!("\"value\":\"${} / 2\"", bore.raw())));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);

    let older = decode_text(&text.replace("\"diameter\":true,", ""));
    let older_sketch = older
        .document
        .feature(feature)
        .unwrap()
        .kind
        .sketch()
        .unwrap();

    assert_eq!(older.issues, Vec::<String>::new());
    assert_eq!(
        older_sketch.constraint(literal),
        Some(&Constraint::Distance {
            from: rim,
            to: axis,
            value: Expression::Measure(12.0, Unit::Millimetre),
        })
    );
    assert!(matches!(
        older_sketch.constraint(driven),
        Some(Constraint::Distance {
            value: Expression::Binary(..),
            ..
        })
    ));

    let mut transaction = document.transaction("Add diameter");
    transaction.edit(Edit::AddSketchConstraint {
        feature,
        id: ConstraintId::from_raw(90),
        constraint: Constraint::AxisDiameter {
            point: rim,
            axis,
            value: Expression::Measure(25.0, Unit::Millimetre),
        },
        inactive: false,
    });
    let transaction = transaction.finish();
    let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
    assert!(text.contains("\"value\":\"12.5 mm\",\"diameter\":true"));
    let record: format::TransactionRecord = through_binary(&text);
    assert_eq!(format::restore_transaction(record), Some(transaction));
}

#[test]
fn an_extrusions_start_offset_is_saved_for_both_kinds_of_ends_and_older_files_have_none() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let lift = transaction.add_parameter("lift", transaction.parse("2 mm").unwrap());
    let sketch = transaction.add_feature("Outline", FeatureKind::from(Sketch::new(Plane::XY)));
    let distance = transaction.add_feature(
        "Distance",
        FeatureKind::Solid(caditor_document::SolidFeature::Extrude(
            caditor_document::Extrude {
                sketch,
                regions: caditor_document::RegionChoice::All,
                extent: caditor_document::ExtrudeExtent::one_side(
                    transaction.parse("4 mm").unwrap(),
                    false,
                ),
                operation: caditor_document::BodyOperation::NewBody,
                start: Some(caditor_document::SolidStart::Distance(
                    Expression::Parameter(lift),
                )),
                other_bodies: Vec::new(),
            },
        )),
    );
    let through = transaction.add_feature(
        "Through",
        FeatureKind::Solid(caditor_document::SolidFeature::Extrude(
            caditor_document::Extrude {
                sketch,
                regions: caditor_document::RegionChoice::All,
                extent: caditor_document::ExtrudeExtent::OneSide {
                    end: caditor_document::ExtrudeEnd::ThroughAll,
                    reversed: false,
                },
                operation: caditor_document::BodyOperation::Remove(distance),
                start: Some(caditor_document::SolidStart::Distance(
                    transaction.parse("-1.5 mm").unwrap(),
                )),
                other_bodies: Vec::new(),
            },
        )),
    );
    let plain = transaction.add_feature(
        "Plain",
        FeatureKind::Solid(caditor_document::SolidFeature::Extrude(
            caditor_document::Extrude {
                sketch,
                regions: caditor_document::RegionChoice::All,
                extent: caditor_document::ExtrudeExtent::one_side(
                    transaction.parse("1 mm").unwrap(),
                    true,
                ),
                operation: caditor_document::BodyOperation::NewBody,
                start: None,
                other_bodies: Vec::new(),
            },
        )),
    );
    document.apply(transaction.finish()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);

    assert_eq!(text.matches("\"start\"").count(), 2);
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    let start_of = |id| match &loaded.document.feature(id).unwrap().kind {
        FeatureKind::Solid(caditor_document::SolidFeature::Extrude(extrude)) => {
            extrude.start.clone()
        }
        other => panic!("expected an extrusion, found {other:?}"),
    };
    assert_eq!(
        start_of(distance),
        Some(caditor_document::SolidStart::Distance(
            Expression::Parameter(lift)
        ))
    );
    assert!(start_of(through).is_some());
    assert_eq!(start_of(plain), None);
}

#[test]
fn combines_are_saved_and_loaded() {
    use caditor_document::{
        BodyOperation, Combine, CombineOperation, Extrude, ExtrudeExtent, RegionChoice,
        SolidFeature,
    };
    let (mut document, base, _) = solid_model();
    let sketch = match &document.feature(base).unwrap().kind {
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => extrude.sketch,
        other => panic!("{other:?}"),
    };
    let mut transaction = document.transaction("Combine");
    let second = transaction.add_feature(
        "Second",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(transaction.parse("2 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
        })),
    );
    let combine = transaction.add_feature(
        "Combine 1",
        FeatureKind::Combine(Combine::new(base, second, CombineOperation::Cut)),
    );
    document.apply(transaction.finish()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);

    assert!(text.contains("\"combine\":{\"body\":"));
    assert!(text.contains("\"operation\":\"cut\""));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);

    let kind = document.feature(combine).unwrap().kind.clone();
    let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id: combine, kind });
    let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
    let record = through_binary(&text);
    assert_eq!(format::restore_transaction(record), Some(transaction));
}

#[test]
fn a_combine_with_several_tools_or_a_kept_tool_is_a_kind_older_readers_report_and_reads_back() {
    use caditor_document::{
        BodyOperation, Combine, CombineOperation, Extrude, ExtrudeExtent, RegionChoice,
        SolidFeature,
    };
    let (mut document, base, _) = solid_model();
    let sketch = match &document.feature(base).unwrap().kind {
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => extrude.sketch,
        other => panic!("{other:?}"),
    };
    let mut transaction = document.transaction("Combine");
    let mut tool = |name: &str, depth: &str| {
        transaction.add_feature(
            name,
            FeatureKind::Solid(SolidFeature::Extrude(Extrude {
                sketch,
                regions: RegionChoice::All,
                extent: ExtrudeExtent::one_side(transaction.parse(depth).unwrap(), false),
                operation: BodyOperation::NewBody,
                start: None,
                other_bodies: Vec::new(),
            })),
        )
    };
    let second = tool("Second", "2 mm");
    let third = tool("Third", "3 mm");
    let combine = transaction.add_feature(
        "Combine 1",
        FeatureKind::Combine(Combine {
            more_tools: vec![third],
            keep_tool: true,
            ..Combine::new(base, second, CombineOperation::Cut)
        }),
    );
    let kept_only = transaction.add_feature(
        "Combine 2",
        FeatureKind::Combine(Combine {
            keep_tool: true,
            ..Combine::new(base, second, CombineOperation::Join)
        }),
    );
    document.apply(transaction.finish()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let older = decode_text(&text.replace("combine_tools", "combine_with_tools"));

    assert!(text.contains("\"combine_tools\":{\"feature\":{\"combine\":{\"body\":"));
    assert!(text.contains(&format!("\"more_tools\":[{}]", third.raw())));
    assert!(text.contains("\"keep_tool\":true"));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert!(older.document.feature(combine).is_none());
    assert!(older.document.feature(kept_only).is_none());
    assert!(!older.issues.is_empty());

    for id in [combine, kept_only] {
        let kind = document.feature(id).unwrap().kind.clone();
        let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id, kind });
        let journaled = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();

        assert_eq!(
            format::restore_transaction(through_binary(&journaled)),
            Some(transaction)
        );
    }
}

#[test]
fn moves_are_saved_and_loaded() {
    use caditor_document::Move;
    let (mut document, base, _) = solid_model();
    let mut transaction = document.transaction("Move");
    let movement = transaction.add_feature(
        "Move 1",
        FeatureKind::Move(Move {
            about: caditor_document::TurnCentre::Origin,
            frame: None,
            body: base,
            offset: [
                transaction.parse("depth * 2").unwrap(),
                transaction.parse("-4 mm").unwrap(),
                transaction.parse("0 mm").unwrap(),
            ],
            turn: [
                transaction.parse("0 deg").unwrap(),
                transaction.parse("15 deg").unwrap(),
                transaction.parse("90 deg").unwrap(),
            ],
            copy: false,
        }),
    );
    let copy = transaction.add_feature(
        "Copy 1",
        FeatureKind::Move(Move {
            about: caditor_document::TurnCentre::Origin,
            frame: None,
            body: base,
            offset: [
                transaction.parse("0 mm").unwrap(),
                transaction.parse("20 mm").unwrap(),
                transaction.parse("0 mm").unwrap(),
            ],
            turn: [
                transaction.parse("0 deg").unwrap(),
                transaction.parse("0 deg").unwrap(),
                transaction.parse("0 deg").unwrap(),
            ],
            copy: true,
        }),
    );
    document.apply(transaction.finish()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);

    assert!(text.contains("\"move\":{\"body\":"));
    assert!(text.contains("\"copy\":{\"body\":"));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert!(loaded.document.feature(copy).unwrap().makes_body());

    let kind = document.feature(movement).unwrap().kind.clone();
    let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id: movement, kind });
    let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
    let record = through_binary(&text);
    assert_eq!(format::restore_transaction(record), Some(transaction));
}

#[test]
fn a_move_with_a_damaged_distance_loads_with_zero_and_says_so() {
    use caditor_document::Move;
    let (mut document, base, _) = solid_model();
    let mut transaction = document.transaction("Move");
    transaction.add_feature(
        "Move 1",
        FeatureKind::Move(Move {
            about: caditor_document::TurnCentre::Origin,
            frame: None,
            body: base,
            offset: [
                transaction.parse("5 mm").unwrap(),
                transaction.parse("0 mm").unwrap(),
                transaction.parse("0 mm").unwrap(),
            ],
            turn: [
                transaction.parse("0 deg").unwrap(),
                transaction.parse("0 deg").unwrap(),
                transaction.parse("0 deg").unwrap(),
            ],
            copy: false,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    let text = encode(&document).unwrap();
    assert!(text.contains("\"offset\":[\"5 mm\""));

    let loaded = decode_text(&text.replace("\"offset\":[\"5 mm\"", "\"offset\":[\"5 +* mm\""));

    assert_eq!(
        loaded.issues,
        ["The distance of “Move 1” could not be read, so it was set to 0 mm."]
    );
}

fn holed_model(style: caditor_document::HoleStyle, through: bool) -> (Document, FeatureId) {
    use caditor_document::{Hole, HoleDepth, SolidFeature};
    let (mut document, base, _) = solid_model();
    let sketch = match &document.feature(base).unwrap().kind {
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => extrude.sketch,
        other => panic!("{other:?}"),
    };
    let mut transaction = document.transaction("Hole");
    let depth = if through {
        HoleDepth::ThroughAll
    } else {
        HoleDepth::Blind(transaction.parse("depth * 2").unwrap())
    };
    let hole = transaction.add_feature(
        "Hole 1",
        FeatureKind::Hole(Hole {
            sketch,
            body: base,
            diameter: transaction.parse("4 mm").unwrap(),
            depth,
            style,
            reversed: through,
            shape: caditor_document::HoleShape::Round,
            standard: None,
            sizing: caditor_document::HoleSizing::Typed,
            bottom: caditor_document::HoleBottom::Flat,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    (document, hole)
}

#[test]
fn holes_of_every_style_are_saved_and_loaded() {
    use caditor_document::HoleStyle;
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    for (style, through) in [
        (HoleStyle::Plain, false),
        (
            HoleStyle::Counterbore {
                diameter: parse("8 mm"),
                depth: parse("1.5 mm"),
            },
            true,
        ),
        (
            HoleStyle::Countersink {
                diameter: parse("8 mm"),
                angle: parse("82 deg"),
            },
            false,
        ),
    ] {
        let (document, hole) = holed_model(style, through);

        let text = encode(&document).unwrap();
        let loaded = decode_text(&text);

        assert!(text.contains("\"hole\":{\"body\":"));
        assert_eq!(loaded.issues, Vec::<String>::new());
        assert_eq!(loaded.document, document);

        let kind = document.feature(hole).unwrap().kind.clone();
        let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id: hole, kind });
        let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
        let record = through_binary(&text);
        assert_eq!(format::restore_transaction(record), Some(transaction));
    }
}

#[test]
fn a_stepped_hole_is_a_kind_older_readers_report_and_reads_its_steps_back() {
    use caditor_document::{HoleSizing, HoleStep, HoleStyle};
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let style = HoleStyle::Stepped(vec![
        HoleStep {
            diameter: parse("10 mm"),
            depth: parse("1 mm"),
        },
        HoleStep {
            diameter: parse("7 mm"),
            depth: parse("$0 / 2"),
        },
    ]);
    let (mut document, hole) = holed_model(style, false);
    let mut sized = document.feature(hole).unwrap().kind.clone();
    if let FeatureKind::Hole(definition) = &mut sized {
        definition.sizing = HoleSizing::Circles;
    }
    let transaction = Transaction::single(
        "Size by circles",
        Edit::SetFeatureKind {
            id: hole,
            kind: sized,
        },
    );
    document.apply(transaction.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let older = decode_text(&text.replace("stepped_hole", "stepped_bore"));
    let damaged = decode_text(&text.replacen("\"depth\":\"$0 / 2\"", "\"depth\":\"((\"", 1));
    let journaled = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();

    assert!(text.contains("\"stepped_hole\":{\"feature\":{\"hole_by_circles\":"));
    assert!(
        text.contains("\"style\":{\"counterbore\":{\"depth\":\"1 mm\",\"diameter\":\"10 mm\"}}")
    );
    assert!(text.contains("\"steps\":[{\"depth\":\"1 mm\",\"diameter\":\"10 mm\"},"));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert!(older.document.feature(hole).is_none());
    assert!(!older.issues.is_empty());
    assert_eq!(damaged.issues.len(), 1, "{:?}", damaged.issues);
    assert!(
        damaged.issues[0].contains("step 2 depth"),
        "{:?}",
        damaged.issues
    );
    assert_eq!(
        format::restore_transaction(through_binary(&journaled)),
        Some(transaction)
    );
}

#[test]
fn a_hole_ending_in_a_drill_point_is_a_kind_older_readers_report_and_reads_back() {
    use caditor_document::{HoleBottom, HoleStep, HoleStyle};
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let style = HoleStyle::Stepped(vec![HoleStep {
        diameter: parse("10 mm"),
        depth: parse("1 mm"),
    }]);
    let (mut document, hole) = holed_model(style, false);
    let mut pointed = document.feature(hole).unwrap().kind.clone();
    if let FeatureKind::Hole(definition) = &mut pointed {
        definition.bottom = HoleBottom::DrillPoint(parse("118 deg"));
    }
    let transaction = Transaction::single(
        "Drill point",
        Edit::SetFeatureKind {
            id: hole,
            kind: pointed,
        },
    );
    document.apply(transaction.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let older = decode_text(&text.replace("drill_point_hole", "drill_tip_hole"));
    let damaged = decode_text(&text.replacen("\"angle\":\"118 deg\"", "\"angle\":\"((\"", 1));
    let journaled = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();

    assert!(
        text.contains("\"drill_point_hole\":{\"angle\":\"118 deg\",\"feature\":{\"stepped_hole\":")
    );
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert!(older.document.feature(hole).is_none());
    assert!(!older.issues.is_empty());
    assert_eq!(damaged.issues.len(), 1, "{:?}", damaged.issues);
    assert!(
        damaged.issues[0].contains("drill point angle"),
        "{:?}",
        damaged.issues
    );
    assert_eq!(
        format::restore_transaction(through_binary(&journaled)),
        Some(transaction)
    );
}

#[test]
fn a_hole_sized_by_its_circles_is_a_record_kind_of_its_own() {
    let (mut document, hole) = holed_model(caditor_document::HoleStyle::Plain, false);
    let mut sized = document.feature(hole).unwrap().kind.clone();
    if let FeatureKind::Hole(definition) = &mut sized {
        definition.sizing = caditor_document::HoleSizing::Circles;
    }
    let transaction = Transaction::single(
        "Size by circles",
        Edit::SetFeatureKind {
            id: hole,
            kind: sized,
        },
    );
    document.apply(transaction.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();

    assert!(text.contains("\"hole_by_circles\":{\"body\":"));
    assert!(!text.contains("\"hole\":{"));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(
        format::restore_transaction(through_binary(&journaled)),
        Some(transaction)
    );
}

#[test]
fn a_hole_scaling_its_head_by_circles_and_new_standard_fits_round_trip() {
    use caditor_document::{FinePitch, HoleFit, HoleSizing, HoleStandard, MetricSize};
    let (mut document, hole) = holed_model(caditor_document::HoleStyle::Plain, false);
    let mut sized = document.feature(hole).unwrap().kind.clone();
    if let FeatureKind::Hole(definition) = &mut sized {
        definition.sizing = HoleSizing::CirclesAndHeads;
        definition.standard = Some(HoleStandard {
            size: MetricSize::M10,
            fit: HoleFit::TappedFine(FinePitch::Third),
        });
    }
    document
        .apply(Transaction::single(
            "Size by circles",
            Edit::SetFeatureKind {
                id: hole,
                kind: sized,
            },
        ))
        .unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let insert = decode_text(
        &text
            .replace("tapped_fine_3", "heat_set_insert")
            .replace("\"M10\"", "\"M8\""),
    );
    let mismatched = decode_text(&text.replace("\"M10\"", "\"M3\""));
    let standard_of = |loaded: &Loaded| {
        loaded
            .document
            .feature(hole)
            .and_then(|feature| feature.kind.hole())
            .and_then(|hole| hole.standard)
    };

    assert!(
        text.contains("\"hole_scaled_by_circles\":{\"body\":"),
        "{text}"
    );
    assert!(text.contains("\"fit\":\"tapped_fine_3\""), "{text}");
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(insert.issues, Vec::<String>::new());
    assert_eq!(
        standard_of(&insert).map(|standard| standard.fit),
        Some(HoleFit::HeatSetInsert)
    );
    assert_eq!(standard_of(&mismatched), None);
    assert_eq!(
        mismatched.issues,
        [
            "The standard size of “Hole 1” (M3 tapped_fine_3) is not one this version of caditor \
          knows, so its sizes are kept as typed values."
        ]
    );
}

#[test]
fn a_hole_with_a_damaged_depth_loads_with_a_default_and_says_so() {
    let (document, _) = holed_model(caditor_document::HoleStyle::Plain, false);
    let text = encode(&document).unwrap();
    assert!(text.contains("\"blind\":\"$0 * 2\""));

    let loaded = decode_text(&text.replace("\"blind\":\"$0 * 2\"", "\"blind\":\"$0 +* 2\""));

    assert_eq!(
        loaded.issues,
        ["The depth of “Hole 1” could not be read, so it was set to 10 mm."]
    );
}

fn mirrored_model(plane: PlaneReference, keep_original: bool) -> (Document, FeatureId) {
    use caditor_document::Mirror;
    let (mut document, base, _) = solid_model();
    let mut transaction = document.transaction("Mirror");
    let mirror = transaction.add_feature(
        "Mirror 1",
        FeatureKind::Mirror(Mirror {
            body: base,
            plane,
            keep_original,
            mirrored: Vec::new(),
        }),
    );
    document.apply(transaction.finish()).unwrap();
    (document, mirror)
}

#[test]
fn mirrors_across_principal_planes_and_faces_are_saved_and_loaded() {
    use caditor_kernel::{FaceName, FaceOrigin, FaceReference};
    let (_, base, _) = solid_model();
    let face = PlaneReference::Face(FaceAttachment {
        body: base,
        face: FaceReference::new(
            FaceName::from_digest(0xface),
            Some(FaceOrigin::EndCap {
                feature: base.raw(),
            }),
            [FaceName::from_digest(7)],
        ),
    });
    for (plane, keep_original) in [
        (PlaneReference::Principal(PrincipalPlane::Xz), false),
        (face, true),
    ] {
        let (document, mirror) = mirrored_model(plane, keep_original);

        let text = encode(&document).unwrap();
        let loaded = decode_text(&text);

        assert!(text.contains("\"mirror\":{\"body\":"));
        assert_eq!(loaded.issues, Vec::<String>::new());
        assert_eq!(loaded.document, document);

        let kind = document.feature(mirror).unwrap().kind.clone();
        let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id: mirror, kind });
        let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
        let record = through_binary(&text);
        assert_eq!(format::restore_transaction(record), Some(transaction));
    }
}

#[test]
fn a_mirror_with_a_damaged_plane_loads_across_the_yz_plane_and_says_so() {
    let (document, mirror) = mirrored_model(PlaneReference::Principal(PrincipalPlane::Xy), true);
    let text = encode(&document).unwrap();
    assert!(text.contains("\"plane\":{\"principal\":\"xy\"}"));

    let loaded = decode_text(&text.replace(
        "\"plane\":{\"principal\":\"xy\"}",
        "\"plane\":{\"principal\":\"uv\"}",
    ));

    assert_eq!(
        loaded.issues,
        [
            "The plane “Mirror 1” mirrors across could not be read, so it mirrors across the YZ plane."
        ]
    );
    let restored = loaded
        .document
        .feature(mirror)
        .unwrap()
        .kind
        .mirror()
        .unwrap();
    assert_eq!(
        restored.plane,
        PlaneReference::Principal(PrincipalPlane::Yz)
    );
    assert!(restored.keep_original);
}

fn split_model(plane: PlaneReference, flipped: bool) -> (Document, FeatureId) {
    use caditor_document::Split;
    let (mut document, base, _) = solid_model();
    let mut transaction = document.transaction("Split");
    let split = transaction.add_feature(
        "Split 1",
        FeatureKind::Split(Split {
            body: base,
            plane,
            flipped,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    (document, split)
}

#[test]
fn splits_are_saved_and_loaded_with_their_side() {
    for flipped in [false, true] {
        let (document, split) = split_model(PlaneReference::Principal(PrincipalPlane::Xz), flipped);

        let text = encode(&document).unwrap();
        let loaded = decode_text(&text);

        assert!(text.contains("\"split\":{\"body\":"));
        assert_eq!(text.contains("\"flipped\":true"), flipped);
        assert_eq!(loaded.issues, Vec::<String>::new());
        assert_eq!(loaded.document, document);

        let kind = document.feature(split).unwrap().kind.clone();
        let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id: split, kind });
        let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
        let record = through_binary(&text);
        assert_eq!(format::restore_transaction(record), Some(transaction));
    }
}

#[test]
fn a_split_with_a_damaged_plane_loads_along_the_yz_plane_and_says_so() {
    let (document, split) = split_model(PlaneReference::Principal(PrincipalPlane::Xy), true);
    let text = encode(&document).unwrap();

    let loaded = decode_text(&text.replace(
        "\"plane\":{\"principal\":\"xy\"}",
        "\"plane\":{\"principal\":\"uv\"}",
    ));

    assert_eq!(
        loaded.issues,
        ["The plane “Split 1” splits along could not be read, so it splits along the YZ plane."]
    );
    let restored = loaded
        .document
        .feature(split)
        .unwrap()
        .kind
        .split()
        .unwrap();
    assert_eq!(
        restored.plane,
        PlaneReference::Principal(PrincipalPlane::Yz)
    );
    assert!(restored.flipped);
}

fn scaled_model(factor: &str) -> (Document, FeatureId) {
    use caditor_document::Scale;
    let (mut document, base, _) = solid_model();
    let mut transaction = document.transaction("Scale");
    let scale = transaction.add_feature(
        "Scale 1",
        FeatureKind::Scale(Scale {
            body: base,
            factor: transaction.parse(factor).unwrap(),
            center: [
                transaction.parse("depth").unwrap(),
                transaction.parse("-4 mm").unwrap(),
                transaction.parse("0 mm").unwrap(),
            ],
        }),
    );
    document.apply(transaction.finish()).unwrap();
    (document, scale)
}

#[test]
fn scales_are_saved_and_loaded() {
    let (document, scale) = scaled_model("25.4");

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);

    assert!(text.contains("\"scale\":{\"body\":"));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);

    let kind = document.feature(scale).unwrap().kind.clone();
    let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id: scale, kind });
    let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
    let record = through_binary(&text);
    assert_eq!(format::restore_transaction(record), Some(transaction));
}

#[test]
fn a_scale_with_a_damaged_factor_loads_as_one_and_says_so() {
    let (document, _) = scaled_model("2.5");
    let text = encode(&document).unwrap();
    assert!(text.contains("\"factor\":\"2.5\""));

    let loaded = decode_text(&text.replace("\"factor\":\"2.5\"", "\"factor\":\"2.5 +*\""));

    assert_eq!(
        loaded.issues,
        ["The scale factor of “Scale 1” could not be read, so it was set to 1."]
    );
}

#[test]
fn projected_geometry_and_its_sources_survive_saving_and_the_journal() {
    let mut document = sample();
    let plain = encode(&document).unwrap();
    let base = named(&document, "Base sketch");
    let side = named(&document, "Side sketch");
    let edge = caditor_kernel::EdgeReference::new(
        caditor_kernel::EdgeName::from_digest(5),
        [
            caditor_kernel::FaceName::from_digest(1),
            caditor_kernel::FaceName::from_digest(2),
        ],
        [
            caditor_kernel::VertexName::from_digest(3),
            caditor_kernel::VertexName::from_digest(4),
        ],
    );
    let mut transaction = document.transaction("Project");
    let line = transaction.add_projection(
        side,
        caditor_document::ProjectionSource::SketchEntity {
            sketch: base,
            entity: EntityId::from_raw(2),
        },
        &caditor_document::Outline::Line {
            start: Point2::ZERO,
            end: Point2::new(4.0, 1.0),
        },
    );
    let arc = transaction.add_projection(
        side,
        caditor_document::ProjectionSource::Edge {
            body: FeatureId::from_raw(90),
            edge,
        },
        &caditor_document::Outline::Arc {
            center: Point2::ZERO,
            start: Point2::new(2.0, 0.0),
            end: Point2::new(0.0, 2.0),
        },
    );
    let corner = transaction.add_projection(
        side,
        caditor_document::ProjectionSource::Vertex {
            body: FeatureId::from_raw(90),
            vertex: caditor_kernel::VertexName::from_digest(7),
        },
        &caditor_document::Outline::Point(Point2::new(-1.0, 3.0)),
    );
    let change = transaction.finish();
    let undo = document.apply(change.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&change)).unwrap());
    let undone: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&undo)).unwrap());
    let sketch = loaded
        .document
        .feature(side)
        .and_then(|feature| feature.kind.sketch())
        .unwrap();

    assert!(!plain.contains("projections"));
    assert!(text.contains("\"projections\""), "{text}");
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert!(
        [line, arc, corner]
            .iter()
            .all(|id| sketch.is_projected(*id))
    );
    assert_eq!(format::restore_transaction(journaled), Some(change));
    assert_eq!(format::restore_transaction(undone), Some(undo));
}

#[test]
fn intersected_geometry_and_its_sources_survive_saving_and_the_journal() {
    let mut document = sample();
    let side = named(&document, "Side sketch");
    let edge = caditor_kernel::EdgeReference::new(
        caditor_kernel::EdgeName::from_digest(15),
        [
            caditor_kernel::FaceName::from_digest(11),
            caditor_kernel::FaceName::from_digest(12),
        ],
        [
            caditor_kernel::VertexName::from_digest(13),
            caditor_kernel::VertexName::from_digest(14),
        ],
    );
    let mut transaction = document.transaction("Intersect");
    let cut = transaction.add_projection(
        side,
        caditor_document::ProjectionSource::Section {
            body: FeatureId::from_raw(90),
            edge,
        },
        &caditor_document::Outline::Circle {
            center: Point2::ZERO,
            radius: 3.0,
        },
    );
    let along = transaction.add_projection(
        side,
        caditor_document::ProjectionSource::DatumPlane {
            datum: FeatureId::from_raw(91),
            reach: 25.0,
        },
        &caditor_document::Outline::Line {
            start: Point2::new(0.0, -25.0),
            end: Point2::new(0.0, 25.0),
        },
    );
    let change = transaction.finish();
    let undo = document.apply(change.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&change)).unwrap());
    let undone: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&undo)).unwrap());
    let sketch = loaded
        .document
        .feature(side)
        .and_then(|feature| feature.kind.sketch())
        .unwrap();

    assert!(text.contains("\"section\""), "{text}");
    assert!(text.contains("\"datum_plane\""), "{text}");
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert!(sketch.is_projected(cut) && sketch.is_projected(along));
    assert_eq!(format::restore_transaction(journaled), Some(change));
    assert_eq!(format::restore_transaction(undone), Some(undo));
}

#[test]
fn an_unreadable_projection_leaves_ordinary_geometry_and_is_reported() {
    let mut document = sample();
    let side = named(&document, "Side sketch");
    let mut transaction = document.transaction("Project");
    let line = transaction.add_projection(
        side,
        caditor_document::ProjectionSource::SketchEntity {
            sketch: FeatureId::from_raw(0),
            entity: EntityId::from_raw(2),
        },
        &caditor_document::Outline::Line {
            start: Point2::ZERO,
            end: Point2::new(4.0, 1.0),
        },
    );
    document.apply(transaction.finish()).unwrap();
    let text = encode(&document)
        .unwrap()
        .replacen("\"sketch_entity\"", "\"hologram\"", 1);

    let loaded = decode_text(&text);
    let sketch = loaded
        .document
        .feature(side)
        .and_then(|feature| feature.kind.sketch())
        .unwrap();

    assert!(issues_mention(
        &loaded,
        "the source of projected geometry could not be read"
    ));
    assert!(sketch.entity(line).is_some());
    assert!(!sketch.is_projected(line));
}

fn named(document: &Document, name: &str) -> FeatureId {
    document
        .features()
        .find(|feature| feature.name == name)
        .map(|feature| feature.id())
        .unwrap()
}

#[test]
fn a_slotted_hole_of_a_standard_size_is_saved_journaled_and_loaded() {
    use caditor_document::{HoleFit, HoleShape, HoleStandard, HoleStyle, MetricSize};
    let (mut document, hole) = holed_model(HoleStyle::Plain, false);
    let plain = encode(&document).unwrap();
    let FeatureKind::Hole(definition) = document.feature(hole).unwrap().kind.clone() else {
        panic!("a hole");
    };
    let change = Transaction::single(
        "Slot",
        Edit::SetFeatureKind {
            id: hole,
            kind: FeatureKind::Hole(caditor_document::Hole {
                shape: HoleShape::Slot {
                    length: Expression::parse_stored("12 mm").unwrap(),
                    angle: Expression::parse_stored("30 deg").unwrap(),
                },
                standard: Some(HoleStandard {
                    size: MetricSize::M2_5,
                    fit: HoleFit::Tapped,
                }),
                ..definition
            }),
        },
    );
    document.apply(change.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&change)).unwrap());

    assert!(!plain.contains("slot") && !plain.contains("standard"));
    assert!(
        text.contains("\"standard\":{\"fit\":\"tapped\",\"size\":\"M2.5\"}"),
        "{text}"
    );
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(format::restore_transaction(journaled), Some(change));

    let unknown = decode_text(&text.replace("\"M2.5\"", "\"M2.6\""));

    assert!(issues_mention(
        &unknown,
        "(M2.6 tapped) is not one this version"
    ));
    let FeatureKind::Hole(read) = &unknown.document.feature(hole).unwrap().kind else {
        panic!("a hole");
    };
    assert_eq!(read.standard, None);
    assert!(matches!(read.shape, HoleShape::Slot { .. }));
}

#[test]
fn a_feature_group_is_saved_journaled_and_cut_to_its_limit_when_too_long() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let first = transaction.add_feature("Outline", FeatureKind::from(Sketch::new(Plane::XY)));
    let second = transaction.add_feature("Holes", FeatureKind::from(Sketch::new(Plane::XY)));
    document.apply(transaction.finish()).unwrap();
    let grouping = document
        .grouping(&[first, second], "Base sketches", "Group features")
        .unwrap();
    document.apply(grouping.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let long = "x".repeat(caditor_document::MAX_GROUP_NAME_CHARS + 5);
    let cut = decode_text(&text.replacen("Base sketches", &long, 1));
    let journaled = serde_json::to_string(&format::transaction_record(&grouping)).unwrap();

    assert!(text.contains("\"group\":\"Base sketches\""));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(cut.issues.len(), 1, "{:?}", cut.issues);
    assert_eq!(
        cut.document.feature(first).unwrap().group.as_deref(),
        Some(&long[..caditor_document::MAX_GROUP_NAME_CHARS])
    );
    assert_eq!(
        format::restore_transaction(through_binary(&journaled)),
        Some(grouping)
    );
}

fn saved_twice(path: &Path) -> FileDigest {
    save(&sample(), path, false).unwrap();
    save(&Document::default(), path, false).unwrap().digest
}

#[test]
fn a_kept_version_is_stored_in_the_file_and_stays_through_later_saves() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let head = saved_twice(&path);
    let listed = crate::history(&path).unwrap();
    assert_eq!(listed.versions.len(), 1);
    assert!(!listed.versions[0].kept);

    set_version_kept(&path, 0, true, Some(&head)).unwrap();

    let kept = crate::history(&path).unwrap();
    assert!(kept.versions[0].kept);
    assert_eq!(kept.versions[0].state, listed.versions[0].state);
    assert_eq!(load(&path).unwrap().digest, Some(head));
    assert_eq!(load(&path).unwrap().document, Document::default());
    assert_eq!(load_version(&path, 0).unwrap().document, sample());
    assert_eq!(files_in(dir.path()), ["model.caditor"]);

    save(&sample(), &path, false).unwrap();
    let after_save = crate::history(&path).unwrap();
    assert_eq!(after_save.versions.len(), 2);
    assert!(after_save.versions[1].kept);
    assert!(!after_save.versions[0].kept);
    assert_eq!(load_version(&path, 1).unwrap().document, sample());

    set_version_kept(&path, 1, false, None).unwrap();
    assert!(
        crate::history(&path)
            .unwrap()
            .versions
            .iter()
            .all(|version| !version.kept)
    );
}

#[test]
fn keeping_a_version_refuses_a_file_changed_outside_and_a_version_that_is_gone() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let head = saved_twice(&path);
    let before = fs::read(&path).unwrap();
    save(&sample(), &path, false).unwrap();
    let outside = fs::read(&path).unwrap();

    assert_eq!(
        set_version_kept(&path, 0, true, Some(&head)),
        Err(KeepVersionError::ChangedOnDisk)
    );
    assert_eq!(fs::read(&path).unwrap(), outside);
    assert_ne!(outside, before);

    assert_eq!(
        set_version_kept(&path, 7, true, None),
        Err(KeepVersionError::NoSuchVersion)
    );
    assert_eq!(
        set_version_kept(&dir.path().join("missing.caditor"), 0, true, None),
        Err(KeepVersionError::Unreadable {
            name: "missing.caditor".to_owned(),
            failure: crate::ReadFailure::NotFound,
        })
    );
    assert_eq!(fs::read(&path).unwrap(), outside);
    assert_eq!(files_in(dir.path()), ["model.caditor"]);
}

#[test]
fn the_storage_worker_keeps_a_version_in_order_with_saves_and_only_for_its_own_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let head = saved_twice(&path);
    let start = Start {
        file: Some(path.clone()),
        on_disk: Some(head),
        ..untitled(&Document::default())
    };
    let storage = Storage::spawn(config(&dir), start, || {}).unwrap();

    storage
        .keep_version(KeepRequest {
            path: path.clone(),
            index: 0,
            kept: true,
        })
        .unwrap();
    let kept = wait_for_report(&storage);
    storage
        .save(save_request(1, &sample(), &path, false))
        .unwrap();
    let saved = wait_for_report(&storage);
    storage
        .keep_version(KeepRequest {
            path: dir.path().join("other.caditor"),
            index: 0,
            kept: true,
        })
        .unwrap();
    let refused = wait_for_report(&storage);

    assert_eq!(
        kept,
        Report::VersionKept {
            path: path.clone(),
            index: 0,
            kept: true,
        }
    );
    assert!(matches!(saved, Report::Saved { ticket: 1, .. }));
    assert_eq!(
        refused,
        Report::KeepFailed {
            path: dir.path().join("other.caditor"),
            error: KeepVersionError::NotTheOpenModel,
        }
    );
    let listed = crate::history(&path).unwrap();
    assert_eq!(listed.versions.len(), 2);
    assert!(listed.versions[1].kept);
    assert!(storage.close(true).wait(WAIT));
}

fn threaded_model() -> (Document, FeatureId) {
    use caditor_document::{Thread, ThreadFamily, ThreadHand, ThreadLength, ThreadSize};
    use caditor_kernel::{FaceName, FaceOrigin, FaceReference};
    let (mut document, base, _) = solid_model();
    let side = FaceReference::new(
        FaceName::from_digest(0xfeed),
        Some(FaceOrigin::Side {
            feature: 1,
            entity: 3,
        }),
        [FaceName::from_digest(2)],
    );
    let family = ThreadFamily::Trapezoidal;
    let mut transaction = document.transaction("Thread");
    let thread = transaction.add_feature(
        "Thread 1",
        FeatureKind::Thread(Thread {
            body: base,
            face: side,
            size: ThreadSize::from_id(family, "Tr 20x4").unwrap(),
            class: family.class_from_id("7e").unwrap(),
            hand: ThreadHand::Left,
            length: ThreadLength::Depth(transaction.parse("depth * 2").unwrap()),
            reversed: true,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    (document, thread)
}

#[test]
fn a_thread_is_saved_and_loaded_as_a_record_of_its_own() {
    let (document, thread) = threaded_model();
    let text = encode(&document).unwrap();
    assert!(text.contains(
        "\"thread\":{\"body\":1,\"class\":\"7e\",\"depth\":\"$0 * 2\",\"face\":{\"face\":\
         \"0000000000000000000000000000feed\",\"neighbours\":[\"00000000000000000000000000000002\"],\
         \"origin\":{\"side\":{\"entity\":3,\"feature\":1}}},\"left_handed\":true,\
         \"reversed\":true,\"size\":\"Tr 20x4\",\"standard\":\"trapezoidal\"}"
    ));
    let loaded = decode_text(&text);
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);

    let kind = document.feature(thread).unwrap().kind.clone();
    let transaction = Transaction::single("Edit", Edit::SetFeatureKind { id: thread, kind });
    let text = serde_json::to_string(&format::transaction_record(&transaction)).unwrap();
    let record = through_binary(&text);
    assert_eq!(format::restore_transaction(record), Some(transaction));
}

#[test]
fn an_unreadable_thread_size_or_class_takes_a_default_and_is_reported() {
    use caditor_document::ThreadSize;
    let (document, thread) = threaded_model();
    let text = encode(&document)
        .unwrap()
        .replacen("Tr 20x4", "Tr 999x1", 1)
        .replacen("\"7e\"", "\"9z\"", 1);
    let loaded = decode_text(&text);
    assert_eq!(
        loaded.issues,
        [
            "The thread size of “Thread 1” could not be read, so it was set to Tr 8x1.5.",
            "The thread class of “Thread 1” could not be read, so it was set to 7H."
        ]
    );
    let restored = loaded.document.feature(thread).unwrap();
    let restored = restored.kind.thread().unwrap();
    assert_eq!(restored.size.id(), "Tr 8x1.5");
    assert_eq!(restored.class.id(), "7H");
    assert_ne!(restored.size, ThreadSize::M8);
}

#[test]
fn coordinate_systems_and_what_uses_them_are_kinds_older_readers_report_and_read_back() {
    use caditor_document::{
        AxisReference, Datum, DatumAxis, DatumFrame, DatumPlane, Move, PointReference,
        PrincipalAxis, TurnCentre,
    };
    use caditor_kernel::VertexName;
    let (mut document, base, _) = solid_model();
    let mut transaction = document.transaction("Coordinate systems");
    let frame = transaction.add_feature(
        "Coordinate system 1",
        FeatureKind::Datum(Datum::Frame(Box::new(DatumFrame {
            origin: PointReference::Vertex {
                body: base,
                vertex: VertexName::from_digest(0xc0),
            },
            x_axis: AxisReference::Principal(PrincipalAxis::Y),
            plane: PlaneReference::Principal(PrincipalPlane::Xz),
            reverse_x: true,
            reverse_z: false,
        }))),
    );
    transaction.add_feature(
        "Axis 1",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Along(AxisReference::Frame {
            frame,
            axis: PrincipalAxis::Z,
        }))),
    );
    transaction.add_feature(
        "Plane 1",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Frame {
                frame,
                plane: PrincipalPlane::Yz,
            },
            rotation: None,
            offset: transaction.parse("3 mm").unwrap(),
        })),
    );
    let sketch = transaction.add_feature(
        "On the system",
        FeatureKind::Sketch(caditor_document::SketchFeature {
            sketch: Sketch::new(Plane::XY),
            attachment: Some(SketchAttachment::Frame {
                frame,
                plane: PrincipalPlane::Xy,
            }),
            projections: Default::default(),
        }),
    );
    let movement = transaction.add_feature(
        "Move body 1",
        FeatureKind::Move(Move {
            body: base,
            offset: std::array::from_fn(|_| transaction.parse("2 mm").unwrap()),
            turn: std::array::from_fn(|_| transaction.parse("10 deg").unwrap()),
            copy: false,
            about: TurnCentre::Body,
            frame: Some(frame),
        }),
    );
    let add = transaction.finish();
    document.apply(add.clone()).unwrap();
    let placed = Transaction::single(
        "Place",
        Edit::SetSketchPlacement {
            feature: sketch,
            plane: Plane::XZ,
            attachment: Some(SketchAttachment::Frame {
                frame,
                plane: PrincipalPlane::Xz,
            }),
        },
    );

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&add)).unwrap());
    let placement: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&placed)).unwrap());
    let older = decode_text(
        &text
            .replace("coordinate_system", "coordinate_frame")
            .replace("move_in_frame", "move_in_axes")
            .replace("sketch_on_frame", "sketch_on_axes"),
    );

    assert!(
        text.contains("\"coordinate_system\":{\"origin\":{\"vertex\":"),
        "{text}"
    );
    assert!(text.contains("\"reverse_x\":true"));
    assert!(text.contains("\"along\":{\"frame\":{\"axis\":\"z\",\"frame\":"));
    assert!(text.contains("\"base\":{\"frame\":{\"frame\":"));
    assert!(text.contains("\"move_in_frame\":{\"feature\":{\"move_about_centre\":"));
    assert!(text.contains("\"sketch_on_frame\":{\"feature\":{\"sketch\":"));
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(format::restore_transaction(journaled), Some(add));
    assert_eq!(format::restore_transaction(placement), Some(placed));
    assert!(older.document.feature(frame).is_none());
    assert!(older.document.feature(movement).is_none());
    assert!(older.document.feature(sketch).is_none());
    assert!(!older.issues.is_empty());
}
