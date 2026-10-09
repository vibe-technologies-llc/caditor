use std::collections::BTreeMap;

use caditor_expression::{
    BinaryOperator, Dimension, EvalError, Expression, ParameterId, Quantity, Unit, format_number,
};
use caditor_geometry::{Point3, Similarity, Vector3};
use caditor_sketch::{Constraint, Entity};

use crate::{
    attachment::SketchFeature,
    blend::ChamferForm,
    datum::{
        AxisReference, Datum, PlaneReference, PlaneThrough, PointBy, PointReference, PrincipalAxis,
        PrincipalPlane,
    },
    document::{Document, Feature, FeatureKind},
    edit::{Edit, Transaction},
    hole::{HoleShape, HoleStyle},
    mate::MatePair,
    movement::{MoveAxis, Pivot, TurnCentre},
    pattern::PatternKind,
    primitive::SizeRule,
    projection::ProjectionSource,
    scaling::{MAX_SCALE_FACTOR, MIN_SCALE_FACTOR},
    solid::{ExtrudeEnd, ExtrudeExtent, RevolveAxis, SolidFeature, SolidStart},
    thread::ThreadLength,
    values::{ParameterValues, evaluation_order},
};

const CENTRE_TOLERANCE: f64 = 1e-9;
const AGREEMENT: f64 = 1e-9;

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
    #[error(
        "{feature} uses {geometry}, which stays where it is, so scaling about a point off it \
         would part {feature} from the rest of the model."
    )]
    CentreOffPrincipal { feature: String, geometry: String },
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
            Self::CentreOffPrincipal { geometry, .. } => {
                format!("Scale about a point on {geometry}, such as the origin.")
            }
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
        if scale.centre.length() > CENTRE_TOLERANCE {
            for feature in self.features() {
                check_anchors(feature, scale.centre)?;
            }
        }
        let mut rescaler = Rescaler {
            factor,
            shift: similarity.apply_point(Point3::ZERO),
            similarity,
            values: scale.values,
            old: ParameterValues::evaluate(self),
            scaled: BTreeMap::new(),
            new: BTreeMap::new(),
            summary: ScaleSummary::default(),
        };
        let mut edits = rescaler.parameters(self)?;
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
        Ok(ScaledModel {
            transaction: Transaction::new(
                format!("Scale model by {}", format_number(factor)),
                edits,
            ),
            summary: rescaler.summary,
        })
    }
}

struct Rescaler {
    factor: f64,
    shift: Vector3,
    similarity: Similarity,
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
                    ProjectionSource::PrincipalPlane {
                        plane: *plane,
                        reach: reach * self.factor,
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
                if let MatePair::Faces(faces) = &mut mate.pair {
                    self.length(&mut faces.distance, name)?;
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
                    PlaneReference::Principal(plane) => {
                        let plane = plane.plane();
                        [
                            self.shift.dot(plane.x_axis()),
                            self.shift.dot(plane.y_axis()),
                        ]
                    }
                    PlaneReference::Datum(_)
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
            FeatureKind::Pattern(pattern) => {
                if let PatternKind::Linear { first, second } = &mut pattern.kind {
                    self.length(&mut first.spacing, name)?;
                    if let Some(second) = second {
                        self.length(&mut second.spacing, name)?;
                    }
                }
            }
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
            | FeatureKind::Remove(_) => {}
        }
        Ok(if kind == feature.kind {
            Vec::new()
        } else {
            vec![Edit::SetFeatureKind {
                id: feature.id(),
                kind,
            }]
        })
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
                let shift = match (&plane.base, &plane.rotation) {
                    (PlaneReference::Principal(principal), None) => {
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

enum Anchor {
    Origin,
    Axis(PrincipalAxis),
    Plane(PrincipalPlane),
}

impl Anchor {
    fn holds(&self, centre: Point3) -> bool {
        match self {
            Self::Origin => centre.length() <= CENTRE_TOLERANCE,
            Self::Axis(axis) => centre.cross(axis.direction()).length() <= CENTRE_TOLERANCE,
            Self::Plane(plane) => centre.dot(plane.plane().normal()).abs() <= CENTRE_TOLERANCE,
        }
    }

    fn name(&self) -> String {
        match self {
            Self::Origin => "the origin".to_owned(),
            Self::Axis(axis) => format!("the {}", axis.name()),
            Self::Plane(plane) => format!("the {}", plane.name()),
        }
    }
}

fn plane_anchor(plane: &PlaneReference) -> Option<Anchor> {
    match plane {
        PlaneReference::Principal(plane) => Some(Anchor::Plane(*plane)),
        PlaneReference::Datum(_) | PlaneReference::Face(_) | PlaneReference::Frame { .. } => None,
    }
}

fn axis_anchor(axis: &AxisReference) -> Option<Anchor> {
    match axis {
        AxisReference::Principal(axis) => Some(Anchor::Axis(*axis)),
        AxisReference::Datum(_)
        | AxisReference::Edge { .. }
        | AxisReference::Face { .. }
        | AxisReference::Sketch { .. }
        | AxisReference::Frame { .. } => None,
    }
}

fn point_anchor(point: &PointReference) -> Option<Anchor> {
    matches!(point, PointReference::Origin).then_some(Anchor::Origin)
}

fn start_anchor(start: Option<&SolidStart>) -> Option<Anchor> {
    match start {
        Some(SolidStart::Plane(plane)) => plane_anchor(plane),
        Some(SolidStart::Distance(_)) | None => None,
    }
}

fn end_anchor(end: &ExtrudeEnd) -> Option<Anchor> {
    match end {
        ExtrudeEnd::UpToFace { target, .. } => plane_anchor(target),
        ExtrudeEnd::Distance(_)
        | ExtrudeEnd::ThroughAll
        | ExtrudeEnd::UpToNext { .. }
        | ExtrudeEnd::UpToSurface { .. } => None,
    }
}

fn anchors(kind: &FeatureKind) -> Vec<Anchor> {
    match kind {
        FeatureKind::Sketch(sketch) => sketch
            .projections
            .values()
            .filter_map(|source| match source {
                ProjectionSource::PrincipalPlane { plane, .. } => Some(Anchor::Plane(*plane)),
                _ => None,
            })
            .collect(),
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => {
            let ends = match &extrude.extent {
                ExtrudeExtent::OneSide { end, .. } => vec![end],
                ExtrudeExtent::TwoSides { forward, backward } => vec![forward, backward],
                ExtrudeExtent::Symmetric { .. } => Vec::new(),
            };
            ends.into_iter()
                .filter_map(end_anchor)
                .chain(start_anchor(extrude.start.as_ref()))
                .chain(extrude.direction.as_deref().and_then(axis_anchor))
                .collect()
        }
        FeatureKind::Solid(SolidFeature::Revolve(revolve)) => {
            let axis = match &revolve.axis {
                RevolveAxis::Model(axis) => axis_anchor(axis),
                RevolveAxis::Sketch(_) => None,
            };
            axis.into_iter()
                .chain(start_anchor(revolve.start.as_ref()))
                .chain(revolve.extent.target().and_then(plane_anchor))
                .collect()
        }
        FeatureKind::Primitive(primitive) => plane_anchor(&primitive.plane).into_iter().collect(),
        FeatureKind::Move(movement) => movement
            .about
            .axis()
            .and_then(axis_anchor)
            .into_iter()
            .collect(),
        FeatureKind::Mate(mate) => match &mate.pair {
            MatePair::Faces(faces) => plane_anchor(&faces.target).into_iter().collect(),
            MatePair::Axes(axes) => axis_anchor(&axes.axis)
                .into_iter()
                .chain(axis_anchor(&axes.target))
                .collect(),
        },
        FeatureKind::Mirror(mirror) => plane_anchor(&mirror.plane).into_iter().collect(),
        FeatureKind::Split(split) => split
            .along
            .plane()
            .and_then(plane_anchor)
            .into_iter()
            .collect(),
        FeatureKind::Pattern(pattern) => match &pattern.kind {
            PatternKind::Circular(circular) => axis_anchor(&circular.axis).into_iter().collect(),
            PatternKind::Linear { .. } => Vec::new(),
        },
        FeatureKind::Datum(datum) => datum_anchors(datum),
        FeatureKind::Hole(hole) => hole
            .depth
            .target()
            .and_then(plane_anchor)
            .into_iter()
            .collect(),
        FeatureKind::Blend(_)
        | FeatureKind::Shell(_)
        | FeatureKind::OffsetFace(_)
        | FeatureKind::Combine(_)
        | FeatureKind::Scale(_)
        | FeatureKind::Import(_)
        | FeatureKind::Remove(_)
        | FeatureKind::Thread(_) => Vec::new(),
    }
}

fn datum_anchors(datum: &Datum) -> Vec<Anchor> {
    match datum {
        Datum::Plane(plane) => match &plane.rotation {
            Some(rotation) => plane_anchor(&plane.base)
                .into_iter()
                .chain(axis_anchor(&rotation.axis))
                .collect(),
            None => Vec::new(),
        },
        Datum::Point(_) => Vec::new(),
        Datum::Frame(frame) => point_anchor(&frame.origin).into_iter().collect(),
        Datum::PlaneThrough(_) | Datum::Axis(_) | Datum::PointBy(_) => datum
            .planes()
            .into_iter()
            .filter_map(plane_anchor)
            .chain(datum.axes().into_iter().filter_map(axis_anchor))
            .chain(datum.points().into_iter().filter_map(point_anchor))
            .collect(),
    }
}

fn check_anchors(feature: &Feature, centre: Point3) -> Result<(), ModelScaleError> {
    match anchors(&feature.kind)
        .into_iter()
        .find(|anchor| !anchor.holds(centre))
    {
        Some(anchor) => Err(ModelScaleError::CentreOffPrincipal {
            feature: feature.name.clone(),
            geometry: anchor.name(),
        }),
        None => Ok(()),
    }
}
