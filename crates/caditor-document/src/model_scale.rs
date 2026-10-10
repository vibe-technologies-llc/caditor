use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use caditor_expression::{
    BinaryOperator, Dimension, EvalError, Expression, ParameterId, Quantity, Unit, format_number,
};
use caditor_geometry::{Point3, Similarity, Vector3};
use caditor_sketch::{Constraint, Entity};

use crate::{
    attachment::SketchFeature,
    blend::ChamferForm,
    datum::{
        AxisReference, Datum, DatumAxis, DatumFrame, DatumPlane, DatumPoint, PlaneReference,
        PlaneThrough, PointBy, PointReference, PrincipalGeometry, PrincipalPlane,
    },
    document::{Document, Feature, FeatureId, FeatureKind},
    edit::{Edit, Transaction},
    hole::{HoleDepth, HoleShape, HoleStyle},
    mate::{AngleSides, AxisMate, FaceAxisMate, Mate, MatePair, PointMate, PointTarget},
    movement::{MoveAxis, Pivot, TurnCentre},
    pattern::PatternKind,
    primitive::SizeRule,
    projection::ProjectionSource,
    scaling::{MAX_SCALE_FACTOR, MIN_SCALE_FACTOR},
    solid::{
        Extrude, ExtrudeEnd, ExtrudeExtent, Revolve, RevolveAxis, RevolveExtent, SolidFeature,
        SolidStart,
    },
    thread::ThreadLength,
    values::{ParameterValues, evaluation_order},
};

const CENTRE_TOLERANCE: f64 = 1e-9;
const AGREEMENT: f64 = 1e-9;
const SCALED_ORIGIN: &str = "Origin, scaled";
const SCALED_FRAME: &str = "Axes and planes, scaled";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScaledValues {
    #[default]
    Plain,
    AndParameters,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelScale {
    pub factor: f64,
    pub centre: Point3,
    pub values: ScaledValues,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScaleSummary {
    pub sketches: usize,
    pub features: usize,
    pub parameters: usize,
    pub kept: Vec<String>,
    pub cleared_standards: Vec<String>,
    pub threads: Vec<String>,
    pub moved: Vec<DisplacedFeature>,
    pub datums: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplacedFeature {
    pub feature: FeatureId,
    pub name: String,
    pub geometry: Vec<PrincipalGeometry>,
    projects: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScaledModel {
    pub transaction: Transaction,
    pub summary: ScaleSummary,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ModelScaleError {
    #[error("The factor must be more than zero, and {} is not.", format_number(*.0))]
    FactorNotPositive(f64),
    #[error(
        "The factor {} is outside what caditor can model, from {} to {}.",
        format_number(*.0),
        format_number(MIN_SCALE_FACTOR),
        format_number(MAX_SCALE_FACTOR)
    )]
    FactorOutOfRange(f64),
    #[error("A factor of 1 leaves the model as it is.")]
    FactorIsOne,
    #[error("The centre is not a usable point.")]
    CentreNotUsable,
    #[error("A value of {name} would grow too long to store once scaled.")]
    TooLong { name: String },
}

impl ModelScaleError {
    pub fn remedy(&self) -> String {
        match self {
            Self::FactorNotPositive(_) | Self::FactorOutOfRange(_) | Self::FactorIsOne => {
                "Enter a factor above zero, such as 2 to double the size or 25.4 for a part drawn \
                 in inches."
                    .to_owned()
            }
            Self::CentreNotUsable => "Choose the origin or another point.".to_owned(),
            Self::TooLong { name } => {
                format!("Shorten the expressions of {name}, or scale only plain values.")
            }
        }
    }
}

impl Document {
    pub fn scaled(&self, scale: &ModelScale) -> Result<ScaledModel, ModelScaleError> {
        let factor = scale.factor;
        if !factor.is_finite() || factor <= 0.0 {
            return Err(ModelScaleError::FactorNotPositive(factor));
        }
        if !(MIN_SCALE_FACTOR..=MAX_SCALE_FACTOR).contains(&factor) {
            return Err(ModelScaleError::FactorOutOfRange(factor));
        }
        if factor == 1.0 {
            return Err(ModelScaleError::FactorIsOne);
        }
        let similarity = Similarity::scaling(scale.centre, factor)
            .filter(|_| scale.centre.is_finite())
            .ok_or(ModelScaleError::CentreNotUsable)?;
        let shift = similarity.apply_point(Point3::ZERO);
        let displaced = self.displaced_by_scale(scale.centre);
        let placed = placed_datums(self, &displaced, shift);
        let mut rescaler = Rescaler {
            factor,
            centre: scale.centre,
            shift,
            similarity,
            datums: placed.datums,
            values: scale.values,
            old: ParameterValues::evaluate(self),
            scaled: BTreeMap::new(),
            new: BTreeMap::new(),
            summary: ScaleSummary::default(),
        };
        let mut edits = placed.edits;
        edits.extend(rescaler.parameters(self)?);
        for feature in self.features() {
            let changed = match &feature.kind {
                FeatureKind::Sketch(sketch) => rescaler.sketch(feature, sketch)?,
                _ => rescaler.feature(feature)?,
            };
            match (&feature.kind, changed.is_empty()) {
                (_, true) => {}
                (FeatureKind::Sketch(_), false) => rescaler.summary.sketches += 1,
                (_, false) => rescaler.summary.features += 1,
            }
            edits.extend(changed);
        }
        edits.extend(rescaler.views(self));
        rescaler.summary.moved = displaced;
        rescaler.summary.datums = placed.names;
        Ok(ScaledModel {
            transaction: Transaction::new(
                format!("Scale model by {}", format_number(factor)),
                edits,
            ),
            summary: rescaler.summary,
        })
    }

    pub fn displaced_by_scale(&self, centre: Point3) -> Vec<DisplacedFeature> {
        self.features()
            .filter_map(|feature| {
                let used = !self.dependents_of(&[feature.id()]).is_empty();
                let geometry = displaced_geometry(&feature.kind, centre, used);
                (!geometry.is_empty()).then(|| DisplacedFeature {
                    feature: feature.id(),
                    name: feature.name.clone(),
                    geometry,
                    projects: feature.kind.sketch().is_some(),
                })
            })
            .collect()
    }
}

struct Rescaler {
    factor: f64,
    centre: Point3,
    shift: Vector3,
    similarity: Similarity,
    datums: ScaledDatums,
    values: ScaledValues,
    old: ParameterValues,
    scaled: BTreeMap<ParameterId, i8>,
    new: BTreeMap<ParameterId, Quantity>,
    summary: ScaleSummary,
}

impl Rescaler {
    fn times(&self, power: i8) -> f64 {
        self.factor.powi(i32::from(power))
    }

    fn new_value(&self, id: ParameterId) -> Result<Quantity, EvalError> {
        self.new
            .get(&id)
            .copied()
            .map_or_else(|| self.old.value(id), Ok)
    }

    fn parameters(&mut self, document: &Document) -> Result<Vec<Edit>, ModelScaleError> {
        let mut edits = Vec::new();
        for id in evaluation_order(document) {
            let Some(parameter) = document.parameter(id) else {
                continue;
            };
            let Ok(old) = self.old.value(id) else {
                continue;
            };
            let power = old.dimension.length_power();
            let rewritten = match self.values {
                ScaledValues::AndParameters => true,
                ScaledValues::Plain => {
                    parameter.owner.is_some()
                        && power != 0
                        && parameter.expression.parameters().is_empty()
                }
            };
            if !rewritten {
                continue;
            }
            if power != 0 {
                self.scaled.insert(id, power);
            }
            let expression = self.rewritten(&parameter.expression, power, &parameter.name)?;
            if let Ok(value) = expression.evaluate(&|used| self.new_value(used)) {
                self.new.insert(id, value);
            }
            if expression != parameter.expression {
                self.summary.parameters += 1;
                edits.push(Edit::SetParameterExpression { id, expression });
            }
        }
        Ok(edits)
    }

    fn rewritten(
        &self,
        expression: &Expression,
        power: i8,
        name: &str,
    ) -> Result<Expression, ModelScaleError> {
        let old = self.old.evaluate_expression(expression).ok();
        let candidate = normalized(
            expression.with_lengths_scaled(self.factor, power, &|id| self.old.value(id)),
            name,
        )?;
        if self.agrees(&candidate, old, power) {
            return Ok(candidate);
        }
        let mut exact = expression.clone();
        for id in expression.parameters() {
            if let Some(used) = self.scaled.get(&id) {
                let unscaled = Expression::binary(
                    BinaryOperator::Divide,
                    Expression::Parameter(id),
                    Expression::number(self.times(*used)),
                );
                exact = exact.inlining(id, &unscaled).map_err(|_| too_long(name))?;
            }
        }
        if power != 0 {
            exact = exact.times_number(self.times(power));
        }
        normalized(exact, name)
    }

    fn agrees(&self, candidate: &Expression, old: Option<Quantity>, power: i8) -> bool {
        let Some(old) = old else {
            return true;
        };
        let expected = old.value * self.times(power);
        candidate
            .evaluate(&|id| self.new_value(id))
            .is_ok_and(|new| {
                new.dimension == old.dimension
                    && (new.value - expected).abs() <= AGREEMENT * expected.abs().max(1.0)
            })
    }

    fn follows_others(&self, expression: &Expression) -> bool {
        self.values == ScaledValues::Plain
            && expression
                .parameters()
                .iter()
                .any(|id| !self.scaled.contains_key(id))
    }

    fn keep(&mut self, name: &str) {
        if !self.summary.kept.iter().any(|kept| kept == name) {
            self.summary.kept.push(name.to_owned());
        }
    }

    fn placed(
        &mut self,
        expression: &mut Expression,
        shift: f64,
        name: &str,
    ) -> Result<(), ModelScaleError> {
        if self.follows_others(expression) {
            self.keep(name);
            return Ok(());
        }
        let scaled = self.rewritten(expression, 1, name)?;
        *expression = shifted(scaled, shift, name)?;
        Ok(())
    }

    fn length(&mut self, expression: &mut Expression, name: &str) -> Result<(), ModelScaleError> {
        self.placed(expression, 0.0, name)
    }

    fn lengths<'a>(
        &mut self,
        expressions: impl IntoIterator<Item = &'a mut Expression>,
        name: &str,
    ) -> Result<(), ModelScaleError> {
        for expression in expressions {
            self.length(expression, name)?;
        }
        Ok(())
    }

    fn sketch(
        &mut self,
        feature: &Feature,
        definition: &SketchFeature,
    ) -> Result<Vec<Edit>, ModelScaleError> {
        let id = feature.id();
        let sketch = &definition.sketch;
        let mut edits = Vec::new();
        let plane = sketch.plane().mapped(&self.similarity);
        if plane != sketch.plane() {
            edits.push(Edit::SetSketchPlacement {
                feature: id,
                plane,
                attachment: definition.attachment.clone(),
            });
        }
        for (entity, held) in sketch.entities() {
            let scaled = match held {
                Entity::Point(point) => Entity::Point(*point * self.factor),
                Entity::Circle { center, radius } => Entity::Circle {
                    center: *center,
                    radius: radius * self.factor,
                },
                Entity::Ellipse {
                    center,
                    major,
                    minor_radius,
                } => Entity::Ellipse {
                    center: *center,
                    major: *major,
                    minor_radius: minor_radius * self.factor,
                },
                Entity::EllipticalArc {
                    center,
                    major,
                    minor_radius,
                    start,
                    end,
                } => Entity::EllipticalArc {
                    center: *center,
                    major: *major,
                    minor_radius: minor_radius * self.factor,
                    start: *start,
                    end: *end,
                },
                Entity::Line { .. } | Entity::Arc { .. } | Entity::Spline { .. } => continue,
            };
            if scaled != *held {
                edits.push(Edit::SetSketchEntity {
                    feature: id,
                    id: entity,
                    entity: scaled,
                });
            }
        }
        for (constraint, held) in sketch.constraints() {
            let label = sketch
                .label_offset(constraint)
                .map(|offset| offset * self.factor);
            if let Constraint::Fix { point, at } = held {
                edits.push(Edit::RemoveSketchConstraint {
                    feature: id,
                    id: constraint,
                });
                edits.push(Edit::AddSketchConstraint {
                    feature: id,
                    id: constraint,
                    constraint: Constraint::Fix {
                        point: *point,
                        at: *at * self.factor,
                    },
                    inactive: !sketch.is_active(constraint),
                    label,
                });
                continue;
            }
            if let Some(value) = held.dimension()
                && held.dimension_kind() == Some(Dimension::LENGTH)
            {
                let mut scaled = value.clone();
                self.length(&mut scaled, &feature.name)?;
                if scaled != *value {
                    edits.push(Edit::SetDimension {
                        feature: id,
                        constraint,
                        value: scaled,
                    });
                }
            }
            if label.is_some() {
                edits.push(Edit::SetSketchLabel {
                    feature: id,
                    id: constraint,
                    offset: label,
                });
            }
        }
        for (entity, source) in &definition.projections {
            let scaled = match source {
                ProjectionSource::DatumPlane { datum, reach } => ProjectionSource::DatumPlane {
                    datum: *datum,
                    reach: reach * self.factor,
                },
                ProjectionSource::PrincipalPlane { plane, reach } => {
                    match self.datums.planes.get(plane) {
                        Some(datum) => ProjectionSource::DatumPlane {
                            datum: *datum,
                            reach: reach * self.factor,
                        },
                        None => ProjectionSource::PrincipalPlane {
                            plane: *plane,
                            reach: reach * self.factor,
                        },
                    }
                }
                ProjectionSource::Edge { .. }
                | ProjectionSource::Vertex { .. }
                | ProjectionSource::SketchEntity { .. }
                | ProjectionSource::Section { .. } => continue,
            };
            edits.push(Edit::SetSketchProjection {
                feature: id,
                id: *entity,
                source: Some(scaled),
            });
        }
        Ok(edits)
    }

    fn feature(&mut self, feature: &Feature) -> Result<Vec<Edit>, ModelScaleError> {
        let name = feature.name.as_str();
        let mut kind = feature.kind.clone();
        match &mut kind {
            FeatureKind::Solid(SolidFeature::Extrude(extrude)) => {
                match &mut extrude.extent {
                    ExtrudeExtent::Symmetric { distance } => self.length(distance, name)?,
                    ExtrudeExtent::OneSide { end, .. } => self.end(end, name)?,
                    ExtrudeExtent::TwoSides { forward, backward } => {
                        self.end(forward, name)?;
                        self.end(backward, name)?;
                    }
                }
                self.start(&mut extrude.start, name)?;
                if let Some(wall) = &mut extrude.wall {
                    self.length(&mut wall.thickness, name)?;
                }
            }
            FeatureKind::Solid(SolidFeature::Revolve(revolve)) => {
                self.start(&mut revolve.start, name)?;
                if let Some(wall) = &mut revolve.wall {
                    self.length(&mut wall.thickness, name)?;
                }
            }
            FeatureKind::Blend(blend) => {
                self.length(&mut blend.size, name)?;
                if let ChamferForm::TwoDistances { second } = &mut blend.form {
                    self.length(second, name)?;
                }
            }
            FeatureKind::Shell(shell) => self.length(&mut shell.thickness, name)?,
            FeatureKind::OffsetFace(offset) => self.length(&mut offset.distance, name)?,
            FeatureKind::Mate(mate) => {
                for distance in mate.lengths_mut() {
                    self.length(distance, name)?;
                }
            }
            FeatureKind::Thread(thread) => {
                if let ThreadLength::Depth(depth) = &mut thread.length {
                    self.length(depth, name)?;
                }
                self.summary.threads.push(name.to_owned());
            }
            FeatureKind::Primitive(primitive) => {
                let rules = primitive.shape.rules();
                for (size, rule) in primitive.shape.sizes_mut().into_iter().zip(rules) {
                    if rule != SizeRule::Sides {
                        self.length(size, name)?;
                    }
                }
                let shifts = match &primitive.plane {
                    PlaneReference::Principal(plane)
                        if holds(PrincipalGeometry::Plane(*plane), self.centre) =>
                    {
                        let plane = plane.plane();
                        [
                            self.shift.dot(plane.x_axis()),
                            self.shift.dot(plane.y_axis()),
                        ]
                    }
                    PlaneReference::Principal(_)
                    | PlaneReference::Datum(_)
                    | PlaneReference::Face(_)
                    | PlaneReference::Frame { .. } => [0.0; 2],
                };
                for (at, shift) in primitive.at.iter_mut().zip(shifts) {
                    self.placed(at, shift, name)?;
                }
            }
            FeatureKind::Move(movement) => {
                let turned_about_origin =
                    matches!(movement.about, TurnCentre::Origin) && movement.frame.is_none();
                let shift = if turned_about_origin {
                    movement
                        .placement(&self.old, Pivot::Point(Point3::ZERO), None)
                        .map_or(Vector3::ZERO, |placement| {
                            self.shift - placement.apply_vector(self.shift)
                        })
                } else {
                    Vector3::ZERO
                };
                for axis in MoveAxis::ALL {
                    let along = *axis.of(&shift.to_array());
                    self.placed(axis.of_mut(&mut movement.offset), along, name)?;
                }
            }
            FeatureKind::Scale(scale) => {
                let shift = match scale.frame {
                    Some(_) => Vector3::ZERO,
                    None => self.shift,
                };
                for axis in MoveAxis::ALL {
                    let along = *axis.of(&shift.to_array());
                    self.placed(axis.of_mut(&mut scale.center), along, name)?;
                }
            }
            FeatureKind::Hole(hole) => {
                let before = hole.clone();
                self.length(&mut hole.diameter, name)?;
                self.lengths(hole.depth.expressions_mut(), name)?;
                match &mut hole.style {
                    HoleStyle::Plain => {}
                    HoleStyle::Counterbore { diameter, depth } => {
                        self.lengths([diameter, depth], name)?;
                    }
                    HoleStyle::Countersink { diameter, .. } => self.length(diameter, name)?,
                    HoleStyle::Stepped(steps) => {
                        for step in steps {
                            self.lengths([&mut step.diameter, &mut step.depth], name)?;
                        }
                    }
                }
                if let HoleShape::Slot { length, .. } = &mut hole.shape {
                    self.length(length, name)?;
                }
                if let Some(depth) = &mut hole.thread.depth {
                    self.length(depth, name)?;
                }
                if hole.standard.is_some() && hole.diameter != before.diameter {
                    hole.standard = None;
                    self.summary.cleared_standards.push(name.to_owned());
                }
            }
            FeatureKind::Pattern(pattern) => match &mut pattern.kind {
                PatternKind::Linear { first, second } => {
                    self.length(&mut first.spacing, name)?;
                    if let Some(second) = second {
                        self.length(&mut second.spacing, name)?;
                    }
                }
                PatternKind::Curve(curve) => self.length(&mut curve.spacing, name)?,
                PatternKind::Circular(_) | PatternKind::Points(_) => {}
            },
            FeatureKind::Datum(datum) => self.datum(datum, name)?,
            FeatureKind::Import(import) => {
                let shift = match import.placement.frame {
                    Some(_) => Vector3::ZERO,
                    None => self.shift,
                };
                for axis in MoveAxis::ALL {
                    let along = *axis.of(&shift.to_array());
                    self.placed(axis.of_mut(&mut import.placement.offset), along, name)?;
                }
                let scale = &mut import.placement.scale;
                if self.follows_others(scale) {
                    self.keep(name);
                } else {
                    let multiplied = match scale {
                        Expression::Number(value) => Expression::Number(*value * self.factor),
                        _ => scale.clone().times_number(self.factor),
                    };
                    *scale = normalized(multiplied, name)?;
                }
            }
            FeatureKind::Sketch(_)
            | FeatureKind::Combine(_)
            | FeatureKind::Mirror(_)
            | FeatureKind::Split(_)
            | FeatureKind::SplitFace(_)
            | FeatureKind::Remove(_)
            | FeatureKind::Measurement(_) => {}
        }
        self.retarget(&mut kind);
        Ok(if kind == feature.kind {
            Vec::new()
        } else {
            vec![Edit::SetFeatureKind {
                id: feature.id(),
                kind,
            }]
        })
    }

    fn retarget(&self, kind: &mut FeatureKind) {
        for reference in anchored(kind) {
            if reference
                .geometry()
                .is_some_and(|geometry| !holds(geometry, self.centre))
            {
                reference.retarget(&self.datums);
            }
        }
    }

    fn end(&mut self, end: &mut ExtrudeEnd, name: &str) -> Result<(), ModelScaleError> {
        self.lengths(end.expressions_mut(), name)
    }

    fn start(&mut self, start: &mut Option<SolidStart>, name: &str) -> Result<(), ModelScaleError> {
        match start {
            Some(SolidStart::Distance(distance)) => self.length(distance, name),
            Some(SolidStart::Plane(_)) | None => Ok(()),
        }
    }

    fn datum(&mut self, datum: &mut Datum, name: &str) -> Result<(), ModelScaleError> {
        match datum {
            Datum::Plane(plane) => {
                let shift = match (&plane.base, &plane.rotation, self.datums.frame) {
                    (PlaneReference::Principal(principal), None, Some(frame)) => {
                        plane.base = PlaneReference::Frame {
                            frame,
                            plane: *principal,
                        };
                        0.0
                    }
                    (PlaneReference::Principal(principal), None, None) => {
                        self.shift.dot(principal.plane().normal())
                    }
                    _ => 0.0,
                };
                self.placed(&mut plane.offset, shift, name)
            }
            Datum::Point(point) => {
                let shift = match point.base {
                    PointReference::Origin => self.shift,
                    _ => Vector3::ZERO,
                };
                for axis in MoveAxis::ALL {
                    let along = *axis.of(&shift.to_array());
                    self.placed(axis.of_mut(&mut point.offset), along, name)?;
                }
                Ok(())
            }
            Datum::PlaneThrough(PlaneThrough::SquareToCurve(station))
            | Datum::PointBy(PointBy::Along(station)) => self.length(&mut station.distance, name),
            Datum::PlaneThrough(_) | Datum::Axis(_) | Datum::PointBy(_) | Datum::Frame(_) => Ok(()),
        }
    }

    fn views(&self, document: &Document) -> Option<Edit> {
        let views = document.saved_views();
        if views.is_empty() {
            return None;
        }
        let mut scaled = views.clone();
        let views = scaled
            .named
            .iter_mut()
            .map(|named| &mut named.view)
            .chain(scaled.home.as_mut());
        for view in views {
            view.target = self.similarity.apply_point(view.target);
            view.distance *= self.factor;
        }
        Some(Edit::SetSavedViews {
            views: Box::new(scaled),
        })
    }
}

fn too_long(name: &str) -> ModelScaleError {
    ModelScaleError::TooLong {
        name: name.to_owned(),
    }
}

fn normalized(expression: Expression, name: &str) -> Result<Expression, ModelScaleError> {
    Expression::parse_stored(&expression.to_stored_text()).map_err(|_| too_long(name))
}

fn shifted(expression: Expression, shift: f64, name: &str) -> Result<Expression, ModelScaleError> {
    if shift == 0.0 {
        return Ok(expression);
    }
    let literal = expression
        .is_literal()
        .then(|| literal_unit(&expression))
        .flatten();
    if let Some(unit) = literal
        && let Ok(value) = expression.evaluate(&|_| Err(EvalError::ParameterMissing))
    {
        return Ok(match unit {
            Some(unit) => Expression::measure((value.value + shift) / unit.in_base_units(), unit),
            None => Expression::number(value.value + shift),
        });
    }
    let (operator, amount) = if shift < 0.0 {
        (BinaryOperator::Subtract, -shift)
    } else {
        (BinaryOperator::Add, shift)
    };
    normalized(
        Expression::binary(
            operator,
            expression,
            Expression::measure(amount, Unit::Millimetre),
        ),
        name,
    )
}

fn literal_unit(expression: &Expression) -> Option<Option<Unit>> {
    match expression {
        Expression::Number(_) => Some(None),
        Expression::Measure(_, unit) => Some(Some(*unit)),
        Expression::Negate(inner) => literal_unit(inner),
        _ => None,
    }
}

fn holds(geometry: PrincipalGeometry, centre: Point3) -> bool {
    match geometry {
        PrincipalGeometry::Origin => centre.length() <= CENTRE_TOLERANCE,
        PrincipalGeometry::Axis(axis) => {
            centre.cross(axis.direction()).length() <= CENTRE_TOLERANCE
        }
        PrincipalGeometry::Plane(plane) => {
            centre.dot(plane.plane().normal()).abs() <= CENTRE_TOLERANCE
        }
    }
}

pub fn principal_words(geometry: PrincipalGeometry) -> String {
    match geometry {
        PrincipalGeometry::Origin => "the origin".to_owned(),
        PrincipalGeometry::Axis(axis) => format!("the {}", axis.name()),
        PrincipalGeometry::Plane(plane) => format!("the {}", plane.name()),
    }
}

fn unused_name(document: &Document, stem: &str) -> String {
    let taken = |name: &str| document.features().any(|feature| feature.name == name);
    if !taken(stem) {
        return stem.to_owned();
    }
    (2..=document.features().len() + 2)
        .map(|number| format!("{stem} {number}"))
        .find(|name| !taken(name))
        .unwrap_or_else(|| format!("{stem} {}", document.next_feature_id()))
}

fn millimetres(value: f64) -> Expression {
    Expression::measure(value, Unit::Millimetre)
}

#[derive(Debug, Default)]
struct ScaledDatums {
    origin: Option<FeatureId>,
    frame: Option<FeatureId>,
    planes: BTreeMap<PrincipalPlane, FeatureId>,
}

struct DatumPlacing<'a> {
    document: &'a Document,
    next_id: u64,
    edits: Vec<Edit>,
    names: Vec<String>,
}

impl DatumPlacing<'_> {
    fn insert(&mut self, stem: &str, datum: Datum) -> FeatureId {
        let id = FeatureId::from_raw(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        let name = unused_name(self.document, stem);
        self.edits.push(Edit::InsertFeature {
            index: self.edits.len(),
            feature: Arc::new(Feature::new(id, name.clone(), FeatureKind::Datum(datum))),
        });
        self.names.push(name);
        id
    }
}

struct PlacedDatums {
    datums: ScaledDatums,
    edits: Vec<Edit>,
    names: Vec<String>,
}

fn placed_datums(
    document: &Document,
    displaced: &[DisplacedFeature],
    shift: Vector3,
) -> PlacedDatums {
    let projected: BTreeSet<PrincipalPlane> = displaced
        .iter()
        .filter(|moved| moved.projects)
        .flat_map(|moved| &moved.geometry)
        .filter_map(|geometry| match geometry {
            PrincipalGeometry::Plane(plane) => Some(*plane),
            PrincipalGeometry::Origin | PrincipalGeometry::Axis(_) => None,
        })
        .collect();
    let held = || {
        displaced
            .iter()
            .filter(|moved| !moved.projects)
            .flat_map(|moved| &moved.geometry)
    };
    let framed = held().any(|geometry| *geometry != PrincipalGeometry::Origin);
    let at_origin = framed || held().any(|geometry| *geometry == PrincipalGeometry::Origin);
    let mut placing = DatumPlacing {
        document,
        next_id: document.next_feature_id(),
        edits: Vec::new(),
        names: Vec::new(),
    };
    let mut datums = ScaledDatums::default();
    if at_origin {
        let origin = placing.insert(
            SCALED_ORIGIN,
            Datum::Point(DatumPoint {
                base: PointReference::Origin,
                offset: [shift.x, shift.y, shift.z].map(millimetres),
            }),
        );
        datums.origin = Some(origin);
        if framed {
            datums.frame = Some(placing.insert(
                SCALED_FRAME,
                Datum::Frame(Box::new(DatumFrame {
                    origin: PointReference::Datum(origin),
                    ..DatumFrame::world()
                })),
            ));
        }
    }
    for plane in projected {
        let id = placing.insert(
            &format!("{}, scaled", plane.name()),
            Datum::Plane(DatumPlane {
                base: PlaneReference::Principal(plane),
                rotation: None,
                offset: millimetres(shift.dot(plane.plane().normal())),
            }),
        );
        datums.planes.insert(plane, id);
    }
    PlacedDatums {
        datums,
        edits: placing.edits,
        names: placing.names,
    }
}

enum Anchored<'a> {
    Plane(&'a mut PlaneReference),
    Axis(&'a mut AxisReference),
    Point(&'a mut PointReference),
}

impl Anchored<'_> {
    fn geometry(&self) -> Option<PrincipalGeometry> {
        match self {
            Self::Plane(PlaneReference::Principal(plane)) => Some(PrincipalGeometry::Plane(*plane)),
            Self::Axis(AxisReference::Principal(axis)) => Some(PrincipalGeometry::Axis(*axis)),
            Self::Point(PointReference::Origin) => Some(PrincipalGeometry::Origin),
            Self::Plane(_) | Self::Axis(_) | Self::Point(_) => None,
        }
    }

    fn retarget(self, datums: &ScaledDatums) {
        match (self, datums.frame, datums.origin) {
            (Self::Plane(reference), Some(frame), _) => {
                if let PlaneReference::Principal(plane) = *reference {
                    *reference = PlaneReference::Frame { frame, plane };
                }
            }
            (Self::Axis(reference), Some(frame), _) => {
                if let AxisReference::Principal(axis) = *reference {
                    *reference = AxisReference::Frame { frame, axis };
                }
            }
            (Self::Point(reference), _, Some(origin)) => {
                if matches!(reference, PointReference::Origin) {
                    *reference = PointReference::Datum(origin);
                }
            }
            (Self::Plane(_) | Self::Axis(_), None, _) | (Self::Point(_), _, None) => {}
        }
    }
}

fn ends(extent: &mut ExtrudeExtent) -> Vec<&mut ExtrudeEnd> {
    match extent {
        ExtrudeExtent::OneSide { end, .. } => vec![end],
        ExtrudeExtent::TwoSides { forward, backward } => vec![forward, backward],
        ExtrudeExtent::Symmetric { .. } => Vec::new(),
    }
}

fn start_anchored(start: &mut Option<SolidStart>) -> Option<Anchored<'_>> {
    match start {
        Some(SolidStart::Plane(plane)) => Some(Anchored::Plane(plane)),
        Some(SolidStart::Distance(_)) | None => None,
    }
}

fn axis_pair(axes: &mut AxisMate) -> Vec<Anchored<'_>> {
    vec![
        Anchored::Axis(&mut axes.axis),
        Anchored::Axis(&mut axes.target),
    ]
}

fn mate_anchored(mate: &mut Mate) -> Vec<Anchored<'_>> {
    match &mut mate.pair {
        MatePair::Faces(faces) => vec![Anchored::Plane(&mut faces.target)],
        MatePair::Axes(axes) => axis_pair(axes),
        MatePair::FaceAxis(both) => {
            let FaceAxisMate { faces, axes } = &mut **both;
            std::iter::once(Anchored::Plane(&mut faces.target))
                .chain(axis_pair(axes))
                .collect()
        }
        MatePair::Angle(angle) => match &mut angle.sides {
            AngleSides::Faces(faces) => vec![Anchored::Plane(&mut faces.target)],
            AngleSides::Axes(axes) => axis_pair(axes),
        },
        MatePair::Tangent(pair) => vec![Anchored::Plane(&mut pair.target)],
        MatePair::Point(mated) => {
            let PointMate { point, target } = &mut **mated;
            let target = match target {
                PointTarget::Point(target) => Anchored::Point(target),
                PointTarget::Plane(target) => Anchored::Plane(target),
            };
            vec![Anchored::Point(point), target]
        }
    }
}

fn datum_anchored(datum: &mut Datum) -> Vec<Anchored<'_>> {
    match datum {
        Datum::Plane(plane) => match &mut plane.rotation {
            Some(rotation) => vec![
                Anchored::Plane(&mut plane.base),
                Anchored::Axis(&mut rotation.axis),
            ],
            None => Vec::new(),
        },
        Datum::Point(_) => Vec::new(),
        Datum::Frame(frame) => vec![Anchored::Point(&mut frame.origin)],
        Datum::PlaneThrough(through) => match through {
            PlaneThrough::Points(points) => points.iter_mut().map(Anchored::Point).collect(),
            PlaneThrough::Midway(first, second) => {
                vec![Anchored::Plane(first), Anchored::Plane(second)]
            }
            PlaneThrough::AxisAndPoint(axis, point) | PlaneThrough::NormalTo(axis, point) => {
                vec![Anchored::Axis(axis), Anchored::Point(point)]
            }
            PlaneThrough::Tangent(tangent) | PlaneThrough::TangentAt(tangent) => {
                vec![Anchored::Point(&mut tangent.toward)]
            }
            PlaneThrough::Lines(first, second) => {
                vec![Anchored::Axis(first), Anchored::Axis(second)]
            }
            PlaneThrough::SquareToCurve(_) => Vec::new(),
        },
        Datum::Axis(axis) => match axis {
            DatumAxis::Along(along) => vec![Anchored::Axis(along)],
            DatumAxis::Intersection(first, second) => {
                vec![Anchored::Plane(first), Anchored::Plane(second)]
            }
            DatumAxis::Points(first, second) => {
                vec![Anchored::Point(first), Anchored::Point(second)]
            }
            DatumAxis::NormalTo(plane, point) => {
                vec![Anchored::Plane(plane), Anchored::Point(point)]
            }
            DatumAxis::SquareToFace(tangent) => vec![Anchored::Point(&mut tangent.toward)],
        },
        Datum::PointBy(by) => match by {
            PointBy::LinesCross(first, second) => {
                vec![Anchored::Axis(first), Anchored::Axis(second)]
            }
            PointBy::AxisAndPlane(axis, plane) => {
                vec![Anchored::Axis(axis), Anchored::Plane(plane)]
            }
            PointBy::ThreePlanes(planes) => planes.iter_mut().map(Anchored::Plane).collect(),
            PointBy::Along(_) | PointBy::EdgeMiddle { .. } | PointBy::FaceCentre { .. } => {
                Vec::new()
            }
        },
    }
}

fn extrude_anchored(extrude: &mut Extrude) -> Vec<Anchored<'_>> {
    let Extrude {
        extent,
        start,
        direction,
        ..
    } = extrude;
    ends(extent)
        .into_iter()
        .filter_map(|end| match end {
            ExtrudeEnd::UpToFace { target, .. } => Some(Anchored::Plane(target)),
            ExtrudeEnd::Distance(_)
            | ExtrudeEnd::ThroughAll
            | ExtrudeEnd::UpToNext { .. }
            | ExtrudeEnd::UpToSurface { .. } => None,
        })
        .chain(start_anchored(start))
        .chain(direction.as_deref_mut().map(Anchored::Axis))
        .collect()
}

fn revolve_anchored(revolve: &mut Revolve) -> Vec<Anchored<'_>> {
    let Revolve {
        axis,
        start,
        extent,
        ..
    } = revolve;
    let axis = match axis {
        RevolveAxis::Model(axis) => Some(Anchored::Axis(axis)),
        RevolveAxis::Sketch(_) => None,
    };
    let target = match extent {
        RevolveExtent::UpTo { target, .. } => Some(Anchored::Plane(target)),
        RevolveExtent::Full
        | RevolveExtent::OneSide { .. }
        | RevolveExtent::Symmetric { .. }
        | RevolveExtent::TwoSides { .. } => None,
    };
    axis.into_iter()
        .chain(start_anchored(start))
        .chain(target)
        .collect()
}

fn anchored(kind: &mut FeatureKind) -> Vec<Anchored<'_>> {
    match kind {
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => extrude_anchored(extrude),
        FeatureKind::Solid(SolidFeature::Revolve(revolve)) => revolve_anchored(revolve),
        FeatureKind::Primitive(primitive) => vec![Anchored::Plane(&mut primitive.plane)],
        FeatureKind::Move(movement) => match &mut movement.about {
            TurnCentre::Axis(turn) => vec![Anchored::Axis(&mut turn.axis)],
            TurnCentre::Origin | TurnCentre::Body => Vec::new(),
        },
        FeatureKind::Mate(mate) => mate_anchored(mate),
        FeatureKind::Mirror(mirror) => vec![Anchored::Plane(&mut mirror.plane)],
        FeatureKind::Split(split) => split
            .along
            .plane_mut()
            .map(Anchored::Plane)
            .into_iter()
            .collect(),
        FeatureKind::SplitFace(split) => split
            .along
            .plane_mut()
            .map(Anchored::Plane)
            .into_iter()
            .collect(),
        FeatureKind::Pattern(pattern) => match &mut pattern.kind {
            PatternKind::Circular(circular) => vec![Anchored::Axis(&mut circular.axis)],
            PatternKind::Points(points) => vec![Anchored::Point(&mut points.base)],
            PatternKind::Linear { .. } | PatternKind::Curve(_) => Vec::new(),
        },
        FeatureKind::Datum(datum) => datum_anchored(datum),
        FeatureKind::Hole(hole) => match &mut hole.depth {
            HoleDepth::UpToFace { target, .. } => vec![Anchored::Plane(target)],
            HoleDepth::Blind(_) | HoleDepth::ThroughAll | HoleDepth::UpToNext { .. } => Vec::new(),
        },
        FeatureKind::Sketch(_)
        | FeatureKind::Blend(_)
        | FeatureKind::Shell(_)
        | FeatureKind::OffsetFace(_)
        | FeatureKind::Combine(_)
        | FeatureKind::Scale(_)
        | FeatureKind::Import(_)
        | FeatureKind::Remove(_)
        | FeatureKind::Thread(_)
        | FeatureKind::Measurement(_) => Vec::new(),
    }
}

fn projected_plane(source: &ProjectionSource) -> Option<PrincipalPlane> {
    match source {
        ProjectionSource::PrincipalPlane { plane, .. } => Some(*plane),
        ProjectionSource::Edge { .. }
        | ProjectionSource::Vertex { .. }
        | ProjectionSource::SketchEntity { .. }
        | ProjectionSource::Section { .. }
        | ProjectionSource::DatumPlane { .. } => None,
    }
}

fn displaced_geometry(kind: &FeatureKind, centre: Point3, used: bool) -> Vec<PrincipalGeometry> {
    let found: Vec<PrincipalGeometry> = match kind {
        FeatureKind::Sketch(sketch) => sketch
            .projections
            .values()
            .filter_map(projected_plane)
            .map(PrincipalGeometry::Plane)
            .collect(),
        _ => {
            let mut copy = kind.clone();
            anchored(&mut copy)
                .iter()
                .filter_map(Anchored::geometry)
                .collect()
        }
    };
    let mut displaced = Vec::new();
    for geometry in found {
        if !holds(geometry, centre) && !displaced.contains(&geometry) {
            displaced.push(geometry);
        }
    }
    if let Some(plane) = offset_from_principal(kind)
        && used
        && !holds(PrincipalGeometry::Origin, centre)
    {
        let geometry = PrincipalGeometry::Plane(plane);
        if !displaced.contains(&geometry) {
            displaced.push(geometry);
        }
    }
    displaced
}

fn offset_from_principal(kind: &FeatureKind) -> Option<PrincipalPlane> {
    match kind {
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(plane),
            rotation: None,
            ..
        })) => Some(*plane),
        _ => None,
    }
}
