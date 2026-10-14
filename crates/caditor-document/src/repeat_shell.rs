use std::collections::BTreeMap;

use caditor_kernel::{FaceId, ShellError, Solid, Walls, shell_copies};

use crate::{
    pattern::{Instance, SeedWords},
    recompute::{CancelToken, Failure, FeatureError, FixTarget},
    repeat_blend::{CopiedNames, CopyWords, Followed},
    shell::{NamedFace, ShellRecipe},
    solid::NameIndex,
};

#[derive(Debug)]
pub(crate) enum CopyShellError {
    Cancelled,
    OriginalOpening,
    Missing { copy: Instance },
    Shell(ShellError),
}

struct Hollowing {
    open: Vec<FaceId>,
    faces: BTreeMap<FaceId, f64>,
}

impl CopiedNames {
    fn copied_faces<'a>(
        &self,
        names: &'a NameIndex,
        copy: Instance,
        face: &NamedFace,
    ) -> Option<&'a [FaceId]> {
        match self.follow(copy, face.name, face.origin) {
            Followed::Copied(name) => Some(names.faces_named(name)),
            Followed::Kept => None,
            Followed::Lost => Some(&[]),
        }
    }

    fn hollowing(
        &self,
        body: &Solid,
        recipe: &ShellRecipe,
        copies: &[Instance],
        cancel: &CancelToken,
    ) -> Result<Hollowing, CopyShellError> {
        let names = NameIndex::of(body);
        let mut open = Vec::new();
        for face in &recipe.open {
            match names.faces_named(face.name) {
                [] => return Err(CopyShellError::OriginalOpening),
                found => open.extend_from_slice(found),
            }
        }
        let mut faces: BTreeMap<FaceId, f64> = recipe
            .faces
            .iter()
            .flat_map(|(face, thickness)| {
                names
                    .faces_named(face.name)
                    .iter()
                    .map(|found| (*found, *thickness))
            })
            .collect();
        for &copy in copies {
            if cancel.is_cancelled() {
                return Err(CopyShellError::Cancelled);
            }
            let openings: Vec<&[FaceId]> = recipe
                .open
                .iter()
                .filter_map(|face| self.copied_faces(&names, copy, face))
                .collect();
            let missed_the_body =
                !openings.is_empty() && openings.iter().all(|found| found.is_empty());
            if missed_the_body {
                continue;
            }
            if openings.iter().any(|found| found.is_empty()) {
                return Err(CopyShellError::Missing { copy });
            }
            open.extend(openings.into_iter().flatten());
            for (face, thickness) in &recipe.faces {
                let found = self.copied_faces(&names, copy, face).unwrap_or_default();
                faces.extend(found.iter().map(|found| (*found, *thickness)));
            }
        }
        open.sort_unstable();
        open.dedup();
        Ok(Hollowing { open, faces })
    }

    pub(crate) fn shell(
        &self,
        body: &Solid,
        recipe: &ShellRecipe,
        copies: &[Instance],
        cancel: &CancelToken,
    ) -> Result<Solid, CopyShellError> {
        let Hollowing { open, faces } = self.hollowing(body, recipe, copies, cancel)?;
        let walls = Walls {
            thickness: recipe.thickness,
            side: recipe.side,
            faces,
        };
        shell_copies(body, &open, &walls, self.pattern, recipe.feature.raw()).map_err(|error| {
            match error {
                ShellError::Cancelled(_) => CopyShellError::Cancelled,
                error => CopyShellError::Shell(error),
            }
        })
    }
}

pub(crate) fn failure(error: &CopyShellError, words: &CopyWords<'_>) -> Failure {
    let CopyWords {
        feature,
        repeated: shell,
        seed,
        copy,
        copies,
        clear,
    } = words;
    let SeedWords {
        feature: kind,
        repeated,
        ..
    } = seed;
    let leave_out = format!("or leave {shell} out of the {kind}");
    let (reason, remedy) = match error {
        CopyShellError::Cancelled => return Failure::Cancelled,
        CopyShellError::OriginalOpening => (
            format!(
                "{shell} could not be {repeated}: a face it opens is no longer part of the body \
                 once {copies} are added."
            ),
            format!("{clear} the faces {shell} opens are not covered by {copies}, {leave_out}."),
        ),
        CopyShellError::Missing { copy: at } => (
            format!(
                "{shell} could not be {repeated} on {}: a face it opens on the original has no \
                 match there, as the copy meets the body differently.",
                copy(*at)
            ),
            format!(
                "{clear} {} stands clear of other features, {leave_out}.",
                copy(*at)
            ),
        ),
        CopyShellError::Shell(error) => {
            let why = match error {
                ShellError::TooThick => {
                    format!("the walls are too thick for the body with {copies}")
                }
                ShellError::Overhang { .. } => {
                    "a face leans over an opening, so its wall would be cut thin".to_owned()
                }
                error => {
                    log::warn!("{shell} could not hollow {copies}: {error}");
                    format!("the walls could not be built with {copies}")
                }
            };
            (
                format!("{shell} could not hollow {copies}: {why}."),
                format!("{clear} there is room for the walls around {copies}, {leave_out}."),
            )
        }
    };
    Failure::Error(Box::new(FeatureError {
        reason,
        remedy,
        fix: Some(FixTarget::Feature(*feature)),
        constraints: Vec::new(),
        place: None,
    }))
}
