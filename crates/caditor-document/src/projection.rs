use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    f64::consts::TAU,
};

use caditor_geometry::{Plane, Point2, Point3, Ray, Vector2};
use caditor_kernel::{
    BooleanError, Curve, Curve2, Edge, EdgeId, EdgeReference, FaceReference, HiddenLineError,
    HiddenLineView, MeshQuality, OutlinePiece, ReferenceError, Solid, VertexId, VertexName,
    ViewSource, outer_outline, vertex_names,
};
use caditor_sketch::{
    ArcGeometry, BSpline, EllipseGeometry, Entity, EntityId, FitSpacing, Sketch, SplineKind,
};

use crate::{
    attachment::SketchFeature,
    datum::{DatumResult, PrincipalAxis, PrincipalPlane},
    document::{Feature, FeatureId},
    edge_run::EdgeRun,
    edit::{Edit, TransactionBuilder},
    failure::Cause,
    origins,
    pieces::{Resolution, pieces_of_one_edge},
    recompute::{Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    section::{SectionError, datum_outline, section_solid},
    split_edges::{Followed, SplitEdges, Unrun, joined},
    tolerance::{self, DIRECTION_TOLERANCE, POSITION_TOLERANCE},
};

pub const PROJECTED_SPLINE_POINTS: usize = 33;
const FULL_TURN_SLACK: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq)]
pub enum OutlineOrigin {
    Edge(EdgeReference),
    Silhouette(FaceReference),
}

impl OutlineOrigin {
    fn same_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Edge(edge), Self::Edge(other)) => edge.name() == other.name(),
            (Self::Silhouette(face), Self::Silhouette(other)) => face.name() == other.name(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutlinePart {
    pub origin: OutlineOrigin,
    pub occurrence: u32,
    pub of: u32,
}

impl OutlinePart {
    fn heap_size(&self) -> usize {
        size_of::<Self>()
            + match &self.origin {
                OutlineOrigin::Edge(_) => 0,
                OutlineOrigin::Silhouette(face) => face.heap_size(),
            }
    }
}

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
    PrincipalPlane {
        plane: PrincipalPlane,
        reach: f64,
    },
    DatumAxis {
        datum: FeatureId,
        reach: f64,
    },
    PrincipalAxis {
        axis: PrincipalAxis,
        reach: f64,
    },
    DatumPoint {
        datum: FeatureId,
    },
    Origin,
    BodyOutline {
        body: FeatureId,
        part: Box<OutlinePart>,
    },
}

impl ProjectionSource {
    pub(crate) fn heap_size(&self) -> usize {
        match self {
            Self::BodyOutline { part, .. } => part.heap_size(),
            _ => 0,
        }
    }

    pub fn body(&self) -> Option<FeatureId> {
        match self {
            Self::Edge { body, .. }
            | Self::Vertex { body, .. }
            | Self::Section { body, .. }
            | Self::BodyOutline { body, .. } => Some(*body),
            Self::SketchEntity { .. }
            | Self::DatumPlane { .. }
            | Self::PrincipalPlane { .. }
            | Self::DatumAxis { .. }
            | Self::PrincipalAxis { .. }
            | Self::DatumPoint { .. }
            | Self::Origin => None,
        }
    }

    pub fn sketch(&self) -> Option<FeatureId> {
        match self {
            Self::SketchEntity { sketch, .. } => Some(*sketch),
            _ => None,
        }
    }

    pub fn sketch_entity(&self) -> Option<(FeatureId, EntityId)> {
        match self {
            Self::SketchEntity { sketch, entity } => Some((*sketch, *entity)),
            _ => None,
        }
    }

    pub fn plane_datum(&self) -> Option<FeatureId> {
        match self {
            Self::DatumPlane { datum, .. } => Some(*datum),
            _ => None,
        }
    }

    pub fn axis_datum(&self) -> Option<FeatureId> {
        match self {
            Self::DatumAxis { datum, .. } => Some(*datum),
            _ => None,
        }
    }

    pub fn point_datum(&self) -> Option<FeatureId> {
        match self {
            Self::DatumPoint { datum } => Some(*datum),
            _ => None,
        }
    }

    pub fn reach(&self) -> Option<f64> {
        match self {
            Self::DatumPlane { reach, .. }
            | Self::PrincipalPlane { reach, .. }
            | Self::DatumAxis { reach, .. }
            | Self::PrincipalAxis { reach, .. } => Some(*reach),
            Self::Edge { .. }
            | Self::Vertex { .. }
            | Self::SketchEntity { .. }
            | Self::Section { .. }
            | Self::DatumPoint { .. }
            | Self::Origin
            | Self::BodyOutline { .. } => None,
        }
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        match self {
            Self::Edge { edge, .. } | Self::Section { edge, .. } => origins::of_edge(edge),
            Self::BodyOutline { part, .. } => match &part.origin {
                OutlineOrigin::Edge(edge) => origins::of_edge(edge),
                OutlineOrigin::Silhouette(face) => origins::of_face(face),
            },
            Self::Vertex { .. }
            | Self::SketchEntity { .. }
            | Self::DatumPlane { .. }
            | Self::PrincipalPlane { .. }
            | Self::DatumAxis { .. }
            | Self::PrincipalAxis { .. }
            | Self::DatumPoint { .. }
            | Self::Origin => BTreeSet::new(),
        }
    }

    pub fn feature(&self) -> Option<FeatureId> {
        match self {
            Self::Edge { body, .. }
            | Self::Vertex { body, .. }
            | Self::Section { body, .. }
            | Self::BodyOutline { body, .. } => Some(*body),
            Self::SketchEntity { sketch, .. } => Some(*sketch),
            Self::DatumPlane { datum, .. }
            | Self::DatumAxis { datum, .. }
            | Self::DatumPoint { datum } => Some(*datum),
            Self::PrincipalPlane { .. } | Self::PrincipalAxis { .. } | Self::Origin => None,
        }
    }

    pub fn is_same_source(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Edge { body, edge },
                Self::Edge {
                    body: other_body,
                    edge: other,
                },
            )
            | (
                Self::Section { body, edge },
                Self::Section {
                    body: other_body,
                    edge: other,
                },
            ) => body == other_body && edge.name() == other.name(),
            (Self::DatumPlane { datum, .. }, Self::DatumPlane { datum: other, .. })
            | (Self::DatumAxis { datum, .. }, Self::DatumAxis { datum: other, .. }) => {
                datum == other
            }
            (Self::PrincipalPlane { plane, .. }, Self::PrincipalPlane { plane: other, .. }) => {
                plane == other
            }
            (Self::PrincipalAxis { axis, .. }, Self::PrincipalAxis { axis: other, .. }) => {
                axis == other
            }
            (
                Self::BodyOutline { body, part },
                Self::BodyOutline {
                    body: other_body,
                    part: other,
                },
            ) => {
                body == other_body
                    && part.occurrence == other.occurrence
                    && part.origin.same_as(&other.origin)
            }
            _ => self == other,
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
    Spline {
        points: Vec<Point2>,
        kind: SplineKind,
    },
    Ellipse {
        center: Point2,
        major: Point2,
        minor_radius: f64,
    },
    EllipticalArc {
        center: Point2,
        major: Point2,
        minor_radius: f64,
        start: Point2,
        end: Point2,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SplineForm {
    Open,
    Closed,
    Fit(FitSpacing),
    ClosedFit(FitSpacing),
    Conic,
    Piece,
    Knotted(bool),
    Graded(usize),
}

impl SplineForm {
    fn of(kind: &SplineKind) -> Self {
        match *kind {
            SplineKind::Control { closed: false } => Self::Open,
            SplineKind::Control { closed: true } => Self::Closed,
            SplineKind::Fit {
                closed: false,
                spacing,
            } => Self::Fit(spacing),
            SplineKind::Fit {
                closed: true,
                spacing,
            } => Self::ClosedFit(spacing),
            SplineKind::Conic { .. } => Self::Conic,
            SplineKind::Piece { .. } => Self::Piece,
            SplineKind::Knotted { closed, .. } => Self::Knotted(closed),
            SplineKind::Graded { degree, .. } => Self::Graded(degree),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Point,
    Line,
    Circle,
    Arc,
    Spline(usize, SplineForm),
    Ellipse,
    EllipticalArc,
}

impl Shape {
    fn of(entity: &Entity) -> Self {
        match entity {
            Entity::Point(_) => Self::Point,
            Entity::Line { .. } => Self::Line,
            Entity::Circle { .. } => Self::Circle,
            Entity::Arc { .. } => Self::Arc,
            Entity::Spline { points, kind } => Self::Spline(points.len(), SplineForm::of(kind)),
            Entity::Ellipse { .. } => Self::Ellipse,
            Entity::EllipticalArc { .. } => Self::EllipticalArc,
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
            Self::Spline { points, kind } => Shape::Spline(points.len(), SplineForm::of(kind)),
            Self::Ellipse { .. } => Shape::Ellipse,
            Self::EllipticalArc { .. } => Shape::EllipticalArc,
        }
    }

    fn elliptical(
        ellipse: &EllipseGeometry,
        local: impl Fn(Point2) -> Point2,
        same_turn: bool,
    ) -> Self {
        let center = local(ellipse.center);
        let major = local(ellipse.center + ellipse.major);
        let minor_radius = ellipse.minor_radius;
        if ellipse.is_full() {
            return Self::Ellipse {
                center,
                major,
                minor_radius,
            };
        }
        let (start, end) = (
            local(ellipse.point_at(ellipse.start)),
            local(ellipse.point_at(ellipse.end())),
        );
        let (start, end) = if same_turn {
            (start, end)
        } else {
            (end, start)
        };
        Self::EllipticalArc {
            center,
            major,
            minor_radius,
            start,
            end,
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
        Some(Self::Spline {
            points: spline.control_points().to_vec(),
            kind: SplineKind::OPEN,
        })
    }

    fn fitted(
        self,
        wanted: Option<Shape>,
        samples: impl FnOnce(usize) -> Vec<Point2>,
    ) -> Option<Self> {
        match wanted {
            None => Some(self),
            Some(shape) if shape == self.shape() => Some(self),
            Some(Shape::Spline(count, SplineForm::Open)) => Self::spline_through(&samples(count)),
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
    Unjoined,
    PartlyTraced,
    NotCut,
    Uncuttable,
    Parallel,
    Cancelled,
    OutlineUnclear,
    OutlineChanged,
}

type Sections = BTreeMap<FeatureId, Result<Solid, ProjectionError>>;
type Splits<'a> = BTreeMap<FeatureId, SplitEdges<'a>>;
type BodyOutlines = BTreeMap<FeatureId, Result<Vec<OutlinePiece>, ProjectionError>>;

#[derive(Default)]
struct Sources<'a> {
    sections: Sections,
    splits: Splits<'a>,
    outlines: BodyOutlines,
}

fn outline_pieces(solid: &Solid, plane: &Plane) -> Result<Vec<OutlinePiece>, HiddenLineError> {
    outer_outline(
        solid,
        &HiddenLineView::square_to(*plane),
        &solid.tolerance_for(&MeshQuality::SMOOTH),
    )
}

fn piece_samples(piece: &OutlinePiece, count: usize) -> Vec<Point2> {
    let last = count.saturating_sub(1).max(1) as f64;
    (0..count)
        .map(|index| piece.curve.point(piece.range.at(index as f64 / last)))
        .collect()
}

fn piece_outline(piece: &OutlinePiece, wanted: Option<Shape>) -> Option<Outline> {
    let (start, end) = (
        piece.curve.point(piece.range.start()),
        piece.curve.point(piece.range.end()),
    );
    let whole = piece.range.length() >= TAU - FULL_TURN_SLACK;
    let natural = match &piece.curve {
        Curve2::Line(_) => Outline::Line { start, end },
        Curve2::Circle(circle) if whole => Outline::Circle {
            center: circle.center(),
            radius: circle.radius(),
        },
        Curve2::Circle(circle) => {
            let (start, end) = if circle.is_counter_clockwise() {
                (start, end)
            } else {
                (end, start)
            };
            Outline::Arc {
                center: circle.center(),
                start,
                end,
            }
        }
        Curve2::Ellipse(ellipse) => {
            let major = ellipse.center() + ellipse.x_axis() * ellipse.major_radius();
            if whole {
                Outline::Ellipse {
                    center: ellipse.center(),
                    major,
                    minor_radius: ellipse.minor_radius(),
                }
            } else {
                let (start, end) = if ellipse.is_counter_clockwise() {
                    (start, end)
                } else {
                    (end, start)
                };
                Outline::EllipticalArc {
                    center: ellipse.center(),
                    major,
                    minor_radius: ellipse.minor_radius(),
                    start,
                    end,
                }
            }
        }
        _ => Outline::through(&piece_samples(piece, PROJECTED_SPLINE_POINTS))?,
    };
    natural.fitted(wanted, |count| piece_samples(piece, count))
}

pub fn body_outline(
    solid: &Solid,
    body: FeatureId,
    plane: &Plane,
) -> Result<Vec<(ProjectionSource, Outline)>, HiddenLineError> {
    let pieces = outline_pieces(solid, plane)?;
    Ok(pieces
        .iter()
        .filter_map(|piece| {
            let origin = match piece.source {
                ViewSource::Edge { edge, .. } => {
                    OutlineOrigin::Edge(EdgeReference::capture(solid, edge)?)
                }
                ViewSource::Silhouette { face, .. } => {
                    OutlineOrigin::Silhouette(FaceReference::capture(solid, face)?)
                }
            };
            let part = OutlinePart {
                origin,
                occurrence: u32::try_from(piece.occurrence).ok()?,
                of: u32::try_from(piece.of).ok()?,
            };
            Some((
                ProjectionSource::BodyOutline {
                    body,
                    part: Box::new(part),
                },
                piece_outline(piece, None)?,
            ))
        })
        .collect())
}

fn resolved_piece<'p>(
    solid: &Solid,
    pieces: &'p [OutlinePiece],
    part: &OutlinePart,
) -> Result<&'p OutlinePiece, ProjectionError> {
    let of_source: Vec<&OutlinePiece> = match &part.origin {
        OutlineOrigin::Edge(reference) => {
            let id = reference.resolve(solid).map_err(lost_reference)?;
            pieces
                .iter()
                .filter(|piece| matches!(piece.source, ViewSource::Edge { edge, .. } if edge == id))
                .collect()
        }
        OutlineOrigin::Silhouette(reference) => {
            let id = reference.resolve(solid).map_err(lost_reference)?;
            pieces
                .iter()
                .filter(
                    |piece| matches!(piece.source, ViewSource::Silhouette { face, .. } if face == id),
                )
                .collect()
        }
    };
    if of_source.len() != part.of as usize {
        return Err(ProjectionError::OutlineChanged);
    }
    of_source
        .into_iter()
        .find(|piece| piece.occurrence == part.occurrence as usize)
        .ok_or(ProjectionError::OutlineChanged)
}

fn lost_reference<T>(error: ReferenceError<T>) -> ProjectionError {
    match error {
        ReferenceError::Ambiguous(_) => ProjectionError::Ambiguous,
        ReferenceError::Missing => ProjectionError::Missing,
    }
}

pub fn edge_outline(solid: &Solid, edge: EdgeId, plane: &Plane) -> Option<Outline> {
    let found = solid.edge(edge)?;
    curve_outline(&EdgeRun::single(edge, found), plane, None)
}

pub fn vertex_outline(solid: &Solid, vertex: VertexId, plane: &Plane) -> Option<Outline> {
    let point = solid.vertex(vertex)?.point();
    Some(Outline::Point(plane.to_local(point)))
}

pub fn axis_outline(axis: &Ray, plane: &Plane, reach: f64) -> Option<Outline> {
    if !reach.is_finite() || reach <= 0.0 {
        return None;
    }
    let direction = axis.direction();
    let flat = Vector2::new(plane.x_axis().dot(direction), plane.y_axis().dot(direction));
    let through = plane.to_local(axis.origin());
    let length = flat.length();
    if length <= DIRECTION_TOLERANCE {
        return Some(Outline::Point(through));
    }
    let along = flat / length;
    let foot = through - along * through.dot(along);
    Some(Outline::Line {
        start: foot - along * reach,
        end: foot + along * reach,
    })
}

pub fn point_outline(point: Point3, plane: &Plane) -> Outline {
    Outline::Point(plane.to_local(point))
}

pub fn sketch_outline(source: &Sketch, entity: EntityId, plane: &Plane) -> Option<Outline> {
    sketch_entity_outline(source, entity, plane, None)
}

fn curve_outline(run: &EdgeRun<'_>, plane: &Plane, wanted: Option<Shape>) -> Option<Outline> {
    let edges = run.edges();
    let (first, first_forward) = *edges.first()?;
    let local = |point: Point3| plane.to_local(point);
    let ends: Vec<Point2> = edges
        .iter()
        .flat_map(|(edge, forward)| {
            let interval = edge.interval();
            let ends = [
                local(edge.curve().point(interval.start())),
                local(edge.curve().point(interval.end())),
            ];
            if *forward { ends } else { [ends[1], ends[0]] }
        })
        .collect();
    let (run_start, run_end) = (*ends.first()?, *ends.last()?);
    let sampled = |count: usize| -> Vec<Point2> {
        let last = count.saturating_sub(1).max(1) as f64;
        if edges.len() > 1 {
            let length = run.length();
            return (0..count)
                .map(|index| local(run.at(length * index as f64 / last).0))
                .collect();
        }
        let interval = first.interval();
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
    let whole = |edge: &Edge| {
        edges.len() == 1
            && edge.is_closed()
            && edge.interval().length() >= TAU - tolerance::DIRECTION_TOLERANCE
    };
    let natural = match first.curve() {
        Curve::Line(_) => Outline::through(&ends)?,
        Curve::Circle(circle) => {
            if tolerance::parallel(circle.frame().normal(), plane.normal()) {
                let center = local(circle.center());
                if whole(first) {
                    Outline::Circle {
                        center,
                        radius: circle.radius(),
                    }
                } else {
                    let counter_clockwise =
                        first_forward == (circle.frame().normal().dot(plane.normal()) > 0.0);
                    let (start, end) = if counter_clockwise {
                        (run_start, run_end)
                    } else {
                        (run_end, run_start)
                    };
                    Outline::Arc { center, start, end }
                }
            } else {
                Outline::through(&sampled(PROJECTED_SPLINE_POINTS))?
            }
        }
        Curve::Ellipse(ellipse)
            if tolerance::parallel(ellipse.frame().normal(), plane.normal()) =>
        {
            let frame = ellipse.frame();
            let center = local(ellipse.center());
            let major_end = local(ellipse.center() + frame.x_axis() * ellipse.major_radius());
            if whole(first) {
                Outline::Ellipse {
                    center,
                    major: major_end,
                    minor_radius: ellipse.minor_radius(),
                }
            } else {
                let counter_clockwise = first_forward == (frame.normal().dot(plane.normal()) > 0.0);
                let (start, end) = if counter_clockwise {
                    (run_start, run_end)
                } else {
                    (run_end, run_start)
                };
                Outline::EllipticalArc {
                    center,
                    major: major_end,
                    minor_radius: ellipse.minor_radius(),
                    start,
                    end,
                }
            }
        }
        _ => Outline::through(&sampled(PROJECTED_SPLINE_POINTS))?,
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
        Entity::Spline { points, kind } => Outline::Spline {
            points: points
                .iter()
                .map(|point| source.point(*point).map(local))
                .collect::<Option<Vec<_>>>()?,
            kind: kind.clone(),
        },
        Entity::Ellipse { .. } | Entity::EllipticalArc { .. } => {
            let ellipse = source.ellipse(entity)?;
            if aligned {
                Outline::elliptical(&ellipse, local, same_turn)
            } else {
                Outline::through(&ellipse_samples(&ellipse, PROJECTED_SPLINE_POINTS, &local))?
            }
        }
    };
    let spline = source.spline(entity);
    let circle = source.circle(entity);
    let arc = source.arc(entity);
    let ellipse = source.ellipse(entity);
    natural.fitted(wanted, |count| {
        let last = count.saturating_sub(1).max(1) as f64;
        if let Some(ellipse) = &ellipse {
            return ellipse_samples(ellipse, count, &local);
        }
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

fn ellipse_samples(
    ellipse: &EllipseGeometry,
    count: usize,
    local: &impl Fn(Point2) -> Point2,
) -> Vec<Point2> {
    let last = count.saturating_sub(1).max(1) as f64;
    (0..count)
        .map(|index| local(ellipse.point_at(ellipse.start + ellipse.sweep * index as f64 / last)))
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Projected {
    pub id: EntityId,
    pub entities: Vec<(EntityId, Entity)>,
}

impl TransactionBuilder<'_> {
    pub fn add_projection(
        &mut self,
        feature: FeatureId,
        source: ProjectionSource,
        outline: &Outline,
    ) -> EntityId {
        self.add_projected(feature, source, outline).id
    }

    pub fn add_projected(
        &mut self,
        feature: FeatureId,
        source: ProjectionSource,
        outline: &Outline,
    ) -> Projected {
        let mut entities = Vec::new();
        let mut add = |builder: &mut Self, entity: Entity| {
            let id = builder.add_sketch_entity(feature, entity.clone());
            entities.push((id, entity));
            id
        };
        let projected = match outline {
            Outline::Point(position) => add(self, Entity::Point(*position)),
            Outline::Line { start, end } => {
                let start = add(self, Entity::Point(*start));
                let end = add(self, Entity::Point(*end));
                add(self, Entity::Line { start, end })
            }
            Outline::Circle { center, radius } => {
                let center = add(self, Entity::Point(*center));
                add(
                    self,
                    Entity::Circle {
                        center,
                        radius: *radius,
                    },
                )
            }
            Outline::Arc { center, start, end } => {
                let center = add(self, Entity::Point(*center));
                let start = add(self, Entity::Point(*start));
                let end = add(self, Entity::Point(*end));
                add(self, Entity::Arc { center, start, end })
            }
            Outline::Spline { points, kind } => {
                let points = points
                    .iter()
                    .map(|position| add(self, Entity::Point(*position)))
                    .collect();
                add(
                    self,
                    Entity::Spline {
                        points,
                        kind: kind.clone(),
                    },
                )
            }
            Outline::Ellipse {
                center,
                major,
                minor_radius,
            } => {
                let center = add(self, Entity::Point(*center));
                let major = add(self, Entity::Point(*major));
                add(
                    self,
                    Entity::Ellipse {
                        center,
                        major,
                        minor_radius: *minor_radius,
                    },
                )
            }
            Outline::EllipticalArc {
                center,
                major,
                minor_radius,
                start,
                end,
            } => {
                let center = add(self, Entity::Point(*center));
                let major = add(self, Entity::Point(*major));
                let start = add(self, Entity::Point(*start));
                let end = add(self, Entity::Point(*end));
                add(
                    self,
                    Entity::EllipticalArc {
                        center,
                        major,
                        minor_radius: *minor_radius,
                        start,
                        end,
                    },
                )
            }
        };
        self.edits.push(Edit::SetSketchProjection {
            feature,
            id: projected,
            source: Some(source),
        });
        Projected {
            id: projected,
            entities,
        }
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
    let mut sources = Sources::default();
    for (entity, source) in &definition.projections {
        let Some(existing) = sketch.entity(*entity) else {
            continue;
        };
        let wanted = Shape::of(existing);
        let outline = outline_of(source, plane, wanted, feature.id(), inputs, &mut sources)
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

fn traced_error(unrun: Unrun) -> ProjectionError {
    match unrun {
        Unrun::Missing => ProjectionError::Missing,
        Unrun::Tied => ProjectionError::Ambiguous,
        Unrun::Gapped => ProjectionError::Unjoined,
        Unrun::PartlyTraced => ProjectionError::PartlyTraced,
    }
}

fn edge_in(
    run: Result<EdgeRun<'_>, Unrun>,
    plane: &Plane,
    wanted: Shape,
) -> Result<Outline, ProjectionError> {
    let run = run.map_err(traced_error)?;
    curve_outline(&run, plane, Some(wanted)).ok_or(ProjectionError::ChangedShape)
}

fn section_run<'a>(solid: &'a Solid, edge: &EdgeReference) -> Result<EdgeRun<'a>, Unrun> {
    let resolved = edge.resolve(solid);
    let leading = resolved.as_ref().ok().copied();
    let followed = Followed::Resolved(Resolution::of(resolved, |pieces| {
        pieces_of_one_edge(solid, pieces)
    }));
    joined(solid, followed, leading)
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

fn outline_of<'a>(
    source: &ProjectionSource,
    plane: &Plane,
    wanted: Shape,
    sketch: FeatureId,
    inputs: &Inputs<'a>,
    sources: &mut Sources<'a>,
) -> Result<Outline, ProjectionError> {
    match source {
        ProjectionSource::BodyOutline { body, part } => {
            let solid = inputs
                .body(*body)
                .ok_or(ProjectionError::SourceUnavailable)?;
            let pieces = sources.outlines.entry(*body).or_insert_with(|| {
                outline_pieces(solid, plane).map_err(|error| match error {
                    HiddenLineError::Cancelled(_) => ProjectionError::Cancelled,
                    error => {
                        log::warn!("the outline of a body could not be worked out: {error}");
                        ProjectionError::OutlineUnclear
                    }
                })
            });
            let pieces = pieces.as_ref().map_err(|error| *error)?;
            let piece = resolved_piece(solid, pieces, part)?;
            piece_outline(piece, Some(wanted)).ok_or(ProjectionError::ChangedShape)
        }
        ProjectionSource::Edge { body, edge } => {
            let solid = inputs
                .body(*body)
                .ok_or(ProjectionError::SourceUnavailable)?;
            let split = sources
                .splits
                .entry(*body)
                .or_insert_with(|| SplitEdges::new(inputs.document, solid));
            edge_in(split.run(edge), plane, wanted)
        }
        ProjectionSource::Section { body, edge } => {
            let section = section_of(*body, plane, sketch, inputs, &mut sources.sections)?;
            match edge_in(section_run(section, edge), plane, wanted) {
                Err(ProjectionError::Missing) => Err(ProjectionError::NotCut),
                outcome => outcome,
            }
        }
        ProjectionSource::DatumPlane { datum, reach } => {
            let datum = datum_result(inputs, *datum)
                .and_then(DatumResult::plane)
                .ok_or(ProjectionError::SourceUnavailable)?;
            datum_outline(&datum, plane, *reach)
                .filter(|outline| outline.shape() == wanted)
                .ok_or(ProjectionError::Parallel)
        }
        ProjectionSource::PrincipalPlane {
            plane: principal,
            reach,
        } => datum_outline(&principal.plane(), plane, *reach)
            .filter(|outline| outline.shape() == wanted)
            .ok_or(ProjectionError::Parallel),
        ProjectionSource::DatumAxis { datum, reach } => {
            let axis = datum_result(inputs, *datum)
                .and_then(DatumResult::axis)
                .ok_or(ProjectionError::SourceUnavailable)?;
            axis_outline(&axis, plane, *reach)
                .filter(|outline| outline.shape() == wanted)
                .ok_or(ProjectionError::ChangedShape)
        }
        ProjectionSource::PrincipalAxis { axis, reach } => axis
            .ray()
            .and_then(|ray| axis_outline(&ray, plane, *reach))
            .filter(|outline| outline.shape() == wanted)
            .ok_or(ProjectionError::ChangedShape),
        ProjectionSource::DatumPoint { datum } => {
            let point = datum_result(inputs, *datum)
                .and_then(DatumResult::point)
                .ok_or(ProjectionError::SourceUnavailable)?;
            Some(point_outline(point, plane))
                .filter(|outline| outline.shape() == wanted)
                .ok_or(ProjectionError::ChangedShape)
        }
        ProjectionSource::Origin => Some(point_outline(Point3::ZERO, plane))
            .filter(|outline| outline.shape() == wanted)
            .ok_or(ProjectionError::ChangedShape),
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

fn datum_result<'a>(inputs: &'a Inputs<'_>, datum: FeatureId) -> Option<&'a DatumResult> {
    match inputs.features.get(&datum)?.as_ref() {
        FeatureResult::Datum(result) => Some(result),
        _ => None,
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
        (
            Entity::Spline { points, kind },
            Outline::Spline {
                points: positions,
                kind: wanted,
            },
        ) if points.len() == positions.len() && kind.same_form(wanted) => {
            for (id, position) in points.iter().zip(positions) {
                set_point(sketch, *id, *position)?;
            }
            if &kind != wanted {
                sketch
                    .replace_entity(
                        entity,
                        Entity::Spline {
                            points,
                            kind: wanted.clone(),
                        },
                    )
                    .map_err(|_| ())?;
            }
            Ok(())
        }
        (
            Entity::Ellipse { center, major, .. },
            Outline::Ellipse {
                center: at,
                major: toward,
                minor_radius,
            },
        ) => {
            set_point(sketch, center, *at)?;
            set_point(sketch, major, *toward)?;
            sketch
                .replace_entity(
                    entity,
                    Entity::Ellipse {
                        center,
                        major,
                        minor_radius: *minor_radius,
                    },
                )
                .map(|_| ())
                .map_err(|_| ())
        }
        (
            Entity::EllipticalArc {
                center,
                major,
                start,
                end,
                ..
            },
            Outline::EllipticalArc {
                center: at,
                major: toward,
                minor_radius,
                start: from,
                end: to,
            },
        ) => {
            set_point(sketch, center, *at)?;
            set_point(sketch, major, *toward)?;
            set_point(sketch, start, *from)?;
            set_point(sketch, end, *to)?;
            sketch
                .replace_entity(
                    entity,
                    Entity::EllipticalArc {
                        center,
                        major,
                        minor_radius: *minor_radius,
                        start,
                        end,
                    },
                )
                .map(|_| ())
                .map_err(|_| ())
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
    let source_name = match source {
        ProjectionSource::PrincipalPlane { plane, .. } => format!("the {}", plane.name()),
        ProjectionSource::PrincipalAxis { axis, .. } => format!("the {}", axis.name()),
        ProjectionSource::Origin => "the origin".to_owned(),
        _ => source
            .feature()
            .and_then(|source| inputs.document.feature(source))
            .map(|source| source.name.clone())
            .unwrap_or_else(|| "a deleted feature".to_owned()),
    };
    let what = match source {
        ProjectionSource::Edge { .. } => format!("an edge of {source_name}"),
        ProjectionSource::Vertex { .. } => format!("a corner of {source_name}"),
        ProjectionSource::SketchEntity { .. } => format!("geometry of {source_name}"),
        ProjectionSource::Section { .. } => format!("the cut through {source_name}"),
        ProjectionSource::BodyOutline { .. } => format!("the outline of {source_name}"),
        ProjectionSource::DatumPlane { .. } | ProjectionSource::PrincipalPlane { .. } => {
            format!("the cut along {source_name}")
        }
        ProjectionSource::DatumAxis { .. }
        | ProjectionSource::PrincipalAxis { .. }
        | ProjectionSource::DatumPoint { .. }
        | ProjectionSource::Origin => source_name,
    };
    let drawn = matches!(
        source,
        ProjectionSource::Section { .. }
            | ProjectionSource::DatumPlane { .. }
            | ProjectionSource::PrincipalPlane { .. }
    );
    let fix = match error {
        ProjectionError::SourceUnavailable | ProjectionError::Parallel => Some(FixTarget::Feature(
            source.feature().unwrap_or_else(|| feature.id()),
        )),
        ProjectionError::Missing
        | ProjectionError::Ambiguous
        | ProjectionError::ChangedShape
        | ProjectionError::Unjoined
        | ProjectionError::PartlyTraced
        | ProjectionError::NotCut
        | ProjectionError::Uncuttable
        | ProjectionError::OutlineUnclear
        | ProjectionError::OutlineChanged
        | ProjectionError::Cancelled => Some(FixTarget::Feature(feature.id())),
    };
    let error = FeatureError::new(ProjectionCause {
        label,
        what,
        error,
        drawn,
    });
    Failure::from(match fix {
        Some(fix) => error.fixing(fix),
        None => error,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProjectionCause {
    label: String,
    what: String,
    error: ProjectionError,
    drawn: bool,
}

impl ProjectionCause {
    pub(crate) fn reason(&self) -> String {
        let Self {
            label, what, error, ..
        } = self;
        let verb = if self.drawn {
            "drawn from"
        } else {
            "projected from"
        };
        match error {
            ProjectionError::SourceUnavailable => format!(
                "{label} is {verb} {what}, which is not available at this point in the tree."
            ),
            ProjectionError::Missing => {
                format!("{label} is {verb} {what} that no longer exists.")
            }
            ProjectionError::Ambiguous => format!(
                "{label} is {verb} {what} that was split into separate parts, so it is unclear \
                 which one to follow."
            ),
            ProjectionError::ChangedShape => format!(
                "{label} is {verb} {what} whose shape in this sketch changed kind, for example a \
                 line now seen end-on."
            ),
            ProjectionError::Unjoined => format!(
                "{label} is {verb} {what} that a trim, split or break upstream cut into parts \
                 which no longer join end to end into one curve, so no single projected curve \
                 follows it."
            ),
            ProjectionError::PartlyTraced => format!(
                "{label} is {verb} {what} that a trim, split or break upstream cut into parts, \
                 and only some of the parts can still be traced."
            ),
            ProjectionError::NotCut => format!(
                "{label} is {verb} {what}, but the sketch plane no longer cuts the face it came \
                 from."
            ),
            ProjectionError::Uncuttable => format!(
                "{label} is {verb} {what}, which could not be worked out: the body could not be \
                 cut along the sketch plane."
            ),
            ProjectionError::Parallel => format!(
                "{label} is {verb} {what}, which now lies parallel to the sketch, so the two no \
                 longer meet in a line."
            ),
            ProjectionError::Cancelled => {
                format!("{label} is {verb} {what}, which was cancelled.")
            }
            ProjectionError::OutlineUnclear => format!(
                "{label} is {verb} {what}, which could not be worked out from this sketch's \
                 direction."
            ),
            ProjectionError::OutlineChanged => format!(
                "{label} is {verb} {what}, which is now made of a different number of curves, \
                 so it is unclear which one to follow."
            ),
        }
    }

    pub(crate) fn remedy(&self) -> String {
        let redo = if self.drawn {
            "intersect what you want"
        } else {
            "project the geometry you want"
        };
        format!(
            "Open the sketch, delete {} and {redo} in its place.",
            self.label
        )
    }
}

impl From<ProjectionCause> for Cause {
    fn from(cause: ProjectionCause) -> Self {
        Self::Projection(cause)
    }
}
