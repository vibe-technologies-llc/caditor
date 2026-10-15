use std::{borrow::Cow, collections::BTreeSet};

use crate::{
    body_appearance::BodyAppearance,
    document::{Document, FeatureId},
    edit::{Edit, EditError, Transaction},
};

pub const MAX_BODY_FOLDER_DEPTH: usize = 8;
const BODY_GROUP_NAME_STEM: &str = "Group";
const FOLDER_SEPARATOR: &str = " › ";

pub fn folder_label(folder: &[String]) -> String {
    folder.join(FOLDER_SEPARATOR)
}

pub fn is_within(folder: &[String], outer: &[String]) -> bool {
    folder.starts_with(outer)
}

fn common_folder<'a>(mut folders: impl Iterator<Item = &'a [String]>) -> Vec<String> {
    let Some(first) = folders.next() else {
        return Vec::new();
    };
    let mut shared = first.len();
    for folder in folders {
        shared = first
            .iter()
            .zip(folder)
            .take(shared)
            .take_while(|(one, other)| one == other)
            .count();
    }
    first.iter().take(shared).cloned().collect()
}

fn rebased(folder: &[String], from: &[String], onto: &[String]) -> Vec<String> {
    onto.iter()
        .chain(folder.iter().skip(from.len()))
        .cloned()
        .collect()
}

pub struct BodyGrouping {
    pub transaction: Transaction,
    pub folder: Vec<String>,
}

impl Document {
    pub fn body_folder(&self, body: FeatureId) -> Vec<String> {
        self.body_appearance(body)
            .map(|appearance| appearance.folder.clone())
            .unwrap_or_default()
    }

    fn appearances(&self) -> impl Iterator<Item = (FeatureId, &BodyAppearance)> {
        self.features()
            .filter(|feature| feature.makes_body())
            .flat_map(|feature| {
                let pieces = feature
                    .piece_looks
                    .appearances()
                    .filter_map(|(key, appearance)| Some((feature.id().piece(key)?, appearance)));
                std::iter::once((feature.id(), &feature.appearance)).chain(pieces)
            })
    }

    pub fn body_folder_names(&self) -> BTreeSet<&str> {
        self.appearances()
            .flat_map(|(_, appearance)| appearance.folder.iter().map(String::as_str))
            .collect()
    }

    pub fn body_folder_members(&self, folder: &[String]) -> Vec<FeatureId> {
        self.appearances()
            .filter(|(_, appearance)| !appearance.folder.is_empty())
            .filter(|(_, appearance)| is_within(&appearance.folder, folder))
            .map(|(body, _)| body)
            .collect()
    }

    pub fn unused_body_folder_name(&self) -> String {
        let taken = self.body_folder_names();
        (1..)
            .map(|number| format!("{BODY_GROUP_NAME_STEM} {number}"))
            .find(|name| !taken.contains(name.as_str()))
            .unwrap_or_else(|| BODY_GROUP_NAME_STEM.to_owned())
    }

    pub fn body_grouping(
        &self,
        chosen: &[FeatureId],
        listed: &[FeatureId],
        label: impl Into<String>,
    ) -> Result<BodyGrouping, EditError> {
        let chosen: BTreeSet<FeatureId> = chosen.iter().copied().collect();
        let folders: Vec<(FeatureId, Vec<String>)> = chosen
            .iter()
            .map(|body| (*body, self.body_folder(*body)))
            .collect();
        let mut parent = common_folder(folders.iter().map(|(_, folder)| folder.as_slice()));
        while !parent.is_empty()
            && listed
                .iter()
                .filter(|body| is_within(&self.body_folder(**body), &parent))
                .all(|body| chosen.contains(body))
        {
            parent.pop();
        }
        let folder: Vec<String> = parent
            .iter()
            .cloned()
            .chain([self.unused_body_folder_name()])
            .collect();
        let moves = folders
            .iter()
            .map(|(body, from)| (*body, rebased(from, &parent, &folder)))
            .collect();
        Ok(BodyGrouping {
            transaction: self.refoldering(moves, label)?,
            folder,
        })
    }

    pub fn body_group_renaming(
        &self,
        folder: &[String],
        name: &str,
        label: impl Into<String>,
    ) -> Result<Transaction, EditError> {
        let Some((_, parent)) = folder.split_last() else {
            return Ok(Transaction::new(label, Vec::new()));
        };
        let renamed: Vec<String> = parent.iter().cloned().chain([name.to_owned()]).collect();
        self.moving_folder(folder, &renamed, label)
    }

    pub fn body_ungrouping(
        &self,
        folder: &[String],
        label: impl Into<String>,
    ) -> Result<Transaction, EditError> {
        let parent = folder.split_last().map_or(&[][..], |(_, parent)| parent);
        self.moving_folder(folder, parent, label)
    }

    pub fn body_group_moving(
        &self,
        folder: &[String],
        into: &[String],
        label: impl Into<String>,
    ) -> Result<Transaction, EditError> {
        let Some(name) = folder.last() else {
            return Ok(Transaction::new(label, Vec::new()));
        };
        if is_within(into, folder) {
            return Err(EditError::BodyFolderIntoItself);
        }
        let moved: Vec<String> = into.iter().cloned().chain([name.clone()]).collect();
        self.moving_folder(folder, &moved, label)
    }

    pub fn bodies_moving(
        &self,
        bodies: &[FeatureId],
        into: &[String],
        label: impl Into<String>,
    ) -> Result<Transaction, EditError> {
        let moves = bodies.iter().map(|body| (*body, into.to_vec())).collect();
        self.refoldering(moves, label)
    }

    pub fn bodies_moving_out(
        &self,
        bodies: &[FeatureId],
        label: impl Into<String>,
    ) -> Result<Transaction, EditError> {
        let moves = bodies
            .iter()
            .map(|body| {
                let mut folder = self.body_folder(*body);
                folder.pop();
                (*body, folder)
            })
            .collect();
        self.refoldering(moves, label)
    }

    fn moving_folder(
        &self,
        folder: &[String],
        onto: &[String],
        label: impl Into<String>,
    ) -> Result<Transaction, EditError> {
        let moves = self
            .body_folder_members(folder)
            .into_iter()
            .map(|body| (body, rebased(&self.body_folder(body), folder, onto)))
            .collect();
        self.refoldering(moves, label)
    }

    fn refoldering(
        &self,
        moves: Vec<(FeatureId, Vec<String>)>,
        label: impl Into<String>,
    ) -> Result<Transaction, EditError> {
        let mut edits = Vec::new();
        let mut seen = BTreeSet::new();
        for (body, folder) in moves {
            if !seen.insert(body) {
                continue;
            }
            let appearance = self
                .body_appearance(body)
                .ok_or(EditError::MissingFeature)?;
            if appearance.folder == folder {
                continue;
            }
            edits.push(Edit::SetBodyAppearance {
                id: body,
                appearance: BodyAppearance {
                    folder,
                    ..Cow::into_owned(appearance)
                },
            });
        }
        let transaction = Transaction::new(label, edits);
        self.check(&transaction)?;
        Ok(transaction)
    }
}
