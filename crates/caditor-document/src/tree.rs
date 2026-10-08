use std::collections::BTreeSet;

use crate::{
    document::{Document, Feature, FeatureId, RollbackBar, TreeRow},
    edit::{Edit, EditError, Transaction},
};

#[derive(Debug, Clone, Copy)]
enum Slot {
    After(Option<FeatureId>),
    Before(Option<FeatureId>),
}

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
                .dependencies()
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
            edits.extend(self.membership_edits(&order, &BTreeSet::from([id])));
        }
        if bar != self.rollback {
            edits.push(Edit::SetRollbackBar { bar });
        }
        let transaction = Transaction::new(label, edits);
        self.check(&transaction)?;
        Ok(transaction)
    }

    pub fn move_features(
        &self,
        features: &[FeatureId],
        gap: usize,
        label: impl Into<String>,
    ) -> Result<Transaction, EditError> {
        let chosen: BTreeSet<FeatureId> = features.iter().copied().collect();
        if let [only] = features {
            return self.move_row(TreeRow::Feature(*only), gap, label);
        }
        if chosen.iter().any(|id| self.feature_index(*id).is_none()) {
            return Err(EditError::MissingFeature);
        }
        let rows = self.tree_rows();
        if gap > rows.len() {
            return Err(EditError::OutOfRange(gap));
        }
        let is_chosen = |row: &TreeRow| matches!(row, TreeRow::Feature(id) if chosen.contains(id));
        let (above, below) = rows.split_at(gap);
        let last_above = above.iter().rev().find_map(|row| match row {
            TreeRow::Feature(id) => Some(*id),
            TreeRow::Bar => None,
        });
        let anchor = below.iter().find_map(|row| match row {
            TreeRow::Feature(id) if !chosen.contains(id) => Some(*id),
            TreeRow::Feature(_) | TreeRow::Bar => None,
        });
        let moved: Vec<TreeRow> = rows.iter().copied().filter(is_chosen).collect();
        let mut final_rows: Vec<TreeRow> = above
            .iter()
            .copied()
            .filter(|row| !is_chosen(row))
            .collect();
        final_rows.extend(moved.iter().copied());
        final_rows.extend(below.iter().copied().filter(|row| !is_chosen(row)));
        let order: Vec<FeatureId> = final_rows
            .iter()
            .filter_map(|row| match row {
                TreeRow::Feature(id) => Some(*id),
                TreeRow::Bar => None,
            })
            .collect();
        let bar_at = final_rows
            .iter()
            .position(|row| *row == TreeRow::Bar)
            .unwrap_or(order.len());
        let bar = order
            .get(bar_at)
            .map_or(RollbackBar::AtEnd, |id| RollbackBar::Before(*id));

        let mut current: Vec<FeatureId> = self.features().map(Feature::id).collect();
        let gap_feature = above
            .iter()
            .filter(|row| matches!(row, TreeRow::Feature(_)))
            .count();
        let (sinking, rising): (Vec<FeatureId>, Vec<FeatureId>) = current
            .iter()
            .copied()
            .filter(|id| chosen.contains(id))
            .partition(|id| {
                self.feature_index(*id)
                    .is_some_and(|index| index < gap_feature)
            });
        let mut edits = Vec::new();
        let mut place = |current: &mut Vec<FeatureId>, id: FeatureId, slot: Slot| {
            let old = current.iter().position(|candidate| *candidate == id);
            current.retain(|candidate| *candidate != id);
            let index = match slot {
                Slot::After(Some(after)) => current
                    .iter()
                    .position(|candidate| *candidate == after)
                    .map_or(0, |found| found + 1),
                Slot::After(None) => 0,
                Slot::Before(before) => before
                    .and_then(|before| current.iter().position(|candidate| *candidate == before))
                    .unwrap_or(current.len()),
            };
            current.insert(index, id);
            if old != Some(index) {
                edits.push(Edit::MoveFeature { id, index });
            }
        };
        let mut after = last_above;
        for id in &rising {
            place(&mut current, *id, Slot::After(after));
            after = Some(*id);
        }
        let mut before = rising.first().copied().or(anchor);
        for id in sinking.iter().rev() {
            place(&mut current, *id, Slot::Before(before));
            before = Some(*id);
        }
        if !edits.is_empty() {
            edits.extend(self.membership_edits(&order, &chosen));
        }
        if bar != self.rollback {
            edits.push(Edit::SetRollbackBar { bar });
        }
        let transaction = Transaction::new(label, edits);
        self.check(&transaction)?;
        Ok(transaction)
    }
}
