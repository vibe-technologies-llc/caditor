use std::collections::{BTreeMap, BTreeSet};

use caditor_kernel::{
    BlendError, BlendShape, EdgeId, EdgeName, EdgeNaming, EdgeReference, FaceCopy, FaceName,
    FaceOrigin, ReferenceError, Solid, VertexName, blend_copy,
};

use crate::{
    blend::{BlendRecipe, BlendedEdge},
    document::FeatureId,
    pattern::{Instance, SeedWords},
    recompute::{CancelToken, Failure, FeatureError, FixTarget},
};

pub(crate) struct CopiedNames {
    pub(crate) pattern: u64,
    tools: BTreeSet<u64>,
    blends: BTreeSet<u64>,
    renamed: BTreeMap<(Instance, FaceName), FaceName>,
}

pub(crate) enum Followed {
    Copied(FaceName),
    Kept,
    Lost,
}

#[derive(Debug)]
pub(crate) enum CopyBlendError {
    Cancelled,
    NoRepeatedEdge,
    Missing { copy: Instance, edges: usize },
    Ambiguous { copy: Instance },
    Blend { copy: Instance, error: BlendError },
}

impl CopiedNames {
    pub(crate) fn new(pattern: FeatureId) -> Self {
        Self {
            pattern: pattern.raw(),
            tools: BTreeSet::new(),
            blends: BTreeSet::new(),
            renamed: BTreeMap::new(),
        }
    }

    pub(crate) fn copying_tools_of(&mut self, feature: FeatureId) {
        self.tools.insert(feature.raw());
    }

    fn made_by_repeated(&self, origin: Option<FaceOrigin>) -> Option<u64> {
        let feature = origin.filter(|origin| origin.copy().is_none())?.feature();
        (self.tools.contains(&feature) || self.blends.contains(&feature)).then_some(feature)
    }

    pub(crate) fn follow(
        &self,
        copy: Instance,
        name: FaceName,
        origin: Option<FaceOrigin>,
    ) -> Followed {
        if let Some(renamed) = self.renamed.get(&(copy, name)) {
            return Followed::Copied(*renamed);
        }
        match self.made_by_repeated(origin) {
            Some(feature) if self.tools.contains(&feature) => {
                Followed::Copied(FaceName::pattern(self.pattern, copy, name))
            }
            Some(_) => Followed::Lost,
            None => Followed::Kept,
        }
    }

    fn is_repeated(&self, edge: &BlendedEdge) -> bool {
        edge.reference
            .origins()
            .iter()
            .any(|origin| self.made_by_repeated(*origin).is_some())
    }

    fn copied_reference(&self, copy: Instance, edge: &BlendedEdge) -> Option<EdgeReference> {
        let faces = edge.reference.faces();
        let origins = edge.reference.origins();
        let mut copied = [FaceName::NONE; 2];
        for ((name, origin), slot) in faces.iter().zip(origins).zip(&mut copied) {
            *slot = match self.follow(copy, *name, origin) {
                Followed::Copied(renamed) => renamed,
                Followed::Kept => *name,
                Followed::Lost => return None,
            };
        }
        let [first, second] = copied;
        let [start, end] = &edge.ends;
        Some(EdgeReference::new(
            EdgeName::between(first, second),
            copied,
            [
                self.copied_vertex(copy, start),
                self.copied_vertex(copy, end),
            ],
        ))
    }

    fn copied_vertex(
        &self,
        copy: Instance,
        around: &BTreeMap<FaceName, Option<FaceOrigin>>,
    ) -> VertexName {
        VertexName::of_faces(around.iter().map(|(name, origin)| {
            match self.follow(copy, *name, *origin) {
                Followed::Copied(renamed) => renamed,
                Followed::Kept | Followed::Lost => *name,
            }
        }))
    }

    fn corner_names(
        &self,
        copy: Instance,
        blend: FeatureId,
        edges: &[&BlendedEdge],
    ) -> Vec<((Instance, FaceName), FaceName)> {
        edges
            .iter()
            .flat_map(|edge| &edge.ends)
            .map(|around| {
                let original = VertexName::of_faces(around.keys().copied());
                (
                    (copy, FaceName::corner(blend.raw(), original)),
                    FaceName::corner(self.pattern, self.copied_vertex(copy, around)),
                )
            })
            .collect()
    }

    pub(crate) fn blend(
        &mut self,
        body: Solid,
        recipe: &BlendRecipe,
        copies: &[Instance],
        cancel: &CancelToken,
    ) -> Result<Solid, CopyBlendError> {
        let repeated: Vec<&BlendedEdge> = recipe
            .edges
            .iter()
            .filter(|edge| self.is_repeated(edge))
            .collect();
        if repeated.is_empty() {
            return Err(CopyBlendError::NoRepeatedEdge);
        }
        let mut body = body;
        let mut renamed = Vec::new();
        for &copy in copies {
            if cancel.is_cancelled() {
                return Err(CopyBlendError::Cancelled);
            }
            let naming = EdgeNaming::new(&body);
            let mut found: Vec<(EdgeName, EdgeId)> = Vec::new();
            let mut missing = 0;
            for edge in &repeated {
                let resolved = self
                    .copied_reference(copy, edge)
                    .map(|reference| reference.resolve_by_names_in(&naming));
                match resolved {
                    Some(Ok(id)) => found.push((edge.reference.name(), id)),
                    Some(Err(ReferenceError::Ambiguous(_))) => {
                        return Err(CopyBlendError::Ambiguous { copy });
                    }
                    None | Some(Err(ReferenceError::Missing)) => missing += 1,
                }
            }
            if found.is_empty() {
                continue;
            }
            if missing > 0 {
                return Err(CopyBlendError::Missing {
                    copy,
                    edges: missing,
                });
            }
            let mut edges: Vec<EdgeId> = found.iter().map(|(_, id)| *id).collect();
            edges.sort_unstable();
            edges.dedup();
            let made = FaceCopy {
                pattern: self.pattern,
                index: copy,
            };
            let blended = blend_copy(&body, &edges, recipe.shape, made, recipe.feature.raw())
                .map_err(|error| match error {
                    BlendError::Cancelled(_) => CopyBlendError::Cancelled,
                    error => CopyBlendError::Blend { copy, error },
                })?;
            renamed.extend(self.corner_names(copy, recipe.feature, &repeated));
            for (original, id) in found {
                if let Some(edge) = body.edge(id) {
                    renamed.push((
                        (copy, FaceName::blend(recipe.feature.raw(), original)),
                        FaceName::blend(self.pattern, edge.name()),
                    ));
                }
            }
            body = blended;
        }
        self.renamed.extend(renamed);
        self.blends.insert(recipe.feature.raw());
        Ok(body)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CopyWords<'a> {
    pub feature: FeatureId,
    pub repeated: &'a str,
    pub seed: &'a SeedWords,
    pub copy: &'a dyn Fn(Instance) -> String,
    pub copies: &'a str,
    pub clear: &'a str,
}

impl<'a> CopyWords<'a> {
    pub(crate) fn naming<'b>(&self, repeated: &'b str) -> CopyWords<'b>
    where
        'a: 'b,
    {
        CopyWords { repeated, ..*self }
    }
}

fn noun(shape: BlendShape) -> &'static str {
    match shape {
        BlendShape::Fillet { .. } => "fillet",
        BlendShape::Chamfer { .. }
        | BlendShape::TwoDistanceChamfer { .. }
        | BlendShape::AngledChamfer { .. } => "chamfer",
    }
}

fn size_name(shape: BlendShape) -> &'static str {
    match shape {
        BlendShape::Fillet { .. } => "radius",
        BlendShape::Chamfer { .. }
        | BlendShape::TwoDistanceChamfer { .. }
        | BlendShape::AngledChamfer { .. } => "distance",
    }
}

pub(crate) fn failure(
    error: &CopyBlendError,
    recipe: &BlendRecipe,
    words: &CopyWords<'_>,
) -> Failure {
    let CopyWords {
        feature,
        repeated: blend,
        seed,
        copy,
        clear,
        ..
    } = words;
    let SeedWords {
        feature: kind,
        repeats,
        repeated,
        verb,
        ..
    } = seed;
    let noun = noun(recipe.shape);
    let leave_out = format!("or leave {blend} out of the {kind}");
    let (reason, remedy) = match error {
        CopyBlendError::Cancelled => return Failure::Cancelled,
        CopyBlendError::NoRepeatedEdge => (
            format!(
                "{blend} blends no edge of the features this {kind} {repeats}, so its {noun} \
                 has nothing to follow on the copies."
            ),
            format!("Also {verb} the feature whose edges {blend} blends, {leave_out}."),
        ),
        CopyBlendError::Missing { copy: at, edges } => {
            let several = if *edges == 1 {
                "an edge".to_owned()
            } else {
                format!("{edges} edges")
            };
            (
                format!(
                    "{blend} could not be {repeated} on {}: {several} it blends on the original \
                     have no match there, as the copy meets the body differently.",
                    copy(*at)
                ),
                format!(
                    "{clear} {} stands clear of other features, {leave_out}.",
                    copy(*at)
                ),
            )
        }
        CopyBlendError::Ambiguous { copy: at } => (
            format!(
                "{blend} could not be {repeated} on {}: an edge it blends matches several \
                 edges there.",
                copy(*at)
            ),
            format!(
                "{clear} {} stands clear of other features, {leave_out}.",
                copy(*at)
            ),
        ),
        CopyBlendError::Blend { copy: at, error } => {
            let place = copy(*at);
            let why = match error {
                BlendError::TooLarge(_) => format!(
                    "the {} is too large for the faces there",
                    size_name(recipe.shape)
                ),
                BlendError::Unsupported(_) | BlendError::UnsupportedEnd { .. } => {
                    "the faces around its edges there differ from the original's".to_owned()
                }
                error => {
                    log::warn!("{blend} could not be repeated on {place}: {error}");
                    format!("the {noun} could not be built there")
                }
            };
            (
                format!("{blend} could not be {repeated} on {place}: {why}."),
                format!("{clear} {place} stands clear of other features, {leave_out}."),
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
