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
}

#[derive(Debug, Clone, Default)]
struct Steps {
    steps: VecDeque<Step>,
    bytes: usize,
}

impl Steps {
    fn last(&self) -> Option<&Transaction> {
        self.steps.back().map(|step| &step.transaction)
    }

    fn push(&mut self, transaction: Transaction) {
        self.push_within(transaction, MAX_UNDO_BYTES);
    }

    fn push_within(&mut self, transaction: Transaction, budget: usize) {
        let size = transaction.approximate_size();
        self.bytes = self.bytes.saturating_add(size);
        self.steps.push_back(Step { transaction, size });
        while self.steps.len() > MAX_UNDO_STEPS || (self.bytes > budget && self.steps.len() > 1) {
            self.pop_front();
        }
    }

    fn pop(&mut self) -> Option<Transaction> {
        let step = self.steps.pop_back()?;
        self.bytes = self.bytes.saturating_sub(step.size);
        Some(step.transaction)
    }

    fn pop_front(&mut self) {
        if let Some(step) = self.steps.pop_front() {
            self.bytes = self.bytes.saturating_sub(step.size);
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
}

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

    pub fn next_undo(&self) -> Option<&Transaction> {
        self.undo.last()
    }

    pub fn next_redo(&self) -> Option<&Transaction> {
        self.redo.last()
    }

    pub fn apply(&mut self, transaction: Transaction) -> Result<(), EditError> {
        if transaction.is_empty() {
            return Ok(());
        }
        let inverse = self.document.apply(transaction)?;
        self.undo.push(inverse);
        self.redo.clear();
        self.revision += 1;
        Ok(())
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
        let Some(transaction) = from.pop() else {
            return Ok(None);
        };
        match document.apply(transaction.clone()) {
            Ok(inverse) => {
                let label = inverse.label().to_owned();
                to.push(inverse);
                Ok(Some(label))
            }
            Err(error) => {
                from.push(transaction);
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use caditor_expression::Expression;
    use caditor_kernel::Solid;

    use super::*;
    use crate::{document::FeatureKind, edit::Edit, import::Import};

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

        steps.push_within(import_of(1 << 20), budget);
        steps.push_within(import_of(2 << 20), budget);
        let both = labels(&steps);
        steps.push_within(import_of(3 << 20), budget);
        let last_two = labels(&steps);
        steps.push_within(import_of(8 << 20), budget);
        let oversized = labels(&steps);
        let popped = steps.pop().map(|step| step.label().to_owned());

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
}
