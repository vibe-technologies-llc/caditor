use std::{f64::consts::TAU, sync::Arc};

use caditor_document::{
    FeatureId, FeatureResult, Outline, PROJECTED_SPLINE_POINTS, ProjectionSource, SolidResult,
    vertex_outline,
};
use caditor_geometry::{Plane, Point2, Point3, Vector2};
use caditor_kernel::{Edge, EdgeName, VertexName};

use crate::{
    bodies,
    model::Model,
    projecting,
    snap::{self, Accept, Pointer, Screen, Snapped, Target},
    visibility,
};

const SAME_POINT: f64 = 1e-9;
const ARC_PREVIEW_SEGMENTS: usize = 64;

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

    fn takes(self, accept: Accept) -> bool {
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
struct SnapPoint {
    position: Point2,
    depth: f64,
    source: usize,
    part: BodyPart,
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
    depth: f64,
    source: usize,
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
        for (body, state) in &basis.bodies {
            let Some(result) = state.solid() else {
                continue;
            };
            let name = document
                .feature(*body)
                .map_or_else(|| "a body".to_owned(), |feature| feature.name.clone());
            snaps.add_corners(result, *body, &name, &basis.plane);
            snaps.add_edges(result, *body, &name, &basis.plane);
        }
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

    fn add_corners(&mut self, result: &SolidResult, body: FeatureId, name: &str, plane: &Plane) {
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
            });
        }
    }

    fn add_edges(&mut self, result: &SolidResult, body: FeatureId, name: &str, plane: &Plane) {
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
                    });
                }
            }
            self.edges.push(SnapEdge {
                path,
                depth,
                source,
            });
        }
    }

    pub fn point(&self, screen: &impl Screen, pointer: Pointer, accept: Accept) -> Option<Snapped> {
        nearest(
            self.points
                .iter()
                .filter(|point| point.part.takes(accept))
                .map(|point| (point.position, point.source, point.part)),
            screen,
            pointer,
            snap::POINT_TOLERANCE,
        )
        .map(|(position, source, part)| self.snapped(position, source, part))
    }

    pub fn edge(&self, screen: &impl Screen, pointer: Pointer, accept: Accept) -> Option<Snapped> {
        if !BodyPart::Edge.takes(accept) {
            return None;
        }
        nearest(
            self.edges.iter().filter_map(|edge| {
                Some((
                    edge.path.closest(pointer.sketch)?,
                    edge.source,
                    BodyPart::Edge,
                ))
            }),
            screen,
            pointer,
            snap::CURVE_TOLERANCE,
        )
        .map(|(position, source, part)| self.snapped(position, source, part))
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
    use std::f64::consts::FRAC_PI_2;

    use caditor_document::Outline;
    use caditor_geometry::{Point2, Vector2};

    use super::{BodyPart, Path, middle_of, nearest};
    use crate::snap::{Pointer, Screen, tests::Scaled};

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
}
