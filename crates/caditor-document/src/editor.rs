use std::collections::VecDeque;

use crate::{
    document::Document,
    edit::{EditError, Transaction},
};

pub const MAX_UNDO_STEPS: usize = 500;
pub const MAX_UNDO_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone)]
struct Step {
    transaction: Transaction,
    size: usize,
    serial: u64,
}

#[derive(Debug, Clone, Default)]
struct Steps {
    steps: VecDeque<Step>,
    bytes: usize,
    dropped: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UndoMark {
    newest: Option<u64>,
    next: u64,
    dropped: u64,
}

impl Steps {
    fn labels(&self) -> impl Iterator<Item = &str> {
        self.transactions().map(Transaction::label)
    }

    fn transactions(&self) -> impl Iterator<Item = &Transaction> {
        self.steps.iter().rev().map(|step| &step.transaction)
    }

    fn last(&self) -> Option<&Transaction> {
        self.steps.back().map(|step| &step.transaction)
    }

    fn serials(&self) -> impl Iterator<Item = (u64, &Transaction)> {
        self.steps
            .iter()
            .rev()
            .map(|step| (step.serial, &step.transaction))
    }

    fn push(&mut self, transaction: Transaction, serial: u64) {
        self.push_within(transaction, serial, MAX_UNDO_BYTES);
    }

    fn push_within(&mut self, transaction: Transaction, serial: u64, budget: usize) {
        let size = transaction.approximate_size();
        self.bytes = self.bytes.saturating_add(size);
        self.steps.push_back(Step {
            transaction,
            size,
            serial,
        });
        while self.steps.len() > MAX_UNDO_STEPS || (self.bytes > budget && self.steps.len() > 1) {
            self.pop_front();
        }
    }

    fn pop(&mut self) -> Option<(Transaction, u64)> {
        let step = self.steps.pop_back()?;
        self.bytes = self.bytes.saturating_sub(step.size);
        Some((step.transaction, step.serial))
    }

    fn pop_front(&mut self) {
        if let Some(step) = self.steps.pop_front() {
            self.bytes = self.bytes.saturating_sub(step.size);
            self.dropped += 1;
        }
    }

    fn clear(&mut self) {
        self.steps.clear();
        self.bytes = 0;
    }
}

#[derive(Debug, Clone, Default)]
pub struct Editor {
    document: Document,
    undo: Steps,
    redo: Steps,
    revision: u64,
    next_serial: u64,
}

#[derive(Debug, Clone)]
pub struct Base {
    document: Document,
    revision: u64,
}

impl Base {
    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn prepare(self, transaction: Transaction) -> Result<Prepared, EditError> {
        let mut document = self.document;
        let before = document.clone();
        let inverse = document.apply(transaction.clone())?;
        let changes_content = !document.same_content(&before);
        Ok(Prepared {
            document: if changes_content { document } else { before },
            transaction,
            inverse,
            revision: self.revision,
            changes_content,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Prepared {
    document: Document,
    transaction: Transaction,
    inverse: Transaction,
    revision: u64,
    changes_content: bool,
}

impl Prepared {
    pub fn transaction(&self) -> &Transaction {
        &self.transaction
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the model changed while the change was being prepared")]
pub struct Stale;

impl Editor {
    pub fn new(document: Document) -> Self {
        Self {
            document,
            ..Self::default()
        }
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(Transaction::label)
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(Transaction::label)
    }

    pub fn undo_labels(&self) -> impl Iterator<Item = &str> {
        self.undo.labels()
    }

    pub fn redo_labels(&self) -> impl Iterator<Item = &str> {
        self.redo.labels()
    }

    pub fn undo_steps(&self) -> impl Iterator<Item = &Transaction> {
        self.undo.transactions()
    }

    pub fn redo_steps(&self) -> impl Iterator<Item = &Transaction> {
        self.redo.transactions()
    }

    pub fn next_undo(&self) -> Option<&Transaction> {
        self.undo.last()
    }

    pub fn next_redo(&self) -> Option<&Transaction> {
        self.redo.last()
    }

    pub fn undo_mark(&self) -> UndoMark {
        UndoMark {
            newest: self.undo.steps.back().map(|step| step.serial),
            next: self.next_serial,
            dropped: self.undo.dropped,
        }
    }

    pub fn undo_steps_since(&self, mark: UndoMark) -> Option<Vec<&Transaction>> {
        let mut since = Vec::new();
        let mut older = self.undo.serials();
        loop {
            match older.next() {
                Some((serial, transaction)) if serial >= mark.next => since.push(transaction),
                Some((serial, _)) => return (Some(serial) == mark.newest).then_some(since),
                None => {
                    let whole = mark.newest.is_none() && self.undo.dropped == mark.dropped;
                    return whole.then_some(since);
                }
            }
        }
    }

    fn push_undo(&mut self, inverse: Transaction) {
        let serial = self.next_serial;
        self.next_serial += 1;
        self.undo.push(inverse, serial);
        self.redo.clear();
        self.revision += 1;
    }

    pub fn apply(&mut self, transaction: Transaction) -> Result<bool, EditError> {
        if transaction.is_empty() {
            return Ok(false);
        }
        let mut applied = self.document.clone();
        let inverse = applied.apply(transaction)?;
        if applied.same_content(&self.document) {
            return Ok(false);
        }
        self.document = applied;
        self.push_undo(inverse);
        Ok(true)
    }

    pub fn base(&self) -> Base {
        Base {
            document: self.document.clone(),
            revision: self.revision,
        }
    }

    pub fn commit(&mut self, prepared: Prepared) -> Result<Transaction, Stale> {
        if prepared.revision != self.revision {
            return Err(Stale);
        }
        if prepared.transaction.is_empty() || !prepared.changes_content {
            return Ok(Transaction::new(prepared.transaction.label(), Vec::new()));
        }
        self.document = prepared.document;
        self.push_undo(prepared.inverse);
        Ok(prepared.transaction)
    }

    pub fn undo(&mut self) -> Result<Option<String>, EditError> {
        Self::replay(&mut self.document, &mut self.undo, &mut self.redo)
            .inspect(|replayed| self.bump_if(replayed.is_some()))
    }

    pub fn redo(&mut self) -> Result<Option<String>, EditError> {
        Self::replay(&mut self.document, &mut self.redo, &mut self.undo)
            .inspect(|replayed| self.bump_if(replayed.is_some()))
    }

    fn bump_if(&mut self, changed: bool) {
        if changed {
            self.revision += 1;
        }
    }

    fn replay(
        document: &mut Document,
        from: &mut Steps,
        to: &mut Steps,
    ) -> Result<Option<String>, EditError> {
        let Some((transaction, serial)) = from.pop() else {
            return Ok(None);
        };
        match document.apply(transaction.clone()) {
            Ok(inverse) => {
                let label = inverse.label().to_owned();
                to.push(inverse, serial);
                Ok(Some(label))
            }
            Err(error) => {
                from.push(transaction, serial);
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use caditor_expression::{BinaryOperator, Expression, ParameterId};
    use caditor_kernel::Solid;
    use caditor_sketch::{Entity, EntityId};

    use super::*;
    use crate::{
        document::{FeatureId, FeatureKind},
        edit::Edit,
        import::Import,
    };

    fn import_of(bytes: usize) -> Transaction {
        let mut document = Document::default();
        let mut transaction = document.transaction(format!("Import {bytes}"));
        transaction.add_feature(
            "Imported",
            FeatureKind::Import(Import::new(
                "part.step",
                Solid::default(),
                "x".repeat(bytes),
            )),
        );
        let transaction = transaction.finish();
        document.apply(transaction.clone()).unwrap();
        transaction
    }

    #[test]
    fn a_transaction_counts_the_imports_it_holds() {
        let small = import_of(0).approximate_size();
        let large = import_of(1 << 20).approximate_size();

        assert!(small < 4096, "{small}");
        assert!(large >= small + (1 << 20));
        assert!(large < small + (1 << 20) + 4096);
    }

    fn spline_of(controls: u64) -> Transaction {
        Transaction::single(
            "Spline",
            Edit::AddSketchEntity {
                feature: FeatureId::from_raw(1),
                id: EntityId::from_raw(1),
                entity: Entity::spline((2..2 + controls).map(EntityId::from_raw).collect()),
                construction: false,
            },
        )
    }

    fn sum_of(terms: usize) -> Transaction {
        let mut total = Expression::number(1.0);
        for _ in 0..terms {
            total = Expression::binary(BinaryOperator::Add, total, Expression::number(1.0));
        }
        Transaction::single(
            "Dimension",
            Edit::SetParameterExpression {
                id: ParameterId::from_raw(1),
                expression: total,
            },
        )
    }

    #[test]
    fn a_transaction_counts_spline_controls_and_expression_trees() {
        let none = spline_of(0).approximate_size();
        let many = spline_of(10_000).approximate_size();
        let short = sum_of(0).approximate_size();
        let long = sum_of(1_000).approximate_size();

        assert!(many >= none + 10_000 * size_of::<EntityId>());
        assert!(long >= short + 1_000 * size_of::<Expression>());
    }

    #[test]
    fn the_editor_lists_its_undo_and_redo_steps_newest_first() {
        let mut editor = Editor::default();
        for name in ["one", "two", "three"] {
            let mut transaction = editor.document().transaction(format!("Add {name}"));
            transaction.add_parameter(name.to_owned(), Expression::number(1.0));
            editor.apply(transaction.finish()).unwrap();
        }
        editor.undo().unwrap();
        editor.undo().unwrap();

        assert_eq!(editor.undo_labels().collect::<Vec<_>>(), ["Add one"]);
        assert_eq!(
            editor.redo_labels().collect::<Vec<_>>(),
            ["Add two", "Add three"]
        );
    }

    fn labels(steps: &Steps) -> Vec<String> {
        steps
            .steps
            .iter()
            .map(|step| step.transaction.label().to_owned())
            .collect()
    }

    #[test]
    fn large_steps_push_the_oldest_out_of_the_byte_budget() {
        let mut steps = Steps::default();
        let budget = (5 << 20) + (64 << 10);

        steps.push_within(import_of(1 << 20), 0, budget);
        steps.push_within(import_of(2 << 20), 1, budget);
        let both = labels(&steps);
        steps.push_within(import_of(3 << 20), 2, budget);
        let last_two = labels(&steps);
        steps.push_within(import_of(8 << 20), 3, budget);
        let oversized = labels(&steps);
        let popped = steps.pop().map(|(step, _)| step.label().to_owned());

        assert_eq!(both, ["Import 1048576", "Import 2097152"]);
        assert_eq!(last_two, ["Import 2097152", "Import 3145728"]);
        assert_eq!(oversized, ["Import 8388608"]);
        assert_eq!(popped.as_deref(), Some("Import 8388608"));
        assert_eq!(steps.bytes, 0);
    }

    #[test]
    fn undo_keeps_only_the_latest_steps() {
        let mut document = Document::default();

        let mut transaction = document.transaction("Add");
        let width = transaction.add_parameter("width", Expression::Number(0.0));
        document.apply(transaction.finish()).unwrap();
        let mut editor = Editor::new(document);
        let set = |value: usize| {
            Transaction::single(
                format!("Set {value}"),
                Edit::SetParameterExpression {
                    id: width,
                    expression: Expression::Number(value as f64),
                },
            )
        };
        for value in 1..=MAX_UNDO_STEPS + 20 {
            editor.apply(set(value)).unwrap();
        }
        let mut undone = 0;
        while editor.undo().unwrap().is_some() {
            undone += 1;
        }
        let value = editor
            .document()
            .parameter(width)
            .unwrap()
            .expression
            .clone();

        assert_eq!(undone, MAX_UNDO_STEPS);
        assert_eq!(value, Expression::Number(20.0));
        assert_eq!(editor.redo_label(), Some("Set 21"));
    }

    fn add_parameter(document: &Document, name: &str) -> Transaction {
        let mut transaction = document.transaction(format!("Add {name}"));
        transaction.add_parameter(name, Expression::Number(1.0));
        transaction.finish()
    }

    #[test]
    fn a_change_prepared_elsewhere_commits_like_one_applied_here_and_undoes() {
        let mut editor = Editor::new(Document::default());
        let base = editor.base();
        let transaction = add_parameter(base.document(), "width");

        let prepared = base.prepare(transaction.clone()).unwrap();
        let committed = editor.commit(prepared).unwrap();
        let mut applied = Editor::new(Document::default());
        applied.apply(transaction.clone()).unwrap();

        assert_eq!(committed, transaction);
        assert_eq!(editor.revision(), 1);
        assert!(editor.document().same_content(applied.document()));
        assert_eq!(editor.undo_label(), Some("Add width"));
        assert_eq!(editor.undo().unwrap().as_deref(), Some("Add width"));
        assert!(editor.document().parameter_named("width").is_none());
        assert_eq!(editor.redo().unwrap().as_deref(), Some("Add width"));
        assert!(editor.document().parameter_named("width").is_some());
    }

    #[test]
    fn a_change_prepared_on_an_older_model_is_refused_as_stale() {
        let mut editor = Editor::new(Document::default());
        let base = editor.base();
        let late = add_parameter(base.document(), "width");
        editor
            .apply(add_parameter(editor.document(), "height"))
            .unwrap();

        let prepared = base.prepare(late).unwrap();
        let refused = editor.commit(prepared);

        assert_eq!(refused.unwrap_err(), Stale);
        assert_eq!(editor.revision(), 1);
        assert!(editor.document().parameter_named("width").is_none());
        assert_eq!(editor.undo_label(), Some("Add height"));
    }

    #[test]
    fn preparing_a_refused_change_reports_its_error() {
        let editor = Editor::new(Document::default());
        let base = editor.base();
        let mut transaction = base.document().transaction("Twice");
        transaction.add_parameter("width", Expression::Number(1.0));
        transaction.add_parameter("width", Expression::Number(2.0));
        let twice = transaction.finish();

        assert!(base.prepare(twice).is_err());
    }

    fn editor_with_parameter() -> (Editor, caditor_expression::ParameterId) {
        let mut document = Document::default();
        let mut transaction = document.transaction("Add");
        let width = transaction.add_parameter("width", Expression::Number(3.0));
        document.apply(transaction.finish()).unwrap();
        (Editor::new(document), width)
    }

    fn set_width(id: caditor_expression::ParameterId, value: f64) -> Transaction {
        Transaction::single(
            "Set width",
            Edit::SetParameterExpression {
                id,
                expression: Expression::Number(value),
            },
        )
    }

    #[test]
    fn an_edit_that_changes_nothing_is_not_an_undo_step_and_keeps_the_redo_history() {
        let (mut editor, width) = editor_with_parameter();
        editor.apply(set_width(width, 4.0)).unwrap();
        editor.undo().unwrap();
        let revision = editor.revision();

        let same = editor.apply(set_width(width, 3.0)).unwrap();
        let hidden_twice = Transaction::new(
            "Hide",
            vec![
                Edit::SetPrincipalHidden {
                    geometry: crate::datum::PrincipalGeometry::ALL[0],
                    hidden: true,
                },
                Edit::SetPrincipalHidden {
                    geometry: crate::datum::PrincipalGeometry::ALL[0],
                    hidden: false,
                },
            ],
        );
        let round_trip = editor.apply(hidden_twice).unwrap();

        assert!(!same);
        assert!(!round_trip);
        assert_eq!(editor.revision(), revision);
        assert_eq!(editor.undo_label(), None);
        assert_eq!(editor.redo_label(), Some("Set width"));
    }

    fn labels_since(editor: &Editor, mark: UndoMark) -> Option<Vec<String>> {
        editor
            .undo_steps_since(mark)
            .map(|steps| steps.iter().map(|step| step.label().to_owned()).collect())
    }

    #[test]
    fn the_steps_since_a_mark_are_the_newer_ones_kept_through_undo_and_redo() {
        let (mut editor, width) = editor_with_parameter();
        editor.apply(set_width(width, 4.0)).unwrap();
        let mark = editor.undo_mark();
        editor.apply(set_width(width, 5.0)).unwrap();
        editor
            .apply(add_parameter(editor.document(), "height"))
            .unwrap();

        let both = labels_since(&editor, mark);
        editor.undo().unwrap();
        let one = labels_since(&editor, mark);
        editor.redo().unwrap();
        let again = labels_since(&editor, mark);

        assert_eq!(
            both,
            Some(vec!["Add height".to_owned(), "Set width".to_owned()])
        );
        assert_eq!(one, Some(vec!["Set width".to_owned()]));
        assert_eq!(again, both);
    }

    #[test]
    fn undoing_past_a_mark_breaks_it_even_after_redoing_or_new_steps() {
        let (mut editor, width) = editor_with_parameter();
        editor.apply(set_width(width, 4.0)).unwrap();
        let mark = editor.undo_mark();

        editor.undo().unwrap();
        let undone_past = labels_since(&editor, mark);
        editor.redo().unwrap();
        let redone = labels_since(&editor, mark);
        editor.undo().unwrap();
        editor.apply(set_width(width, 6.0)).unwrap();
        let replaced = labels_since(&editor, mark);

        assert_eq!(undone_past, None);
        assert_eq!(redone, Some(Vec::new()));
        assert_eq!(replaced, None);
    }

    #[test]
    fn a_mark_on_an_empty_history_breaks_once_steps_since_it_are_dropped() {
        let (mut editor, width) = editor_with_parameter();
        let mark = editor.undo_mark();
        editor.apply(set_width(width, 4.0)).unwrap();
        let kept = labels_since(&editor, mark).map(|labels| labels.len());

        for value in 0..MAX_UNDO_STEPS {
            editor.apply(set_width(width, value as f64 + 10.0)).unwrap();
        }
        let dropped = labels_since(&editor, mark);

        assert_eq!(kept, Some(1));
        assert_eq!(dropped, None);
    }

    #[test]
    fn a_real_edit_reports_that_it_changed_the_model() {
        let (mut editor, width) = editor_with_parameter();

        let changed = editor.apply(set_width(width, 5.0)).unwrap();

        assert!(changed);
        assert_eq!(editor.revision(), 1);
        assert_eq!(editor.undo_label(), Some("Set width"));
    }

    #[test]
    fn a_prepared_edit_that_changes_nothing_commits_as_an_empty_change() {
        let (mut editor, width) = editor_with_parameter();
        let prepared = editor.base().prepare(set_width(width, 3.0)).unwrap();

        let committed = editor.commit(prepared).unwrap();

        assert!(committed.is_empty());
        assert_eq!(editor.revision(), 0);
        assert_eq!(editor.undo_label(), None);
    }

    #[test]
    fn a_flag_set_to_its_value_changes_nothing() {
        let mut document = Document::default();
        let mut transaction = document.transaction("Add");
        let feature = transaction.add_feature(
            "Imported",
            FeatureKind::Import(Import::new("part.step", Solid::default(), String::new())),
        );
        document.apply(transaction.finish()).unwrap();
        let mut editor = Editor::new(document);
        let hide = |hidden| {
            Transaction::single(
                "Hide",
                Edit::SetFeatureHidden {
                    id: feature,
                    hidden,
                },
            )
        };

        let unchanged = editor.apply(hide(false)).unwrap();
        let changed = editor.apply(hide(true)).unwrap();
        let again = editor.apply(hide(true)).unwrap();

        assert!(!unchanged);
        assert!(changed);
        assert!(!again);
        assert_eq!(editor.revision(), 1);
        assert_eq!(editor.undo().unwrap().as_deref(), Some("Hide"));
        assert_eq!(editor.undo().unwrap(), None);
    }
}
