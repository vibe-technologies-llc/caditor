use std::collections::BTreeSet;

use crate::{
    document::{Document, Feature, FeatureId, RollbackBar, TreeRow},
    edit::{Edit, EditError, Transaction},
};

impl Document {
    pub fn dependents_of(&self, ids: &[FeatureId]) -> Vec<FeatureId> {
        let mut reached: BTreeSet<FeatureId> = ids.iter().copied().collect();
        let mut dependents = Vec::new();
        for feature in self.features() {
            let id = feature.id();
            if reached.contains(&id) {
                continue;
            }
            if feature
                .kind
                .features()
                .iter()
                .any(|used| reached.contains(used))
            {
                reached.insert(id);
                dependents.push(id);
            }
        }
        dependents
    }

    pub fn deletion(&self, ids: &[FeatureId], label: impl Into<String>) -> Transaction {
        let doomed: BTreeSet<FeatureId> = ids.iter().copied().collect();
        let bar_moves = match self.rollback {
            RollbackBar::Before(first) if doomed.contains(&first) => {
                let bar = self
                    .features()
                    .skip(self.bar_index())
                    .map(Feature::id)
                    .find(|id| !doomed.contains(id))
                    .map_or(RollbackBar::AtEnd, RollbackBar::Before);
                Some(Edit::SetRollbackBar { bar })
            }
            RollbackBar::Before(_) | RollbackBar::AtEnd => None,
        };
        let removals = self
            .features()
            .rev()
            .filter(|feature| doomed.contains(&feature.id()))
            .map(|feature| Edit::RemoveFeature { id: feature.id() });
        Transaction::new(label, bar_moves.into_iter().chain(removals).collect())
    }

    pub fn suppression(
        &self,
        ids: &[FeatureId],
        suppressed: bool,
        label: impl Into<String>,
    ) -> Transaction {
        let edits = self
            .features()
            .filter(|feature| ids.contains(&feature.id()) && feature.suppressed != suppressed)
            .map(|feature| Edit::SetFeatureSuppressed {
                id: feature.id(),
                suppressed,
            })
            .collect();
        Transaction::new(label, edits)
    }

    pub fn roll_to(&self, bar: RollbackBar, label: impl Into<String>) -> Transaction {
        let edits = (bar != self.rollback)
            .then_some(Edit::SetRollbackBar { bar })
            .into_iter()
            .collect();
        Transaction::new(label, edits)
    }

    pub fn move_row(
        &self,
        row: TreeRow,
        gap: usize,
        label: impl Into<String>,
    ) -> Result<Transaction, EditError> {
        let mut rows = self.tree_rows();
        let from = rows
            .iter()
            .position(|candidate| *candidate == row)
            .ok_or(EditError::MissingFeature)?;
        if gap > rows.len() {
            return Err(EditError::OutOfRange(gap));
        }
        let moved = rows.remove(from);
        rows.insert(if gap > from { gap - 1 } else { gap }, moved);

        let order: Vec<FeatureId> = rows
            .iter()
            .filter_map(|candidate| match candidate {
                TreeRow::Feature(id) => Some(*id),
                TreeRow::Bar => None,
            })
            .collect();
        let bar_at = rows
            .iter()
            .position(|candidate| *candidate == TreeRow::Bar)
            .unwrap_or(order.len());
        let bar = order
            .get(bar_at)
            .map_or(RollbackBar::AtEnd, |id| RollbackBar::Before(*id));

        let mut edits = Vec::new();
        if let TreeRow::Feature(id) = row
            && let Some(index) = order.iter().position(|candidate| *candidate == id)
            && self.feature_index(id) != Some(index)
        {
            edits.push(Edit::MoveFeature { id, index });
        }
        if bar != self.rollback {
            edits.push(Edit::SetRollbackBar { bar });
        }
        let transaction = Transaction::new(label, edits);
        self.check(&transaction)?;
        Ok(transaction)
    }
}
