use std::{collections::BTreeSet, slice};

use caditor_kernel::{
    LoftError, LoftProfile, LoftSkin, MAX_LOFT_PROFILES, ProfileError, RegionReference, Selection,
    loft, resolve_regions,
};

use crate::{
    datum::{PointReference, Resolver},
    document::{Feature, FeatureId, list_names},
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
    pub operation: BodyOperation,
}

impl Loft {
    pub fn new(profiles: Vec<LoftSection>) -> Self {
        Self {
            profiles,
            end: None,
            skin: LoftSkin::Smooth,
            operation: BodyOperation::NewBody,
        }
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
            format!("It needs the profile drawn in {name}, which is not available."),
            format!("Fix {name} first, or leave it out of {}.", feature.name),
            FixTarget::Feature(sketch_id),
        ));
    };
    let context = solid::Context::of_sketch(feature, name.clone(), sketch_id, &sketch.geometry);
    let no_shape = || {
        failed(
            format!(
                "{name} has no closed shape for {} to pass through.",
                feature.name
            ),
            format!(
                "Close an outline in {name}, or leave it out of {}.",
                feature.name
            ),
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
            format!(
                "{name} holds {count} separate closed shapes, and {} takes one shape from each \
                 profile.",
                feature.name
            ),
            format!(
                "Choose one region of {name}: select it in the view and use Use selected under \
                 Profiles, or keep one shape in the sketch."
            ),
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
    let name = &feature.name;
    let own = FixTarget::Feature(feature.id());
    match error {
        LoftError::Cancelled(_) => Failure::Cancelled,
        LoftError::TooFewProfiles => Failure::Waiting(MissingInput::Profiles),
        LoftError::TooManyProfiles => failed(
            format!(
                "{name} passes through {} profiles, more than the {MAX_LOFT_PROFILES} a loft can \
                 take.",
                definition.profiles.len()
            ),
            "Leave some profiles out of the loft.".to_owned(),
            own,
        ),
        LoftError::Holes { profile } => failed(
            format!(
                "The shape of {} has a hole, and {name} passes only through outlines without \
                 holes.",
                row_name(inputs, definition, *profile)
            ),
            "Choose a region without holes, or cut the hole after the loft.".to_owned(),
            row_fix(feature, definition, *profile),
        ),
        LoftError::AlongProfile { profile } => failed(
            format!(
                "{} lies along the way {name} runs, so the loft cannot pass through it.",
                row_name(inputs, definition, *profile)
            ),
            "Place the profiles one after another, each across the way the loft runs, for \
             example on parallel planes."
                .to_owned(),
            row_fix(feature, definition, *profile),
        ),
        LoftError::Coincident { profiles } => {
            let named: Vec<String> = profiles
                .iter()
                .map(|row| row_name(inputs, definition, *row))
                .collect();
            failed(
                format!(
                    "{} lie in the same place, so {name} has no length between them.",
                    list_names(&named)
                ),
                "Move one of them away from the other, or leave one out of the loft.".to_owned(),
                own,
            )
        }
        LoftError::TooLarge => failed(
            format!("{name} reaches farther than caditor models."),
            "Make the profiles smaller or bring them closer to the origin.".to_owned(),
            own,
        ),
        LoftError::TooIntricate => failed(
            format!("The profiles of {name} have too many curves to loft through."),
            "Draw the profiles with fewer curves.".to_owned(),
            own,
        ),
        LoftError::CrossesItself => failed(
            format!("The sides of {name} run into each other between its profiles."),
            "Turn the profiles so their corners line up, space them further apart, or switch \
             between Smooth and Ruled."
                .to_owned(),
            own,
        ),
        LoftError::Geometry(_) | LoftError::Build(_) => {
            log::warn!("{name} could not be built: {error}");
            failed(
                format!("{name} cannot be built through these profiles."),
                "Change a profile slightly, or switch between Smooth and Ruled.".to_owned(),
                own,
            )
        }
    }
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
    let made = loft(&profiles, apex, definition.skin, feature.id().raw())
        .map_err(|error| failure(feature, inputs, definition, &error))?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let context = Context::new(feature, inputs, LOFT_SHAPE);
    combined(&context, made, definition.operation)
}
