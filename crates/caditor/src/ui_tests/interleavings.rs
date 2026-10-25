use caditor_checks::seeded::Stream;
use caditor_document::{CancelToken, FeatureState, ModelEvaluator, Recompute};
use egui::{Key, Modifiers};

use super::{Harness, add_peg, extruded_plate};

const STEPS: usize = 14;
const FEWEST_INTERLEAVED: usize = 1;
const CASES_PER_TEST: u64 = 20;
const WIDTHS: [&str; 4] = ["10 mm", "25 mm", "40 mm", "60 mm"];
const FRAMES_LET_THROUGH: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Width(usize),
    Plate,
    Peg,
    Undo,
    Redo,
    Hold,
    Release,
    Frames,
}

fn step_from(stream: &mut Stream, held: bool) -> Step {
    let word = stream.next_word();
    if held {
        return match word % 4 {
            0 => Step::Undo,
            1 => Step::Redo,
            2 => Step::Frames,
            _ => Step::Release,
        };
    }
    match word % 8 {
        0 | 1 => Step::Width((word >> 8) as usize % WIDTHS.len()),
        2 => Step::Plate,
        3 => Step::Peg,
        4 => Step::Undo,
        5 => Step::Redo,
        6 => Step::Hold,
        _ => Step::Frames,
    }
}

fn undo(harness: &mut Harness) {
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.frame();
}

fn redo(harness: &mut Harness) {
    harness.key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();
    harness.frame();
}

fn assert_shows_the_documents_own_evaluation(harness: &Harness, history: &[Step]) {
    let document = harness.document();
    let cancel = CancelToken::new(|| false);
    let fresh = Recompute::default().run(document, &ModelEvaluator, &cancel, &|_, _| {});
    let shown = harness.model.evaluation();

    for feature in document.features() {
        let id = feature.id();
        let (Some(own), Some(theirs)) = (shown.feature(id), fresh.feature(id)) else {
            panic!("feature {id:?} is evaluated by only one side after {history:?}");
        };
        let same = match (&own.state, &theirs.state) {
            (FeatureState::Failed(_), FeatureState::Failed(_)) => true,
            (own, theirs) => own == theirs,
        };
        assert!(
            same,
            "feature {id:?} shows {:?} though a fresh recompute says {:?}, after {history:?}",
            own.state, theirs.state
        );
    }
}

fn play(seed: u64) -> bool {
    let mut stream = Stream::new(seed);
    let mut harness = Harness::new();
    let mut held = false;
    let mut plate = false;
    let mut peg = false;
    let mut history = Vec::new();
    let mut changed_while_held = false;

    for _ in 0..STEPS {
        let step = step_from(&mut stream, held);
        let step = match step {
            Step::Plate if plate => Step::Frames,
            Step::Peg if !plate || peg => Step::Frames,
            other => other,
        };
        history.push(step);
        changed_while_held |= held && matches!(step, Step::Undo | Step::Redo);
        match step {
            Step::Width(index) => harness.edit_width(WIDTHS[index]),
            Step::Plate => {
                extruded_plate(&mut harness);
                plate = true;
            }
            Step::Peg => {
                add_peg(&mut harness);
                peg = true;
            }
            Step::Undo => undo(&mut harness),
            Step::Redo => redo(&mut harness),
            Step::Hold => {
                harness.model.hold_evaluations(true);
                held = true;
            }
            Step::Release => {
                harness.model.hold_evaluations(false);
                held = false;
            }
            Step::Frames => {
                for _ in 0..FRAMES_LET_THROUGH {
                    harness.frame();
                }
            }
        }
    }

    harness.model.hold_evaluations(false);
    harness.settle();
    assert_shows_the_documents_own_evaluation(&harness, &history);
    changed_while_held
}

fn play_share(share: u64) {
    let mut interleaved = 0;
    for case in 0..CASES_PER_TEST {
        let seed = share * CASES_PER_TEST + case;
        match std::panic::catch_unwind(|| play(seed)) {
            Ok(changed_while_held) => interleaved += usize::from(changed_while_held),
            Err(failure) => {
                eprintln!("the interleaving of seed {seed} failed");
                std::panic::resume_unwind(failure);
            }
        }
    }
    assert!(
        interleaved >= FEWEST_INTERLEAVED,
        "only {interleaved} scenarios undid or redid while a recompute was held"
    );
}

#[test]
fn edits_undo_and_redo_around_held_recomputes_leave_the_evaluation_of_the_document_first_share() {
    play_share(0);
}

#[test]
fn edits_undo_and_redo_around_held_recomputes_leave_the_evaluation_of_the_document_second_share() {
    play_share(1);
}

#[test]
fn edits_undo_and_redo_around_held_recomputes_leave_the_evaluation_of_the_document_third_share() {
    play_share(2);
}

#[test]
fn edits_undo_and_redo_around_held_recomputes_leave_the_evaluation_of_the_document_fourth_share() {
    play_share(3);
}
