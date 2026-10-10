use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::{
    boolean::{
        BooleanError, Input,
        assemble::assemble,
        faces,
        heal::{check_closed, uses},
        imprint::{self, Arrangement},
        select::{Class, KeptFace, classify},
    },
    interrupt::{self, Interrupted},
    naming::{EdgeName, FaceName, SplitPiece},
    topology::{FaceId, Solid},
};

#[derive(Debug, Clone, PartialEq, Error)]
pub enum FaceSplitError {
    #[error("none of the faces to split is on the body")]
    NoFaces,
    #[error("the tool divides none of the faces")]
    Undivided,
    #[error("the faces could not be divided: {0}")]
    Boolean(BooleanError),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

impl From<BooleanError> for FaceSplitError {
    fn from(error: BooleanError) -> Self {
        match error {
            BooleanError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            error => Self::Boolean(error),
        }
    }
}

pub fn split_faces(
    solid: &Solid,
    faces: &[FaceId],
    tool: &Solid,
    feature: u64,
) -> Result<Solid, FaceSplitError> {
    imprinted(solid, faces, tool, feature).or_else(|error| {
        interrupt::check()?;
        Err(error)
    })
}

fn imprinted(
    solid: &Solid,
    faces: &[FaceId],
    tool: &Solid,
    feature: u64,
) -> Result<Solid, FaceSplitError> {
    interrupt::check()?;
    let chosen: BTreeSet<FaceId> = faces
        .iter()
        .copied()
        .filter(|face| solid.face(*face).is_some())
        .collect();
    if chosen.is_empty() {
        return Err(FaceSplitError::NoFaces);
    }
    let input = Input::imprinting(solid, tool, chosen.clone());
    let arrangement = imprint::imprint(&input)?;
    interrupt::check()?;
    let split = faces::split(&input, &arrangement)?;
    interrupt::check()?;
    let mut kept = Vec::new();
    let mut divided = false;
    for face in split {
        interrupt::check()?;
        let original = input
            .face(face.key)
            .ok_or_else(|| BooleanError::split().or_faces([face.key]))?;
        let is_chosen = chosen.contains(&face.key.face);
        divided |= is_chosen && face.fragments.len() > 1;
        for fragment in face.fragments {
            let name = if is_chosen {
                let class = classify(
                    &input,
                    face.key,
                    original.surface(),
                    original.sense(),
                    &fragment,
                )?;
                FaceName::split(feature, original.name(), piece_of(class))
            } else {
                original.name()
            };
            kept.push(KeptFace {
                key: face.key,
                surface: original.surface().clone(),
                sense: original.sense(),
                name,
                origin: original.origin(),
                fragment,
            });
        }
    }
    if !divided {
        return Err(FaceSplitError::Undivided);
    }
    check_closed(&arrangement, &kept)?;
    let renamed = edge_names(&arrangement, &kept, &chosen);
    Ok(assemble(&input, &arrangement, kept, &renamed)?)
}

fn piece_of(class: Class) -> SplitPiece {
    match class {
        Class::Outside => SplitPiece::Outside,
        Class::Inside | Class::Coincident(_) => SplitPiece::Inside,
    }
}

fn edge_names(
    arrangement: &Arrangement,
    faces: &[KeptFace],
    chosen: &BTreeSet<FaceId>,
) -> BTreeMap<usize, EdgeName> {
    uses(faces)
        .into_iter()
        .filter_map(|(piece, users)| {
            let around: Vec<&KeptFace> = users
                .iter()
                .filter_map(|(index, _)| faces.get(*index))
                .collect();
            if !around.iter().any(|face| chosen.contains(&face.key.face)) {
                return None;
            }
            let name = match (users.as_slice(), around.as_slice()) {
                ([(first, _), (second, _)], _) if first == second => {
                    EdgeName::seam(faces.get(*first)?.name)
                }
                (_, [first, second]) => EdgeName::between(first.name, second.name),
                _ => arrangement.source(arrangement.piece(piece)?.source)?.name,
            };
            Some((piece, name))
        })
        .collect()
}
