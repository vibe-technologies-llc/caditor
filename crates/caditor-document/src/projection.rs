use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    f64::consts::TAU,
};

use caditor_geometry::{Plane, Point2, Point3};
use caditor_kernel::{
    BooleanError, Curve, EdgeId, EdgeReference, ReferenceError, Solid, VertexId, VertexName,
    vertex_names,
};
use caditor_sketch::{ArcGeometry, BSpline, Entity, EntityId, Sketch};

use crate::{
    attachment::SketchFeature,
    document::{Feature, FeatureId},
    edit::{Edit, TransactionBuilder},
    origins,
    pieces::pieces_of_one_edge,
    recompute::{Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    section::{SectionError, datum_outline, section_solid},
    tolerance::{self, POSITION_TOLERANCE},
};

pub const PROJECTED_SPLINE_POINTS: usize = 33;

#[derive(Debug, Clone, PartialEq)]
pub enum ProjectionSource {
    Edge {
        body: FeatureId,
        edge: EdgeReference,
    },
    Vertex {
        body: FeatureId,
        vertex: VertexName,
    },
    SketchEntity {
        sketch: FeatureId,
        entity: EntityId,
    },
    Section {
        body: FeatureId,
        edge: EdgeReference,
    },
    DatumPlane {
        datum: FeatureId,
        reach: f64,
    },
}

impl ProjectionSource {
    pub fn body(&self) -> Option<FeatureId> {
        match self {
            Self::Edge { body, .. } | Self::Vertex { body, .. } | Self::Section { body, .. } => {
                Some(*body)
            }
            Self::SketchEntity { .. } | Self::DatumPlane { .. } => None,
        }
    }

    pub fn sketch(&self) -> Option<FeatureId> {
        match self {
            Self::SketchEntity { sketch, .. } => Some(*sketch),
            Self::Edge { .. }
            | Self::Vertex { .. }
            | Self::Section { .. }
            | Self::DatumPlane { .. } => None,
        }
    }

    pub fn datum(&self) -> Option<FeatureId> {
        match self {
            Self::DatumPlane { datum, .. } => Some(*datum),
            Self::Edge { .. }
            | Self::Vertex { .. }
            | Self::SketchEntity { .. }
            | Self::Section { .. } => None,
        }
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        match self {
            Self::Edge { edge, .. } | Self::Section { edge, .. } => origins::of_edge(edge),
            Self::Vertex { .. } | Self::SketchEntity { .. } | Self::DatumPlane { .. } => {
                BTreeSet::new()
            }
        }
    }

    pub fn feature(&self) -> FeatureId {
        match self {
            Self::Edge { body, .. } | Self::Vertex { body, .. } | Self::Section { body, .. } => {
                *body
            }
            Self::SketchEntity { sketch, .. } => *sketch,
            Self::DatumPlane { datum, .. } => *datum,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outline {
    Point(Point2),
    Line {
        start: Point2,
        end: Point2,
    },
    Circle {
        center: Point2,
        radius: f64,
    },
    Arc {
        center: Point2,
        start: Point2,
        end: Point2,
    },
    Spline(Vec<Point2>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Point,
    Line,
    Circle,
    Arc,
    Spline(usize),
}

impl Shape {
    fn of(entity: &Entity) -> Self {
        match entity {
            Entity::Point(_) => Self::Point,
            Entity::Line { .. } => Self::Line,
            Entity::Circle { .. } => Self::Circle,
            Entity::Arc { .. } => Self::Arc,
            Entity::Spline { control_points } => Self::Spline(control_points.len()),
        }
    }
}

impl Outline {
    fn shape(&self) -> Shape {
        match self {
            Self::Point(_) => Shape::Point,
            Self::Line { .. } => Shape::Line,
            Self::Circle { .. } => Shape::Circle,
            Self::Arc { .. } => Shape::Arc,
            Self::Spline(points) => Shape::Spline(points.len()),
        }
    }

    fn through(samples: &[Point2]) -> Option<Self> {
        let (first, last) = (*samples.first()?, *samples.last()?);
        let extent = samples
            .iter()
            .map(|sample| sample.distance(first))
            .fold(0.0, f64::max);
        if extent <= POSITION_TOLERANCE {
            return Some(Self::Point(first));
        }
        let farthest = samples
            .iter()
            .copied()
            .max_by(|a, b| a.distance(first).total_cmp(&b.distance(first)))?;
        let direction = (farthest - first).try_normalize()?;
        let straight = samples
            .iter()
            .all(|sample| direction.perp_dot(*sample - first).abs() <= POSITION_TOLERANCE);
        if straight {
            let along = |point: &Point2| direction.dot(*point - first);
            let low = samples
                .iter()
                .copied()
                .min_by(|a, b| along(a).total_cmp(&along(b)))?;
            let high = samples
                .iter()
                .copied()
                .max_by(|a, b| along(a).total_cmp(&along(b)))?;
            let (start, end) = if first.distance(low) <= last.distance(low) {
                (low, high)
            } else {
                (high, low)
            };
            return Some(Self::Line { start, end });
        }
        Self::spline_through(samples)
    }

    fn spline_through(samples: &[Point2]) -> Option<Self> {
        let spline = BSpline::interpolate(samples)?;
        Some(Self::Spline(spline.control_points().to_vec()))
    }

    fn fitted(
        self,
        wanted: Option<Shape>,
        samples: impl FnOnce(usize) -> Vec<Point2>,
    ) -> Option<Self> {
        match wanted {
            None => Some(self),
            Some(shape) if shape == self.shape() => Some(self),
            Some(Shape::Spline(count)) => Self::spline_through(&samples(count)),
            Some(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionError {
    SourceUnavailable,
    Missing,
    Ambiguous,
    ChangedShape,
    NotCut,
    Uncuttable,
    Parallel,
    Cancelled,
}

type Sections = BTreeMap<FeatureId, Result<Solid, ProjectionError>>;

pub fn edge_outline(solid: &Solid, edge: EdgeId, plane: &Plane) -> Option<Outline> {
    curve_outline(solid, &[edge], plane, None)
}

pub fn vertex_outline(solid: &Solid, vertex: VertexId, plane: &Plane) -> Option<Outline> {
    let point = solid.vertex(vertex)?.point();
    Some(Outline::Point(plane.to_local(point)))
}

pub fn sketch_outline(source: &Sketch, entity: EntityId, plane: &Plane) -> Option<Outline> {
    sketch_entity_outline(source, entity, plane, None)
}

fn curve_outline(
    solid: &Solid,
    pieces: &[EdgeId],
    plane: &Plane,
    wanted: Option<Shape>,
) -> Option<Outline> {
    let edges: Vec<_> = pieces
        .iter()
        .map(|id| solid.edge(*id))
        .collect::<Option<_>>()?;
    let first = *edges.first()?;
    let local = |point: Point3| plane.to_local(point);
    let ends: Vec<Point2> = edges
        .iter()
        .flat_map(|edge| {
            let interval = edge.interval();
            [
                local(edge.curve().point(interval.start())),
                local(edge.curve().point(interval.end())),
            ]
        })
        .collect();
    let sampled = |count: usize| -> Vec<Point2> {
        let interval = first.interval();
        let last = count.saturating_sub(1).max(1) as f64;
        (0..count)
            .map(|index| {
                let t = index as f64 / last;
                local(
                    first
                        .curve()
                        .point(interval.start() + (interval.end() - interval.start()) * t),
                )
            })
            .collect()
    };
    let natural = match first.curve() {
        Curve::Line(_) => Outline::through(&ends)?,
        Curve::Circle(circle) if edges.len() == 1 => {
            let alignment = circle.frame().normal().dot(plane.normal());
            if tolerance::parallel(circle.frame().normal(), plane.normal()) {
                let center = local(circle.center());
                let interval = first.interval();
                if first.is_closed() && interval.length() >= TAU - tolerance::DIRECTION_TOLERANCE {
                    Outline::Circle {
                        center,
                        radius: circle.radius(),
                    }
                } else {
                    let (start, end) = (*ends.first()?, *ends.get(1)?);
                    let (start, end) = if alignment > 0.0 {
                        (start, end)
                    } else {
                        (end, start)
                    };
                    Outline::Arc { center, start, end }
                }
            } else {
                Outline::through(&sampled(PROJECTED_SPLINE_POINTS))?
            }
        }
        _ if edges.len() == 1 => Outline::through(&sampled(PROJECTED_SPLINE_POINTS))?,
        _ => return None,
    };
    natural.fitted(wanted, sampled)
}

fn sketch_entity_outline(
    source: &Sketch,
    entity: EntityId,
    plane: &Plane,
    wanted: Option<Shape>,
) -> Option<Outline> {
    let from = source.plane();
    let local = |point: Point2| plane.to_local(from.to_world(point));
    let aligned = tolerance::parallel(from.normal(), plane.normal());
    let same_turn = from.normal().dot(plane.normal()) > 0.0;
    let arc_samples = |arc: ArcGeometry, count: usize| -> Vec<Point2> {
        let last = count.saturating_sub(1).max(1) as f64;
        (0..count)
            .map(|index| local(arc.point_at(arc.start_angle + arc.sweep * index as f64 / last)))
            .collect()
    };
    let natural = match source.entity(entity)? {
        Entity::Point(position) => Outline::Point(local(*position)),
        Entity::Line { .. } => {
            let (start, end) = source.line_endpoints(entity)?;
            Outline::through(&[local(start), local(end)])?
        }
        Entity::Circle { .. } => {
            let (center, radius) = source.circle(entity)?;
            let full = ArcGeometry::full_circle(center, radius);
            if aligned {
                Outline::Circle {
                    center: local(center),
                    radius,
                }
            } else {
                Outline::through(&arc_samples(full, PROJECTED_SPLINE_POINTS))?
            }
        }
        Entity::Arc { .. } => {
            let arc = source.arc(entity)?;
            if aligned {
                let (start, end) = (
                    local(arc.point_at(arc.start_angle)),
                    local(arc.point_at(arc.end_angle())),
                );
                let (start, end) = if same_turn {
                    (start, end)
                } else {
                    (end, start)
                };
                Outline::Arc {
                    center: local(arc.center),
                    start,
                    end,
                }
            } else {
                Outline::through(&arc_samples(arc, PROJECTED_SPLINE_POINTS))?
            }
        }
        Entity::Spline { .. } => {
            let spline = source.spline(entity)?;
            Outline::Spline(spline.control_points().iter().copied().map(local).collect())
        }
    };
    let spline = source.spline(entity);
    let circle = source.circle(entity);
    let arc = source.arc(entity);
    natural.fitted(wanted, |count| {
        let last = count.saturating_sub(1).max(1) as f64;
        match (spline, arc, circle) {
            (Some(spline), _, _) => (0..count)
                .map(|index| local(spline.point_at(index as f64 / last)))
                .collect(),
            (None, Some(arc), _) => arc_samples(arc, count),
            (None, None, Some((center, radius))) => {
                arc_samples(ArcGeometry::full_circle(center, radius), count)
            }
            (None, None, None) => Vec::new(),
        }
    })
}

impl TransactionBuilder<'_> {
    pub fn add_projection(
        &mut self,
        feature: FeatureId,
        source: ProjectionSource,
        outline: &Outline,
    ) -> EntityId {
        let point = |builder: &mut Self, position: Point2| {
            builder.add_sketch_entity(feature, Entity::Point(position))
        };
        let projected = match outline {
            Outline::Point(position) => point(self, *position),
            Outline::Line { start, end } => {
                let start = point(self, *start);
                let end = point(self, *end);
                self.add_sketch_entity(feature, Entity::Line { start, end })
            }
            Outline::Circle { center, radius } => {
                let center = point(self, *center);
                self.add_sketch_entity(
                    feature,
                    Entity::Circle {
                        center,
                        radius: *radius,
                    },
                )
            }
            Outline::Arc { center, start, end } => {
                let center = point(self, *center);
                let start = point(self, *start);
                let end = point(self, *end);
                self.add_sketch_entity(feature, Entity::Arc { center, start, end })
            }
            Outline::Spline(control) => {
                let control_points = control
                    .iter()
                    .map(|position| point(self, *position))
                    .collect();
                self.add_sketch_entity(feature, Entity::Spline { control_points })
            }
        };
        self.edits.push(Edit::SetSketchProjection {
            feature,
            id: projected,
            source: Some(source),
        });
        projected
    }
}

pub(crate) fn refreshed<'a>(
    feature: &Feature,
    definition: &'a SketchFeature,
    plane: &Plane,
    inputs: &Inputs<'_>,
) -> Result<Cow<'a, Sketch>, Failure> {
    if definition.projections.is_empty() {
        return Ok(Cow::Borrowed(&definition.sketch));
    }
    let mut sketch = definition.sketch.clone();
    let mut sections = Sections::new();
    for (entity, source) in &definition.projections {
        let Some(existing) = sketch.entity(*entity) else {
            continue;
        };
        let wanted = Shape::of(existing);
        let outline = outline_of(source, plane, wanted, feature.id(), inputs, &mut sections)
            .map_err(|error| match error {
                ProjectionError::Cancelled => Failure::Cancelled,
                error => projection_failure(feature, &sketch, *entity, source, error, inputs),
            })?;
        place(&mut sketch, *entity, &outline).map_err(|()| {
            projection_failure(
                feature,
                &sketch,
                *entity,
                source,
                ProjectionError::ChangedShape,
                inputs,
            )
        })?;
    }
    Ok(Cow::Owned(sketch))
}

fn edge_in(
    solid: &Solid,
    edge: &EdgeReference,
    plane: &Plane,
    wanted: Shape,
) -> Result<Outline, ProjectionError> {
    let pieces = match edge.resolve(solid) {
        Ok(edge) => vec![edge],
        Err(ReferenceError::Ambiguous(pieces)) if pieces_of_one_edge(solid, &pieces) => pieces,
        Err(ReferenceError::Ambiguous(_)) => return Err(ProjectionError::Ambiguous),
        Err(ReferenceError::Missing) => return Err(ProjectionError::Missing),
    };
    curve_outline(solid, &pieces, plane, Some(wanted)).ok_or(ProjectionError::ChangedShape)
}

fn section_of<'a>(
    body: FeatureId,
    plane: &Plane,
    sketch: FeatureId,
    inputs: &Inputs<'_>,
    sections: &'a mut Sections,
) -> Result<&'a Solid, ProjectionError> {
    let section = sections.entry(body).or_insert_with(|| {
        let solid = inputs
            .body(body)
            .ok_or(ProjectionError::SourceUnavailable)?;
        section_solid(solid, plane, sketch).map_err(|error| match error {
            SectionError::Misses => ProjectionError::NotCut,
            SectionError::Boolean(BooleanError::Cancelled(_)) => ProjectionError::Cancelled,
            SectionError::HalfSpace(_) | SectionError::Boolean(_) => ProjectionError::Uncuttable,
        })
    });
    section.as_ref().map_err(|error| *error)
}

fn outline_of(
    source: &ProjectionSource,
    plane: &Plane,
    wanted: Shape,
    sketch: FeatureId,
    inputs: &Inputs<'_>,
    sections: &mut Sections,
) -> Result<Outline, ProjectionError> {
    match source {
        ProjectionSource::Edge { body, edge } => {
            let solid = inputs
                .body(*body)
                .ok_or(ProjectionError::SourceUnavailable)?;
            edge_in(solid, edge, plane, wanted)
        }
        ProjectionSource::Section { body, edge } => {
            let section = section_of(*body, plane, sketch, inputs, sections)?;
            match edge_in(section, edge, plane, wanted) {
                Err(ProjectionError::Missing) => Err(ProjectionError::NotCut),
                outcome => outcome,
            }
        }
        ProjectionSource::DatumPlane { datum, reach } => {
            let datum = inputs
                .features
                .get(datum)
                .and_then(|result| match result.as_ref() {
                    FeatureResult::Datum(result) => result.plane(),
                    _ => None,
                })
                .ok_or(ProjectionError::SourceUnavailable)?;
            datum_outline(&datum, plane, *reach)
                .filter(|outline| outline.shape() == wanted)
                .ok_or(ProjectionError::Parallel)
        }
        ProjectionSource::Vertex { body, vertex } => {
            let solid = inputs
                .body(*body)
                .ok_or(ProjectionError::SourceUnavailable)?;
            let names = vertex_names(solid);
            let found: Vec<VertexId> = names
                .iter()
                .filter(|(_, name)| *name == vertex)
                .map(|(id, _)| *id)
                .collect();
            match found.as_slice() {
                [one] => vertex_outline(solid, *one, plane)
                    .filter(|outline| outline.shape() == wanted)
                    .ok_or(ProjectionError::ChangedShape),
                [] => Err(ProjectionError::Missing),
                _ => Err(ProjectionError::Ambiguous),
            }
        }
        ProjectionSource::SketchEntity { sketch, entity } => {
            let result = inputs
                .features
                .get(sketch)
                .and_then(|result| result.sketch())
                .ok_or(ProjectionError::SourceUnavailable)?;
            if result.geometry.entity(*entity).is_none() {
                return Err(ProjectionError::Missing);
            }
            sketch_entity_outline(&result.geometry, *entity, plane, Some(wanted))
                .ok_or(ProjectionError::ChangedShape)
        }
    }
}

fn place(sketch: &mut Sketch, entity: EntityId, outline: &Outline) -> Result<(), ()> {
    let set_point = |sketch: &mut Sketch, id: EntityId, position: Point2| {
        sketch
            .replace_entity(id, Entity::Point(position))
            .map(|_| ())
            .map_err(|_| ())
    };
    match (sketch.entity(entity).cloned().ok_or(())?, outline) {
        (Entity::Point(_), Outline::Point(position)) => set_point(sketch, entity, *position),
        (
            Entity::Line { start, end },
            Outline::Line {
                start: from,
                end: to,
            },
        ) => {
            set_point(sketch, start, *from)?;
            set_point(sketch, end, *to)
        }
        (Entity::Circle { center, .. }, Outline::Circle { center: at, radius }) => {
            set_point(sketch, center, *at)?;
            sketch
                .replace_entity(
                    entity,
                    Entity::Circle {
                        center,
                        radius: *radius,
                    },
                )
                .map(|_| ())
                .map_err(|_| ())
        }
        (
            Entity::Arc { center, start, end },
            Outline::Arc {
                center: at,
                start: from,
                end: to,
            },
        ) => {
            set_point(sketch, center, *at)?;
            set_point(sketch, start, *from)?;
            set_point(sketch, end, *to)
        }
        (Entity::Spline { control_points }, Outline::Spline(positions))
            if control_points.len() == positions.len() =>
        {
            for (id, position) in control_points.iter().zip(positions) {
                set_point(sketch, *id, *position)?;
            }
            Ok(())
        }
        _ => Err(()),
    }
}

fn projection_failure(
    feature: &Feature,
    sketch: &Sketch,
    entity: EntityId,
    source: &ProjectionSource,
    error: ProjectionError,
    inputs: &Inputs<'_>,
) -> Failure {
    let label = sketch.entity_label(entity);
    let source_name = inputs
        .document
        .feature(source.feature())
        .map(|source| source.name.clone())
        .unwrap_or_else(|| "a deleted feature".to_owned());
    let what = match source {
        ProjectionSource::Edge { .. } => format!("an edge of {source_name}"),
        ProjectionSource::Vertex { .. } => format!("a corner of {source_name}"),
        ProjectionSource::SketchEntity { .. } => format!("geometry of {source_name}"),
        ProjectionSource::Section { .. } => format!("the cut through {source_name}"),
        ProjectionSource::DatumPlane { .. } => format!("the cut along {source_name}"),
    };
    let verb = match source {
        ProjectionSource::Section { .. } | ProjectionSource::DatumPlane { .. } => "drawn from",
        ProjectionSource::Edge { .. }
        | ProjectionSource::Vertex { .. }
        | ProjectionSource::SketchEntity { .. } => "projected from",
    };
    let redo = match source {
        ProjectionSource::Section { .. } | ProjectionSource::DatumPlane { .. } => {
            "intersect what you want"
        }
        ProjectionSource::Edge { .. }
        | ProjectionSource::Vertex { .. }
        | ProjectionSource::SketchEntity { .. } => "project the geometry you want",
    };
    let reason = match error {
        ProjectionError::SourceUnavailable => {
            format!("{label} is {verb} {what}, which is not available at this point in the tree.")
        }
        ProjectionError::Missing => {
            format!("{label} is {verb} {what} that no longer exists.")
        }
        ProjectionError::Ambiguous => format!(
            "{label} is {verb} {what} that was split into separate parts, so it is unclear which \
             one to follow."
        ),
        ProjectionError::ChangedShape => format!(
            "{label} is {verb} {what} whose shape in this sketch changed kind, for example a \
             line now seen end-on."
        ),
        ProjectionError::NotCut => format!(
            "{label} is {verb} {what}, but the sketch plane no longer cuts the face it came from."
        ),
        ProjectionError::Uncuttable => format!(
            "{label} is {verb} {what}, which could not be worked out: the body could not be cut \
             along the sketch plane."
        ),
        ProjectionError::Parallel => format!(
            "{label} is {verb} {what}, which now lies parallel to the sketch, so the two no \
             longer meet in a line."
        ),
        ProjectionError::Cancelled => format!("{label} is {verb} {what}, which was cancelled."),
    };
    let fix = match error {
        ProjectionError::SourceUnavailable | ProjectionError::Parallel => {
            Some(FixTarget::Feature(source.feature()))
        }
        ProjectionError::Missing
        | ProjectionError::Ambiguous
        | ProjectionError::ChangedShape
        | ProjectionError::NotCut
        | ProjectionError::Uncuttable
        | ProjectionError::Cancelled => Some(FixTarget::Feature(feature.id())),
    };
    Failure::Error(Box::new(FeatureError {
        reason,
        remedy: format!("Open the sketch, delete {label} and {redo} in its place."),
        fix,
        constraints: Vec::new(),
        place: None,
    }))
}
