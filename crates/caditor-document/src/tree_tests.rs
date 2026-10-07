use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::SamplingTolerance;
use caditor_sketch::Sketch;

use crate::*;

fn rectangle(plane: Plane, min: (f64, f64), max: (f64, f64)) -> Sketch {
    let mut sketch = Sketch::new(plane);
    let corners = [
        Point2::new(min.0, min.1),
        Point2::new(max.0, min.1),
        Point2::new(max.0, max.1),
        Point2::new(min.0, max.1),
    ];
    for index in 0..4 {
        sketch.add_line(corners[index], corners[(index + 1) % 4]);
    }
    sketch
}

fn extrude(sketch: FeatureId, distance: &str, operation: BodyOperation) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent: ExtrudeExtent::one_side(Expression::parse_stored(distance).unwrap(), false),
        operation,
        start: None,
        other_bodies: Vec::new(),
    }))
}

fn evaluate(document: &Document, engine: &mut Recompute) -> Evaluation {
    engine.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn volume(evaluation: &Evaluation, body: FeatureId) -> f64 {
    evaluation
        .body(body)
        .unwrap()
        .tessellate(&SamplingTolerance::new(1e-3, 0.1).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn state(evaluation: &Evaluation, feature: FeatureId) -> &FeatureState {
    &evaluation.feature(feature).unwrap().state
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match state(evaluation, feature) {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

fn names(document: &Document) -> Vec<&str> {
    document
        .features()
        .map(|feature| feature.name.as_str())
        .collect()
}

const WHOLE: f64 = 320.0 - 8.0 + 36.0;

struct Model {
    document: Document,
    outline: FeatureId,
    base: FeatureId,
    hole: FeatureId,
    pocket: FeatureId,
    lug: FeatureId,
    boss: FeatureId,
    note: FeatureId,
}

fn model() -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (10.0, 8.0))),
    );
    let base = transaction.add_feature("Base", extrude(outline, "4 mm", BodyOperation::NewBody));
    let top = Plane::from_frame(Point3::new(0.0, 0.0, 4.0), Vector3::Z, Vector3::X).unwrap();
    let hole = transaction.add_feature(
        "Hole sketch",
        FeatureKind::from(rectangle(top, (2.0, 2.0), (4.0, 4.0))),
    );
    let pocket = transaction.add_feature(
        "Pocket",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: hole,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("2 mm").unwrap(), true),
            operation: BodyOperation::Remove(base),
            start: None,
            other_bodies: Vec::new(),
        })),
    );
    let lug = transaction.add_feature(
        "Lug sketch",
        FeatureKind::from(rectangle(top, (6.0, 2.0), (9.0, 6.0))),
    );
    let boss = transaction.add_feature("Boss", extrude(lug, "3 mm", BodyOperation::Add(base)));
    let note = transaction.add_feature(
        "Note",
        FeatureKind::from(rectangle(Plane::XZ, (0.0, 0.0), (1.0, 1.0))),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        outline,
        base,
        hole,
        pocket,
        lug,
        boss,
        note,
    }
}

#[test]
fn a_suppressed_feature_is_skipped_and_its_dependents_fail_naming_it_until_unsuppressed() {
    let model = model();
    let mut editor = Editor::new(model.document.clone());
    let mut engine = Recompute::default();

    editor
        .apply(
            model
                .document
                .suppression(&[model.lug], true, "Suppress Lug sketch"),
        )
        .unwrap();
    let suppressed = evaluate(editor.document(), &mut engine);
    editor.undo().unwrap();
    let restored = evaluate(editor.document(), &mut engine);
    editor.redo().unwrap();

    assert_eq!(state(&suppressed, model.lug), &FeatureState::Suppressed);
    assert!(suppressed.feature(model.lug).unwrap().result.is_none());
    let error = failure(&suppressed, model.boss);
    assert_eq!(error.reason, "It uses Lug sketch, which is suppressed.");
    assert_eq!(
        error.remedy,
        "Unsuppress Lug sketch, or suppress this feature too."
    );
    assert_eq!(error.fix, Some(FixTarget::Unsuppress(model.lug)));
    assert_eq!(suppressed.failed_count(), 1);
    assert_eq!(state(&suppressed, model.pocket), &FeatureState::UpToDate);
    assert_eq!(state(&suppressed, model.note), &FeatureState::UpToDate);
    assert!((volume(&suppressed, model.base) - (320.0 - 8.0)).abs() < 0.05);
    assert_eq!(restored.failed_count(), 0);
    assert!((volume(&restored, model.base) - WHOLE).abs() < 0.05);
    assert!(editor.document().feature(model.lug).unwrap().suppressed);
    assert!(!editor.document().same_content(&model.document));
}

#[test]
fn a_suppressed_change_to_a_body_leaves_later_features_building_on_the_body_without_it() {
    let model = model();
    let mut document = model.document.clone();
    let mut engine = Recompute::default();

    document
        .apply(document.suppression(&[model.pocket], true, "Suppress"))
        .unwrap();
    let evaluation = evaluate(&document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(state(&evaluation, model.pocket), &FeatureState::Suppressed);
    assert_eq!(state(&evaluation, model.boss), &FeatureState::UpToDate);
    assert!((volume(&evaluation, model.base) - (320.0 + 36.0)).abs() < 0.05);
}

#[test]
fn a_suppressed_body_is_gone_rather_than_shown_stale() {
    let model = model();
    let mut document = model.document.clone();
    let mut engine = Recompute::default();
    let whole = evaluate(&document, &mut engine);

    document
        .apply(document.suppression(&[model.base], true, "Suppress Base"))
        .unwrap();
    let evaluation = evaluate(&document, &mut engine);
    document
        .apply(document.suppression(&[model.base], false, "Unsuppress Base"))
        .unwrap();
    let back = evaluate(&document, &mut engine);

    assert!((volume(&whole, model.base) - WHOLE).abs() < 0.05);
    assert!(evaluation.body(model.base).is_none());
    assert!(!evaluation.is_stale(model.base));
    assert_eq!(evaluation.bodies().count(), 0);
    assert_eq!(
        failure(&evaluation, model.pocket).reason,
        "It uses Base, which is suppressed."
    );
    assert_eq!(evaluation.failed_count(), 2);
    assert_eq!(back.failed_count(), 0);
    assert!((volume(&back, model.base) - WHOLE).abs() < 0.05);
}

#[test]
fn features_below_the_rollback_bar_are_not_computed_and_rolling_forward_reuses_them() {
    let model = model();
    let mut editor = Editor::new(model.document.clone());
    let mut engine = Recompute::default();
    evaluate(editor.document(), &mut engine);

    editor
        .apply(
            editor
                .document()
                .roll_to(RollbackBar::Before(model.hole), "Roll back"),
        )
        .unwrap();
    let rolled_back = evaluate(editor.document(), &mut engine);
    let bar = editor.document().bar_index();
    editor.undo().unwrap();
    let forward = evaluate(editor.document(), &mut engine);
    editor.redo().unwrap();

    assert_eq!(bar, 2);
    for below in [model.hole, model.pocket, model.lug, model.boss, model.note] {
        assert_eq!(state(&rolled_back, below), &FeatureState::RolledBack);
        assert!(rolled_back.feature(below).unwrap().result.is_none());
        assert!(editor.document().is_rolled_back(below));
        assert!(!editor.document().is_active(below));
    }
    assert!(editor.document().is_active(model.base));
    assert!(rolled_back.is_complete());
    assert!((volume(&rolled_back, model.base) - 320.0).abs() < 0.05);
    assert!(rolled_back.recomputed().is_empty());
    assert!(forward.recomputed().is_empty());
    assert!((volume(&forward, model.base) - WHOLE).abs() < 0.05);
    assert_eq!(
        editor.document().rollback_bar(),
        RollbackBar::Before(model.hole)
    );
    assert!(!editor.document().same_content(&model.document));
}

#[test]
fn new_features_go_in_right_above_the_rollback_bar_which_stays_below_them() {
    let model = model();
    let mut editor = Editor::new(model.document.clone());
    editor
        .apply(
            editor
                .document()
                .roll_to(RollbackBar::Before(model.hole), "Roll back"),
        )
        .unwrap();

    let mut transaction = editor.document().transaction("Add");
    let first = transaction.add_feature(
        "Rib sketch",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 6.0), (10.0, 9.0))),
    );
    let second = transaction.add_feature(
        "Rib",
        extrude(first, "4 mm", BodyOperation::Add(model.base)),
    );
    editor.apply(transaction.finish()).unwrap();
    let inserted = editor.document().clone();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&inserted, &mut engine);
    editor.undo().unwrap();

    assert_eq!(
        names(&inserted),
        [
            "Outline",
            "Base",
            "Rib sketch",
            "Rib",
            "Hole sketch",
            "Pocket",
            "Lug sketch",
            "Boss",
            "Note"
        ]
    );
    assert_eq!(inserted.bar_index(), 4);
    assert!(inserted.is_active(first) && inserted.is_active(second));
    assert!((volume(&evaluation, model.base) - 360.0).abs() < 0.05);
    let mut rolled = model.document.clone();
    rolled
        .apply(rolled.roll_to(RollbackBar::Before(model.hole), "Roll back"))
        .unwrap();
    assert!(editor.document().same_content(&rolled));
}

#[test]
fn deleting_a_feature_can_take_its_dependents_along_or_leave_them_failing_until_undone() {
    let model = model();
    let dependents = model.document.dependents_of(&[model.outline]);
    let mut editor = Editor::new(model.document.clone());
    let mut engine = Recompute::default();

    editor
        .apply(model.document.deletion(&[model.outline], "Delete Outline"))
        .unwrap();
    let kept = evaluate(editor.document(), &mut engine);
    editor.undo().unwrap();
    let undone = evaluate(editor.document(), &mut engine);
    let everything: Vec<FeatureId> = std::iter::once(model.outline)
        .chain(dependents.iter().copied())
        .collect();
    editor
        .apply(
            model
                .document
                .deletion(&everything, "Delete Outline and its dependents"),
        )
        .unwrap();
    let remaining: Vec<String> = names(editor.document())
        .into_iter()
        .map(str::to_owned)
        .collect();
    editor.undo().unwrap();

    assert_eq!(dependents, [model.base, model.pocket, model.boss]);
    let base = failure(&kept, model.base);
    assert_eq!(base.reason, "It uses a feature that no longer exists.");
    assert_eq!(
        base.remedy,
        "Undo the deletion, or delete this feature too."
    );
    assert_eq!(
        failure(&kept, model.pocket).reason,
        "It uses Base, which has an error."
    );
    assert_eq!(state(&kept, model.note), &FeatureState::UpToDate);
    assert_eq!(undone.failed_count(), 0);
    assert!((volume(&undone, model.base) - WHOLE).abs() < 0.05);
    assert_eq!(remaining, ["Hole sketch", "Lug sketch", "Note"]);
    assert_eq!(editor.document(), &model.document);
}

#[test]
fn deleting_the_feature_below_the_rollback_bar_keeps_the_bar_where_it_was() {
    let model = model();
    let mut document = model.document.clone();
    document
        .apply(document.roll_to(RollbackBar::Before(model.hole), "Roll back"))
        .unwrap();
    let direct = document.check(&Transaction::single(
        "Delete",
        Edit::RemoveFeature { id: model.hole },
    ));

    let mut editor = Editor::new(document.clone());
    editor
        .apply(document.deletion(&[model.hole, model.pocket], "Delete"))
        .unwrap();
    let after = editor.document().rollback_bar();
    let last = editor.document().deletion(&[model.note], "Delete Note");
    editor
        .apply(
            editor
                .document()
                .roll_to(RollbackBar::Before(model.note), "Roll"),
        )
        .unwrap();
    let at_the_end = editor.document().deletion(&[model.note], "Delete Note");
    editor.apply(at_the_end).unwrap();

    assert_eq!(
        direct,
        Err(EditError::RollbackBarAbove("Hole sketch".to_owned()))
    );
    assert_eq!(after, RollbackBar::Before(model.lug));
    assert_eq!(last.edits().len(), 1);
    assert_eq!(editor.document().rollback_bar(), RollbackBar::AtEnd);
    editor.undo().unwrap();
    editor.undo().unwrap();
    editor.undo().unwrap();
    assert_eq!(editor.document(), &document);
}

#[test]
fn rows_move_only_where_dependencies_allow_and_crossing_the_bar_rolls_them_back_or_forward() {
    let model = model();
    let mut document = model.document.clone();
    document
        .apply(document.roll_to(RollbackBar::Before(model.lug), "Roll back"))
        .unwrap();
    let rows = document.tree_rows();
    let bar_row = rows.iter().position(|row| *row == TreeRow::Bar).unwrap();

    let above_its_sketch = document.move_row(TreeRow::Feature(model.boss), 0, "Move");
    let below_its_user = document.move_row(TreeRow::Feature(model.outline), 3, "Move");
    let in_place = document
        .move_row(TreeRow::Feature(model.base), 1, "Move")
        .unwrap();
    let note_to_top = document
        .move_row(TreeRow::Feature(model.note), 0, "Move")
        .unwrap();
    let pocket_below_bar = document
        .move_row(TreeRow::Feature(model.pocket), bar_row + 1, "Move")
        .unwrap();
    let bar_to_top = document.move_row(TreeRow::Bar, 0, "Roll").unwrap();

    assert_eq!(bar_row, 4);
    assert_eq!(
        above_its_sketch,
        Err(EditError::AboveDependency {
            name: "Boss".to_owned(),
            other: "Base".to_owned()
        })
    );
    assert_eq!(
        below_its_user,
        Err(EditError::BelowDependent {
            name: "Outline".to_owned(),
            other: "Base".to_owned()
        })
    );
    assert!(in_place.is_empty());
    assert_eq!(
        note_to_top.edits(),
        [Edit::MoveFeature {
            id: model.note,
            index: 0
        }]
    );
    let mut moved = document.clone();
    moved.apply(note_to_top).unwrap();
    assert_eq!(moved.rollback_bar(), RollbackBar::Before(model.lug));
    assert!(moved.is_active(model.note));
    let mut crossed = document.clone();
    let undo = crossed.apply(pocket_below_bar).unwrap();
    assert_eq!(
        names(&crossed),
        [
            "Outline",
            "Base",
            "Hole sketch",
            "Pocket",
            "Lug sketch",
            "Boss",
            "Note"
        ]
    );
    assert_eq!(crossed.rollback_bar(), RollbackBar::Before(model.pocket));
    assert!(crossed.is_rolled_back(model.pocket));
    crossed.apply(undo).unwrap();
    assert_eq!(crossed, document);
    assert_eq!(
        bar_to_top.edits(),
        [Edit::SetRollbackBar {
            bar: RollbackBar::Before(model.outline)
        }]
    );
}

#[test]
fn restoring_another_version_carries_its_rollback_bar_and_suppressed_features() {
    let model = model();
    let mut target = model.document.clone();
    target
        .apply(target.suppression(&[model.lug, model.boss], true, "Suppress"))
        .unwrap();
    target
        .apply(target.roll_to(RollbackBar::Before(model.pocket), "Roll back"))
        .unwrap();
    let mut current = model.document.clone();
    current
        .apply(current.roll_to(RollbackBar::Before(model.base), "Roll back"))
        .unwrap();

    let mut editor = Editor::new(current.clone());
    editor
        .apply(current.transaction_to(&target, "Restore"))
        .unwrap();

    assert!(editor.document().same_content(&target));
    assert!(editor.document().feature(model.boss).unwrap().suppressed);
    editor.undo().unwrap();
    assert!(editor.document().same_content(&current));
}

#[test]
fn a_feature_using_one_that_is_gone_can_be_inserted_and_its_id_is_never_handed_out() {
    let mut document = Document::default();
    let gone = FeatureId::from_raw(40);
    let dangling = Feature::new(
        FeatureId::from_raw(3),
        "Base".to_owned(),
        extrude(gone, "4 mm", BodyOperation::NewBody),
    );
    document
        .apply(Transaction::single(
            "Load",
            Edit::InsertFeature {
                index: 0,
                feature: std::sync::Arc::new(dangling),
            },
        ))
        .unwrap();
    let mut transaction = document.transaction("Add");
    let added = transaction.add_feature("Sketch", FeatureKind::from(Sketch::new(Plane::XY)));
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);

    let later = Feature::new(
        FeatureId::from_raw(60),
        "Later".to_owned(),
        extrude(added, "1 mm", BodyOperation::NewBody),
    );
    let above_its_sketch = document.check(&Transaction::single(
        "Insert",
        Edit::InsertFeature {
            index: 0,
            feature: std::sync::Arc::new(later),
        },
    ));

    assert_eq!(added, FeatureId::from_raw(41));
    assert_eq!(
        failure(&evaluation, FeatureId::from_raw(3)).reason,
        "It uses a feature that no longer exists."
    );
    assert_eq!(above_its_sketch, Err(EditError::MissingFeature));
}

#[test]
fn a_dependent_says_why_it_fails_when_what_it_uses_goes_from_failing_to_suppressed() {
    let model = model();
    let mut document = model.document.clone();
    let mut engine = Recompute::default();
    document
        .apply(Transaction::single(
            "Flatten",
            Edit::SetFeatureKind {
                id: model.base,
                kind: extrude(model.outline, "0 mm", BodyOperation::NewBody),
            },
        ))
        .unwrap();
    let failing = evaluate(&document, &mut engine);

    document
        .apply(document.suppression(&[model.base], true, "Suppress Base"))
        .unwrap();
    let suppressed = evaluate(&document, &mut engine);

    assert_eq!(
        failure(&failing, model.pocket).reason,
        "It uses Base, which has an error."
    );
    assert_eq!(
        failure(&suppressed, model.pocket).reason,
        "It uses Base, which is suppressed."
    );
}

#[test]
fn several_features_move_together_keeping_their_order_and_undo_as_one_step() {
    let model = model();
    let document = model.document.clone();

    let to_the_end = document
        .move_features(&[model.pocket, model.hole], 7, "Move 2 features")
        .unwrap();
    let up_together = document
        .move_features(&[model.note, model.lug], 2, "Move 2 features")
        .unwrap();
    let both_ways = document
        .move_features(
            &[model.hole, model.pocket, model.note],
            5,
            "Move 3 features",
        )
        .unwrap();
    let past_its_user = document.move_features(&[model.hole, model.note], 4, "Move");
    let in_place = document
        .move_features(&[model.hole, model.pocket], 2, "Move")
        .unwrap();

    let mut moved = document.clone();
    moved.apply(to_the_end).unwrap();

    assert_eq!(
        names(&moved),
        [
            "Outline",
            "Base",
            "Lug sketch",
            "Boss",
            "Note",
            "Hole sketch",
            "Pocket"
        ]
    );

    let mut moved = document.clone();
    moved.apply(up_together).unwrap();

    assert_eq!(
        names(&moved),
        [
            "Outline",
            "Base",
            "Lug sketch",
            "Note",
            "Hole sketch",
            "Pocket",
            "Boss"
        ]
    );

    let mut moved = document.clone();
    let undo = moved.apply(both_ways).unwrap();

    assert_eq!(
        names(&moved),
        [
            "Outline",
            "Base",
            "Lug sketch",
            "Hole sketch",
            "Pocket",
            "Note",
            "Boss"
        ]
    );

    moved.apply(undo).unwrap();

    assert_eq!(moved, document);
    assert_eq!(
        past_its_user,
        Err(EditError::BelowDependent {
            name: "Hole sketch".to_owned(),
            other: "Pocket".to_owned()
        })
    );
    assert!(in_place.is_empty());
}

#[test]
fn features_moved_together_across_the_rollback_bar_roll_back_together() {
    let model = model();
    let mut document = model.document.clone();
    document
        .apply(document.roll_to(RollbackBar::Before(model.boss), "Roll back"))
        .unwrap();

    let below_the_bar = document
        .move_features(&[model.hole, model.pocket], 8, "Move 2 features")
        .unwrap();
    document.apply(below_the_bar).unwrap();

    assert_eq!(
        names(&document),
        [
            "Outline",
            "Base",
            "Lug sketch",
            "Boss",
            "Note",
            "Hole sketch",
            "Pocket"
        ]
    );
    assert_eq!(document.rollback_bar(), RollbackBar::Before(model.boss));
    assert!(document.is_rolled_back(model.hole));
    assert!(document.is_rolled_back(model.pocket));
    assert!(document.is_active(model.lug));
}

#[test]
fn a_feature_moved_up_with_what_it_uses_never_passes_it_on_the_way() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let profile = transaction.add_feature(
        "Profile",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (10.0, 8.0))),
    );
    transaction.add_feature(
        "Spare",
        FeatureKind::from(rectangle(Plane::XZ, (0.0, 0.0), (1.0, 1.0))),
    );
    let block = transaction.add_feature("Block", extrude(profile, "4 mm", BodyOperation::NewBody));
    document.apply(transaction.finish()).unwrap();

    let together = document
        .move_features(&[profile, block], 1, "Move 2 features")
        .unwrap();
    document.apply(together).unwrap();

    assert_eq!(names(&document), ["Profile", "Block", "Spare"]);
}
