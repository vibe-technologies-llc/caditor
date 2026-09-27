use crate::{
    document::Document,
    edit::{EditError, Transaction},
};

#[derive(Debug, Clone, Default)]
pub struct Editor {
    document: Document,
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
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
        from: &mut Vec<Transaction>,
        to: &mut Vec<Transaction>,
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
