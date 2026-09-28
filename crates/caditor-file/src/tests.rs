use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use caditor_document::{Document, Edit, Editor, FeatureId, FeatureKind, Transaction};
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, Sketch};
use tempfile::TempDir;

use super::*;

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
    assert!(text.starts_with("{\"format\":\"caditor\",\"version\":4}\n"));
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

    let loaded = decode_lines(&lines);

    assert_eq!(
        loaded.issues,
        [
            "This model was made by a newer version of caditor (format 7). Anything this version \
             does not understand was left out.",
            "The feature “Pad” is a kind this version of caditor does not know (loft), so it \
             was left out. It may come from a newer version.",
            "Line 7 holds something this version of caditor does not know (assembly), so it was \
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
    let sketch = document.features().next().unwrap().kind.sketch().unwrap();
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
        ["The start of the file is damaged; the rest was read as a version 4 model."]
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

fn version_one_sample() -> Document {
    let mut document = Document::default();
    let mut transaction = document.transaction("Sample");
    let width = transaction.add_parameter("width", transaction.parse("40 mm").unwrap());
    transaction.add_feature(
        "Base sketch",
        FeatureKind::from(dimensioned_line(
            Plane::XY,
            40.0,
            Expression::Parameter(width),
        )),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

#[test]
fn a_version_1_file_still_loads_exactly() {
    let text = r#"{"format":"caditor","version":1}
{"parameter":{"id":0,"name":"width","expression":"40 mm"}}
{"feature":{"id":0,"name":"Base sketch","sketch":{"plane":{"origin":[0.0,0.0,0.0],"normal":[0.0,0.0,1.0],"x_axis":[1.0,0.0,0.0]},"entities":[{"id":0,"point":[0.0,0.0]},{"id":1,"point":[40.0,0.0]},{"id":2,"line":{"start":0,"end":1}}],"constraints":[{"id":3,"horizontal":2},{"id":4,"distance":{"from":0,"to":1,"value":"$0"}}],"next_id":5}}}
{"next_ids":{"parameter":1,"feature":1}}
"#;
    let loaded = decode(text.as_bytes()).unwrap();
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, version_one_sample());
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
        value: Expression::Measure(30.0, Unit::Degree),
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
        "\"radius\":{\"entity\":",
        "\"coincident\":[0,18446744073709551615]",
        "\"value\":\"30 deg\"",
    ] {
        assert!(text.contains(record), "{record} is missing from {text}");
    }

    let loaded = decode(text.as_bytes()).unwrap();
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

    let loaded = decode(text.as_bytes()).unwrap();

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
    ] {
        assert!(text.contains(record), "{record} is missing from {text}");
    }

    let record: format::TransactionRecord = serde_json::from_str(&text).unwrap();
    assert_eq!(format::restore_transaction(record), Some(transaction));

    let damaged: format::TransactionRecord =
        serde_json::from_str(&text.replace("\"$0 / 3\"", "\"$0 //\"")).unwrap();
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
    let original = fs::read_to_string(&journal).unwrap();

    let mut lines: Vec<String> = original.lines().map(str::to_owned).collect();
    let (_, entry) = lines[3].split_once(",\"entry\":").unwrap();
    let entry = entry
        .strip_suffix('}')
        .unwrap()
        .replace("set_parameter_expression", "bend_sheet");
    let crc = crc32fast::hash(entry.as_bytes());
    lines[3] = format!("{{\"crc\":\"{crc:08x}\",\"entry\":{entry}}}");
    fs::write(&journal, lines.join("\n")).unwrap();

    let Inspection::Recoverable(recovered) = inspect(&journal).unwrap() else {
        panic!("the journal should still be recoverable");
    };
    assert_eq!(recovered.changes(), 1);
    assert_eq!(recovered.issues.len(), 1);
}

fn solid_model() -> (Document, FeatureId, FeatureId) {
    use caditor_document::{
        BodyOperation, Extrude, ExtrudeExtent, RegionChoice, Revolve, RevolveExtent, SolidFeature,
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
            extent: ExtrudeExtent::TwoSides {
                forward: Expression::Parameter(depth),
                backward: transaction.parse("1 mm").unwrap(),
            },
            operation: BodyOperation::NewBody,
        })),
    );
    let turned = transaction.add_feature(
        "Turned",
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch,
            regions: RegionChoice::All,
            axis: EntityId::HORIZONTAL_AXIS,
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
    let loaded = decode(text.as_bytes()).unwrap();
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
}

#[test]
fn a_damaged_solid_value_falls_back_and_is_reported() {
    let (document, _, _) = solid_model();
    let text = encode(&document)
        .unwrap()
        .replace("\"angle\":\"90 deg\"", "\"angle\":\"90 ((\"");
    let loaded = decode(text.as_bytes()).unwrap();
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
    let record = serde_json::from_str(&text).unwrap();
    assert_eq!(format::restore_transaction(record), Some(transaction));
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
        "\"attachment\":{\"body\":1,\"face\":\"feed0000000000000000000000000001\",\"origin\":\
         {\"end_cap\":{\"feature\":1}},\"neighbours\":[\"00000000000000000000000000000003\",\
         \"ffffffffffffffffffffffffffffffff\"]}"
    ));
    let loaded = decode(text.as_bytes()).unwrap();
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
}

#[test]
fn a_sketch_whose_face_cannot_be_read_stays_where_it_was() {
    let (document, _, sketch) = attached_model();
    let text = encode(&document)
        .unwrap()
        .replace("feed0000000000000000000000000001", "not a digest");
    let loaded = decode(text.as_bytes()).unwrap();
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
fn a_sketch_on_a_lost_body_stays_where_it_was() {
    let (document, base, sketch) = attached_model();
    let lines: Vec<String> = encode(&document)
        .unwrap()
        .lines()
        .filter(|line| !line.contains("\"name\":\"Base\""))
        .map(str::to_owned)
        .collect();
    let loaded = decode_lines(&lines);
    assert!(loaded.document.feature(base).is_none());
    assert!(issues_mention(
        &loaded,
        "“Top” lay on a face of a body that could not be restored, so the sketch now stays where \
         it was."
    ));
    let restored = loaded.document.feature(sketch).unwrap();
    assert!(restored.kind.attachment().is_none());
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
        let record = serde_json::from_str(&text).unwrap();
        assert_eq!(format::restore_transaction(record), Some(transaction));
    }
}
