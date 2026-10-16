use std::{collections::BTreeSet, slice};

use caditor_expression::{Dimension, Expression, ParameterId};
use caditor_kernel::{
    LoftEnds, LoftError, LoftProfile, LoftSkin, MAX_LOFT_PROFILES, ProfileError, RegionReference,
    Selection, loft_through, resolve_regions,
};

use crate::{
    datum::{PointReference, Resolver},
    document::{Feature, FeatureId, list_names},
    failure::Cause,
    primitive::{Context, combined},
    recompute::{CancelToken, Failure, FeatureResult, FixTarget, Inputs, MissingInput},
    solid::{self, BodyOperation, profile_failure},
    sweep::failed,
};

pub const LOFT_SHAPE: &str = "loft";

#[derive(Debug, Clone, PartialEq)]
pub struct LoftSection {
    pub sketch: FeatureId,
    pub region: Option<RegionReference>,
}

impl LoftSection {
    pub fn whole(sketch: FeatureId) -> Self {
        Self {
            sketch,
            region: None,
        }
    }

    pub fn heap_size(&self) -> usize {
        self.region.as_ref().map_or(0, |region| {
            size_of::<RegionReference>() + region.heap_size()
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Loft {
    pub profiles: Vec<LoftSection>,
    pub end: Option<PointReference>,
    pub skin: LoftSkin,
    pub closed: bool,
    pub square_start: Option<Expression>,
    pub square_end: Option<Expression>,
    pub operation: BodyOperation,
}

impl Loft {
    pub fn new(profiles: Vec<LoftSection>) -> Self {
        Self {
            profiles,
            end: None,
            skin: LoftSkin::Smooth,
            closed: false,
            square_start: None,
            square_end: None,
            operation: BodyOperation::NewBody,
        }
    }

    pub fn expressions(&self) -> Vec<&Expression> {
        self.square_start
            .iter()
            .chain(self.square_end.iter())
            .collect()
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        self.square_start
            .iter_mut()
            .chain(self.square_end.iter_mut())
            .collect()
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.expressions()
            .into_iter()
            .flat_map(Expression::parameters)
            .collect()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.expressions()
            .into_iter()
            .any(|expression| expression.uses(parameter))
    }

    pub fn has_ends_shaped(&self) -> bool {
        self.closed || self.square_start.is_some() || self.square_end.is_some()
    }

    pub fn sketches(&self) -> impl Iterator<Item = FeatureId> + '_ {
        self.profiles.iter().map(|section| section.sketch)
    }

    pub fn reference_sketches(&self) -> BTreeSet<FeatureId> {
        self.sketches()
            .chain(self.end.as_ref().and_then(PointReference::sketch))
            .collect()
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let end = self.end.as_ref();
        self.operation
            .target()
            .into_iter()
            .chain(self.reference_sketches())
            .chain(end.and_then(PointReference::datum))
            .chain(end.and_then(PointReference::frame))
            .chain(end.and_then(PointReference::body))
            .collect()
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.end
            .as_ref()
            .map(PointReference::origin_features)
            .unwrap_or_default()
    }

    pub fn heap_size(&self) -> usize {
        size_of_val(self.profiles.as_slice())
            + self
                .profiles
                .iter()
                .map(LoftSection::heap_size)
                .sum::<usize>()
            + self.end.as_ref().map_or(0, PointReference::heap_size)
            + self
                .expressions()
                .into_iter()
                .map(Expression::heap_size)
                .sum::<usize>()
    }

    pub fn rows(&self) -> usize {
        self.profiles.len() + usize::from(self.end.is_some())
    }

    #[must_use]
    pub fn with_features_renamed(&self, rename: &dyn Fn(FeatureId) -> FeatureId) -> Self {
        Self {
            profiles: self
                .profiles
                .iter()
                .map(|section| LoftSection {
                    sketch: rename(section.sketch),
                    region: section.region.clone(),
                })
                .collect(),
            ..self.clone()
        }
    }
}

pub fn skin_name(skin: LoftSkin) -> &'static str {
    match skin {
        LoftSkin::Smooth => "Smooth",
        LoftSkin::Ruled => "Ruled",
    }
}

fn sketch_name(inputs: &Inputs<'_>, sketch: FeatureId) -> String {
    inputs.document.body_label(sketch).unwrap_or_default()
}

fn profile_of(
    feature: &Feature,
    inputs: &Inputs<'_>,
    section: &LoftSection,
) -> Result<LoftProfile, Failure> {
    let sketch_id = section.sketch;
    let name = sketch_name(inputs, sketch_id);
    let Some(FeatureResult::Sketch(sketch)) = inputs.features.get(&sketch_id).map(AsRef::as_ref)
    else {
        return Err(failed(
            LoftCause::ProfileUnavailable {
                sketch: name,
                feature: feature.name.clone(),
            },
            FixTarget::Feature(sketch_id),
        ));
    };
    let context = solid::Context::of_sketch(feature, name.clone(), sketch_id, &sketch.geometry);
    let no_shape = || {
        failed(
            LoftCause::NoShape {
                sketch: name.clone(),
                feature: feature.name.clone(),
            },
            FixTarget::Feature(sketch_id),
        )
    };
    let wording = |error: ProfileError| match error {
        ProfileError::EmptySelection => no_shape(),
        other => profile_failure(&context, &other),
    };
    let profile = sketch.profile().map_err(wording)?;
    let selection = match &section.region {
        None => Selection::EvenDepth,
        Some(reference) => Selection::Regions(
            resolve_regions(slice::from_ref(reference), profile.regions())
                .map_err(wording)?
                .keys,
        ),
    };
    let mut regions = profile.select(&selection).map_err(wording)?;
    match regions.len() {
        0 => Err(no_shape()),
        1 => Ok(LoftProfile {
            plane: sketch.geometry.plane(),
            region: regions.remove(0),
        }),
        count => Err(failed(
            LoftCause::SeveralShapes {
                sketch: name,
                count,
                feature: feature.name.clone(),
            },
            FixTarget::Feature(feature.id()),
        )),
    }
}

fn row_name(inputs: &Inputs<'_>, definition: &Loft, row: usize) -> String {
    definition.profiles.get(row).map_or_else(
        || "the end point".to_owned(),
        |section| sketch_name(inputs, section.sketch),
    )
}

fn row_fix(feature: &Feature, definition: &Loft, row: usize) -> FixTarget {
    FixTarget::Feature(
        definition
            .profiles
            .get(row)
            .map_or(feature.id(), |section| section.sketch),
    )
}

fn failure(
    feature: &Feature,
    inputs: &Inputs<'_>,
    definition: &Loft,
    error: &LoftError,
) -> Failure {
    let name = feature.name.clone();
    let own = FixTarget::Feature(feature.id());
    match error {
        LoftError::Cancelled(_) => Failure::Cancelled,
        LoftError::TooFewProfiles => Failure::Waiting(MissingInput::Profiles),
        LoftError::TooManyProfiles => failed(
            LoftCause::TooManyProfiles {
                feature: name,
                count: definition.profiles.len(),
            },
            own,
        ),
        LoftError::Holes { profile } => failed(
            LoftCause::Holes {
                row: row_name(inputs, definition, *profile),
                feature: name,
            },
            row_fix(feature, definition, *profile),
        ),
        LoftError::AlongProfile { profile } => failed(
            LoftCause::AlongProfile {
                row: row_name(inputs, definition, *profile),
                feature: name,
            },
            row_fix(feature, definition, *profile),
        ),
        LoftError::Coincident { profiles } => failed(
            LoftCause::Coincident {
                rows: profiles
                    .iter()
                    .map(|row| row_name(inputs, definition, *row))
                    .collect(),
                feature: name,
            },
            own,
        ),
        LoftError::TooLarge => failed(LoftCause::TooLarge { feature: name }, own),
        LoftError::TooIntricate => failed(LoftCause::TooIntricate { feature: name }, own),
        LoftError::CrossesItself => failed(LoftCause::CrossesItself { feature: name }, own),
        LoftError::ClosedWithApex => failed(LoftCause::ClosedWithApex { feature: name }, own),
        LoftError::TooFewToClose => failed(LoftCause::TooFewToClose { feature: name }, own),
        LoftError::SquareAtApex => failed(LoftCause::SquareAtApex { feature: name }, own),
        LoftError::SquareOnRuled => failed(LoftCause::SquareOnRuled { feature: name }, own),
        LoftError::SquareOnClosed => failed(LoftCause::SquareOnClosed { feature: name }, own),
        LoftError::UnusableWeight => weightless(feature),
        LoftError::Geometry(_) | LoftError::Build(_) => {
            log::warn!("{name} could not be built: {error}");
            failed(LoftCause::Unbuilt { feature: name }, own)
        }
    }
}

fn weightless(feature: &Feature) -> Failure {
    failed(
        LoftCause::Weightless {
            feature: feature.name.clone(),
        },
        FixTarget::Feature(feature.id()),
    )
}

fn weight(
    context: &Context<'_>,
    feature: &Feature,
    expression: Option<&Expression>,
    what: &str,
) -> Result<Option<f64>, Failure> {
    let Some(expression) = expression else {
        return Ok(None);
    };
    let value = context.value_as(expression, what, Dimension::NONE)?;
    if value <= 0.0 || !value.is_finite() {
        return Err(weightless(feature));
    }
    Ok(Some(value))
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Loft,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    if definition.profiles.is_empty() || definition.rows() < 2 {
        return Err(Failure::Waiting(MissingInput::Profiles));
    }
    if definition.profiles.len() > MAX_LOFT_PROFILES {
        return Err(failure(
            feature,
            inputs,
            definition,
            &LoftError::TooManyProfiles,
        ));
    }
    let mut profiles = Vec::with_capacity(definition.profiles.len());
    for section in &definition.profiles {
        profiles.push(profile_of(feature, inputs, section)?);
        if cancel.is_cancelled() {
            return Err(Failure::Cancelled);
        }
    }
    let apex = definition
        .end
        .as_ref()
        .map(|end| Resolver { feature, inputs }.point(end))
        .transpose()?;
    let context = Context::new(feature, inputs, LOFT_SHAPE);
    let ends = LoftEnds {
        closed: definition.closed,
        start: weight(
            &context,
            feature,
            definition.square_start.as_ref(),
            "start weight",
        )?,
        end: weight(
            &context,
            feature,
            definition.square_end.as_ref(),
            "end weight",
        )?,
    };
    let made = loft_through(&profiles, apex, (definition.skin, ends), feature.id().raw())
        .map_err(|error| failure(feature, inputs, definition, &error))?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    combined(&context, made, definition.operation)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LoftCause {
    ProfileUnavailable {
        sketch: String,
        feature: String,
    },
    NoShape {
        sketch: String,
        feature: String,
    },
    SeveralShapes {
        sketch: String,
        count: usize,
        feature: String,
    },
    TooManyProfiles {
        feature: String,
        count: usize,
    },
    Holes {
        row: String,
        feature: String,
    },
    AlongProfile {
        row: String,
        feature: String,
    },
    Coincident {
        rows: Vec<String>,
        feature: String,
    },
    TooLarge {
        feature: String,
    },
    TooIntricate {
        feature: String,
    },
    CrossesItself {
        feature: String,
    },
    ClosedWithApex {
        feature: String,
    },
    TooFewToClose {
        feature: String,
    },
    SquareAtApex {
        feature: String,
    },
    SquareOnRuled {
        feature: String,
    },
    SquareOnClosed {
        feature: String,
    },
    Weightless {
        feature: String,
    },
    Unbuilt {
        feature: String,
    },
}

impl LoftCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::ProfileUnavailable { sketch, .. } => {
                format!("It needs the profile drawn in {sketch}, which is not available.")
            }
            Self::NoShape { sketch, feature } => {
                format!("{sketch} has no closed shape for {feature} to pass through.")
            }
            Self::SeveralShapes {
                sketch,
                count,
                feature,
            } => format!(
                "{sketch} holds {count} separate closed shapes, and {feature} takes one shape \
                 from each profile."
            ),
            Self::TooManyProfiles { feature, count } => format!(
                "{feature} passes through {count} profiles, more than the {MAX_LOFT_PROFILES} a \
                 loft can take."
            ),
            Self::Holes { row, feature } => format!(
                "The shape of {row} has a hole, and {feature} passes only through outlines \
                 without holes."
            ),
            Self::AlongProfile { row, feature } => format!(
                "{row} lies along the way {feature} runs, so the loft cannot pass through it."
            ),
            Self::Coincident { rows, feature } => format!(
                "{} lie in the same place, so {feature} has no length between them.",
                list_names(rows)
            ),
            Self::TooLarge { feature } => format!("{feature} reaches farther than caditor models."),
            Self::TooIntricate { feature } => {
                format!("The profiles of {feature} have too many curves to loft through.")
            }
            Self::CrossesItself { feature } => {
                format!("The sides of {feature} run into each other between its profiles.")
            }
            Self::ClosedWithApex { feature } => format!(
                "{feature} closes back to its first profile, so it cannot also end at a point."
            ),
            Self::TooFewToClose { feature } => format!(
                "{feature} closes back to its first profile, which takes at least three \
                 profiles."
            ),
            Self::SquareAtApex { feature } => {
                format!("{feature} ends at a point, which it cannot reach square to a profile.")
            }
            Self::SquareOnRuled { feature } => format!(
                "A ruled loft runs straight between its profiles, so {feature} cannot leave its \
                 ends square."
            ),
            Self::SquareOnClosed { feature } => format!(
                "{feature} closes back to its first profile, so it has no ends to leave square."
            ),
            Self::Weightless { feature } => {
                format!("The weight of a square end of {feature} must be more than zero.")
            }
            Self::Unbuilt { feature } => {
                format!("{feature} cannot be built through these profiles.")
            }
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::ProfileUnavailable { sketch, feature } => {
                format!("Fix {sketch} first, or leave it out of {feature}.")
            }
            Self::NoShape { sketch, feature } => {
                format!("Close an outline in {sketch}, or leave it out of {feature}.")
            }
            Self::SeveralShapes { sketch, .. } => format!(
                "Choose one region of {sketch}: select it in the view and use Use selected under \
                 Profiles, or keep one shape in the sketch."
            ),
            Self::TooManyProfiles { .. } => "Leave some profiles out of the loft.".to_owned(),
            Self::Holes { .. } => {
                "Choose a region without holes, or cut the hole after the loft.".to_owned()
            }
            Self::AlongProfile { .. } => {
                "Place the profiles one after another, each across the way the loft runs, for \
                 example on parallel planes."
                    .to_owned()
            }
            Self::Coincident { .. } => {
                "Move one of them away from the other, or leave one out of the loft.".to_owned()
            }
            Self::TooLarge { .. } => {
                "Make the profiles smaller or bring them closer to the origin.".to_owned()
            }
            Self::TooIntricate { .. } => "Draw the profiles with fewer curves.".to_owned(),
            Self::CrossesItself { .. } => {
                "Turn the profiles so their corners line up, space them further apart, or switch \
                 between Smooth and Ruled."
                    .to_owned()
            }
            Self::ClosedWithApex { .. } => {
                "Leave out the end point, or stop closing the loft.".to_owned()
            }
            Self::TooFewToClose { .. } => {
                "Add another profile, or stop closing the loft.".to_owned()
            }
            Self::SquareAtApex { .. } => "Set End to Free, or end at the last profile.".to_owned(),
            Self::SquareOnRuled { .. } => {
                "Switch Sides to Smooth, or set Start and End to Free.".to_owned()
            }
            Self::SquareOnClosed { .. } => "Set Start and End to Free.".to_owned(),
            Self::Weightless { .. } => {
                "Enter a weight above zero, such as 1 for a gentle turn or 2 to keep square \
                 longer."
                    .to_owned()
            }
            Self::Unbuilt { .. } => {
                "Change a profile slightly, or switch between Smooth and Ruled.".to_owned()
            }
        }
    }
}

impl From<LoftCause> for Cause {
    fn from(cause: LoftCause) -> Self {
        Self::Loft(cause)
    }
}
