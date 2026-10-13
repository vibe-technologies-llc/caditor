use std::collections::{BTreeMap, BTreeSet};

use caditor_expression::{Dimension, Expression, ParameterId, Quantity, Unit};

use crate::{
    attachment::SketchAttachment,
    body_appearance::BodyAppearance,
    datum::{
        AxisReference, CurveStation, Datum, DatumAxis, FaceTangent, PlaneReference, PlaneThrough,
        PointBy, PointReference,
    },
    document::{Document, Feature, FeatureId, FeatureKind},
    edit::{EditError, Transaction},
    hole::HoleDepth,
    inlining::expressions_mut,
    mate::{AngleSides, AxisMate, MatePair, PointTarget},
    measurement::MeasuredItem,
    movement::TurnCentre,
    paste_picks,
    pattern::PatternKind,
    projection::ProjectionSource,
    reference_image::ImagePlane,
    solid::{BodyOperation, ExtrudeEnd, RevolveAxis, RevolveExtent, SolidFeature, SolidStart},
    split::SplitAlong,
    values::ParameterValues,
};

const PLACEHOLDER_BASE: u64 = u64::MAX / 2;
const COPY_SUFFIX: &str = "copy";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteOrigin {
    ThisDocument,
    Elsewhere,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CarriedParameter {
    pub name: String,
    pub value: Option<Quantity>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CarriedParameters {
    carried: BTreeMap<ParameterId, CarriedParameter>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CarryError {
    #[error(
        "it uses the parameter {name}, which this model does not have, and its value could not be \
         copied"
    )]
    Unknown { name: String },
    #[error("it uses a parameter that was not copied with it")]
    Uncopied,
    #[error("its expression grows too long once the copied values are written into it")]
    TooLong,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PasteRefusal {
    #[error(
        "it picks faces or edges of {name}, which was copied with it, and a copy cannot name them"
    )]
    CopiedGeometry { name: String },
    #[error("its {what} has no match on the copy of {name}")]
    PickNotFound { what: String, name: String },
    #[error("its {what} matches more than one place on the copy of {name}")]
    PickAmbiguous { what: String, name: String },
    #[error(
        "the faces or edges it picks on {name} could not be found as they were when copied; copy \
         them again once they compute"
    )]
    PicksUnreadable { name: String },
    #[error("the copy of {name} could not be computed, so what it picks on it could not be found")]
    CopyUncomputed { name: String },
    #[error("it refers to {name}, which was copied with it, in a way a copy cannot follow")]
    Unfollowed { name: String },
    #[error("it uses {name}, which could not be pasted")]
    LeftOutInput { name: String },
    #[error("it uses a feature that was not copied with it and is not in this model")]
    OutsideFeature,
    #[error("it uses a feature that is no longer in the model or is rolled back")]
    MissingFeature,
    #[error(transparent)]
    Parameter(#[from] CarryError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LeftOut {
    pub name: String,
    pub reason: PasteRefusal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeaturePaste {
    pub transaction: Transaction,
    pub pasted: Vec<FeatureId>,
    pub left_out: Vec<LeftOut>,
    pub inlined: usize,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PasteError {
    #[error("nothing was copied")]
    NothingCopied,
    #[error("no copied feature could be pasted")]
    NothingPasted { left_out: Vec<LeftOut> },
    #[error("the paste was cancelled")]
    Cancelled,
    #[error(transparent)]
    Edit(#[from] EditError),
}

impl CarriedParameters {
    pub fn of(
        document: &Document,
        values: &ParameterValues,
        ids: impl IntoIterator<Item = ParameterId>,
    ) -> Self {
        let mut carried = Self::default();
        for id in ids {
            if let Some(name) = document.parameter_name(id) {
                carried.insert(
                    id,
                    CarriedParameter {
                        name: name.to_owned(),
                        value: values.value(id).ok(),
                    },
                );
            }
        }
        carried
    }

    pub fn insert(&mut self, id: ParameterId, parameter: CarriedParameter) {
        self.carried.insert(id, parameter);
    }

    pub fn iter(&self) -> impl Iterator<Item = (ParameterId, &CarriedParameter)> {
        self.carried.iter().map(|(id, parameter)| (*id, parameter))
    }

    pub fn is_empty(&self) -> bool {
        self.carried.is_empty()
    }

    pub fn carry(
        &self,
        expression: &Expression,
        target: &Document,
        origin: PasteOrigin,
    ) -> Result<Carried, CarryError> {
        let mut replacements = BTreeMap::new();
        let mut inlined = 0;
        for id in expression.parameters() {
            if origin == PasteOrigin::ThisDocument && target.parameter(id).is_some() {
                continue;
            }
            let carried = self.carried.get(&id).ok_or(CarryError::Uncopied)?;
            let replacement = match target.parameter_named(&carried.name) {
                Some(same_name) => Expression::Parameter(same_name.id()),
                None => {
                    inlined += 1;
                    carried
                        .value
                        .and_then(literal)
                        .ok_or_else(|| CarryError::Unknown {
                            name: carried.name.clone(),
                        })?
                }
            };
            replacements.insert(id, replacement);
        }
        if replacements.is_empty() {
            return Ok(Carried {
                expression: expression.clone(),
                inlined,
            });
        }
        let expression = expression
            .substituting(&|id| replacements.get(&id).cloned())
            .map_err(|_| CarryError::TooLong)?;
        Ok(Carried {
            expression,
            inlined,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Carried {
    pub expression: Expression,
    pub inlined: usize,
}

pub fn literal(value: Quantity) -> Option<Expression> {
    match value.dimension {
        Dimension::NONE => Some(Expression::number(value.value)),
        Dimension::LENGTH => Some(Expression::measure(value.value, Unit::Millimetre)),
        Dimension::ANGLE => Some(Expression::measure(value.value, Unit::Degree)),
        _ => None,
    }
}

pub fn feature_parameters(feature: &Feature) -> BTreeSet<ParameterId> {
    let mut kind = feature.kind.clone();
    let mut used: BTreeSet<ParameterId> = expressions_mut(&mut kind)
        .into_iter()
        .flat_map(|expression| expression.parameters())
        .collect();
    if let FeatureKind::Sketch(sketch) = &feature.kind {
        used.extend(
            sketch
                .sketch
                .constraints()
                .filter_map(|(_, constraint)| constraint.dimension())
                .flat_map(Expression::parameters),
        );
    }
    used.extend(
        feature
            .appearance
            .density
            .iter()
            .flat_map(Expression::parameters),
    );
    used
}

pub(crate) struct Copying<'a> {
    pub(crate) names: &'a BTreeMap<FeatureId, &'a str>,
    pub(crate) renamed: &'a BTreeMap<FeatureId, FeatureId>,
    pub(crate) deep: &'a [FeatureId],
}

impl Copying<'_> {
    pub(crate) fn name_of(&self, feature: FeatureId) -> String {
        self.names
            .get(&feature)
            .copied()
            .unwrap_or_default()
            .to_owned()
    }

    pub(crate) fn first_deep_name(&self) -> String {
        self.deep
            .first()
            .map(|deep| self.name_of(*deep))
            .unwrap_or_default()
    }
}

pub(crate) enum PickFailure {
    Refused(PasteRefusal),
    Cancelled,
}

pub(crate) trait PickFinder {
    fn find(
        &mut self,
        target: &Document,
        so_far: &Transaction,
        feature: &Feature,
        kind: &mut FeatureKind,
        copying: &Copying<'_>,
    ) -> Result<(), PickFailure>;
}

struct Unsought;

impl PickFinder for Unsought {
    fn find(
        &mut self,
        _target: &Document,
        _so_far: &Transaction,
        _feature: &Feature,
        _kind: &mut FeatureKind,
        copying: &Copying<'_>,
    ) -> Result<(), PickFailure> {
        Err(PickFailure::Refused(PasteRefusal::CopiedGeometry {
            name: copying.first_deep_name(),
        }))
    }
}

struct Pasted {
    kind: FeatureKind,
    appearance: BodyAppearance,
    inlined: usize,
    deep: Vec<FeatureId>,
}

impl Document {
    pub fn paste_features(
        &self,
        features: &[Feature],
        parameters: &CarriedParameters,
        origin: PasteOrigin,
    ) -> Result<FeaturePaste, PasteError> {
        self.plan_paste(features, parameters, origin, &mut Unsought)
    }

    pub(crate) fn plan_paste(
        &self,
        features: &[Feature],
        parameters: &CarriedParameters,
        origin: PasteOrigin,
        finder: &mut dyn PickFinder,
    ) -> Result<FeaturePaste, PasteError> {
        if features.is_empty() {
            return Err(PasteError::NothingCopied);
        }
        let copied: BTreeMap<FeatureId, &str> = features
            .iter()
            .map(|feature| (feature.id(), feature.name.as_str()))
            .collect();
        let label = match features {
            [only] => format!("Paste {}", only.name),
            _ => format!("Paste {} features", features.len()),
        };
        let mut builder = self.transaction(label.clone());
        let mut renamed: BTreeMap<FeatureId, FeatureId> = BTreeMap::new();
        let mut names: BTreeSet<String> = self
            .features()
            .map(|feature| feature.name.clone())
            .collect();
        let mut pasted = Vec::new();
        let mut left_out = Vec::new();
        let mut inlined = 0;
        for feature in features {
            let refused = |reason| LeftOut {
                name: feature.name.clone(),
                reason,
            };
            let Pasted {
                mut kind,
                appearance,
                inlined: carried,
                deep,
            } = match self.pasted_kind(feature, &copied, &renamed, parameters, origin) {
                Ok(found) => found,
                Err(reason) => {
                    left_out.push(refused(reason));
                    continue;
                }
            };
            if !deep.is_empty() {
                let so_far = Transaction::new(label.clone(), builder.edits.clone());
                let copying = Copying {
                    names: &copied,
                    renamed: &renamed,
                    deep: &deep,
                };
                match finder.find(self, &so_far, feature, &mut kind, &copying) {
                    Ok(()) => {}
                    Err(PickFailure::Refused(reason)) => {
                        left_out.push(refused(reason));
                        continue;
                    }
                    Err(PickFailure::Cancelled) => return Err(PasteError::Cancelled),
                }
            }
            let name = copy_name(&feature.name, &names);
            names.insert(name.clone());
            let mut copy = Feature::new(feature.id(), name, kind);
            copy.hidden = feature.hidden;
            copy.suppressed = feature.suppressed;
            copy.appearance = appearance;
            copy.group.clone_from(&feature.group);
            let id = builder.add_copied_feature(copy);
            renamed.insert(feature.id(), id);
            pasted.push(id);
            inlined += carried;
        }
        if pasted.is_empty() {
            return Err(PasteError::NothingPasted { left_out });
        }
        let transaction = builder.finish();
        self.check(&transaction)?;
        Ok(FeaturePaste {
            transaction,
            pasted,
            left_out,
            inlined,
        })
    }

    fn pasted_kind(
        &self,
        feature: &Feature,
        copied: &BTreeMap<FeatureId, &str>,
        renamed: &BTreeMap<FeatureId, FeatureId>,
        parameters: &CarriedParameters,
        origin: PasteOrigin,
    ) -> Result<Pasted, PasteRefusal> {
        let inputs: Vec<FeatureId> = feature
            .kind
            .dependencies()
            .into_iter()
            .filter(|input| copied.contains_key(input))
            .collect();
        if let Some(missing) = inputs.iter().find(|input| !renamed.contains_key(input)) {
            return Err(PasteRefusal::LeftOutInput {
                name: copied.get(missing).copied().unwrap_or_default().to_owned(),
            });
        }
        let placeholder_of = |input: FeatureId| {
            inputs
                .iter()
                .position(|candidate| *candidate == input)
                .map(|index| FeatureId::from_raw(PLACEHOLDER_BASE + index as u64))
        };
        let mut kind = feature.kind.clone();
        kind.rename_features(&|id| placeholder_of(id).unwrap_or(id));
        let unfollowed = kind.features();
        if let Some(input) = inputs.iter().find(|input| unfollowed.contains(input)) {
            return Err(PasteRefusal::Unfollowed {
                name: copied.get(input).copied().unwrap_or_default().to_owned(),
            });
        }
        let dependencies = kind.dependencies();
        let picked_corners = paste_picks::vertex_bodies(&kind);
        let deep: Vec<FeatureId> = inputs
            .iter()
            .enumerate()
            .filter(|(index, input)| {
                dependencies.contains(input)
                    || picked_corners
                        .contains(&FeatureId::from_raw(PLACEHOLDER_BASE + *index as u64))
            })
            .map(|(_, input)| *input)
            .collect();
        let outside = dependencies
            .iter()
            .filter(|dependency| dependency.raw() < PLACEHOLDER_BASE && !deep.contains(dependency));
        for dependency in outside {
            match origin {
                PasteOrigin::Elsewhere => return Err(PasteRefusal::OutsideFeature),
                PasteOrigin::ThisDocument => {
                    if self.feature(*dependency).is_none() || self.is_rolled_back(*dependency) {
                        return Err(PasteRefusal::MissingFeature);
                    }
                }
            }
        }
        kind.rename_features(&|id| {
            id.raw()
                .checked_sub(PLACEHOLDER_BASE)
                .and_then(|index| inputs.get(usize::try_from(index).ok()?))
                .and_then(|input| renamed.get(input))
                .copied()
                .unwrap_or(id)
        });
        let mut appearance = feature.appearance.clone();
        let inlined = carry_values(&mut kind, &mut appearance, parameters, self, origin)?;
        Ok(Pasted {
            kind,
            appearance,
            inlined,
            deep,
        })
    }
}

pub(crate) fn carry_values(
    kind: &mut FeatureKind,
    appearance: &mut BodyAppearance,
    parameters: &CarriedParameters,
    target: &Document,
    origin: PasteOrigin,
) -> Result<usize, CarryError> {
    let mut inlined = 0;
    for expression in expressions_mut(kind) {
        let carried = parameters.carry(expression, target, origin)?;
        inlined += carried.inlined;
        *expression = carried.expression;
    }
    if let FeatureKind::Sketch(sketch) = kind {
        let dimensions: Vec<_> = sketch
            .sketch
            .constraints()
            .filter_map(|(id, constraint)| Some((id, constraint.dimension()?.clone())))
            .collect();
        for (id, value) in dimensions {
            let carried = parameters.carry(&value, target, origin)?;
            inlined += carried.inlined;
            if carried.expression != value {
                sketch.sketch.set_dimension(id, carried.expression).ok();
            }
        }
    }
    if let Some(density) = &mut appearance.density {
        let carried = parameters.carry(density, target, origin)?;
        inlined += carried.inlined;
        *density = carried.expression;
    }
    Ok(inlined)
}

fn copy_name(name: &str, taken: &BTreeSet<String>) -> String {
    let first = format!("{name} {COPY_SUFFIX}");
    if !taken.contains(&first) {
        return first;
    }
    (2..=taken.len() + 2)
        .map(|number| format!("{first} {number}"))
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or(first)
}

type Rename<'a> = &'a dyn Fn(FeatureId) -> FeatureId;

impl FeatureKind {
    pub fn rename_features(&mut self, rename: Rename<'_>) {
        let each = |ids: &mut Vec<FeatureId>| {
            for id in ids {
                *id = rename(*id);
            }
        };
        match self {
            Self::Sketch(sketch) => {
                match &mut sketch.attachment {
                    Some(SketchAttachment::Face(face)) => face.body = rename(face.body),
                    Some(SketchAttachment::Datum(datum)) => *datum = rename(*datum),
                    Some(SketchAttachment::Frame { frame, .. }) => *frame = rename(*frame),
                    None => {}
                }
                for source in sketch.projections.values_mut() {
                    match source {
                        ProjectionSource::Edge { body, .. }
                        | ProjectionSource::Vertex { body, .. }
                        | ProjectionSource::Section { body, .. } => *body = rename(*body),
                        ProjectionSource::SketchEntity { sketch, .. } => *sketch = rename(*sketch),
                        ProjectionSource::DatumPlane { datum, .. }
                        | ProjectionSource::DatumAxis { datum, .. }
                        | ProjectionSource::DatumPoint { datum } => *datum = rename(*datum),
                        ProjectionSource::PrincipalPlane { .. }
                        | ProjectionSource::PrincipalAxis { .. }
                        | ProjectionSource::Origin => {}
                    }
                }
            }
            Self::Solid(SolidFeature::Extrude(extrude)) => {
                extrude.sketch = rename(extrude.sketch);
                extrude.operation = renamed_operation(extrude.operation, rename);
                each(&mut extrude.other_bodies);
                for end in extrude.extent.ends_mut() {
                    match end {
                        ExtrudeEnd::UpToFace { target, .. } => rename_plane(target, rename),
                        ExtrudeEnd::UpToSurface { face } => face.body = rename(face.body),
                        ExtrudeEnd::Distance(_)
                        | ExtrudeEnd::ThroughAll
                        | ExtrudeEnd::UpToNext { .. } => {}
                    }
                }
                rename_start(extrude.start.as_mut(), rename);
                if let Some(direction) = extrude.direction.as_deref_mut() {
                    rename_axis(direction, rename);
                }
            }
            Self::Solid(SolidFeature::Revolve(revolve)) => {
                revolve.sketch = rename(revolve.sketch);
                revolve.operation = renamed_operation(revolve.operation, rename);
                each(&mut revolve.other_bodies);
                if let RevolveAxis::Model(axis) = &mut revolve.axis {
                    rename_axis(axis, rename);
                }
                if let RevolveExtent::UpTo { target, .. } = &mut revolve.extent {
                    rename_plane(target, rename);
                }
                rename_start(revolve.start.as_mut(), rename);
            }
            Self::Solid(SolidFeature::Sweep(sweep)) => {
                sweep.sketch = rename(sweep.sketch);
                sweep.operation = renamed_operation(sweep.operation, rename);
                each(&mut sweep.other_bodies);
                sweep.path = sweep.path.with_features_renamed(&rename);
            }
            Self::Blend(blend) => blend.body = rename(blend.body),
            Self::Shell(shell) => shell.body = rename(shell.body),
            Self::OffsetFace(offset) => offset.body = rename(offset.body),
            Self::Primitive(primitive) => {
                primitive.operation = renamed_operation(primitive.operation, rename);
                rename_plane(&mut primitive.plane, rename);
            }
            Self::ReferenceImage(image) => match &mut image.plane {
                ImagePlane::Reference(plane) => rename_plane(plane, rename),
                ImagePlane::Sketch(sketch) => *sketch = rename(*sketch),
            },
            Self::Pipe(pipe) => {
                pipe.operation = renamed_operation(pipe.operation, rename);
                pipe.path = pipe.path.with_features_renamed(&rename);
            }
            Self::Combine(combine) => {
                combine.body = rename(combine.body);
                combine.tool = rename(combine.tool);
                each(&mut combine.more_tools);
            }
            Self::Move(movement) => {
                movement.body = rename(movement.body);
                movement.frame = movement.frame.map(rename);
                if let TurnCentre::Axis(turn) = &mut movement.about {
                    rename_axis(&mut turn.axis, rename);
                }
            }
            Self::Mate(mate) => {
                mate.body = rename(mate.body);
                rename_mate(&mut mate.pair, rename);
            }
            Self::Mirror(mirror) => {
                mirror.body = rename(mirror.body);
                each(&mut mirror.mirrored);
                rename_plane(&mut mirror.plane, rename);
            }
            Self::Split(split) => {
                split.body = rename(split.body);
                rename_along(&mut split.along, rename);
            }
            Self::SplitFace(split) => {
                split.body = rename(split.body);
                rename_along(&mut split.along, rename);
                if let Some(direction) = split.carry.direction_mut() {
                    rename_axis(direction, rename);
                }
            }
            Self::Scale(scale) => {
                scale.body = rename(scale.body);
                scale.frame = scale.frame.map(rename);
            }
            Self::Hole(hole) => {
                hole.body = rename(hole.body);
                hole.sketch = rename(hole.sketch);
                if let HoleDepth::UpToFace { target, .. } = &mut hole.depth {
                    rename_plane(target, rename);
                }
            }
            Self::Pattern(pattern) => {
                pattern.body = rename(pattern.body);
                each(&mut pattern.repeated);
                match &mut pattern.kind {
                    PatternKind::Curve(curve) => curve.sketch = rename(curve.sketch),
                    PatternKind::Points(points) => {
                        points.sketch = rename(points.sketch);
                        rename_point(&mut points.base, rename);
                    }
                    PatternKind::Linear { first, second } => {
                        rename_axis(&mut first.axis, rename);
                        if let Some(second) = second {
                            rename_axis(&mut second.axis, rename);
                        }
                    }
                    PatternKind::Circular(circular) => {
                        if let Some(axis) = &mut circular.axis {
                            rename_axis(axis, rename);
                        }
                    }
                }
            }
            Self::Remove(remove) => remove.body = rename(remove.body),
            Self::Thread(thread) => thread.body = rename(thread.body),
            Self::Import(import) => import.placement.frame = import.placement.frame.map(rename),
            Self::Measurement(measurement) => {
                measurement.parameter = None;
                for item in measurement.reading.items_mut() {
                    rename_measured(item, rename);
                }
            }
            Self::Datum(datum) => rename_datum(datum, rename),
        }
    }
}

fn rename_start(start: Option<&mut SolidStart>, rename: Rename<'_>) {
    if let Some(SolidStart::Plane(target)) = start {
        rename_plane(target, rename);
    }
}

fn rename_along(along: &mut SplitAlong, rename: Rename<'_>) {
    match along {
        SplitAlong::Body(body) => *body = rename(*body),
        SplitAlong::Sketch(sketch) => *sketch = rename(*sketch),
        SplitAlong::Plane(plane) => rename_plane(plane, rename),
        SplitAlong::Surface(surface) => surface.body = rename(surface.body),
    }
}

fn rename_plane(plane: &mut PlaneReference, rename: Rename<'_>) {
    match plane {
        PlaneReference::Principal(_) => {}
        PlaneReference::Datum(datum) => *datum = rename(*datum),
        PlaneReference::Face(attachment) => attachment.body = rename(attachment.body),
        PlaneReference::Frame { frame, .. } => *frame = rename(*frame),
    }
}

fn rename_axis(axis: &mut AxisReference, rename: Rename<'_>) {
    match axis {
        AxisReference::Principal(_) => {}
        AxisReference::Datum(datum) => *datum = rename(*datum),
        AxisReference::Edge { body, .. } | AxisReference::Face { body, .. } => {
            *body = rename(*body);
        }
        AxisReference::Sketch { sketch, .. } => *sketch = rename(*sketch),
        AxisReference::Frame { frame, .. } => *frame = rename(*frame),
    }
}

fn rename_point(point: &mut PointReference, rename: Rename<'_>) {
    match point {
        PointReference::Origin => {}
        PointReference::Datum(datum) | PointReference::Frame(datum) => *datum = rename(*datum),
        PointReference::Vertex { body, .. }
        | PointReference::Centre { body, .. }
        | PointReference::SurfaceCentre { body, .. } => *body = rename(*body),
        PointReference::Sketch { sketch, .. } => *sketch = rename(*sketch),
    }
}

fn rename_tangent(tangent: &mut FaceTangent, rename: Rename<'_>) {
    tangent.body = rename(tangent.body);
    rename_point(&mut tangent.toward, rename);
}

fn rename_station(station: &mut CurveStation, rename: Rename<'_>) {
    station.body = rename(station.body);
}

fn rename_datum(datum: &mut Datum, rename: Rename<'_>) {
    match datum {
        Datum::Plane(plane) => {
            rename_plane(&mut plane.base, rename);
            if let Some(rotation) = &mut plane.rotation {
                rename_axis(&mut rotation.axis, rename);
            }
        }
        Datum::PlaneThrough(through) => match through {
            PlaneThrough::Points(points) => {
                for point in points {
                    rename_point(point, rename);
                }
            }
            PlaneThrough::Midway(first, second) => {
                rename_plane(first, rename);
                rename_plane(second, rename);
            }
            PlaneThrough::AxisAndPoint(axis, point) | PlaneThrough::NormalTo(axis, point) => {
                rename_axis(axis, rename);
                rename_point(point, rename);
            }
            PlaneThrough::Tangent(tangent) | PlaneThrough::TangentAt(tangent) => {
                rename_tangent(tangent, rename);
            }
            PlaneThrough::SquareToCurve(station) => rename_station(station, rename),
            PlaneThrough::Lines(first, second) => {
                rename_axis(first, rename);
                rename_axis(second, rename);
            }
        },
        Datum::Axis(axis) => match axis {
            DatumAxis::Along(axis) => rename_axis(axis, rename),
            DatumAxis::Intersection(first, second) => {
                rename_plane(first, rename);
                rename_plane(second, rename);
            }
            DatumAxis::Points(first, second) => {
                rename_point(first, rename);
                rename_point(second, rename);
            }
            DatumAxis::NormalTo(plane, point) => {
                rename_plane(plane, rename);
                rename_point(point, rename);
            }
            DatumAxis::SquareToFace(tangent) => rename_tangent(tangent, rename),
        },
        Datum::Point(point) => rename_point(&mut point.base, rename),
        Datum::PointBy(by) => match by {
            PointBy::LinesCross(first, second) => {
                rename_axis(first, rename);
                rename_axis(second, rename);
            }
            PointBy::AxisAndPlane(axis, plane) => {
                rename_axis(axis, rename);
                rename_plane(plane, rename);
            }
            PointBy::ThreePlanes(planes) => {
                for plane in planes {
                    rename_plane(plane, rename);
                }
            }
            PointBy::Along(station) => rename_station(station, rename),
            PointBy::EdgeMiddle { body, .. } | PointBy::FaceCentre { body, .. } => {
                *body = rename(*body);
            }
        },
        Datum::Frame(frame) => {
            rename_point(&mut frame.origin, rename);
            rename_axis(&mut frame.x_axis, rename);
            rename_plane(&mut frame.plane, rename);
        }
    }
}

fn rename_mate(pair: &mut MatePair, rename: Rename<'_>) {
    let axes = |axes: &mut AxisMate| {
        rename_axis(&mut axes.axis, rename);
        rename_axis(&mut axes.target, rename);
    };
    match pair {
        MatePair::Faces(faces) => rename_plane(&mut faces.target, rename),
        MatePair::Axes(both) => axes(both),
        MatePair::FaceAxis(both) => {
            rename_plane(&mut both.faces.target, rename);
            axes(&mut both.axes);
        }
        MatePair::Angle(angle) => match &mut angle.sides {
            AngleSides::Faces(faces) => rename_plane(&mut faces.target, rename),
            AngleSides::Axes(both) => axes(both),
        },
        MatePair::Tangent(tangent) => rename_plane(&mut tangent.target, rename),
        MatePair::Point(point) => {
            rename_point(&mut point.point, rename);
            match &mut point.target {
                PointTarget::Point(target) => rename_point(target, rename),
                PointTarget::Plane(plane) => rename_plane(plane, rename),
            }
        }
        MatePair::FaceOnRound(resting) => resting.round.body = rename(resting.round.body),
    }
}

fn rename_measured(item: &mut MeasuredItem, rename: Rename<'_>) {
    match item {
        MeasuredItem::Point(point) => rename_point(point, rename),
        MeasuredItem::Axis(axis) => rename_axis(axis, rename),
        MeasuredItem::Plane(plane) => rename_plane(plane, rename),
        MeasuredItem::Edge { body, .. }
        | MeasuredItem::Face { body, .. }
        | MeasuredItem::Body(body) => *body = rename(*body),
        MeasuredItem::Sketch { sketch, .. } => *sketch = rename(*sketch),
    }
}

fn renamed_operation(operation: BodyOperation, rename: Rename<'_>) -> BodyOperation {
    match operation.target() {
        Some(body) => operation.with_target(rename(body)),
        None => operation,
    }
}
