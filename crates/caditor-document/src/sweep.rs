use std::collections::BTreeSet;

use caditor_geometry::Plane;
use caditor_kernel::{
    EdgeNaming, EdgeReference, PathSegment, PathSweepError, ReferenceError, Region, Solid,
    SweepOrientation, sweep,
};
use caditor_sketch::EntityId;

use crate::{
    describe::describe_edge,
    document::{FeatureId, list_names},
    origins,
    recompute::{Failure, FeatureResult, FixTarget, Inputs},
    solid::{BodyOperation, Context, RegionChoice, profile_curve, sweep_failure},
};

pub const SWEEP_SHAPE: &str = "sweep";

#[derive(Debug, Clone, PartialEq, Default)]
pub enum SweepPath {
    #[default]
    Unchosen,
    Curves {
        sketch: FeatureId,
        curves: Vec<EntityId>,
    },
    Edges {
        body: FeatureId,
        edges: Vec<EdgeReference>,
    },
}

impl SweepPath {
    pub fn sketch(&self) -> Option<FeatureId> {
        match self {
            Self::Curves { sketch, .. } => Some(*sketch),
            Self::Unchosen | Self::Edges { .. } => None,
        }
    }

    pub fn body(&self) -> Option<FeatureId> {
        match self {
            Self::Edges { body, .. } => Some(*body),
            Self::Unchosen | Self::Curves { .. } => None,
        }
    }

    pub fn curves(&self) -> &[EntityId] {
        match self {
            Self::Curves { curves, .. } => curves,
            Self::Unchosen | Self::Edges { .. } => &[],
        }
    }

    pub fn edges(&self) -> &[EdgeReference] {
        match self {
            Self::Edges { edges, .. } => edges,
            Self::Unchosen | Self::Curves { .. } => &[],
        }
    }

    pub fn len(&self) -> usize {
        self.curves().len() + self.edges().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn holds_curve(&self, sketch: FeatureId, curve: EntityId) -> bool {
        self.sketch() == Some(sketch) && self.curves().contains(&curve)
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.edges().iter().flat_map(origins::of_edge).collect()
    }

    pub fn heap_size(&self) -> usize {
        size_of_val(self.curves()) + size_of_val(self.edges())
    }

    #[must_use]
    pub fn with_features_renamed(&self, rename: &dyn Fn(FeatureId) -> FeatureId) -> Self {
        match self {
            Self::Unchosen => Self::Unchosen,
            Self::Curves { sketch, curves } => Self::Curves {
                sketch: rename(*sketch),
                curves: curves.clone(),
            },
            Self::Edges { body, edges } => Self::Edges {
                body: rename(*body),
                edges: edges.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sweep {
    pub sketch: FeatureId,
    pub regions: RegionChoice,
    pub path: SweepPath,
    pub orientation: SweepOrientation,
    pub operation: BodyOperation,
    pub other_bodies: Vec<FeatureId>,
}

impl Sweep {
    pub fn new(sketch: FeatureId, path: SweepPath) -> Self {
        Self {
            sketch,
            regions: RegionChoice::All,
            path,
            orientation: SweepOrientation::Square,
            operation: BodyOperation::NewBody,
            other_bodies: Vec::new(),
        }
    }
}

struct Path {
    segments: Vec<PathSegment>,
    labels: Vec<String>,
    fix: FixTarget,
    owner: String,
}

impl Path {
    fn labels(&self, segments: &[usize]) -> String {
        let named: Vec<String> = segments
            .iter()
            .filter_map(|segment| self.labels.get(*segment).cloned())
            .collect();
        list_names(&named)
    }

    fn label(&self, segment: usize) -> String {
        self.labels.get(segment).cloned().unwrap_or_default()
    }
}

fn feature_name(inputs: &Inputs<'_>, feature: FeatureId) -> String {
    inputs
        .document
        .feature(feature)
        .map(|feature| feature.name.clone())
        .unwrap_or_default()
}

fn no_path(context: &Context<'_>) -> Failure {
    context.error(
        format!("No path is chosen for {}.", context.feature.name),
        "Choose the curves of a sketch or the edges of a body to sweep along.".to_owned(),
        context.own(),
    )
}

fn curve_path(
    context: &Context<'_>,
    inputs: &Inputs<'_>,
    sketch_id: FeatureId,
    curves: &[EntityId],
) -> Result<Path, Failure> {
    let sketch_name = feature_name(inputs, sketch_id);
    let Some(FeatureResult::Sketch(sketch)) = inputs.features.get(&sketch_id).map(AsRef::as_ref)
    else {
        return Err(context.error(
            format!("It needs the path drawn in {sketch_name}, which is not available."),
            format!("Fix {sketch_name} first, or choose another path."),
            FixTarget::Feature(sketch_id),
        ));
    };
    let geometry = &sketch.geometry;
    let plane = geometry.plane();
    let mut segments = Vec::with_capacity(curves.len());
    let mut labels = Vec::with_capacity(curves.len());
    for curve in curves {
        let missing = || {
            context.error(
                format!(
                    "A curve of the path is no longer a curve of {sketch_name}, so {} cannot \
                     follow it.",
                    context.feature.name
                ),
                "Choose the path again.".to_owned(),
                context.own(),
            )
        };
        let (flat, range) = profile_curve(geometry, *curve)
            .ok_or_else(missing)?
            .curve()
            .map_err(|_| missing())?;
        let placed = flat.on_plane(&plane).map_err(|_| missing())?;
        segments.push(PathSegment::new(placed, range, u128::from(curve.raw())));
        labels.push(geometry.entity_label(*curve));
    }
    Ok(Path {
        segments,
        labels,
        fix: FixTarget::Feature(sketch_id),
        owner: format!(" of {sketch_name}"),
    })
}

fn edge_path(
    context: &Context<'_>,
    inputs: &Inputs<'_>,
    body: FeatureId,
    edges: &[EdgeReference],
) -> Result<Path, Failure> {
    let body_name = feature_name(inputs, body);
    let solid: &Solid = inputs.body(body).ok_or_else(|| inputs.missing_body(body))?;
    let naming = EdgeNaming::new(solid);
    let mut segments = Vec::with_capacity(edges.len());
    let mut labels = Vec::with_capacity(edges.len());
    for reference in edges {
        let id = reference.resolve_in(&naming).map_err(|error| {
            let reason = match error {
                ReferenceError::Ambiguous { .. } => format!(
                    "An edge of the path now matches several edges of {body_name}, so {} cannot \
                     tell which to follow.",
                    context.feature.name
                ),
                ReferenceError::Missing => format!(
                    "An edge of the path is no longer part of {body_name}, so {} cannot follow it.",
                    context.feature.name
                ),
            };
            context.error(reason, "Choose the path again.".to_owned(), context.own())
        })?;
        let edge = solid.edge(id).ok_or_else(|| {
            context.error(
                format!("An edge of the path is no longer part of {body_name}."),
                "Choose the path again.".to_owned(),
                context.own(),
            )
        })?;
        segments.push(PathSegment::new(
            edge.curve().clone(),
            edge.interval(),
            reference.name().digest(),
        ));
        labels.push(describe_edge(inputs.document, solid, id));
    }
    Ok(Path {
        segments,
        labels,
        fix: context.own(),
        owner: format!(" of {body_name}"),
    })
}

pub(crate) fn swept(
    context: &Context<'_>,
    inputs: &Inputs<'_>,
    (plane, regions): (&Plane, &[Region]),
    definition: &Sweep,
) -> Result<Solid, Failure> {
    if definition.path.is_empty() {
        return Err(no_path(context));
    }
    let path = match &definition.path {
        SweepPath::Unchosen => return Err(no_path(context)),
        SweepPath::Curves { sketch, curves } => curve_path(context, inputs, *sketch, curves)?,
        SweepPath::Edges { body, edges } => edge_path(context, inputs, *body, edges)?,
    };
    if path.segments.is_empty() {
        return Err(no_path(context));
    }
    sweep(
        plane,
        regions,
        &path.segments,
        definition.orientation,
        context.feature.id().raw(),
    )
    .map_err(|error| failure(context, &path, &error))
}

fn failure(context: &Context<'_>, path: &Path, error: &PathSweepError) -> Failure {
    let name = &context.feature.name;
    let owner = &path.owner;
    let in_path = |reason: String, remedy: &str| context.error(reason, remedy.to_owned(), path.fix);
    match error {
        PathSweepError::Cancelled(_) => Failure::Cancelled,
        PathSweepError::Sweep(error) => sweep_failure(context, SWEEP_SHAPE, error),
        PathSweepError::NoPath => no_path(context),
        PathSweepError::SegmentTooShort { segment } => in_path(
            format!(
                "{}{owner} is too short to sweep along.",
                capital(&path.label(*segment))
            ),
            "Lengthen it, or leave it out of the path.",
        ),
        PathSweepError::Branches { segments } => in_path(
            format!(
                "The path branches where {}{owner} meet.",
                path.labels(segments)
            ),
            "Choose curves or edges that follow one another end to end, without forks.",
        ),
        PathSweepError::Apart { segments } => in_path(
            format!(
                "{}{owner} {} not join the rest of the path.",
                capital(&path.labels(segments)),
                if segments.len() == 1 { "does" } else { "do" }
            ),
            "Choose curves or edges that meet end to end, or close the gap between them.",
        ),
        PathSweepError::UnsupportedPath { segment } => in_path(
            format!("{name} cannot follow {}{owner}.", path.label(*segment)),
            "Choose lines, arcs, splines or edges as the path.",
        ),
        PathSweepError::TooLong => in_path(
            "The path reaches farther than the largest model allows.".to_owned(),
            "Choose a shorter path.",
        ),
        PathSweepError::StartsAlongProfile => context.error(
            format!(
                "The path leaves the profile of {} along the profile's own plane, so the sweep \
                 would be flat.",
                context.sketch_name
            ),
            "Draw the path so it starts across the profile's plane, in a sketch on a plane \
             crossing the profile's."
                .to_owned(),
            path.fix,
        ),
        PathSweepError::TurnsAlongProfile { segment } => context.error(
            format!(
                "Kept parallel to its sketch, the profile would slide along itself where {}{owner} \
                 turns along the profile's plane.",
                path.label(*segment)
            ),
            "Keep the profile square to the path, or choose a path that keeps rising out of the \
             profile's plane."
                .to_owned(),
            context.own(),
        ),
        PathSweepError::Corner { segments } => in_path(
            format!(
                "The path turns a sharp corner between {}{owner}, which a profile kept square to \
                 the path cannot follow.",
                path.labels(segments)
            ),
            "Round the corner with a fillet in the path, keep the profile parallel, or meet it \
             with straight lines on both sides so it is mitred.",
        ),
        PathSweepError::CornerTooSharp { segments } => in_path(
            format!(
                "The corner between {}{owner} is too sharp for the profile: its mitre would cut \
                 past the end of a line.",
                path.labels(segments)
            ),
            "Lengthen the lines at the corner, open the corner up, or make the profile smaller.",
        ),
        PathSweepError::ProfileTooLarge { segment } => context.error(
            format!(
                "The profile of {} is too large for the bend of {}{owner}: it would reach past the \
                 centre the path bends around.",
                context.sketch_name,
                path.label(*segment)
            ),
            "Make the profile smaller or the bend gentler.".to_owned(),
            context.in_sketch(),
        ),
        PathSweepError::ClosedPathMissesProfile => in_path(
            format!(
                "The closed path never crosses the plane of {}, so the profile is not on it.",
                context.sketch_name
            ),
            "Move the profile onto the path, or the path through the profile.",
        ),
        PathSweepError::ClosedPathTwists => in_path(
            "Following the closed path brings the profile back turned, so the sweep would not \
             close."
                .to_owned(),
            "Use a closed path lying in one plane, or an open path.",
        ),
        PathSweepError::TooIntricate { segment } => in_path(
            format!(
                "{}{owner} bends too intricately to sweep along.",
                capital(&path.label(*segment))
            ),
            "Simplify that curve.",
        ),
        PathSweepError::CrossesItself => context.error(
            format!("{name} runs into itself along its path."),
            "Make the profile smaller, or move the parts of the path farther apart.".to_owned(),
            context.own(),
        ),
    }
}

fn capital(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}
