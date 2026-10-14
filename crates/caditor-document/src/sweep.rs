use std::collections::BTreeSet;

use caditor_expression::{Dimension, Expression, Unit, format_decimals};
use caditor_geometry::{Plane, Vector3};
use caditor_kernel::{
    EdgeNaming, EdgeReference, PathSegment, PathSweepError, ReferenceError, Region, Solid,
    SweepOrientation, SweepShape, sweep_shaped,
};
use caditor_sketch::EntityId;

use crate::{
    describe::describe_edge,
    document::{Feature, FeatureId, list_names},
    origins,
    recompute::{Failure, FeatureError, FeatureResult, FixTarget, Inputs, MissingInput},
    solid::{BodyOperation, Context, RegionChoice, profile_curve, sweep_failure},
};

pub const SWEEP_SHAPE: &str = "sweep";
const TWIST_DECIMALS: usize = 3;

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
    Helix {
        helix: FeatureId,
    },
}

impl SweepPath {
    pub fn sketch(&self) -> Option<FeatureId> {
        match self {
            Self::Curves { sketch, .. } => Some(*sketch),
            Self::Unchosen | Self::Edges { .. } | Self::Helix { .. } => None,
        }
    }

    pub fn body(&self) -> Option<FeatureId> {
        match self {
            Self::Edges { body, .. } => Some(*body),
            Self::Unchosen | Self::Curves { .. } | Self::Helix { .. } => None,
        }
    }

    pub fn helix(&self) -> Option<FeatureId> {
        match self {
            Self::Helix { helix } => Some(*helix),
            Self::Unchosen | Self::Curves { .. } | Self::Edges { .. } => None,
        }
    }

    pub fn curves(&self) -> &[EntityId] {
        match self {
            Self::Curves { curves, .. } => curves,
            Self::Unchosen | Self::Edges { .. } | Self::Helix { .. } => &[],
        }
    }

    pub fn edges(&self) -> &[EdgeReference] {
        match self {
            Self::Edges { edges, .. } => edges,
            Self::Unchosen | Self::Curves { .. } | Self::Helix { .. } => &[],
        }
    }

    pub fn features(&self) -> impl Iterator<Item = FeatureId> {
        self.sketch()
            .into_iter()
            .chain(self.body())
            .chain(self.helix())
    }

    pub fn len(&self) -> usize {
        self.curves().len() + self.edges().len() + usize::from(self.helix().is_some())
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
            Self::Helix { helix } => Self::Helix {
                helix: rename(*helix),
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
    pub twist: Expression,
    pub scale: Expression,
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
            twist: untwisted(),
            scale: unscaled(),
            operation: BodyOperation::NewBody,
            other_bodies: Vec::new(),
        }
    }

    pub fn expressions(&self) -> Vec<&Expression> {
        vec![&self.twist, &self.scale]
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        vec![&mut self.twist, &mut self.scale]
    }

    pub fn is_shaped(&self) -> bool {
        !is_literally(&self.twist, 0.0) || !is_literally(&self.scale, 1.0)
    }
}

pub fn untwisted() -> Expression {
    Expression::measure(0.0, Unit::Degree)
}

pub fn unscaled() -> Expression {
    Expression::number(1.0)
}

fn is_literally(expression: &Expression, literal: f64) -> bool {
    matches!(
        expression,
        Expression::Number(value) | Expression::Measure(value, _) if *value == literal
    )
}

pub(crate) struct Path {
    pub(crate) segments: Vec<PathSegment>,
    labels: Vec<String>,
    pub(crate) fix: FixTarget,
    pub(crate) owner: String,
    pub(crate) up: Vector3,
}

impl Path {
    pub(crate) fn labels(&self, segments: &[usize]) -> String {
        let named: Vec<String> = segments
            .iter()
            .filter_map(|segment| self.labels.get(*segment).cloned())
            .collect();
        list_names(&named)
    }

    pub(crate) fn label(&self, segment: usize) -> String {
        self.labels.get(segment).cloned().unwrap_or_default()
    }
}

fn feature_name(inputs: &Inputs<'_>, feature: FeatureId) -> String {
    inputs.document.body_label(feature).unwrap_or_default()
}

fn no_path() -> Failure {
    Failure::Waiting(MissingInput::Path)
}

pub(crate) fn failed(reason: String, remedy: String, fix: FixTarget) -> Failure {
    Failure::Error(Box::new(FeatureError {
        reason,
        remedy,
        fix: Some(fix),
        constraints: Vec::new(),
        place: None,
    }))
}

fn curve_path(
    feature: &Feature,
    inputs: &Inputs<'_>,
    sketch_id: FeatureId,
    curves: &[EntityId],
) -> Result<Path, Failure> {
    let sketch_name = feature_name(inputs, sketch_id);
    let Some(FeatureResult::Sketch(sketch)) = inputs.features.get(&sketch_id).map(AsRef::as_ref)
    else {
        return Err(failed(
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
            failed(
                format!(
                    "A curve of the path is no longer a curve of {sketch_name}, so {} cannot \
                     follow it.",
                    feature.name
                ),
                "Choose the path again.".to_owned(),
                FixTarget::Feature(feature.id()),
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
        up: plane.normal(),
    })
}

fn edge_path(
    feature: &Feature,
    inputs: &Inputs<'_>,
    body: FeatureId,
    edges: &[EdgeReference],
) -> Result<Path, Failure> {
    let body_name = feature_name(inputs, body);
    let own = FixTarget::Feature(feature.id());
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
                    feature.name
                ),
                ReferenceError::Missing => format!(
                    "An edge of the path is no longer part of {body_name}, so {} cannot follow it.",
                    feature.name
                ),
            };
            failed(reason, "Choose the path again.".to_owned(), own)
        })?;
        let edge = solid.edge(id).ok_or_else(|| {
            failed(
                format!("An edge of the path is no longer part of {body_name}."),
                "Choose the path again.".to_owned(),
                own,
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
        fix: own,
        owner: format!(" of {body_name}"),
        up: Vector3::Z,
    })
}

fn helix_path(inputs: &Inputs<'_>, helix: FeatureId) -> Result<Path, Failure> {
    let helix_name = feature_name(inputs, helix);
    let Some(FeatureResult::Helix(result)) = inputs.features.get(&helix).map(AsRef::as_ref) else {
        return Err(failed(
            format!("It needs {helix_name} as its path, which is not available."),
            format!("Fix {helix_name} first, or choose another path."),
            FixTarget::Feature(helix),
        ));
    };
    let segments = result
        .turns
        .iter()
        .zip(1_u128..)
        .map(|(turn, key)| turn.path_segment(key))
        .collect();
    let labels = (1..=result.turns.len())
        .map(|turn| format!("turn {turn}"))
        .collect();
    Ok(Path {
        segments,
        labels,
        fix: FixTarget::Feature(helix),
        owner: format!(" of {helix_name}"),
        up: result.frame.normal(),
    })
}

pub(crate) fn path_of(
    feature: &Feature,
    inputs: &Inputs<'_>,
    path: &SweepPath,
) -> Result<Path, Failure> {
    let built = match path {
        SweepPath::Unchosen => return Err(no_path()),
        SweepPath::Curves { sketch, curves } => curve_path(feature, inputs, *sketch, curves)?,
        SweepPath::Edges { body, edges } => edge_path(feature, inputs, *body, edges)?,
        SweepPath::Helix { helix } => helix_path(inputs, *helix)?,
    };
    if built.segments.is_empty() {
        return Err(no_path());
    }
    Ok(built)
}

fn shape_of(
    context: &Context<'_>,
    inputs: &Inputs<'_>,
    definition: &Sweep,
) -> Result<SweepShape, Failure> {
    let twist = context.value(
        &definition.twist,
        Dimension::ANGLE,
        "twist",
        inputs.parameters,
    )?;
    let scale = context.value(
        &definition.scale,
        Dimension::NONE,
        "end scale",
        inputs.parameters,
    )?;
    if scale <= 0.0 {
        return Err(context.error(
            format!(
                "The end scale of {} must be more than zero, and {} is not.",
                context.feature.name,
                format_decimals(scale, TWIST_DECIMALS)
            ),
            "Enter an end scale above zero, such as 0.5 to end at half the size or 1 to keep \
             it."
            .to_owned(),
            context.own(),
        ));
    }
    Ok(SweepShape {
        twist: twist.to_radians(),
        scale,
    })
}

pub(crate) fn swept(
    context: &Context<'_>,
    inputs: &Inputs<'_>,
    (plane, regions): (&Plane, &[Region]),
    definition: &Sweep,
) -> Result<Solid, Failure> {
    let shape = shape_of(context, inputs, definition)?;
    let path = path_of(context.feature, inputs, &definition.path)?;
    sweep_shaped(
        plane,
        regions,
        &path.segments,
        (definition.orientation, shape),
        context.feature.id().raw(),
    )
    .map_err(|error| failure(context, &path, &error, shape))
}

pub(crate) fn path_failure(feature: &Feature, path: &Path, error: &PathSweepError) -> Failure {
    let name = &feature.name;
    let owner = &path.owner;
    let own = FixTarget::Feature(feature.id());
    let in_path = |reason: String, remedy: &str| failed(reason, remedy.to_owned(), path.fix);
    match error {
        PathSweepError::Cancelled(_) => Failure::Cancelled,
        PathSweepError::NoPath => no_path(),
        PathSweepError::SegmentTooShort { segment } => in_path(
            format!(
                "{}{owner} is too short to follow.",
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
        PathSweepError::TooIntricate { segment } => in_path(
            format!(
                "{}{owner} bends too intricately to follow.",
                capital(&path.label(*segment))
            ),
            "Simplify that curve.",
        ),
        PathSweepError::Corner { segments } => in_path(
            format!(
                "The path turns a sharp corner between {}{owner}, which {name} cannot follow.",
                path.labels(segments)
            ),
            "Round the corner with a fillet in the path, or meet it with straight lines on both \
             sides.",
        ),
        PathSweepError::CornerTooSharp { segments } => in_path(
            format!(
                "The corner between {}{owner} turns back too sharply for {name} to follow.",
                path.labels(segments)
            ),
            "Open the corner up, or lengthen the lines at it.",
        ),
        PathSweepError::ProfileTooLarge { segment } => in_path(
            format!(
                "{name} is too large for the bend of {}{owner}: it would reach past the centre \
                 the path bends around.",
                path.label(*segment)
            ),
            "Make it smaller or the bend gentler.",
        ),
        PathSweepError::ClosedPathTwists { .. } => in_path(
            "Following the closed path brings the section back turned, so it would not close."
                .to_owned(),
            "Use a closed path lying in one plane, or an open path.",
        ),
        PathSweepError::CrossesItself => failed(
            format!("{name} runs into itself along its path."),
            "Make it smaller, or move the parts of the path farther apart.".to_owned(),
            own,
        ),
        PathSweepError::Sweep(_)
        | PathSweepError::StartsAlongProfile
        | PathSweepError::TurnsAlongProfile { .. }
        | PathSweepError::ClosedPathMissesProfile
        | PathSweepError::ClosedPathScales
        | PathSweepError::UnusableShape
        | PathSweepError::ShapedCorner { .. } => {
            log::warn!("{name} could not follow its path: {error}");
            failed(
                format!("{name} cannot be built along this path."),
                "Choose another path or change its sizes.".to_owned(),
                own,
            )
        }
    }
}

fn failure(
    context: &Context<'_>,
    path: &Path,
    error: &PathSweepError,
    shape: SweepShape,
) -> Failure {
    let name = &context.feature.name;
    let owner = &path.owner;
    let in_path = |reason: String, remedy: &str| context.error(reason, remedy.to_owned(), path.fix);
    match error {
        PathSweepError::Sweep(error) => sweep_failure(context, SWEEP_SHAPE, error),
        PathSweepError::SegmentTooShort { segment } => in_path(
            format!(
                "{}{owner} is too short to sweep along.",
                capital(&path.label(*segment))
            ),
            "Lengthen it, or leave it out of the path.",
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
        PathSweepError::ClosedPathTwists {
            turned: Some(turned),
        } => {
            let needed = (shape.twist - turned).to_degrees();
            context.error(
                format!(
                    "Following the closed path brings the profile back turned by {}°, so {name} \
                     would not close.",
                    format_decimals(turned.to_degrees(), TWIST_DECIMALS)
                ),
                format!(
                    "Set the twist to {}°, or that plus or minus whole turns, so the profile comes \
                     back where it started.",
                    format_decimals(needed, TWIST_DECIMALS)
                ),
                context.own(),
            )
        }
        PathSweepError::ClosedPathTwists { turned: None } => in_path(
            "Following the closed path brings the profile back tilted, so the sweep would not \
             close."
                .to_owned(),
            "Draw the profile square to the path where they meet, use a closed path lying in one \
             plane, or an open path.",
        ),
        PathSweepError::ClosedPathScales => context.error(
            format!("{name} follows a closed path, so its end is its start and cannot be scaled."),
            "Set the end scale to 1, or choose an open path.".to_owned(),
            context.own(),
        ),
        PathSweepError::UnusableShape => context.error(
            format!("The twist or end scale of {name} is out of range."),
            "Enter an end scale nearer 1.".to_owned(),
            context.own(),
        ),
        PathSweepError::ShapedCorner { segments } => in_path(
            format!(
                "The path turns a sharp corner between {}{owner}, which a twisted or scaled sweep \
                 cannot mitre.",
                path.labels(segments)
            ),
            "Round the corner with a fillet in the path, or set the twist to 0° and the end \
             scale to 1.",
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
        PathSweepError::Cancelled(_)
        | PathSweepError::NoPath
        | PathSweepError::Branches { .. }
        | PathSweepError::Apart { .. }
        | PathSweepError::UnsupportedPath { .. }
        | PathSweepError::TooLong => path_failure(context.feature, path, error),
    }
}

pub(crate) fn capital(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}
