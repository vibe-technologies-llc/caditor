mod edits;
mod kinds;
mod references;

use std::fmt;

use caditor_document::{
    CancelToken, Document, Edit, Editor, Evaluation, ModelEvaluator, Recompute, Transaction,
};
use caditor_file::JournalEntry;
use libfuzzer_sys::arbitrary::{Result, Unstructured};

pub use self::references::References;
use self::{
    edits::{addition, model_edit, parameter_edit, sketch_edit, tree_edit},
    references::Scene,
};
use crate::{WORK_BUDGET, interrupt_after};

const MOST_STEPS: usize = 24;
const LABEL: &str = "Fuzzed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Generator {
    Addition,
    Parameter,
    Tree,
    Sketch,
    Model,
}

impl fmt::Display for Generator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Addition => "addition",
            Self::Parameter => "parameter_edit",
            Self::Tree => "tree_edit",
            Self::Sketch => "sketch_edit",
            Self::Model => "model_edit",
        };
        formatter.write_str(name)
    }
}

impl Generator {
    pub fn of(edit: &Edit) -> Self {
        match edit {
            Edit::InsertParameter { .. } | Edit::InsertFeature { .. } => Self::Addition,
            Edit::RemoveParameter { .. }
            | Edit::RenameParameter { .. }
            | Edit::SetParameterExpression { .. }
            | Edit::MoveParameter { .. }
            | Edit::SetParameterNote { .. }
            | Edit::SetParameterOwner { .. } => Self::Parameter,
            Edit::RemoveFeature { .. }
            | Edit::RenameFeature { .. }
            | Edit::MoveFeature { .. }
            | Edit::SetFeatureHidden { .. }
            | Edit::SetFeatureSuppressed { .. }
            | Edit::SetFeatureGroup { .. }
            | Edit::SetBodyAppearance { .. }
            | Edit::SetFeaturePieces { .. }
            | Edit::SetPieceStandIns { .. }
            | Edit::SetRollbackBar { .. }
            | Edit::SetPrincipalHidden { .. }
            | Edit::SetFeatureKind { .. } => Self::Tree,
            Edit::SetSketchPlacement { .. }
            | Edit::SetDimension { .. }
            | Edit::AddSketchEntity { .. }
            | Edit::RemoveSketchEntity { .. }
            | Edit::SetSketchEntity { .. }
            | Edit::SetSketchConstruction { .. }
            | Edit::SetSketchProjection { .. }
            | Edit::AddSketchConstraint { .. }
            | Edit::RemoveSketchConstraint { .. }
            | Edit::SetSketchConstraintActive { .. }
            | Edit::SetSketchLabel { .. }
            | Edit::AddSketchText { .. }
            | Edit::RemoveSketchText { .. }
            | Edit::SetSketchText { .. } => Self::Sketch,
            Edit::SetModelProperties { .. }
            | Edit::SetSavedViews { .. }
            | Edit::SetSelectionSets { .. }
            | Edit::SetConfigurations { .. }
            | Edit::SetActiveConfiguration { .. } => Self::Model,
        }
    }
}

fn generators(transaction: &Transaction) -> String {
    let mut names: Vec<String> = transaction
        .edits()
        .iter()
        .map(|edit| Generator::of(edit).to_string())
        .collect();
    names.dedup();
    names.join(", ")
}

enum Step {
    Apply(Transaction),
    Undo,
    Redo,
}

fn step(input: &mut Unstructured, scene: &Scene) -> Result<Option<Step>> {
    Ok(match input.int_in_range(0u8..=11)? {
        0..=3 => addition(input, scene)?.map(Step::Apply),
        4 | 5 => sketch_edit(input, scene)?.map(Step::Apply),
        6 | 7 => tree_edit(input, scene)?.map(Step::Apply),
        8 => parameter_edit(input, scene)?.map(Step::Apply),
        9 => model_edit(input, scene)?.map(Step::Apply),
        10 => Some(Step::Undo),
        _ => Some(Step::Redo),
    })
}

pub fn check_inverse(document: &Document, transaction: &Transaction) {
    let mut applied = document.clone();
    match applied.apply(transaction.clone()) {
        Ok(inverse) => {
            let mut restored = applied.clone();
            if let Err(error) = restored.apply(inverse) {
                panic!(
                    "the inverse of a change from {} could not be applied: {error}\n{transaction:#?}",
                    generators(transaction)
                );
            }
            assert!(
                restored.same_content(document),
                "undoing a change from {} did not restore the document",
                generators(transaction)
            );
            let mut rebuilt = document.clone();
            if let Err(error) = rebuilt.apply(document.transaction_to(&applied, "Restore")) {
                panic!(
                    "a transaction between two states after {} could not be applied: {error}",
                    generators(transaction)
                );
            }
            assert!(
                rebuilt.same_content(&applied),
                "a transaction between two states after {} did not reach the target",
                generators(transaction)
            );
        }
        Err(_) => assert!(
            applied == *document,
            "a refused change from {} still changed the document",
            generators(transaction)
        ),
    }
}

pub fn run(
    input: &mut Unstructured,
    editor: &mut Editor,
    references: &References,
    mut changed: impl FnMut(&Editor, JournalEntry),
) {
    for _ in 0..MOST_STEPS {
        if input.is_empty() {
            break;
        }
        let scene = Scene {
            document: editor.document(),
            references,
        };
        let Ok(Some(step)) = step(input, &scene) else {
            continue;
        };
        let entry = match step {
            Step::Apply(transaction) => {
                check_inverse(editor.document(), &transaction);
                let entry = JournalEntry::Apply(transaction.clone());
                editor
                    .apply(transaction)
                    .is_ok_and(|applied| applied)
                    .then_some(entry)
            }
            Step::Undo => {
                let entry = editor.next_undo().cloned().map(JournalEntry::Undo);
                match editor.undo() {
                    Ok(undone) => undone.and(entry),
                    Err(error) => panic!("undo failed: {error}"),
                }
            }
            Step::Redo => {
                let entry = editor.next_redo().cloned().map(JournalEntry::Redo);
                match editor.redo() {
                    Ok(redone) => redone.and(entry),
                    Err(error) => panic!("redo failed: {error}"),
                }
            }
        };
        if let Some(entry) = entry {
            changed(editor, entry);
        }
    }
}

pub fn recompute(document: &Document) -> Evaluation {
    let interrupt = interrupt_after(WORK_BUDGET);
    let cancel = CancelToken::new(move || interrupt());
    Recompute::default().run(document, &ModelEvaluator, &cancel, &|_, _| {})
}
