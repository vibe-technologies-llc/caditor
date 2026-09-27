use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use caditor_document::{Document, Edit, Editor, FeatureKind, Transaction};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::{Constraint, Entity, Sketch};
use tempfile::TempDir;

use super::*;

const WAIT: Duration = Duration::from_secs(10);

fn dimensioned_line(plane: Plane, length: f64, value: Expression) -> Sketch {
    let mut sketch = Sketch::new(plane);
    let line = sketch.add_line(Point2::ZERO, Point2::new(length, 0.0));
    let Some(Entity::Line { start, end }) = sketch.entity(line).cloned() else {
        panic!("expected a line");
    };
    sketch.add_constraint(Constraint::Horizontal(line));
    sketch.add_constraint(Constraint::Distance {
        from: start,
        to: end,
        value,
    });
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
        FeatureKind::Sketch(dimensioned_line(
            Plane::XY,
            40.0,
            Expression::Parameter(width),
        )),
    );
    let side = dimensioned_line(tilted, 1.0 / 3.0, transaction.parse("height * 2").unwrap());
    transaction.add_feature("Side sketch", FeatureKind::Sketch(side));
    let scrap_feature =
        transaction.add_feature("Scrap", FeatureKind::Sketch(Sketch::new(Plane::YZ)));
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
    encode(document)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn decode_lines(lines: &[String]) -> Loaded {
    decode(lines.join("\n").as_bytes()).unwrap()
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

    assert_eq!(save(&document, &path, false), Ok(None));
    let loaded = load(&path).unwrap();

    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(loaded.document.next_parameter_id(), 3);
    assert_eq!(loaded.document.next_feature_id(), 3);
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("{\"format\":\"caditor\",\"version\":1}\n"));
    assert!(text.contains("\"expression\":\"$0 / 2 + 0.1 mm\""));
    assert_eq!(text.lines().count(), 1 + 2 + 2 + 1);
    assert_eq!(files_in(dir.path()), ["model.caditor"]);
}

#[test]
fn saving_over_a_file_keeps_its_permissions_and_leaves_no_temporary_files() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    fs::write(&path, "old").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

    save(&sample(), &path, false).unwrap();

    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640);
    assert_eq!(files_in(dir.path()), ["model.caditor"]);
    assert_eq!(load(&path).unwrap().document, sample());
}

#[test]
fn a_failed_save_reports_a_plain_reason_and_leaves_the_folder_clean() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("missing").join("model.caditor");
    let error = save(&sample(), &path, false).unwrap_err();
    assert_eq!(error.reason, "its folder no longer exists");
    assert_eq!(files_in(dir.path()), Vec::<String>::new());
}

#[test]
fn overwriting_a_damaged_file_keeps_the_original_as_a_backup() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    fs::write(&path, "damaged original").unwrap();
    fs::write(dir.path().join("model.damaged.caditor"), "earlier backup").unwrap();

    let backup = save(&sample(), &path, true).unwrap().unwrap();

    assert_eq!(backup, dir.path().join("model.damaged-2.caditor"));
    assert_eq!(fs::read_to_string(&backup).unwrap(), "damaged original");
    assert_eq!(load(&path).unwrap().document, sample());
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

    assert_eq!(loaded.issues, ["Line 5 is damaged and was left out."]);
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
    lines[1] = lines[1].replace("\"expression\":\"40 mm\"", "\"expression\":40");
    let loaded = decode_lines(&lines);

    assert!(issues_mention(
        &loaded,
        "The parameter “width” is damaged and was left out."
    ));
    assert!(issues_mention(&loaded, "It was replaced by “width” = 0"));
    let width = loaded.document.parameter_named("width").unwrap();
    assert_eq!(width.expression, Expression::Number(0.0));
    assert_eq!(loaded.document.features().len(), 2);

    lines.remove(1);
    let loaded = decode_lines(&lines);
    assert!(issues_mention(&loaded, "It was replaced by “lost_0” = 0"));
    let height = loaded.document.parameter_named("height").unwrap();
    assert_eq!(
        loaded.document.expression_text(&height.expression),
        "lost_0 / 2 + 0.1 mm"
    );
}

#[test]
fn content_from_a_newer_version_is_reported_and_the_rest_is_kept() {
    let document = sample();
    let mut lines = lines_of(&document);
    lines[0] = "{\"format\":\"caditor\",\"version\":7}".to_owned();
    let base = lines
        .iter()
        .position(|line| line.contains("Base sketch"))
        .unwrap();
    lines[base] = lines[base].replacen(
        "\"entities\":[",
        "\"entities\":[{\"id\":90,\"arc\":{\"center\":0}},",
        1,
    );
    let side = lines
        .iter()
        .position(|line| line.contains("Side sketch"))
        .unwrap();
    lines[side] =
        "{\"feature\":{\"id\":1,\"name\":\"Pad\",\"extrude\":{\"distance\":\"5\"}}}".to_owned();
    lines.push("{\"assembly\":{\"parts\":[]}}".to_owned());

    let loaded = decode_lines(&lines);

    assert_eq!(
        loaded.issues,
        [
            "This model was made by a newer version of caditor (format 7). Anything this version \
             does not understand was left out.",
            "The feature “Pad” is a kind this version of caditor does not know (extrude), so it \
             was left out. It may come from a newer version.",
            "Line 7 holds something this version of caditor does not know (assembly), so it was \
             left out. It may come from a newer version.",
            "In “Base sketch”, an entity of a kind this version of caditor does not know (arc) \
             was left out. It may come from a newer version.",
        ]
    );
    let base_sketch = loaded.document.features().next().unwrap();
    assert_eq!(**document.features().next().as_ref().unwrap(), *base_sketch);
}

#[test]
fn cycles_invalid_names_and_broken_dimensions_are_repaired_and_reported() {
    let text = r#"{"format":"caditor","version":1}
{"parameter":{"id":0,"name":"a","expression":"$1 + 1"}}
{"parameter":{"id":1,"name":"b","expression":"$0 * 2"}}
{"parameter":{"id":2,"name":"mm","expression":"3"}}
{"parameter":{"id":3,"name":"a","expression":"4"}}
{"parameter":{"id":3,"name":"twin","expression":"5"}}
{"parameter":{"id":4,"name":"typo","expression":"4 $$ 2"}}
{"feature":{"id":0,"name":" ","sketch":{"plane":{"origin":[0,0,0],"normal":[0,0,2],"x_axis":[0,0,1]},"entities":[{"id":0,"point":[0,0]},{"id":1,"point":[3,4]},{"id":2,"line":{"start":0,"end":9}}],"constraints":[{"id":3,"distance":{"from":0,"to":1,"value":"?"}},{"id":4,"vertical":2}],"next_id":5}}}
"#;
    let loaded = decode(text.as_bytes()).unwrap();

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
    let FeatureKind::Sketch(sketch) = &document.features().next().unwrap().kind;
    assert_eq!(sketch.entities().len(), 2);
    assert_eq!(sketch.constraints().len(), 1);
    assert_eq!(sketch.plane(), Plane::XY);
}

#[test]
fn files_that_are_not_models_are_refused_as_a_whole() {
    assert_eq!(decode(b""), Err(LoadError::Empty));
    assert_eq!(decode(b"\n  \n"), Err(LoadError::Empty));
    assert_eq!(
        decode(b"\x89PNG\r\n\x1a\n\0\0\0"),
        Err(LoadError::NotAModel)
    );
    assert_eq!(
        decode(b"{\"format\":\"svg\",\"version\":1}\n"),
        Err(LoadError::NotAModel)
    );
    let missing = load(Path::new("/nonexistent/model.caditor")).unwrap_err();
    assert_eq!(missing.to_string(), "it no longer exists");
}

#[test]
fn a_damaged_header_still_recovers_the_records() {
    let mut lines = lines_of(&sample());
    lines[0] = "{\"format\":\"cad".to_owned();
    let loaded = decode_lines(&lines);
    assert_eq!(
        loaded.issues,
        ["The start of the file is damaged; the rest was read as a version 1 model."]
    );
    assert_eq!(loaded.document, sample());
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
    let original = fs::read_to_string(&journal).unwrap();

    let torn = format!("{original}{{\"crc\":\"0000");
    fs::write(&journal, torn).unwrap();
    let Inspection::Recoverable(recovered) = inspect(&journal).unwrap() else {
        panic!("the journal should still be recoverable");
    };
    assert_eq!(recovered.changes(), 3);
    assert_eq!(recovered.issues.len(), 1);

    let mut lines: Vec<&str> = original.lines().collect();
    let flipped = lines[3].replace("60", "70");
    lines[3] = &flipped;
    fs::write(&journal, lines.join("\n")).unwrap();
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
        })
        .unwrap();
    assert_eq!(
        wait_for_report(&storage),
        Report::Saved {
            ticket: 7,
            path: path.clone(),
            backup: None
        }
    );
    assert_eq!(files_in(&recovery), Vec::<String>::new());
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
    assert!(matches!(
        journal_for(&path, Some(&recovery)),
        FileJournal::None
    ));
}

#[test]
fn a_journal_whose_changes_reached_the_file_is_tidied_away() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let base = sample();
    save(&base, &path, false).unwrap();
    let start = Start {
        file: Some(path.clone()),
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
    assert_eq!(files_in(dir.path()), ["model.caditor"]);
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
