use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use arbitrary::Unstructured;
use caditor_document::{
    CancelToken, Document, Edit, Editor, Evaluation, FeatureResult, FeatureState, ModelEvaluator,
    Recompute, Transaction,
};
use caditor_kernel::MeshQuality;

use crate::{
    document::{References, run_steps},
    invariants::assert_sound,
    seeded::for_each_case,
    sketch_tests::largest_shift,
};

const POLL_LIMIT: u64 = 20_000_000;
const STEPS: usize = 120;
const MOST_PERTURBED: usize = 6;
const SOUNDNESS_CASES: u64 = 1500;
const COMPARISON_CASES: u64 = 80;
const REVERSAL_CASES: u64 = 600;
const SAVING_CASES: u64 = 600;
const FEWEST_BODIES_SOUND: usize = 150;
const FEWEST_BODIES_COMPARED: usize = 600;

struct Budget {
    token: CancelToken,
    exhausted: Arc<AtomicBool>,
}

fn budget() -> Budget {
    let polls = Arc::new(AtomicU64::new(0));
    let exhausted = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&exhausted);
    let token = CancelToken::new(move || {
        let spent = polls.fetch_add(1, Ordering::Relaxed) >= POLL_LIMIT;
        if spent {
            flag.store(true, Ordering::Relaxed);
        }
        spent
    });
    Budget { token, exhausted }
}

fn evaluate(recompute: &mut Recompute, document: &Document) -> Option<Evaluation> {
    let budget = budget();
    let evaluation = recompute.run(document, &ModelEvaluator, &budget.token, &|_, _| {});
    (!budget.exhausted.load(Ordering::Relaxed)).then_some(evaluation)
}

const LARGEST_SHIFT: f64 = 1e-6;

fn same_result(own: &FeatureResult, theirs: &FeatureResult) -> bool {
    match (own, theirs) {
        (FeatureResult::Sketch(own), FeatureResult::Sketch(theirs)) => {
            own.solution == theirs.solution
                && (!own.solution.is_fully_constrained()
                    || largest_shift(&own.geometry, &theirs.geometry) <= LARGEST_SHIFT)
        }
        (FeatureResult::Solid(own), FeatureResult::Solid(theirs)) => {
            own.body == theirs.body
                && own.solid == theirs.solid
                && own.cuts().len() == theirs.cuts().len()
                && own.joins().len() == theirs.joins().len()
        }
        (own, theirs) => own == theirs,
    }
}

fn same_results(own: Option<&Arc<FeatureResult>>, theirs: Option<&Arc<FeatureResult>>) -> bool {
    match (own, theirs) {
        (Some(own), Some(theirs)) => same_result(own, theirs),
        (None, None) => true,
        _ => false,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Wording {
    Ignored,
    Compared,
}

fn same_state(own: &FeatureState, theirs: &FeatureState, wording: Wording) -> bool {
    match (own, theirs) {
        (FeatureState::Failed(own), FeatureState::Failed(theirs)) => {
            own.fix == theirs.fix
                && own.constraints == theirs.constraints
                && own.place == theirs.place
                && (wording == Wording::Ignored
                    || (own.reason() == theirs.reason() && own.remedy() == theirs.remedy()))
        }
        (own, theirs) => own == theirs,
    }
}

fn assert_same_evaluation(
    document: &Document,
    incremental: &Evaluation,
    fresh: &Evaluation,
    wording: Wording,
) {
    for feature in document.features() {
        let id = feature.id();
        let (Some(kept), Some(rebuilt)) = (incremental.feature(id), fresh.feature(id)) else {
            assert!(
                incremental.feature(id).is_none() && fresh.feature(id).is_none(),
                "feature {id:?} is evaluated by only one of an incremental and a fresh recompute"
            );
            continue;
        };
        assert!(
            same_state(&kept.state, &rebuilt.state, wording) && kept.healing == rebuilt.healing,
            "feature {id:?} ends {:?} incrementally and {:?} from scratch",
            kept.state,
            rebuilt.state
        );
        if kept.state == FeatureState::UpToDate {
            assert!(
                same_results(kept.result.as_ref(), rebuilt.result.as_ref()),
                "feature {id:?} has a different result incrementally and from scratch"
            );
        }
    }
    let standing = |evaluation: &Evaluation| {
        evaluation
            .bodies()
            .filter(|(body, _)| !evaluation.is_stale(*body))
            .map(|(body, _)| body)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        standing(incremental),
        standing(fresh),
        "the bodies standing differ between an incremental and a fresh recompute"
    );
}

fn assert_bodies_sound(evaluation: &Evaluation) -> usize {
    let mut checked = 0;
    for (body, _) in evaluation.bodies() {
        if let Some(solid) = evaluation.body(body) {
            assert_sound(solid, &MeshQuality::COARSE);
            checked += 1;
        }
    }
    checked
}

fn compare_after_each_step(
    incremental: &mut Recompute,
    editor: &Editor,
    wording: Wording,
) -> usize {
    let document = editor.document();
    let Some(kept) = evaluate(incremental, document) else {
        return 0;
    };
    let Some(fresh) = evaluate(&mut Recompute::default(), document) else {
        return 0;
    };
    assert_same_evaluation(document, &kept, &fresh, wording);
    fresh.bodies().count()
}

fn compare_a_run(input: &mut Unstructured, wording: Wording) -> usize {
    let mut editor = Editor::default();
    let mut incremental = Recompute::default();
    let mut bodies = 0;

    run_steps(
        input,
        &mut editor,
        &References::default(),
        STEPS,
        |editor, _| {
            bodies += compare_after_each_step(&mut incremental, editor, wording);
        },
    );
    let Some(first) = evaluate(&mut Recompute::default(), editor.document()) else {
        return bodies;
    };

    let captured = References::capture(&first);
    run_steps(input, &mut editor, &captured, STEPS, |editor, _| {
        bodies += compare_after_each_step(&mut incremental, editor, wording);
    });
    bodies + perturb_parameters(&mut editor, &mut incremental, wording)
}

fn perturbed(text: &str) -> Option<String> {
    let numeric = text
        .find(|character: char| !(character.is_ascii_digit() || character == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(numeric);
    let value: f64 = number.parse().ok()?;
    Some(format!("{}{unit}", value * 2.0 + 1.0))
}

fn perturb_parameters(editor: &mut Editor, incremental: &mut Recompute, wording: Wording) -> usize {
    let edits: Vec<Transaction> = editor
        .document()
        .parameters()
        .iter()
        .take(MOST_PERTURBED)
        .filter_map(|parameter| {
            let document = editor.document();
            let text = perturbed(&document.expression_text(&parameter.expression))?;
            let expression = document.parse(&text).ok()?;
            Some(Transaction::single(
                "Perturb",
                Edit::SetParameterExpression {
                    id: parameter.id(),
                    expression,
                },
            ))
        })
        .collect();
    let mut bodies = 0;
    for edit in edits {
        if editor.apply(edit).is_ok_and(|applied| applied) {
            bodies += compare_after_each_step(incremental, editor, wording);
        }
    }
    bodies
}

#[test]
fn recomputing_after_each_step_equals_recomputing_from_scratch() {
    let mut bodies = 0;
    for_each_case("incremental recompute", COMPARISON_CASES, |_, input| {
        bodies += compare_a_run(input, Wording::Ignored);
    });
    assert!(
        bodies >= FEWEST_BODIES_COMPARED,
        "only {bodies} bodies were compared, so the generators no longer reach enough"
    );
}

#[test]
fn every_input_kept_in_the_regression_folder_still_holds() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("regressions/document");
    let mut replayed = 0;
    for entry in fs::read_dir(&folder).unwrap() {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        compare_a_run(&mut Unstructured::new(&bytes), Wording::Ignored);
        replayed += 1;
    }
    assert!(replayed > 0, "{} holds no input", folder.display());
}

#[test]
fn every_body_a_random_document_makes_is_valid_meshes_watertight_and_has_finite_mass() {
    let mut bodies = 0;
    for_each_case("sound bodies", SOUNDNESS_CASES, |_, input| {
        let mut editor = Editor::default();

        run_steps(input, &mut editor, &References::default(), STEPS, |_, _| {});
        let Some(evaluation) = evaluate(&mut Recompute::default(), editor.document()) else {
            return;
        };

        bodies += assert_bodies_sound(&evaluation);
    });
    assert!(
        bodies >= FEWEST_BODIES_SOUND,
        "only {bodies} bodies were checked, so the generators no longer reach enough"
    );
}

#[test]
fn undoing_every_step_restores_the_empty_document_and_redoing_them_restores_the_last() {
    for_each_case("undo and redo", REVERSAL_CASES, |_, input| {
        let mut editor = Editor::default();
        run_steps(input, &mut editor, &References::default(), STEPS, |_, _| {});
        let last = editor.document().clone();

        let mut undone = 0;
        while editor.undo().unwrap().is_some() {
            undone += 1;
        }
        assert!(
            editor.document().same_content(Editor::default().document()),
            "undoing {undone} steps did not restore the empty document"
        );

        for _ in 0..undone {
            assert!(
                editor.redo().unwrap().is_some(),
                "a step could not be redone"
            );
        }
        assert!(
            editor.document().same_content(&last),
            "redoing {undone} steps did not restore the last document"
        );
    });
}

#[test]
fn a_random_document_saves_and_loads_to_the_same_content() {
    for_each_case("save and load", SAVING_CASES, |_, input| {
        let mut editor = Editor::default();
        run_steps(input, &mut editor, &References::default(), STEPS, |_, _| {});

        let bytes = caditor_file::fuzzing::save_over(editor.document(), None, 0).unwrap();
        let loaded = caditor_file::decode(&bytes).unwrap();

        assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
        assert!(
            loaded.document.same_content(editor.document()),
            "a saved document loaded differently"
        );
    });
}

#[test]
#[ignore = "a failure's words keep the name a body had before it was renamed (docs/BUGS.md)"]
fn failures_name_things_as_they_are_named_now() {
    for_each_case("failure words", COMPARISON_CASES, |_, input| {
        let mut editor = Editor::default();
        let mut incremental = Recompute::default();

        run_steps(
            input,
            &mut editor,
            &References::default(),
            STEPS,
            |editor, _| {
                compare_after_each_step(&mut incremental, editor, Wording::Compared);
            },
        );
    });
}
