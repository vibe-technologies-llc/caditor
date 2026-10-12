use std::{collections::BTreeSet, sync::Arc};

use crate::{
    document::{Document, Feature, FeatureId, TreeRow},
    edit::{Edit, EditError, Transaction},
};

pub const MAX_GROUP_NAME_CHARS: usize = 120;
const GROUP_NAME_STEM: &str = "Group";

pub fn group_name(name: &str) -> Option<String> {
    let one_line = name.split(['\r', '\n']).collect::<Vec<_>>().join(" ");
    let trimmed = one_line.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

impl Document {
    pub fn group_of(&self, feature: FeatureId) -> Option<&str> {
        self.feature(feature)?.group.as_deref()
    }

    pub(crate) fn group_around(&self, index: usize) -> Option<String> {
        let above = index
            .checked_sub(1)
            .and_then(|above| self.features.get(above))?;
        let below = self.features.get(index)?;
        above
            .group
            .clone()
            .filter(|group| below.group.as_deref() == Some(group))
    }

    pub fn group_run(&self, feature: FeatureId) -> Vec<FeatureId> {
        let Some(at) = self.feature_index(feature) else {
            return Vec::new();
        };
        let Some(group) = self
            .features
            .get(at)
            .and_then(|found| found.group.as_deref())
        else {
            return Vec::new();
        };
        let within = |found: &&Arc<Feature>| found.group.as_deref() == Some(group);
        let Some((above, from)) = self.features.split_at_checked(at) else {
            return Vec::new();
        };
        let before = above.iter().rev().take_while(within).count();
        let after = from.iter().take_while(within).count();
        self.features
            .iter()
            .skip(at - before)
            .take(before + after)
            .map(|found| found.id())
            .collect()
    }

    pub fn unused_group_name(&self) -> String {
        let taken: BTreeSet<&str> = self
            .features()
            .filter_map(|feature| feature.group.as_deref())
            .collect();
        (1..)
            .map(|number| format!("{GROUP_NAME_STEM} {number}"))
            .find(|name| !taken.contains(name.as_str()))
            .unwrap_or_else(|| GROUP_NAME_STEM.to_owned())
    }

    pub fn grouping(
        &self,
        features: &[FeatureId],
        name: &str,
        label: impl Into<String>,
    ) -> Result<Transaction, EditError> {
        let label = label.into();
        let chosen: BTreeSet<FeatureId> = features.iter().copied().collect();
        let indices: Vec<usize> = chosen
            .iter()
            .map(|id| self.feature_index(*id).ok_or(EditError::MissingFeature))
            .collect::<Result<_, _>>()?;
        let (Some(first), Some(last)) = (indices.iter().min(), indices.iter().max()) else {
            return Ok(Transaction::new(label, Vec::new()));
        };
        let mut edits = Vec::new();
        if last - first + 1 != indices.len() {
            let first_id = self
                .features()
                .nth(*first)
                .map(Feature::id)
                .ok_or(EditError::MissingFeature)?;
            let gap = self
                .tree_rows()
                .iter()
                .position(|row| *row == TreeRow::Feature(first_id))
                .unwrap_or(0);
            let ordered: Vec<FeatureId> = self
                .features()
                .map(Feature::id)
                .filter(|id| chosen.contains(id))
                .collect();
            edits.extend(
                self.move_features(&ordered, gap, label.clone())?
                    .edits()
                    .iter()
                    .cloned(),
            );
        }
        let group = Some(name.to_owned());
        edits.extend(chosen.iter().map(|id| Edit::SetFeatureGroup {
            id: *id,
            group: group.clone(),
        }));
        let transaction = Transaction::new(label, edits);
        self.check(&transaction)?;
        Ok(transaction)
    }

    pub fn regrouping(
        &self,
        members: &[FeatureId],
        group: Option<String>,
        label: impl Into<String>,
    ) -> Result<Transaction, EditError> {
        let edits = members
            .iter()
            .filter(|id| self.feature(**id).is_some_and(|found| found.group != group))
            .map(|id| Edit::SetFeatureGroup {
                id: *id,
                group: group.clone(),
            })
            .collect();
        let transaction = Transaction::new(label, edits);
        self.check(&transaction)?;
        Ok(transaction)
    }

    pub(crate) fn membership_edits(
        &self,
        order: &[FeatureId],
        moved: &BTreeSet<FeatureId>,
    ) -> Vec<Edit> {
        let group = |id: Option<&FeatureId>| id.and_then(|id| self.group_of(*id));
        order
            .iter()
            .enumerate()
            .filter(|(_, id)| moved.contains(id))
            .filter_map(|(at, id)| {
                let above = order
                    .get(..at)
                    .and_then(|before| before.iter().rev().find(|other| !moved.contains(other)));
                let below = order
                    .get(at + 1..)
                    .and_then(|after| after.iter().find(|other| !moved.contains(other)));
                let own = self.group_of(*id);
                let (over, under) = (group(above), group(below));
                let wanted = match (over, under) {
                    (Some(over), Some(under)) if over == under => Some(over),
                    _ if own.is_some() && own != over && own != under => None,
                    _ => own,
                };
                (wanted != own).then(|| Edit::SetFeatureGroup {
                    id: *id,
                    group: wanted.map(str::to_owned),
                })
            })
            .collect()
    }
}
