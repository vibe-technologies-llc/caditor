use std::collections::{BTreeMap, BTreeSet};

use caditor_expression::{Dimension, Expression, ParameterId, Quantity, Unit};

use crate::{
    attachment::SketchAttachment,
    body_appearance::BodyAppearance,
    document::{Document, Feature, FeatureId, FeatureKind},
    edit::{EditError, Transaction},
    inlining::expressions_mut,
    pattern::PatternKind,
    projection::ProjectionSource,
    solid::{BodyOperation, SolidFeature},
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

impl Document {
    pub fn paste_features(
        &self,
        features: &[Feature],
        parameters: &CarriedParameters,
        origin: PasteOrigin,
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
        let mut builder = self.transaction(label);
        let mut renamed: BTreeMap<FeatureId, FeatureId> = BTreeMap::new();
        let mut names: BTreeSet<String> = self
            .features()
            .map(|feature| feature.name.clone())
            .collect();
        let mut pasted = Vec::new();
        let mut left_out = Vec::new();
        let mut inlined = 0;
        for feature in features {
            match self.pasted_kind(feature, &copied, &renamed, parameters, origin) {
                Ok((kind, appearance, carried)) => {
                    let name = copy_name(&feature.name, &names);
                    names.insert(name.clone());
                    let mut copy = Feature::new(feature.id(), name, kind);
                    copy.hidden = feature.hidden;
                    copy.suppressed = feature.suppressed;
                    copy.appearance = appearance;
                    let id = builder.add_copied_feature(copy);
                    renamed.insert(feature.id(), id);
                    pasted.push(id);
                    inlined += carried;
                }
                Err(reason) => left_out.push(LeftOut {
                    name: feature.name.clone(),
                    reason,
                }),
            }
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
    ) -> Result<(FeatureKind, BodyAppearance, usize), PasteRefusal> {
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
        let dependencies = kind.dependencies();
        if let Some(deep) = inputs.iter().find(|input| dependencies.contains(input)) {
            return Err(PasteRefusal::CopiedGeometry {
                name: copied.get(deep).copied().unwrap_or_default().to_owned(),
            });
        }
        let outside = dependencies
            .iter()
            .filter(|dependency| dependency.raw() < PLACEHOLDER_BASE);
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
        let mut inlined = 0;
        for expression in expressions_mut(&mut kind) {
            let carried = parameters.carry(expression, self, origin)?;
            inlined += carried.inlined;
            *expression = carried.expression;
        }
        if let FeatureKind::Sketch(sketch) = &mut kind {
            let dimensions: Vec<_> = sketch
                .sketch
                .constraints()
                .filter_map(|(id, constraint)| Some((id, constraint.dimension()?.clone())))
                .collect();
            for (id, value) in dimensions {
                let carried = parameters.carry(&value, self, origin)?;
                inlined += carried.inlined;
                if carried.expression != value {
                    sketch.sketch.set_dimension(id, carried.expression).ok();
                }
            }
        }
        let mut appearance = feature.appearance.clone();
        if let Some(density) = &mut appearance.density {
            let carried = parameters.carry(density, self, origin)?;
            inlined += carried.inlined;
            *density = carried.expression;
        }
        Ok((kind, appearance, inlined))
    }
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

impl FeatureKind {
    pub fn rename_features(&mut self, rename: &dyn Fn(FeatureId) -> FeatureId) {
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
                        ProjectionSource::DatumPlane { datum, .. } => *datum = rename(*datum),
                        ProjectionSource::PrincipalPlane { .. } => {}
                    }
                }
            }
            Self::Solid(SolidFeature::Extrude(extrude)) => {
                extrude.sketch = rename(extrude.sketch);
                extrude.operation = renamed_operation(extrude.operation, rename);
                each(&mut extrude.other_bodies);
            }
            Self::Solid(SolidFeature::Revolve(revolve)) => {
                revolve.sketch = rename(revolve.sketch);
                revolve.operation = renamed_operation(revolve.operation, rename);
                each(&mut revolve.other_bodies);
            }
            Self::Blend(blend) => blend.body = rename(blend.body),
            Self::Shell(shell) => shell.body = rename(shell.body),
            Self::OffsetFace(offset) => offset.body = rename(offset.body),
            Self::Primitive(primitive) => {
                primitive.operation = renamed_operation(primitive.operation, rename);
            }
            Self::Combine(combine) => {
                combine.body = rename(combine.body);
                combine.tool = rename(combine.tool);
                each(&mut combine.more_tools);
            }
            Self::Move(movement) => {
                movement.body = rename(movement.body);
                movement.frame = movement.frame.map(rename);
            }
            Self::Mate(mate) => mate.body = rename(mate.body),
            Self::Mirror(mirror) => {
                mirror.body = rename(mirror.body);
                each(&mut mirror.mirrored);
            }
            Self::Split(split) => {
                split.body = rename(split.body);
                match &mut split.along {
                    SplitAlong::Body(body) => *body = rename(*body),
                    SplitAlong::Sketch(sketch) => *sketch = rename(*sketch),
                    SplitAlong::Plane(_) => {}
                }
            }
            Self::SplitFace(split) => {
                split.body = rename(split.body);
                match &mut split.along {
                    SplitAlong::Body(body) => *body = rename(*body),
                    SplitAlong::Sketch(sketch) => *sketch = rename(*sketch),
                    SplitAlong::Plane(_) => {}
                }
            }
            Self::Scale(scale) => {
                scale.body = rename(scale.body);
                scale.frame = scale.frame.map(rename);
            }
            Self::Hole(hole) => {
                hole.body = rename(hole.body);
                hole.sketch = rename(hole.sketch);
            }
            Self::Pattern(pattern) => {
                pattern.body = rename(pattern.body);
                each(&mut pattern.repeated);
                match &mut pattern.kind {
                    PatternKind::Curve(curve) => curve.sketch = rename(curve.sketch),
                    PatternKind::Points(points) => points.sketch = rename(points.sketch),
                    PatternKind::Linear { .. } | PatternKind::Circular(_) => {}
                }
            }
            Self::Remove(remove) => remove.body = rename(remove.body),
            Self::Thread(thread) => thread.body = rename(thread.body),
            Self::Import(import) => import.placement.frame = import.placement.frame.map(rename),
            Self::Measurement(measurement) => measurement.parameter = None,
            Self::Datum(_) => {}
        }
    }
}

fn renamed_operation(
    operation: BodyOperation,
    rename: &dyn Fn(FeatureId) -> FeatureId,
) -> BodyOperation {
    match operation.target() {
        Some(body) => operation.with_target(rename(body)),
        None => operation,
    }
}
