use std::collections::BTreeSet;

use caditor_geometry::{Plane, Point2};
use caditor_kernel::{
    BooleanError, BooleanOperation, LINEAR_RESOLUTION, LinearExtent, Profile, ProfileCurve,
    ProfileError, Solid, SweepError, boolean, extrude,
};

use crate::{
    datum::{PlaneReference, Resolver},
    document::{Feature, FeatureId},
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    solid::SolidResult,
    trouble,
};

const HALF_SPACE_SIDES: [u64; 4] = [1, 2, 3, 4];
const HALF_SPACE_REACH: f64 = 0.05;
const HALF_SPACE_MARGIN: f64 = 1.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Split {
    pub body: FeatureId,
    pub plane: PlaneReference,
    pub flipped: bool,
}

impl Split {
    pub fn heap_size(&self) -> usize {
        self.plane.heap_size()
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = BTreeSet::from([self.body]);
        used.extend(self.plane.datum());
        used.extend(self.plane.body());
        used
    }
}

struct Context<'a> {
    resolver: Resolver<'a>,
    body_name: String,
}

impl Context<'_> {
    fn error(&self, reason: String, remedy: &str) -> Failure {
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy: remedy.to_owned(),
            fix: Some(FixTarget::Feature(self.resolver.feature.id())),
            constraints: Vec::new(),
            place: None,
        }))
    }

    fn misses(&self) -> Failure {
        self.error(
            format!(
                "The plane does not pass through the body of {}, so nothing lies on one of its \
                 sides.",
                self.body_name
            ),
            "Choose a plane that crosses the body.",
        )
    }

    fn unbuildable(&self, error: &dyn std::fmt::Display) -> Failure {
        log::warn!("{} could not be built: {error}", self.resolver.feature.name);
        self.error(
            format!(
                "The body of {} could not be split along the plane.",
                self.body_name
            ),
            "Move the plane slightly, so it does not run exactly along a face or edge.",
        )
    }

    fn boolean_failure(&self, solid: &Solid, half: &Solid, error: BooleanError) -> Failure {
        match error {
            BooleanError::Cancelled(_) => Failure::Cancelled,
            BooleanError::Empty => self.misses(),
            other => {
                let document = self.resolver.inputs.document;
                let trouble = trouble::boolean_trouble(
                    document,
                    [solid, half],
                    &other,
                    "Move the plane slightly",
                );
                log::warn!("{} could not be built: {other}", self.resolver.feature.name);
                self.error(
                    trouble.reason(format!(
                        "The body of {} could not be split along the plane.",
                        self.body_name
                    )),
                    &trouble.remedy,
                )
                .placed(trouble.place)
            }
        }
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Split,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let body_name = inputs
        .document
        .feature(definition.body)
        .map(|body| body.name.clone())
        .unwrap_or_default();
    let context = Context {
        resolver: Resolver { feature, inputs },
        body_name,
    };
    let plane = context.resolver.plane(&definition.plane)?;
    let Some(solid) = inputs.body(definition.body) else {
        return Err(Failure::Error(Box::new(FeatureError {
            reason: format!("The body made by {} has no shape.", context.body_name),
            remedy: format!("Fix {} first.", context.body_name),
            fix: Some(FixTarget::Feature(definition.body)),
            constraints: Vec::new(),
            place: None,
        })));
    };
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let half = half_space(&context, solid, &plane, definition.flipped, feature)?;
    let kept = boolean(solid, &half, BooleanOperation::Intersection)
        .map_err(|error| context.boolean_failure(solid, &half, error))?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let apart = boolean(solid, &half, BooleanOperation::Difference)
        .map_err(|error| context.boolean_failure(solid, &half, error))?;
    Ok(FeatureResult::Solid(
        SolidResult::new(definition.body, kept)
            .with_others([SolidResult::new(feature.id(), apart)]),
    ))
}

fn half_space(
    context: &Context<'_>,
    solid: &Solid,
    plane: &Plane,
    flipped: bool,
    feature: &Feature,
) -> Result<Solid, Failure> {
    half_space_solid(solid, plane, flipped, feature.id().raw()).map_err(|error| match error {
        HalfSpaceError::Misses => context.misses(),
        other => context.unbuildable(&other),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum HalfSpaceError {
    #[error("the body has no extent")]
    NoExtent,
    #[error("the plane does not pass through the body")]
    Misses,
    #[error("its outline could not be drawn: {0}")]
    Profile(ProfileError),
    #[error("it could not be extruded: {0}")]
    Sweep(SweepError),
}

pub(crate) fn half_space_solid(
    solid: &Solid,
    plane: &Plane,
    flipped: bool,
    feature: u64,
) -> Result<Solid, HalfSpaceError> {
    let bounds = solid.bounding_box().ok_or(HalfSpaceError::NoExtent)?;
    let margin = bounds.diagonal() * HALF_SPACE_REACH + HALF_SPACE_MARGIN;
    let local: Vec<[f64; 3]> = bounds
        .corners()
        .iter()
        .map(|corner| {
            let offset = *corner - plane.origin();
            [
                offset.dot(plane.x_axis()),
                offset.dot(plane.y_axis()),
                offset.dot(plane.normal()),
            ]
        })
        .collect();
    let least = |axis: usize| {
        local
            .iter()
            .filter_map(|corner| corner.get(axis).copied())
            .fold(f64::INFINITY, f64::min)
    };
    let most = |axis: usize| {
        local
            .iter()
            .filter_map(|corner| corner.get(axis).copied())
            .fold(f64::NEG_INFINITY, f64::max)
    };
    let (low, high) = (least(2), most(2));
    if high <= LINEAR_RESOLUTION || low >= -LINEAR_RESOLUTION {
        return Err(HalfSpaceError::Misses);
    }
    let corners = [
        Point2::new(least(0) - margin, least(1) - margin),
        Point2::new(most(0) + margin, least(1) - margin),
        Point2::new(most(0) + margin, most(1) + margin),
        Point2::new(least(0) - margin, most(1) + margin),
    ];
    let curves: Vec<ProfileCurve> = HALF_SPACE_SIDES
        .iter()
        .zip(corners.iter().zip(corners.iter().cycle().skip(1)))
        .map(|(side, (start, end))| ProfileCurve::line(*side, *start, *end))
        .collect();
    let profile = Profile::new(&curves).map_err(HalfSpaceError::Profile)?;
    let extent = if flipped {
        LinearExtent::new(low - margin, 0.0)
    } else {
        LinearExtent::new(0.0, high + margin)
    }
    .map_err(HalfSpaceError::Sweep)?;
    extrude(plane, profile.regions(), extent, feature).map_err(HalfSpaceError::Sweep)
}
