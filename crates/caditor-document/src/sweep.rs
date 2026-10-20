use std::collections::BTreeSet;

use caditor_expression::{Dimension, Expression, Unit};
use caditor_geometry::{Plane, Vector3};
use caditor_kernel::{
    EdgeNaming, EdgeReference, PathSegment, PathSweepError, ReferenceError, Region, Solid,
    SpiralSpec, SweepOrientation, SweepShape, sweep_shaped,
};
use caditor_sketch::EntityId;

use crate::{
    describe::describe_edge,
    document::{Feature, FeatureId, list_names},
    failure::Cause,
    origins,
    recompute::{Failure, FeatureError, FeatureResult, FixTarget, Inputs, MissingInput},
    solid::{BodyOperation, Context, RegionChoice, profile_curve, sweep_failure},
    sweep_cause::{Follower, Segments, SweepCause},
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

pub(crate) fn failed(cause: impl Into<Cause>, fix: FixTarget) -> Failure {
    Failure::from(FeatureError::new(cause).fixing(fix))
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
            SweepCause::PathUnavailable {
                sketch: sketch_name,
            },
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
                SweepCause::CurveGone {
                    sketch: sketch_name.clone(),
                    feature: feature.name.clone(),
                },
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
            let body = body_name.clone();
            let feature = feature.name.clone();
            failed(
                match error {
                    ReferenceError::Ambiguous { .. } => SweepCause::EdgeAmbiguous { body, feature },
                    ReferenceError::Missing => SweepCause::EdgeGone { body, feature },
                },
                own,
            )
        })?;
        let edge = solid.edge(id).ok_or_else(|| {
            failed(
                SweepCause::EdgeMissing {
                    body: body_name.clone(),
                },
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
            SweepCause::HelixUnavailable { helix: helix_name },
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

pub(crate) fn spiral_of(inputs: &Inputs<'_>, path: &SweepPath) -> Option<(String, SpiralSpec)> {
    let helix = path.helix()?;
    let FeatureResult::Helix(result) = inputs.features.get(&helix)?.as_ref() else {
        return None;
    };
    Some((feature_name(inputs, helix), result.spiral?))
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
        return Err(context.error(SweepCause::EndScale {
            feature: context.feature.name.clone(),
            scale,
        }));
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
    path_cause(feature, path, error, Follower::Feature)
}

fn path_cause(
    feature: &Feature,
    path: &Path,
    error: &PathSweepError,
    follower: Follower,
) -> Failure {
    let name = feature.name.clone();
    let owner = path.owner.clone();
    let own = FixTarget::Feature(feature.id());
    let at = |segments: &[usize]| Segments {
        labels: path.labels(segments),
        owner: owner.clone(),
    };
    let in_path = |cause: SweepCause| failed(cause, path.fix);
    match error {
        PathSweepError::Cancelled(_) => Failure::Cancelled,
        PathSweepError::NoPath => no_path(),
        PathSweepError::SegmentTooShort { segment } => in_path(SweepCause::ShortSegment {
            label: path.label(*segment),
            owner,
            follower,
        }),
        PathSweepError::Branches { segments } => in_path(SweepCause::Branches { at: at(segments) }),
        PathSweepError::Apart { segments } => in_path(SweepCause::Apart {
            at: at(segments),
            single: segments.len() == 1,
        }),
        PathSweepError::UnsupportedPath { segment } => in_path(SweepCause::Unsupported {
            name,
            label: path.label(*segment),
            owner,
        }),
        PathSweepError::TooLong => in_path(SweepCause::TooLong),
        PathSweepError::TooIntricate { segment } => in_path(SweepCause::Intricate {
            label: path.label(*segment),
            owner,
            follower,
        }),
        PathSweepError::Corner { segments } => in_path(SweepCause::Corner {
            at: at(segments),
            name,
            follower,
        }),
        PathSweepError::CornerTooSharp { segments } => in_path(SweepCause::CornerTooSharp {
            at: at(segments),
            name,
            follower,
        }),
        PathSweepError::ProfileTooLarge { segment } => in_path(SweepCause::ProfileTooLarge {
            name,
            label: path.label(*segment),
            owner,
            sketch: None,
        }),
        PathSweepError::ClosedPathTwists { .. } => in_path(SweepCause::ClosedTwists),
        PathSweepError::CrossesItself => failed(SweepCause::CrossesItself { name, follower }, own),
        PathSweepError::Sweep(_)
        | PathSweepError::StartsAlongProfile
        | PathSweepError::TurnsAlongProfile { .. }
        | PathSweepError::ClosedPathMissesProfile
        | PathSweepError::ClosedPathScales
        | PathSweepError::UnusableShape
        | PathSweepError::ShapedCorner { .. } => {
            log::warn!("{name} could not follow its path: {error}");
            failed(SweepCause::Unbuilt { name }, own)
        }
    }
}

fn failure(
    context: &Context<'_>,
    path: &Path,
    error: &PathSweepError,
    shape: SweepShape,
) -> Failure {
    let name = context.feature.name.clone();
    let owner = path.owner.clone();
    let at = |segments: &[usize]| Segments {
        labels: path.labels(segments),
        owner: owner.clone(),
    };
    let in_path = |cause: SweepCause| failed(cause, path.fix);
    match error {
        PathSweepError::Sweep(error) => sweep_failure(context, SWEEP_SHAPE, error),
        PathSweepError::SegmentTooShort { segment } => in_path(SweepCause::ShortSegment {
            label: path.label(*segment),
            owner,
            follower: Follower::Sweep,
        }),
        PathSweepError::StartsAlongProfile => failed(
            SweepCause::StartsAlongProfile {
                sketch: context.sketch_name.clone(),
            },
            path.fix,
        ),
        PathSweepError::TurnsAlongProfile { segment } => {
            context.error(SweepCause::TurnsAlongProfile {
                label: path.label(*segment),
                owner,
            })
        }
        PathSweepError::Corner { segments } => in_path(SweepCause::Corner {
            at: at(segments),
            name,
            follower: Follower::Sweep,
        }),
        PathSweepError::CornerTooSharp { segments } => in_path(SweepCause::CornerTooSharp {
            at: at(segments),
            name,
            follower: Follower::Sweep,
        }),
        PathSweepError::ProfileTooLarge { segment } => {
            context.error_in_sketch(SweepCause::ProfileTooLarge {
                name,
                label: path.label(*segment),
                owner,
                sketch: Some(context.sketch_name.clone()),
            })
        }
        PathSweepError::ClosedPathMissesProfile => in_path(SweepCause::ClosedMissesProfile {
            sketch: context.sketch_name.clone(),
        }),
        PathSweepError::ClosedPathTwists {
            turned: Some(turned),
        } => context.error(SweepCause::ClosedTwistsBy {
            name,
            turned: turned.to_degrees(),
            needed: (shape.twist - turned).to_degrees(),
        }),
        PathSweepError::ClosedPathTwists { turned: None } => in_path(SweepCause::ClosedTilted),
        PathSweepError::ClosedPathScales => context.error(SweepCause::ClosedScales { name }),
        PathSweepError::UnusableShape => context.error(SweepCause::UnusableShape { name }),
        PathSweepError::ShapedCorner { segments } => in_path(SweepCause::Corner {
            at: at(segments),
            name,
            follower: Follower::Shaped,
        }),
        PathSweepError::TooIntricate { segment } => in_path(SweepCause::Intricate {
            label: path.label(*segment),
            owner,
            follower: Follower::Sweep,
        }),
        PathSweepError::CrossesItself => context.error(SweepCause::CrossesItself {
            name,
            follower: Follower::Sweep,
        }),
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
