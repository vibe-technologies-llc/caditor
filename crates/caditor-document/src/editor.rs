use std::collections::VecDeque;

use crate::{
    document::Document,
    edit::{EditError, Transaction},
};

pub const MAX_UNDO_STEPS: usize = 500;

#[derive(Debug, Clone, Default)]
pub struct Editor {
    document: Document,
    undo: VecDeque<Transaction>,
    redo: VecDeque<Transaction>,
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
        self.undo.back().map(Transaction::label)
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.back().map(Transaction::label)
    }

    pub fn next_undo(&self) -> Option<&Transaction> {
        self.undo.back()
    }

    pub fn next_redo(&self) -> Option<&Transaction> {
        self.redo.back()
    }

    pub fn apply(&mut self, transaction: Transaction) -> Result<(), EditError> {
        if transaction.is_empty() {
            return Ok(());
        }
        let inverse = self.document.apply(transaction)?;
        remember(&mut self.undo, inverse);
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
        from: &mut VecDeque<Transaction>,
        to: &mut VecDeque<Transaction>,
    ) -> Result<Option<String>, EditError> {
        let Some(transaction) = from.pop_back() else {
            return Ok(None);
        };
        match document.apply(transaction.clone()) {
            Ok(inverse) => {
                let label = inverse.label().to_owned();
                remember(to, inverse);
                Ok(Some(label))
            }
            Err(error) => {
                from.push_back(transaction);
                Err(error)
            }
        }
    }
}

fn remember(steps: &mut VecDeque<Transaction>, step: Transaction) {
    steps.push_back(step);
    while steps.len() > MAX_UNDO_STEPS {
        steps.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use caditor_expression::Expression;

    use super::*;
    use crate::edit::Edit;

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
