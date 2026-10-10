use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use caditor_expression::{Dimension, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Point3, Vector3};
use caditor_kernel::{
    Accuracy, Axis, Curve, EdgeForm, EdgeId, EdgeReference, Element, FaceForm, FaceId,
    FaceReference, Interval, ReferenceError, Solid, angle, curve_measure, distance, edge_measure,
    face_area, face_form,
};
use caditor_sketch::EntityId;

use crate::{
    datum::{
        AxisReference, PlaneReference, PointReference, Resolver, describe_axis, describe_plane,
        describe_point, feature_name,
    },
    describe::describe_origin,
    document::{Document, Feature, FeatureId, FeatureKind},
    edit::{Edit, Transaction},
    origins, paste,
    recompute::{CancelToken, Evaluation, Failure, FeatureResult, FeatureState, Inputs},
    solid::profile_curve,
    values::{ParameterError, ParameterValues, evaluation_order},
};

#[derive(Debug, Clone, PartialEq)]
pub enum MeasuredItem {
    Point(PointReference),
    Axis(AxisReference),
    Plane(PlaneReference),
    Edge {
        body: FeatureId,
        edge: Box<EdgeReference>,
    },
    Face {
        body: FeatureId,
        face: FaceReference,
    },
    Sketch {
        sketch: FeatureId,
        entity: EntityId,
    },
}

impl MeasuredItem {
    pub fn heap_size(&self) -> usize {
        match self {
            Self::Point(point) => point.heap_size(),
            Self::Axis(axis) => axis.heap_size(),
            Self::Plane(plane) => plane.heap_size(),
            Self::Edge { .. } => size_of::<EdgeReference>(),
            Self::Face { face, .. } => face.heap_size(),
            Self::Sketch { .. } => 0,
        }
    }

    pub fn body(&self) -> Option<FeatureId> {
        match self {
            Self::Point(point) => point.body(),
            Self::Axis(axis) => axis.body(),
            Self::Plane(plane) => plane.body(),
            Self::Edge { body, .. } | Self::Face { body, .. } => Some(*body),
            Self::Sketch { .. } => None,
        }
    }

    pub fn sketch(&self) -> Option<FeatureId> {
        match self {
            Self::Point(point) => point.sketch(),
            Self::Axis(axis) => axis.sketch(),
            Self::Sketch { sketch, .. } => Some(*sketch),
            Self::Plane(_) | Self::Edge { .. } | Self::Face { .. } => None,
        }
    }

    pub fn frame(&self) -> Option<FeatureId> {
        match self {
            Self::Point(point) => point.frame(),
            Self::Axis(axis) => axis.frame(),
            Self::Plane(plane) => plane.frame(),
            Self::Edge { .. } | Self::Face { .. } | Self::Sketch { .. } => None,
        }
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let datum = match self {
            Self::Point(point) => point.datum(),
            Self::Axis(axis) => axis.datum(),
            Self::Plane(plane) => plane.datum(),
            Self::Edge { .. } | Self::Face { .. } | Self::Sketch { .. } => None,
        };
        [self.body(), self.sketch(), self.frame(), datum]
            .into_iter()
            .flatten()
            .collect()
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        match self {
            Self::Point(point) => point.origin_features(),
            Self::Axis(axis) => axis.origin_features(),
            Self::Plane(plane) => plane.origin_features(),
            Self::Edge { edge, .. } => origins::of_edge(edge),
            Self::Face { face, .. } => origins::of_face(face).into_iter().collect(),
            Self::Sketch { .. } => BTreeSet::new(),
        }
    }

    pub fn describe(&self, document: &Document) -> String {
        match self {
            Self::Point(point) => describe_point(document, point),
            Self::Axis(axis) => describe_axis(document, axis),
            Self::Plane(plane) => describe_plane(document, plane),
            Self::Edge { body, .. } => format!("an edge of {}", feature_name(document, *body)),
            Self::Face { face, .. } => describe_origin(document, face.origin()),
            Self::Sketch { sketch, entity } => {
                let label = document
                    .feature(*sketch)
                    .and_then(|feature| feature.kind.sketch())
                    .map_or_else(
                        || "a curve".to_owned(),
                        |definition| definition.entity_label(*entity),
                    );
                format!("{label} of {}", feature_name(document, *sketch))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Between {
    Distance,
    Angle,
}

impl Between {
    pub const ALL: [Self; 2] = [Self::Distance, Self::Angle];

    pub fn label(self) -> &'static str {
        match self {
            Self::Distance => "Distance",
            Self::Angle => "Angle",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Of {
    Length,
    Radius,
    Area,
    Sweep,
    Perimeter,
}

impl Of {
    pub const ALL: [Self; 5] = [
        Self::Length,
        Self::Radius,
        Self::Sweep,
        Self::Area,
        Self::Perimeter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Length => "Length",
            Self::Radius => "Radius",
            Self::Area => "Area",
            Self::Sweep => "Sweep",
            Self::Perimeter => "Perimeter",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reading {
    Between {
        quantity: Between,
        first: MeasuredItem,
        second: MeasuredItem,
    },
    Along {
        first: MeasuredItem,
        second: MeasuredItem,
        axis: MeasuredItem,
    },
    Of {
        quantity: Of,
        item: MeasuredItem,
    },
}

pub const ALONG: &str = "Along";

impl Reading {
    pub fn items(&self) -> Vec<&MeasuredItem> {
        match self {
            Self::Between { first, second, .. } => vec![first, second],
            Self::Along {
                first,
                second,
                axis,
            } => vec![first, second, axis],
            Self::Of { item, .. } => vec![item],
        }
    }

    pub fn items_mut(&mut self) -> Vec<&mut MeasuredItem> {
        match self {
            Self::Between { first, second, .. } => vec![first, second],
            Self::Along {
                first,
                second,
                axis,
            } => vec![first, second, axis],
            Self::Of { item, .. } => vec![item],
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Between { quantity, .. } => quantity.label(),
            Self::Along { .. } => ALONG,
            Self::Of { quantity, .. } => quantity.label(),
        }
    }

    pub fn describe(&self, document: &Document) -> String {
        match self {
            Self::Between {
                quantity,
                first,
                second,
            } => format!(
                "{} between {} and {}",
                quantity.label(),
                first.describe(document),
                second.describe(document)
            ),
            Self::Along {
                first,
                second,
                axis,
            } => format!(
                "Offset along {} between {} and {}",
                axis.describe(document),
                first.describe(document),
                second.describe(document)
            ),
            Self::Of { quantity, item } => {
                format!("{} of {}", quantity.label(), item.describe(document))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Measurement {
    pub reading: Reading,
    pub parameter: Option<ParameterId>,
}

impl Measurement {
    pub fn heap_size(&self) -> usize {
        self.reading
            .items()
            .into_iter()
            .map(MeasuredItem::heap_size)
            .sum()
    }

    pub fn bodies(&self) -> BTreeSet<FeatureId> {
        self.reading
            .items()
            .into_iter()
            .filter_map(MeasuredItem::body)
            .collect()
    }

    pub fn sketches(&self) -> BTreeSet<FeatureId> {
        self.reading
            .items()
            .into_iter()
            .filter_map(MeasuredItem::sketch)
            .collect()
    }

    pub fn frames(&self) -> BTreeSet<FeatureId> {
        self.reading
            .items()
            .into_iter()
            .filter_map(MeasuredItem::frame)
            .collect()
    }

    pub fn plane_datums(&self) -> BTreeSet<FeatureId> {
        self.reading
            .items()
            .into_iter()
            .filter_map(|item| match item {
                MeasuredItem::Plane(plane) => plane.datum(),
                _ => None,
            })
            .collect()
    }

    pub fn axis_datums(&self) -> BTreeSet<FeatureId> {
        self.reading
            .items()
            .into_iter()
            .filter_map(|item| match item {
                MeasuredItem::Axis(axis) => axis.datum(),
                _ => None,
            })
            .collect()
    }

    pub fn point_datums(&self) -> BTreeSet<FeatureId> {
        self.reading
            .items()
            .into_iter()
            .filter_map(|item| match item {
                MeasuredItem::Point(point) => point.datum(),
                _ => None,
            })
            .collect()
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = self.sketches();
        used.extend(self.plane_datums());
        used.extend(self.axis_datums());
        used.extend(self.point_datums());
        used.extend(self.frames());
        used.extend(self.bodies());
        used
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.reading
            .items()
            .into_iter()
            .flat_map(MeasuredItem::origin_features)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasurementResult {
    pub value: Quantity,
    pub line: Option<(Point3, Point3)>,
    pub anchor: Point3,
    pub accuracy: Accuracy,
}

enum Located<'a> {
    Point(Point3),
    Axis(Axis),
    Plane {
        origin: Point3,
        normal: Vector3,
    },
    Edge {
        solid: &'a Solid,
        edge: EdgeId,
    },
    Face {
        solid: &'a Solid,
        face: FaceId,
    },
    Curve {
        curve: Box<Curve>,
        interval: Interval,
    },
}

impl Located<'_> {
    fn element(&self) -> Element<'_> {
        match self {
            Self::Point(point) => Element::Point(*point),
            Self::Axis(axis) => Element::Axis(*axis),
            Self::Plane { origin, normal } => Element::Plane {
                origin: *origin,
                normal: *normal,
            },
            Self::Edge { solid, edge } => Element::Edge { solid, edge: *edge },
            Self::Face { solid, face } => Element::Face { solid, face: *face },
            Self::Curve { curve, interval } => Element::Curve {
                curve,
                interval: *interval,
            },
        }
    }

    fn anchor(&self) -> Point3 {
        match self {
            Self::Point(point) => *point,
            Self::Axis(axis) => axis.origin,
            Self::Plane { origin, .. } => *origin,
            Self::Edge { solid, edge } => solid.edge(*edge).map_or(Point3::ZERO, |definition| {
                definition.curve().point(definition.interval().middle())
            }),
            Self::Face { solid, face } => face_middle(solid, *face),
            Self::Curve { curve, interval } => curve.point(interval.middle()),
        }
    }
}

fn face_middle(solid: &Solid, face: FaceId) -> Point3 {
    let middles: Vec<Point3> = solid
        .face(face)
        .and_then(|definition| definition.outer_loop())
        .and_then(|outer| solid.face_loop(outer))
        .map(|outer| {
            outer
                .coedges()
                .iter()
                .filter_map(|coedge| solid.coedge(*coedge))
                .filter_map(|coedge| solid.edge(coedge.edge()))
                .map(|edge| edge.curve().point(edge.interval().middle()))
                .collect()
        })
        .unwrap_or_default();
    let count = middles.len().max(1) as f64;
    middles.iter().copied().sum::<Point3>() / count
}

struct Measuring<'a> {
    resolver: Resolver<'a>,
}

impl<'a> Measuring<'a> {
    fn document(&self) -> &'a Document {
        self.resolver.inputs.document
    }

    fn locate(&self, item: &MeasuredItem) -> Result<Located<'a>, Failure> {
        let document = self.document();
        Ok(match item {
            MeasuredItem::Point(point) => Located::Point(self.resolver.point(point)?),
            MeasuredItem::Axis(axis) => {
                let ray = self.resolver.axis(axis)?;
                Located::Axis(Axis {
                    origin: ray.origin(),
                    direction: ray.direction(),
                })
            }
            MeasuredItem::Plane(plane) => {
                let plane = self.resolver.plane(plane)?;
                Located::Plane {
                    origin: plane.origin(),
                    normal: plane.normal(),
                }
            }
            MeasuredItem::Edge { body, edge } => {
                let solid = self.body(*body)?;
                let name = feature_name(document, *body);
                match edge.resolve(solid) {
                    Ok(found) => Located::Edge { solid, edge: found },
                    Err(ReferenceError::Missing) => {
                        return Err(self.resolver.own_error(
                            format!("The edge it measures is no longer part of {name}."),
                            "Measure again with an edge that is there now, or delete this \
                             measurement.",
                        ));
                    }
                    Err(ReferenceError::Ambiguous(_)) => {
                        return Err(self.resolver.own_error(
                            format!(
                                "The edge of {name} it measures is now several edges, so it is \
                                 unclear which one to follow."
                            ),
                            "Measure again with the edge you mean.",
                        ));
                    }
                }
            }
            MeasuredItem::Face { body, face } => {
                let solid = self.body(*body)?;
                let described = describe_origin(document, face.origin());
                let name = feature_name(document, *body);
                match face.resolve(solid) {
                    Ok(found) => Located::Face { solid, face: found },
                    Err(ReferenceError::Missing) => {
                        return Err(self.resolver.own_error(
                            format!("{described} is no longer part of {name}."),
                            "Measure again with a face that is there now, or delete this \
                             measurement.",
                        ));
                    }
                    Err(ReferenceError::Ambiguous(_)) => {
                        return Err(self.resolver.own_error(
                            format!(
                                "{described} is now several faces of {name}, so it is unclear \
                                 which one to follow."
                            ),
                            "Measure again with the face you mean.",
                        ));
                    }
                }
            }
            MeasuredItem::Sketch { sketch, entity } => {
                let name = feature_name(document, *sketch);
                let result = self
                    .resolver
                    .inputs
                    .features
                    .get(sketch)
                    .and_then(|result| result.sketch())
                    .ok_or_else(|| {
                        self.resolver.error(
                            format!("It measures geometry of {name}, which has an error."),
                            format!("Fix {name} first."),
                            *sketch,
                        )
                    })?;
                let geometry = &result.geometry;
                let plane = geometry.plane();
                let gone = || {
                    self.resolver.own_error(
                        format!("The sketch geometry of {name} it measures no longer exists."),
                        "Measure again with geometry that is there now, or delete this \
                         measurement.",
                    )
                };
                if let Some(point) = geometry.point(*entity) {
                    Located::Point(plane.to_world(point))
                } else {
                    let (curve, interval) = profile_curve(geometry, *entity)
                        .and_then(|curve| curve.curve().ok())
                        .ok_or_else(gone)?;
                    let curve = curve.on_plane(&plane).map_err(|_| gone())?;
                    Located::Curve {
                        curve: Box::new(curve),
                        interval,
                    }
                }
            }
        })
    }

    fn body(&self, body: FeatureId) -> Result<&'a Solid, Failure> {
        self.resolver
            .inputs
            .body(body)
            .ok_or_else(|| self.resolver.inputs.missing_body(body))
    }

    fn unmeasurable(&self, reason: String) -> Failure {
        self.resolver.own_error(
            reason,
            "Measure something that has this value, or delete this measurement.",
        )
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    measurement: &Measurement,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let measuring = Measuring {
        resolver: Resolver { feature, inputs },
    };
    let document = inputs.document;
    let result = match &measurement.reading {
        Reading::Between {
            quantity,
            first,
            second,
        } => {
            let from = measuring.locate(first)?;
            let to = measuring.locate(second)?;
            if cancel.is_cancelled() {
                return Err(Failure::Cancelled);
            }
            let separation = distance(from.element(), to.element());
            let line = separation
                .as_ref()
                .ok()
                .map(|separation| (separation.from, separation.to));
            let anchor = line.map_or_else(
                || from.anchor().midpoint(to.anchor()),
                |(start, end)| start.midpoint(end),
            );
            let words = || {
                format!(
                    "{} and {}",
                    first.describe(document),
                    second.describe(document)
                )
            };
            match quantity {
                Between::Distance => {
                    let separation = separation.map_err(|_| {
                        measuring.unmeasurable(format!(
                            "The distance between {} could not be worked out.",
                            words()
                        ))
                    })?;
                    MeasurementResult {
                        value: Quantity::length(separation.distance),
                        line,
                        anchor,
                        accuracy: separation.accuracy,
                    }
                }
                Between::Angle => {
                    let found = angle(from.element(), to.element()).ok().flatten();
                    let found = found.ok_or_else(|| {
                        measuring.unmeasurable(format!(
                            "{} have no angle between them.",
                            capitalised(&words())
                        ))
                    })?;
                    MeasurementResult {
                        value: Quantity::angle(found.radians.to_degrees()),
                        line,
                        anchor,
                        accuracy: Accuracy::Exact,
                    }
                }
            }
        }
        Reading::Along {
            first,
            second,
            axis,
        } => {
            let from = measuring.locate(first)?;
            let to = measuring.locate(second)?;
            let direction = match measuring.locate(axis)? {
                Located::Axis(line) => line.direction.try_normalize(),
                _ => None,
            }
            .ok_or_else(|| {
                measuring.unmeasurable(format!(
                    "{} gives no direction, so nothing is measured along it.",
                    capitalised(&axis.describe(document))
                ))
            })?;
            if cancel.is_cancelled() {
                return Err(Failure::Cancelled);
            }
            let separation = distance(from.element(), to.element()).map_err(|_| {
                measuring.unmeasurable(format!(
                    "The offset between {} and {} could not be worked out.",
                    first.describe(document),
                    second.describe(document)
                ))
            })?;
            MeasurementResult {
                value: Quantity::length(separation.offset().dot(direction).abs()),
                line: Some((separation.from, separation.to)),
                anchor: separation.from.midpoint(separation.to),
                accuracy: separation.accuracy,
            }
        }
        Reading::Of { quantity, item } => {
            let located = measuring.locate(item)?;
            let described = item.describe(document);
            let anchor = located.anchor();
            let (value, accuracy) = match (quantity, &located) {
                (Of::Length, Located::Edge { solid, edge }) => {
                    let measured = edge_measure(solid, *edge).map_err(|_| {
                        measuring.unmeasurable(format!(
                            "The length of {described} could not be worked out."
                        ))
                    })?;
                    (Quantity::length(measured.length), measured.length_accuracy)
                }
                (Of::Length, Located::Curve { curve, interval }) => {
                    let measured = curve_measure(curve, *interval);
                    (Quantity::length(measured.length), measured.length_accuracy)
                }
                (Of::Radius, Located::Edge { solid, edge }) => {
                    match edge_measure(solid, *edge).map(|measured| measured.form) {
                        Ok(EdgeForm::Circle { radius, .. }) => {
                            (Quantity::length(radius), Accuracy::Exact)
                        }
                        _ => {
                            return Err(measuring.unmeasurable(format!(
                                "{} is no longer round, so it has no radius.",
                                capitalised(&described)
                            )));
                        }
                    }
                }
                (Of::Radius, Located::Curve { curve, interval }) => {
                    match curve_measure(curve, *interval).form {
                        EdgeForm::Circle { radius, .. } => {
                            (Quantity::length(radius), Accuracy::Exact)
                        }
                        _ => {
                            return Err(measuring.unmeasurable(format!(
                                "{} is no longer round, so it has no radius.",
                                capitalised(&described)
                            )));
                        }
                    }
                }
                (Of::Radius, Located::Face { solid, face }) => match face_form(solid, *face) {
                    Ok(FaceForm::Cylinder { radius, .. } | FaceForm::Sphere { radius, .. }) => {
                        (Quantity::length(radius), Accuracy::Exact)
                    }
                    _ => {
                        return Err(measuring.unmeasurable(format!(
                            "{} is no longer a cylinder or a sphere, so it has no radius.",
                            capitalised(&described)
                        )));
                    }
                },
                (Of::Area, Located::Face { solid, face }) => {
                    let area = face_area(solid, *face).ok().flatten().ok_or_else(|| {
                        measuring.unmeasurable(format!(
                            "The area of {described} could not be worked out."
                        ))
                    })?;
                    (Quantity::new(area, AREA), Accuracy::Exact)
                }
                (Of::Sweep, Located::Edge { solid, edge }) => {
                    match edge_measure(solid, *edge).map(|measured| measured.form) {
                        Ok(EdgeForm::Circle { sweep, .. }) => {
                            (Quantity::angle(sweep.to_degrees()), Accuracy::Exact)
                        }
                        _ => return Err(measuring.unmeasurable(no_sweep(&described))),
                    }
                }
                (Of::Sweep, Located::Curve { curve, interval }) => {
                    match curve_measure(curve, *interval).form {
                        EdgeForm::Circle { sweep, .. } => {
                            (Quantity::angle(sweep.to_degrees()), Accuracy::Exact)
                        }
                        _ => return Err(measuring.unmeasurable(no_sweep(&described))),
                    }
                }
                (Of::Perimeter, Located::Face { solid, face }) => {
                    let (length, accuracy) = face_perimeter(solid, *face).ok_or_else(|| {
                        measuring.unmeasurable(format!(
                            "The perimeter of {described} could not be worked out."
                        ))
                    })?;
                    (Quantity::length(length), accuracy)
                }
                (Of::Sweep, _) => {
                    return Err(measuring.unmeasurable(format!(
                        "{} is not an arc, so it has no sweep.",
                        capitalised(&described)
                    )));
                }
                (Of::Perimeter, _) => {
                    return Err(measuring.unmeasurable(format!(
                        "{} is not a face, so it has no perimeter.",
                        capitalised(&described)
                    )));
                }
                (Of::Length, _) => {
                    return Err(measuring.unmeasurable(format!(
                        "{} is not an edge or a curve, so it has no length.",
                        capitalised(&described)
                    )));
                }
                (Of::Radius, _) => {
                    return Err(measuring.unmeasurable(format!(
                        "{} is not round, so it has no radius.",
                        capitalised(&described)
                    )));
                }
                (Of::Area, _) => {
                    return Err(measuring.unmeasurable(format!(
                        "{} is not a face, so it has no area.",
                        capitalised(&described)
                    )));
                }
            };
            MeasurementResult {
                value,
                line: None,
                anchor,
                accuracy,
            }
        }
    };
    Ok(FeatureResult::Measurement(result))
}

const AREA: Dimension = Dimension::new(2, 0);

fn no_sweep(described: &str) -> String {
    format!(
        "{} is no longer an arc, so it has no sweep.",
        capitalised(described)
    )
}

pub fn face_perimeter(solid: &Solid, face: FaceId) -> Option<(f64, Accuracy)> {
    let outer = solid
        .face(face)?
        .loops()
        .first()
        .and_then(|id| solid.face_loop(*id))?;
    let mut length = 0.0;
    let mut accuracy = Accuracy::Exact;
    for coedge in outer.coedges() {
        let edge = solid.coedge(*coedge)?.edge();
        if is_seam_of(solid, edge, face) {
            continue;
        }
        let measured = edge_measure(solid, edge).ok()?;
        length += measured.length;
        if measured.length_accuracy == Accuracy::Approximate {
            accuracy = Accuracy::Approximate;
        }
    }
    Some((length, accuracy))
}

fn is_seam_of(solid: &Solid, edge: EdgeId, face: FaceId) -> bool {
    solid.edge(edge).is_some_and(|definition| {
        definition
            .coedges()
            .iter()
            .all(|coedge| solid.coedge_face(*coedge) == Some(face))
    })
}

pub fn reading_literal(value: Quantity) -> Option<Expression> {
    match value.dimension {
        AREA => Some(Expression::WithUnit(
            Box::new(Expression::number(value.value)),
            Unit::Millimetre,
            2,
        )),
        _ => paste::literal(value),
    }
}

fn capitalised(text: &str) -> String {
    let mut characters = text.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Measured {
    by_parameter: BTreeMap<ParameterId, FeatureId>,
    reads: BTreeMap<ParameterId, BTreeSet<FeatureId>>,
    derived: Vec<(ParameterId, Expression)>,
}

impl Measured {
    pub(crate) fn of(document: &Document) -> Self {
        let by_parameter: BTreeMap<ParameterId, FeatureId> = document
            .features()
            .filter_map(|feature| {
                let parameter = feature.kind.measurement()?.parameter?;
                Some((parameter, feature.id()))
            })
            .collect();
        if by_parameter.is_empty() {
            return Self::default();
        }
        let mut reads: BTreeMap<ParameterId, BTreeSet<FeatureId>> = BTreeMap::new();
        let mut derived = Vec::new();
        for id in evaluation_order(document) {
            if let Some(measurement) = by_parameter.get(&id) {
                reads.insert(id, BTreeSet::from([*measurement]));
                continue;
            }
            let Some(parameter) = document.parameter(id) else {
                continue;
            };
            let read: BTreeSet<FeatureId> = parameter
                .expression
                .parameters()
                .iter()
                .filter_map(|used| reads.get(used))
                .flatten()
                .copied()
                .collect();
            if !read.is_empty() {
                reads.insert(id, read);
                derived.push((id, parameter.expression.clone()));
            }
        }
        Self {
            by_parameter,
            reads,
            derived,
        }
    }

    pub(crate) fn measurement(&self, parameter: ParameterId) -> Option<FeatureId> {
        self.by_parameter.get(&parameter).copied()
    }

    pub(crate) fn pairs(&self) -> impl Iterator<Item = (ParameterId, FeatureId)> + '_ {
        self.by_parameter
            .iter()
            .map(|(parameter, feature)| (*parameter, *feature))
    }

    pub(crate) fn measurements_read(&self, parameter: ParameterId) -> BTreeSet<FeatureId> {
        self.reads.get(&parameter).cloned().unwrap_or_default()
    }

    pub(crate) fn reads_measurement(&self, parameter: ParameterId) -> bool {
        self.reads.contains_key(&parameter)
    }

    pub(crate) fn read_by(&self, kind: &FeatureKind) -> BTreeSet<FeatureId> {
        if self.reads.is_empty() {
            return BTreeSet::new();
        }
        kind.parameters()
            .into_iter()
            .filter_map(|parameter| self.reads.get(&parameter))
            .flatten()
            .copied()
            .collect()
    }

    pub(crate) fn features_read_by(&self, kind: &FeatureKind) -> BTreeSet<FeatureId> {
        let mut used = kind.features();
        used.extend(self.read_by(kind));
        used
    }

    pub(crate) fn dependencies_of(&self, kind: &FeatureKind) -> BTreeSet<FeatureId> {
        let mut used = kind.dependencies();
        used.extend(self.read_by(kind));
        used
    }

    pub(crate) fn derive(&self, parameters: &mut ParameterValues) {
        for (id, expression) in &self.derived {
            let value = parameters
                .evaluate_expression(expression)
                .map_err(ParameterError::Evaluation);
            parameters.set_measured(*id, value);
        }
    }

    pub(crate) fn overlay(
        &self,
        parameters: &ParameterValues,
        feature: &Feature,
        results: &BTreeMap<FeatureId, Arc<FeatureResult>>,
    ) -> Option<ParameterValues> {
        let reads = feature
            .parameters()
            .into_iter()
            .any(|parameter| self.reads_measurement(parameter));
        if !reads {
            return None;
        }
        let mut overlaid = parameters.clone();
        for (parameter, measurement) in self.pairs() {
            if let Some(reading) = results
                .get(&measurement)
                .and_then(|result| result.measurement())
            {
                overlaid.set_measured(parameter, Ok(reading.value));
            }
        }
        self.derive(&mut overlaid);
        Some(overlaid)
    }
}

pub(crate) fn reading_users(document: &Document, parameter: ParameterId) -> Vec<&Feature> {
    let mut reading = BTreeSet::from([parameter]);
    for id in evaluation_order(document) {
        let uses = document.parameter(id).is_some_and(|definition| {
            definition
                .expression
                .parameters()
                .iter()
                .any(|used| reading.contains(used))
        });
        if uses {
            reading.insert(id);
        }
    }
    document
        .features()
        .filter(|feature| {
            feature
                .kind
                .parameters()
                .iter()
                .any(|used| reading.contains(used))
        })
        .collect()
}

impl Document {
    pub fn measurement_of(&self, parameter: ParameterId) -> Option<&Feature> {
        self.features().find(|feature| {
            feature
                .kind
                .measurement()
                .is_some_and(|measurement| measurement.parameter == Some(parameter))
        })
    }

    pub fn dependencies_of(&self, kind: &FeatureKind) -> BTreeSet<FeatureId> {
        Measured::of(self).dependencies_of(kind)
    }

    pub(crate) fn parameter_path(&self, from: ParameterId, to: ParameterId) -> Option<Vec<String>> {
        let mut seen = BTreeSet::new();
        self.path_between(from, to, &mut seen).map(|path| {
            path.into_iter()
                .map(|id| self.parameter_name(id).unwrap_or("?").to_owned())
                .collect()
        })
    }

    fn path_between(
        &self,
        from: ParameterId,
        to: ParameterId,
        seen: &mut BTreeSet<ParameterId>,
    ) -> Option<Vec<ParameterId>> {
        if from == to {
            return Some(vec![to]);
        }
        if !seen.insert(from) {
            return None;
        }
        let used = self.parameter(from)?.expression.parameters();
        used.into_iter().find_map(|next| {
            let mut path = self.path_between(next, to, seen)?;
            path.insert(0, from);
            Some(path)
        })
    }

    pub fn following_readings(&self, evaluation: &Evaluation) -> Transaction {
        let edits = Measured::of(self)
            .pairs()
            .filter_map(|(parameter, measurement)| {
                let status = evaluation.feature(measurement)?;
                if status.state != FeatureState::UpToDate {
                    return None;
                }
                let reading = status.result.as_deref()?.measurement()?.value;
                let expression = reading_literal(reading)?;
                let stored = self.parameter(parameter)?;
                (stored.expression != expression).then_some(Edit::SetParameterExpression {
                    id: parameter,
                    expression,
                })
            })
            .collect();
        Transaction::new(FOLLOWING_READINGS, edits)
    }

    pub(crate) fn keeping_readings(
        &self,
        transaction: Transaction,
        doomed: &BTreeSet<FeatureId>,
    ) -> Transaction {
        let readings: Vec<(ParameterId, Expression)> = self
            .features()
            .filter(|feature| doomed.contains(&feature.id()))
            .filter_map(|feature| feature.kind.measurement()?.parameter)
            .filter_map(|parameter| {
                let stored = self.parameter(parameter)?;
                Some((parameter, stored.expression.clone()))
            })
            .collect();
        if readings.is_empty() {
            return transaction;
        }
        let (label, mut edits) = transaction.into_parts();
        let removed: BTreeSet<ParameterId> = edits
            .iter()
            .filter_map(|edit| match edit {
                Edit::RemoveParameter { id } => Some(*id),
                _ => None,
            })
            .collect();
        edits.extend(
            readings
                .into_iter()
                .filter(|(parameter, _)| !removed.contains(parameter))
                .map(|(id, expression)| Edit::SetParameterExpression { id, expression }),
        );
        Transaction::new(label, edits)
    }
}

const FOLLOWING_READINGS: &str = "Follow the measured readings";
