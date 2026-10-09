use std::collections::BTreeSet;

use caditor_kernel::{
    EdgeId, EdgeNaming, EdgeReference, FaceId, FaceReference, ReferenceError, Solid,
};

use crate::{document::FeatureId, recompute::Evaluation, views::view_name};

pub const MAX_SET_NAME_CHARS: usize = 60;
pub const MAX_SELECTION_SETS: usize = 64;
pub const MAX_SET_MEMBERS: usize = 4096;
const NUMBERED_NAME: &str = "Set";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SetMember {
    Body(FeatureId),
    Face {
        body: FeatureId,
        face: FaceReference,
    },
    Edge {
        body: FeatureId,
        edge: EdgeReference,
    },
}

impl SetMember {
    pub fn body(&self) -> FeatureId {
        match self {
            Self::Body(body) | Self::Face { body, .. } | Self::Edge { body, .. } => *body,
        }
    }

    pub fn heap_size(&self) -> usize {
        match self {
            Self::Face { face, .. } => face.heap_size(),
            Self::Body(_) | Self::Edge { .. } => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectionSet {
    pub name: String,
    pub members: Vec<SetMember>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SelectionSets {
    pub sets: Vec<SelectionSet>,
}

pub fn set_name(text: &str) -> String {
    view_name(text)
}

fn same_name(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

impl SelectionSets {
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    pub fn position(&self, name: &str) -> Option<usize> {
        self.sets.iter().position(|set| same_name(&set.name, name))
    }

    pub fn is_taken(&self, name: &str) -> bool {
        self.position(name).is_some()
    }

    pub fn unused_name(&self) -> String {
        (self.sets.len() + 1..)
            .map(|number| format!("{NUMBERED_NAME} {number}"))
            .find(|name| !self.is_taken(name))
            .unwrap_or_else(|| NUMBERED_NAME.to_owned())
    }

    pub fn bodies(&self) -> impl Iterator<Item = FeatureId> + '_ {
        self.sets
            .iter()
            .flat_map(|set| set.members.iter().map(SetMember::body))
    }

    #[must_use]
    pub fn normalized(self) -> Self {
        Self {
            sets: self
                .sets
                .into_iter()
                .map(|set| SelectionSet {
                    name: set_name(&set.name),
                    members: set.members,
                })
                .collect(),
        }
    }

    pub fn heap_size(&self) -> usize {
        self.sets
            .iter()
            .map(|set| {
                size_of::<SelectionSet>()
                    + set.name.len()
                    + size_of_val(set.members.as_slice())
                    + set.members.iter().map(SetMember::heap_size).sum::<usize>()
            })
            .sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SetItem {
    Face { body: FeatureId, face: FaceId },
    Edge { body: FeatureId, edge: EdgeId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetLoss {
    Body(FeatureId),
    Face(FeatureId),
    Edge(FeatureId),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetResolution {
    pub found: Vec<SetItem>,
    pub lost: Vec<SetLoss>,
}

fn candidates<Id>(resolved: Result<Id, ReferenceError<Id>>) -> Vec<Id> {
    match resolved {
        Ok(id) => vec![id],
        Err(ReferenceError::Ambiguous(tied)) => tied,
        Err(ReferenceError::Missing) => Vec::new(),
    }
}

impl SelectionSet {
    pub fn resolve(&self, evaluation: &Evaluation) -> SetResolution {
        let mut resolved = SetResolution::default();
        let mut seen: BTreeSet<SetItem> = BTreeSet::new();
        let mut namings: Vec<(FeatureId, EdgeNaming)> = Vec::new();
        for member in &self.members {
            let body = member.body();
            let Some(solid) = evaluation
                .body_result(body)
                .and_then(|result| result.solid())
                .map(|result| &result.solid)
            else {
                if !resolved.lost.contains(&SetLoss::Body(body)) {
                    resolved.lost.push(SetLoss::Body(body));
                }
                continue;
            };
            let found = match member {
                SetMember::Body(_) => every_face(body, solid),
                SetMember::Face { face, .. } => candidates(face.resolve(solid))
                    .into_iter()
                    .map(|face| SetItem::Face { body, face })
                    .collect(),
                SetMember::Edge { edge, .. } => {
                    let naming = match namings.iter().position(|(named, _)| *named == body) {
                        Some(index) => namings.get(index).map(|(_, naming)| naming),
                        None => {
                            namings.push((body, EdgeNaming::new(solid)));
                            namings.last().map(|(_, naming)| naming)
                        }
                    };
                    naming
                        .map(|naming| candidates(edge.resolve_in(naming)))
                        .unwrap_or_default()
                        .into_iter()
                        .map(|edge| SetItem::Edge { body, edge })
                        .collect()
                }
            };
            if found.is_empty() {
                resolved.lost.push(match member {
                    SetMember::Body(_) => SetLoss::Body(body),
                    SetMember::Face { .. } => SetLoss::Face(body),
                    SetMember::Edge { .. } => SetLoss::Edge(body),
                });
            }
            resolved
                .found
                .extend(found.into_iter().filter(|item| seen.insert(*item)));
        }
        resolved
    }
}

fn every_face(body: FeatureId, solid: &Solid) -> Vec<SetItem> {
    solid
        .faces()
        .map(|(face, _)| SetItem::Face { body, face })
        .collect()
}
