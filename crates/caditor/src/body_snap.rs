use std::{
    f64::consts::{FRAC_PI_2, TAU},
    sync::Arc,
};

use caditor_document::{
    FeatureId, FeatureResult, Outline, PROJECTED_SPLINE_POINTS, ProjectionSource, SolidResult,
    vertex_outline,
};
use caditor_geometry::{Plane, Point2, Point3, Vector2};
use caditor_kernel::{Edge, EdgeName, VertexName};

use crate::{
    bodies,
    inference::Inferences,
    model::Model,
    projecting,
    snap::{self, Accept, Pointer, Screen, Snapped, Target},
    visibility,
};

const SAME_POINT: f64 = 1e-9;
const SAME_PLACE: f64 = 1e-6;
const ARC_PREVIEW_SEGMENTS: usize = 64;
const SCREEN_SLACK: f64 = 1e-3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyPart {
    Corner,
    Middle,
    Centre,
    Edge,
}

impl BodyPart {
    pub fn is_point_like(self) -> bool {
        !matches!(self, Self::Edge)
    }

    fn takes(self, accept: Accept, inferred: Inferences) -> bool {
        inferred.allows_part(self) && self.accepted(accept)
    }

    fn accepted(self, accept: Accept) -> bool {
        match accept {
            Accept::Anything => true,
            Accept::Points => matches!(self, Self::Corner | Self::Centre),
            Accept::OnCircle { .. } => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BodyItem {
    Corner(VertexName),
    Centre(EdgeName),
    Edge(EdgeName),
}

impl BodyItem {
    pub fn words(self, body: &str) -> String {
        match self {
            Self::Corner(_) => format!("Corner of {body}"),
            Self::Centre(_) => format!("Centre of a round edge of {body}"),
            Self::Edge(_) => format!("Edge of {body}"),
        }
    }

    fn part(self) -> BodyPart {
        match self {
            Self::Corner(_) => BodyPart::Corner,
            Self::Centre(_) => BodyPart::Centre,
            Self::Edge(_) => BodyPart::Edge,
        }
    }

    fn key(self) -> Key {
        match self {
            Self::Corner(vertex) => Key::Vertex(vertex),
            Self::Centre(edge) | Self::Edge(edge) => Key::Edge(edge),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Vertex(VertexName),
    Edge(EdgeName),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyTarget {
    pub source: usize,
    pub part: BodyPart,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub projection: ProjectionSource,
    pub outline: Outline,
    body: String,
    owner: FeatureId,
    key: Key,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Bounds {
    min: Point2,
    max: Point2,
}

impl Bounds {
    fn of(points: impl IntoIterator<Item = Point2>) -> Option<Self> {
        points.into_iter().fold(None, |bounds, point| {
            Some(match bounds {
                Some(Self { min, max }) => Self {
                    min: min.min(point),
                    max: max.max(point),
                },
                None => Self {
                    min: point,
                    max: point,
                },
            })
        })
    }

    fn corners(self) -> [Point2; 4] {
        [
            self.min,
            Point2::new(self.max.x, self.min.y),
            self.max,
            Point2::new(self.min.x, self.max.y),
        ]
    }

    fn may_reach(self, screen: &impl Screen, pointer: Pointer, tolerance: f64) -> bool {
        let [a, b, c, d] = self.corners();
        let (Some(a), Some(b), Some(c), Some(d)) = (
            screen.to_screen(a),
            screen.to_screen(b),
            screen.to_screen(c),
            screen.to_screen(d),
        ) else {
            return true;
        };
        let low = a.min(b).min(c).min(d);
        let high = a.max(b).max(c).max(d);
        let outside = Vector2::new(
            (low.x - pointer.screen.x)
                .max(pointer.screen.x - high.x)
                .max(0.0),
            (low.y - pointer.screen.y)
                .max(pointer.screen.y - high.y)
                .max(0.0),
        );
        let gap = outside.length();
        gap.is_nan() || gap <= tolerance + SCREEN_SLACK
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SnapPoint {
    position: Point2,
    depth: f64,
    source: usize,
    part: BodyPart,
    body: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum Path {
    Segment(Point2, Point2),
    Arc {
        center: Point2,
        radius: f64,
        start: f64,
        sweep: f64,
    },
    Polyline(Vec<Point2>),
}

impl Path {
    fn polyline(&self) -> Vec<Point2> {
        match self {
            Self::Segment(start, end) => vec![*start, *end],
            Self::Arc {
                center,
                radius,
                start,
                sweep,
            } => (0..=ARC_PREVIEW_SEGMENTS)
                .map(|index| {
                    let angle = start + sweep * index as f64 / ARC_PREVIEW_SEGMENTS as f64;
                    *center + Vector2::from_angle(angle) * *radius
                })
                .collect(),
            Self::Polyline(points) => points.clone(),
        }
    }

    fn bounds(&self) -> Option<Bounds> {
        match self {
            Self::Segment(start, end) => Bounds::of([*start, *end]),
            Self::Arc {
                center,
                radius,
                start,
                sweep,
            } => {
                let at = |angle: f64| *center + Vector2::from_angle(angle) * *radius;
                let quadrants = (0..4)
                    .map(|quadrant| quadrant as f64 * FRAC_PI_2)
                    .filter(|angle| (angle - start).rem_euclid(TAU) <= *sweep)
                    .map(at);
                Bounds::of([at(*start), at(start + sweep)].into_iter().chain(quadrants))
            }
            Self::Polyline(points) => Bounds::of(points.iter().copied()),
        }
    }

    fn is_cheap(&self) -> bool {
        matches!(self, Self::Segment(..))
    }

    fn closest(&self, point: Point2) -> Option<Point2> {
        match self {
            Self::Segment(start, end) => Some(closest_on_segment(*start, *end, point)),
            Self::Arc {
                center,
                radius,
                start,
                sweep,
            } => {
                let offset = point - *center;
                if offset.length_squared() == 0.0 {
                    return None;
                }
                let angle = offset.y.atan2(offset.x);
                let turned = (angle - start).rem_euclid(TAU);
                let at = |angle: f64| *center + Vector2::from_angle(angle) * *radius;
                if turned <= *sweep {
                    Some(at(angle))
                } else {
                    let (first, last) = (at(*start), at(start + sweep));
                    Some(if first.distance(point) <= last.distance(point) {
                        first
                    } else {
                        last
                    })
                }
            }
            Self::Polyline(points) => points
                .windows(2)
                .filter_map(|pair| match pair {
                    [start, end] => Some(closest_on_segment(*start, *end, point)),
                    _ => None,
                })
                .min_by(|a, b| a.distance(point).total_cmp(&b.distance(point))),
        }
    }
}

fn closest_on_segment(start: Point2, end: Point2, point: Point2) -> Point2 {
    let along = end - start;
    let length_squared = along.length_squared();
    if length_squared == 0.0 {
        return start;
    }
    start + along * ((point - start).dot(along) / length_squared).clamp(0.0, 1.0)
}

#[derive(Debug, Clone, PartialEq)]
struct SnapEdge {
    path: Path,
    bounds: Option<Bounds>,
    depth: f64,
    source: usize,
    body: usize,
}

impl SnapEdge {
    fn may_reach(&self, screen: &impl Screen, pointer: Pointer, tolerance: f64) -> bool {
        self.path.is_cheap()
            || self
                .bounds
                .is_some_and(|bounds| bounds.may_reach(screen, pointer, tolerance))
    }
}

#[derive(Debug, Clone)]
struct Basis {
    sketch: FeatureId,
    plane: Plane,
    bodies: Vec<(FeatureId, Arc<FeatureResult>)>,
}

impl Basis {
    fn of(model: &Model, sketch: FeatureId) -> Option<Self> {
        let document = model.document();
        let edited = document.feature(sketch)?;
        let plane = match model.displayed_sketch(edited) {
            Some(displayed) => displayed.plane(),
            None => edited.kind.sketch()?.plane(),
        };
        let evaluation = model.evaluation();
        let bodies = evaluation
            .bodies()
            .map(|(body, _)| body)
            .filter(|body| visibility::is_shown(document, *body))
            .filter_map(|body| Some((body, evaluation.body_state_seen_by(sketch, body)?.clone())))
            .collect();
        Some(Self {
            sketch,
            plane,
            bodies,
        })
    }

    fn same(&self, other: &Self) -> bool {
        self.sketch == other.sketch
            && self.plane == other.plane
            && self.bodies.len() == other.bodies.len()
            && self.bodies.iter().zip(&other.bodies).all(
                |((body, state), (other_body, other_state))| {
                    body == other_body && Arc::ptr_eq(state, other_state)
                },
            )
    }
}

#[derive(Debug, Clone, Default)]
pub struct BodySnaps {
    generation: u64,
    basis: Option<Basis>,
    sources: Vec<Source>,
    points: Vec<SnapPoint>,
    edges: Vec<SnapEdge>,
    volumes: Vec<Option<Bounds>>,
}

impl BodySnaps {
    pub fn refreshed(current: &Arc<Self>, model: &Model, sketch: FeatureId) -> Option<Arc<Self>> {
        let basis = Basis::of(model, sketch);
        let unchanged = match (&current.basis, &basis) {
            (Some(kept), Some(basis)) => kept.same(basis),
            (None, None) => true,
            (Some(_), None) | (None, Some(_)) => false,
        };
        if unchanged {
            return None;
        }
        let generation = current.generation.wrapping_add(1);
        Some(Arc::new(match basis {
            Some(basis) => Self::built(model, basis, generation),
            None => Self {
                generation,
                ..Self::default()
            },
        }))
    }

    fn built(model: &Model, basis: Basis, generation: u64) -> Self {
        let mut snaps = Self {
            generation,
            ..Self::default()
        };
        let document = model.document();
        for (index, (body, state)) in basis.bodies.iter().enumerate() {
            let Some(result) = state.solid() else {
                continue;
            };
            let name = document
                .body_label(*body)
                .unwrap_or_else(|| "a body".to_owned());
            snaps.add_corners(result, (*body, index), &name, &basis.plane);
            snaps.add_edges(result, (*body, index), &name, &basis.plane);
        }
        snaps.volumes = snaps.body_volumes(basis.bodies.len());
        snaps.points.sort_by(|a, b| a.depth.total_cmp(&b.depth));
        snaps.edges.sort_by(|a, b| a.depth.total_cmp(&b.depth));
        snaps.basis = Some(basis);
        snaps
    }

    fn add_source(
        &mut self,
        projection: ProjectionSource,
        outline: Outline,
        (owner, body): (FeatureId, &str),
        key: Key,
    ) -> usize {
        self.sources.push(Source {
            projection,
            outline,
            body: body.to_owned(),
            owner,
            key,
        });
        self.sources.len() - 1
    }

    fn body_volumes(&self, count: usize) -> Vec<Option<Bounds>> {
        let mut volumes: Vec<Option<Bounds>> = vec![None; count];
        let points = self
            .points
            .iter()
            .map(|point| (point.body, Bounds::of([point.position])));
        let edges = self.edges.iter().map(|edge| (edge.body, edge.bounds));
        for (body, bounds) in points.chain(edges) {
            let (Some(slot), Some(bounds)) = (volumes.get_mut(body), bounds) else {
                continue;
            };
            *slot = Bounds::of(
                slot.iter()
                    .flat_map(|kept| kept.corners())
                    .chain(bounds.corners()),
            );
        }
        volumes
    }

    fn reachable_bodies(
        &self,
        screen: &impl Screen,
        pointer: Pointer,
        tolerance: f64,
    ) -> Vec<bool> {
        self.volumes
            .iter()
            .map(|volume| volume.is_some_and(|volume| volume.may_reach(screen, pointer, tolerance)))
            .collect()
    }

    fn add_corners(
        &mut self,
        result: &SolidResult,
        (body, index): (FeatureId, usize),
        name: &str,
        plane: &Plane,
    ) {
        let names = result.names();
        for (id, vertex) in result.solid.vertices() {
            let Some(vertex_name) = names.vertex_name(id) else {
                continue;
            };
            if names.vertices_named(vertex_name).len() != 1 {
                continue;
            }
            let Some(outline) = vertex_outline(&result.solid, id, plane) else {
                continue;
            };
            let Outline::Point(position) = outline else {
                continue;
            };
            let depth = plane.signed_distance(vertex.point()).abs();
            let source = self.add_source(
                ProjectionSource::Vertex {
                    body,
                    vertex: vertex_name,
                },
                outline,
                (body, name),
                Key::Vertex(vertex_name),
            );
            self.points.push(SnapPoint {
                position,
                depth,
                source,
                part: BodyPart::Corner,
                body: index,
            });
        }
    }

    fn add_edges(
        &mut self,
        result: &SolidResult,
        (body, index): (FeatureId, usize),
        name: &str,
        plane: &Plane,
    ) {
        let names = result.names();
        for (id, edge) in result.solid.edges() {
            if names.edge_named(edge.name()) != Some(id) || bodies::is_seam(&result.solid, id) {
                continue;
            }
            let Some((projection, outline)) = projecting::edge_projection(result, body, id, plane)
            else {
                continue;
            };
            let samples = edge_samples(edge);
            let depth = samples
                .iter()
                .map(|sample| plane.signed_distance(*sample).abs())
                .sum::<f64>()
                / samples.len().max(1) as f64;
            let local: Vec<Point2> = samples
                .iter()
                .map(|sample| plane.to_local(*sample))
                .collect();
            let Some(path) = path_of(&outline, local) else {
                continue;
            };
            let middle = middle_of(&outline);
            let centre = centre_of(&outline);
            let source = self.add_source(projection, outline, (body, name), Key::Edge(edge.name()));
            for (position, part) in [(middle, BodyPart::Middle), (centre, BodyPart::Centre)] {
                if let Some(position) = position {
                    self.points.push(SnapPoint {
                        position,
                        depth,
                        source,
                        part,
                        body: index,
                    });
                }
            }
            self.edges.push(SnapEdge {
                bounds: path.bounds(),
                path,
                depth,
                source,
                body: index,
            });
        }
    }

    pub fn point(
        &self,
        screen: &impl Screen,
        pointer: Pointer,
        accept: Accept,
        inferred: Inferences,
    ) -> Option<Snapped> {
        self.point_within(screen, pointer, accept, inferred, snap::POINT_TOLERANCE)
    }

    pub fn point_within(
        &self,
        screen: &impl Screen,
        pointer: Pointer,
        accept: Accept,
        inferred: Inferences,
        tolerance: f64,
    ) -> Option<Snapped> {
        let reachable = self.reachable_bodies(screen, pointer, tolerance);
        nearest(
            self.points
                .iter()
                .filter(|point| point.part.takes(accept, inferred))
                .filter(|point| reachable.get(point.body).copied().unwrap_or(false))
                .map(|point| (point.position, point.source, point.part)),
            screen,
            pointer,
            tolerance,
        )
        .map(|(position, source, part)| self.snapped(position, source, part))
    }

    pub fn edge(
        &self,
        screen: &impl Screen,
        pointer: Pointer,
        accept: Accept,
        inferred: Inferences,
    ) -> Option<Snapped> {
        self.edge_within(screen, pointer, accept, inferred, snap::CURVE_TOLERANCE)
    }

    pub fn edge_within(
        &self,
        screen: &impl Screen,
        pointer: Pointer,
        accept: Accept,
        inferred: Inferences,
        tolerance: f64,
    ) -> Option<Snapped> {
        if !BodyPart::Edge.takes(accept, inferred) {
            return None;
        }
        let reachable = self.reachable_bodies(screen, pointer, tolerance);
        nearest(
            self.edges
                .iter()
                .filter(|edge| reachable.get(edge.body).copied().unwrap_or(false))
                .filter(|edge| edge.may_reach(screen, pointer, tolerance))
                .filter_map(|edge| {
                    Some((
                        edge.path.closest(pointer.sketch)?,
                        edge.source,
                        BodyPart::Edge,
                    ))
                }),
            screen,
            pointer,
            tolerance,
        )
        .map(|(position, source, part)| self.snapped(position, source, part))
    }

    pub fn foot(&self, target: BodyTarget, point: Point2) -> Option<Point2> {
        self.source(target)?;
        if target.part != BodyPart::Edge {
            return None;
        }
        self.edges
            .iter()
            .find(|edge| edge.source == target.source)?
            .path
            .closest(point)
    }

    pub fn steppable(&self) -> Vec<(FeatureId, BodyItem)> {
        let mut places: Vec<Point2> = Vec::new();
        let points = self
            .points
            .iter()
            .filter(|point| matches!(point.part, BodyPart::Corner | BodyPart::Centre))
            .filter(|point| {
                let fresh = places
                    .iter()
                    .all(|place| place.distance(point.position) > SAME_PLACE);
                if fresh {
                    places.push(point.position);
                }
                fresh
            })
            .map(|point| (point.source, point.part))
            .collect::<Vec<_>>();
        let mut paths: Vec<Vec<Point2>> = Vec::new();
        let edges = self
            .edges
            .iter()
            .filter(|edge| {
                let path = edge.path.polyline();
                let fresh = paths.iter().all(|seen| !same_path(seen, &path));
                if fresh {
                    paths.push(path);
                }
                fresh
            })
            .map(|edge| (edge.source, BodyPart::Edge));
        points
            .into_iter()
            .chain(edges)
            .filter_map(|(source, part)| {
                self.item(BodyTarget {
                    source,
                    part,
                    generation: self.generation,
                })
            })
            .collect()
    }

    fn snapped(&self, position: Point2, source: usize, part: BodyPart) -> Snapped {
        Snapped {
            position,
            target: Target::Body(BodyTarget {
                source,
                part,
                generation: self.generation,
            }),
        }
    }

    pub fn source(&self, target: BodyTarget) -> Option<&Source> {
        (target.generation == self.generation)
            .then(|| self.sources.get(target.source))
            .flatten()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn item(&self, target: BodyTarget) -> Option<(FeatureId, BodyItem)> {
        let source = self.source(target)?;
        let item = match (target.part, source.key) {
            (BodyPart::Corner, Key::Vertex(vertex)) => BodyItem::Corner(vertex),
            (BodyPart::Centre, Key::Edge(edge)) => BodyItem::Centre(edge),
            (BodyPart::Edge, Key::Edge(edge)) => BodyItem::Edge(edge),
            (BodyPart::Corner | BodyPart::Centre | BodyPart::Edge | BodyPart::Middle, _) => {
                return None;
            }
        };
        Some((source.owner, item))
    }

    pub fn find(&self, body: FeatureId, item: BodyItem) -> Option<BodyTarget> {
        let key = item.key();
        let source = self
            .sources
            .iter()
            .position(|source| source.owner == body && source.key == key)?;
        Some(BodyTarget {
            source,
            part: item.part(),
            generation: self.generation,
        })
    }

    pub fn position(&self, target: BodyTarget) -> Option<Point2> {
        self.source(target)?;
        self.points
            .iter()
            .find(|point| point.source == target.source && point.part == target.part)
            .map(|point| point.position)
    }

    pub fn path(&self, target: BodyTarget) -> Option<Vec<Point2>> {
        self.source(target)?;
        self.edges
            .iter()
            .find(|edge| edge.source == target.source)
            .map(|edge| edge.path.polyline())
    }

    pub fn label(&self, target: BodyTarget) -> Option<String> {
        let body = &self.source(target)?.body;
        Some(match target.part {
            BodyPart::Corner => format!("Corner of {body}"),
            BodyPart::Middle => format!("Middle of an edge of {body}"),
            BodyPart::Centre => format!("Centre of a round edge of {body}"),
            BodyPart::Edge => format!("On an edge of {body}"),
        })
    }
}

fn same_path(first: &[Point2], second: &[Point2]) -> bool {
    let near = |(a, b): (&Point2, &Point2)| a.distance(*b) <= SAME_PLACE;
    first.len() == second.len()
        && (first.iter().zip(second).all(near) || first.iter().zip(second.iter().rev()).all(near))
}

fn nearest(
    candidates: impl Iterator<Item = (Point2, usize, BodyPart)>,
    screen: &impl Screen,
    pointer: Pointer,
    tolerance: f64,
) -> Option<(Point2, usize, BodyPart)> {
    let mut best: Option<(f64, (Point2, usize, BodyPart))> = None;
    for candidate in candidates {
        let Some(on_screen) = screen.to_screen(candidate.0) else {
            continue;
        };
        let offset = on_screen.distance(pointer.screen);
        let closer = best
            .as_ref()
            .is_none_or(|(nearest, _)| offset < *nearest - SAME_POINT);
        if offset <= tolerance && closer {
            best = Some((offset, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

fn edge_samples(edge: &Edge) -> Vec<Point3> {
    let interval = edge.interval();
    let last = PROJECTED_SPLINE_POINTS.saturating_sub(1).max(1) as f64;
    (0..PROJECTED_SPLINE_POINTS)
        .map(|index| {
            let t = index as f64 / last;
            edge.curve()
                .point(interval.start() + (interval.end() - interval.start()) * t)
        })
        .collect()
}

fn path_of(outline: &Outline, samples: Vec<Point2>) -> Option<Path> {
    match outline {
        Outline::Point(_) => None,
        Outline::Line { start, end } => Some(Path::Segment(*start, *end)),
        Outline::Circle { center, radius } => Some(Path::Arc {
            center: *center,
            radius: *radius,
            start: 0.0,
            sweep: TAU,
        }),
        Outline::Arc { center, start, end } => {
            let from = (*start - *center).to_angle();
            let to = (*end - *center).to_angle();
            Some(Path::Arc {
                center: *center,
                radius: center.distance(*start),
                start: from,
                sweep: (to - from).rem_euclid(TAU),
            })
        }
        Outline::Spline { .. } | Outline::Ellipse { .. } | Outline::EllipticalArc { .. } => {
            Some(Path::Polyline(samples))
        }
    }
}

fn middle_of(outline: &Outline) -> Option<Point2> {
    match outline {
        Outline::Line { start, end } => Some((*start + *end) / 2.0),
        Outline::Arc { center, start, end } => {
            let from = (*start - *center).to_angle();
            let to = (*end - *center).to_angle();
            let sweep = (to - from).rem_euclid(TAU);
            Some(*center + Vector2::from_angle(from + sweep / 2.0) * center.distance(*start))
        }
        Outline::Point(_)
        | Outline::Circle { .. }
        | Outline::Spline { .. }
        | Outline::Ellipse { .. }
        | Outline::EllipticalArc { .. } => None,
    }
}

fn centre_of(outline: &Outline) -> Option<Point2> {
    match outline {
        Outline::Circle { center, .. }
        | Outline::Arc { center, .. }
        | Outline::Ellipse { center, .. }
        | Outline::EllipticalArc { center, .. } => Some(*center),
        Outline::Point(_) | Outline::Line { .. } | Outline::Spline { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, f64::consts::FRAC_PI_2};

    use caditor_document::Outline;
    use caditor_geometry::{Point2, Vector2};

    use super::{BodyPart, BodySnaps, Path, SnapEdge, SnapPoint, middle_of, nearest};
    use crate::{
        inference::Inferences,
        snap::{self, Accept, Pointer, Screen, Snapped, tests::Scaled},
    };

    struct Receding {
        near: f64,
        calls: Cell<usize>,
    }

    impl Receding {
        fn new() -> Self {
            Self {
                near: 1.0,
                calls: Cell::new(0),
            }
        }
    }

    impl Screen for Receding {
        fn to_screen(&self, point: Point2) -> Option<Vector2> {
            self.calls.set(self.calls.get() + 1);
            let depth = 20.0 + 0.5 * point.y;
            (depth > self.near).then(|| Vector2::new(point.x, -3.0) * 100.0 / depth)
        }
    }

    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self, low: f64, high: f64) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            low + (high - low) * ((self.0 >> 11) as f64 / (1u64 << 53) as f64)
        }

        fn point(&mut self, low: f64, high: f64) -> Point2 {
            Point2::new(self.next(low, high), self.next(low, high))
        }
    }

    fn crowd(bodies: usize, per_body: usize, seed: u64, home_low: f64) -> BodySnaps {
        let mut random = Lcg(seed);
        let mut snaps = BodySnaps::default();
        for body in 0..bodies {
            let home = random.point(home_low, 30.0);
            for _ in 0..per_body {
                let place = |random: &mut Lcg| home + random.point(0.0, 6.0);
                let path = match snaps.edges.len() % 3 {
                    0 => Path::Segment(place(&mut random), place(&mut random)),
                    1 => Path::Arc {
                        center: place(&mut random),
                        radius: random.next(0.5, 4.0),
                        start: random.next(-3.0, 3.0),
                        sweep: random.next(0.2, 6.0),
                    },
                    _ => Path::Polyline((0..8).map(|_| place(&mut random)).collect()),
                };
                let source = snaps.edges.len();
                let corner = place(&mut random);
                snaps.points.push(SnapPoint {
                    position: corner,
                    depth: 0.0,
                    source,
                    part: BodyPart::Corner,
                    body,
                });
                snaps.points.push(SnapPoint {
                    position: place(&mut random),
                    depth: 0.0,
                    source,
                    part: BodyPart::Middle,
                    body,
                });
                snaps.edges.push(SnapEdge {
                    bounds: path.bounds(),
                    path,
                    depth: 0.0,
                    source,
                    body,
                });
            }
        }
        snaps.volumes = snaps.body_volumes(bodies);
        snaps
    }

    fn whole_scan_point(
        snaps: &BodySnaps,
        screen: &impl Screen,
        pointer: Pointer,
        tolerance: f64,
    ) -> Option<Snapped> {
        nearest(
            snaps
                .points
                .iter()
                .map(|point| (point.position, point.source, point.part)),
            screen,
            pointer,
            tolerance,
        )
        .map(|(position, source, part)| snaps.snapped(position, source, part))
    }

    fn whole_scan_edge(
        snaps: &BodySnaps,
        screen: &impl Screen,
        pointer: Pointer,
        tolerance: f64,
    ) -> Option<Snapped> {
        nearest(
            snaps.edges.iter().filter_map(|edge| {
                Some((
                    edge.path.closest(pointer.sketch)?,
                    edge.source,
                    BodyPart::Edge,
                ))
            }),
            screen,
            pointer,
            tolerance,
        )
        .map(|(position, source, part)| snaps.snapped(position, source, part))
    }

    fn pointers(screen: &Receding, count: usize, seed: u64) -> Vec<Pointer> {
        let mut random = Lcg(seed);
        (0..count)
            .filter_map(|_| {
                let sketch = random.point(-70.0, 40.0);
                Some(Pointer {
                    screen: screen.to_screen(sketch)?,
                    sketch,
                })
            })
            .collect()
    }

    fn pointer_at(sketch: Point2) -> Pointer {
        Pointer {
            screen: Scaled(10.0).to_screen(sketch).unwrap(),
            sketch,
        }
    }

    #[test]
    fn an_arc_path_takes_the_foot_within_its_sweep_and_the_nearer_end_outside_it() {
        let quarter = Path::Arc {
            center: Point2::ZERO,
            radius: 10.0,
            start: 0.0,
            sweep: FRAC_PI_2,
        };

        let inside = quarter.closest(Point2::new(20.0, 20.0)).unwrap();
        let past_the_end = quarter.closest(Point2::new(-5.0, 20.0)).unwrap();
        let before_the_start = quarter.closest(Point2::new(20.0, -1.0)).unwrap();

        assert!(inside.distance(Vector2::splat(10.0 / 2.0_f64.sqrt())) < 1e-9);
        assert!(past_the_end.distance(Point2::new(0.0, 10.0)) < 1e-9);
        assert!(before_the_start.distance(Point2::new(10.0, 0.0)) < 1e-9);
        assert_eq!(quarter.closest(Point2::ZERO), None);
    }

    #[test]
    fn an_arc_middle_lies_halfway_round_its_counter_clockwise_sweep() {
        let middle = middle_of(&Outline::Arc {
            center: Point2::ZERO,
            start: Point2::new(0.0, 10.0),
            end: Point2::new(10.0, 0.0),
        })
        .unwrap();

        assert!(middle.distance(Vector2::splat(-10.0 / 2.0_f64.sqrt())) < 1e-9);
    }

    #[test]
    fn of_two_candidates_at_one_place_the_first_one_nearer_the_sketch_plane_wins() {
        let at = Point2::new(4.0, 2.0);
        let candidates = [
            (at, 0, BodyPart::Corner),
            (at, 1, BodyPart::Corner),
            (Point2::new(30.0, 30.0), 2, BodyPart::Corner),
        ];

        let found = nearest(
            candidates.into_iter(),
            &Scaled(10.0),
            pointer_at(Point2::new(4.2, 2.1)),
            8.0,
        );
        let missed = nearest(
            candidates.into_iter(),
            &Scaled(10.0),
            pointer_at(Point2::new(10.0, 10.0)),
            8.0,
        );

        assert_eq!(found, Some((at, 0, BodyPart::Corner)));
        assert_eq!(missed, None);
    }

    #[test]
    fn an_arc_path_bounds_hold_the_arc_and_only_it() {
        let quarter = Path::Arc {
            center: Point2::ZERO,
            radius: 10.0,
            start: 0.0,
            sweep: FRAC_PI_2,
        };
        let circle = Path::Arc {
            center: Point2::new(5.0, 5.0),
            radius: 2.0,
            start: 1.0,
            sweep: std::f64::consts::TAU,
        };

        let quarter = quarter.bounds().unwrap();
        let circle = circle.bounds().unwrap();

        assert!(quarter.min.distance(Point2::ZERO) < 1e-9);
        assert!(quarter.max.distance(Point2::splat(10.0)) < 1e-9);
        assert!(circle.min.distance(Point2::splat(3.0)) < 1e-9);
        assert!(circle.max.distance(Point2::splat(7.0)) < 1e-9);
    }

    #[test]
    fn bounded_body_scans_pick_what_the_whole_scan_picks_even_near_the_horizon() {
        let screen = Receding::new();
        let snaps = crowd(40, 12, 7, -60.0);
        let mut hits = 0;

        for pointer in pointers(&screen, 600, 11) {
            for tolerance in [snap::POINT_TOLERANCE, snap::HELD_POINT_PULL] {
                let bounded = snaps.point_within(
                    &screen,
                    pointer,
                    Accept::Anything,
                    Inferences::default(),
                    tolerance,
                );
                hits += usize::from(bounded.is_some());
                assert_eq!(
                    bounded,
                    whole_scan_point(&snaps, &screen, pointer, tolerance)
                );
            }
            for tolerance in [snap::CURVE_TOLERANCE, snap::HELD_CURVE_PULL] {
                let bounded = snaps.edge_within(
                    &screen,
                    pointer,
                    Accept::Anything,
                    Inferences::default(),
                    tolerance,
                );
                hits += usize::from(bounded.is_some());
                assert_eq!(
                    bounded,
                    whole_scan_edge(&snaps, &screen, pointer, tolerance)
                );
            }
        }

        assert!(hits > 200);
    }

    #[test]
    fn a_pointer_far_from_every_body_projects_each_body_once_and_nothing_more() {
        let screen = Receding::new();
        let snaps = crowd(50, 40, 3, -20.0);
        let far = Point2::new(5000.0, 5.0);
        let pointer = Pointer {
            screen: screen.to_screen(far).unwrap(),
            sketch: far,
        };

        let point = snaps.point_within(
            &screen,
            pointer,
            Accept::Anything,
            Inferences::default(),
            snap::POINT_TOLERANCE,
        );
        let projections_for_points = screen.calls.replace(0);
        let edge = snaps.edge_within(
            &screen,
            pointer,
            Accept::Anything,
            Inferences::default(),
            snap::CURVE_TOLERANCE,
        );
        let projections_for_edges = screen.calls.replace(0);

        assert_eq!((point, edge), (None, None));
        assert_eq!(projections_for_points, 1 + 4 * 50);
        assert_eq!(projections_for_edges, 4 * 50);
    }

    #[test]
    fn bodies_whose_bounds_cross_the_horizon_are_scanned_whole() {
        let screen = Receding::new();
        let mut snaps = crowd(1, 12, 5, -20.0);
        snaps.points.push(SnapPoint {
            position: Point2::new(0.0, -50.0),
            depth: 0.0,
            source: 999,
            part: BodyPart::Corner,
            body: 0,
        });
        snaps.volumes = snaps.body_volumes(1);
        let at = Point2::new(0.0, 0.0);
        let pointer = Pointer {
            screen: screen.to_screen(at).unwrap(),
            sketch: at,
        };

        let bounded = snaps.point_within(
            &screen,
            pointer,
            Accept::Anything,
            Inferences::default(),
            snap::POINT_TOLERANCE,
        );

        assert_eq!(
            bounded,
            whole_scan_point(&snaps, &screen, pointer, snap::POINT_TOLERANCE)
        );
        assert!(screen.calls.get() > 4);
    }
}
