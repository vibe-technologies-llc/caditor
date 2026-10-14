use caditor_document::{FeatureId, FeatureKind, SolidResult, face_plane};
use caditor_geometry::{Plane, Point2, Point3, Ray, Vector2, Vector3};
use caditor_kernel::{
    Curve, EdgeName, FaceContainment, FaceId, Interval, Line, intersect_curve_surface,
};
use caditor_render::View;

use crate::{
    bodies,
    model::Model,
    scene::{BuiltScene, PickPriority},
    selection::Pickable,
};

const MIDDLE_REACH_POINTS: f64 = 12.0;
const DEPTH_SLACK: f64 = 1e-3;
const SQUARE_ENOUGH: f64 = 0.1;
const MAGNET_STEPS: f64 = 2.0;
const LINE_MARGIN: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Guide {
    Line { base: Point3, direction: Vector3 },
    Points,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapKind {
    Corner,
    EdgeMiddle,
    RoundCentre,
    Face,
}

impl SnapKind {
    pub fn words(self) -> &'static str {
        match self {
            Self::Corner => "to a corner",
            Self::EdgeMiddle => "to an edge's middle",
            Self::RoundCentre => "to a round edge's centre",
            Self::Face => "to a face",
        }
    }

    fn rank(self) -> PickPriority {
        match self {
            Self::Corner | Self::RoundCentre => PickPriority::Point,
            Self::EdgeMiddle => PickPriority::Curve,
            Self::Face => PickPriority::Surface,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snap {
    pub kind: SnapKind,
    pub point: Point3,
    normal: Option<Vector3>,
}

impl Snap {
    pub fn holds(&self, snapped: f64, pulled: Option<f64>, step: f64) -> bool {
        match self.kind {
            SnapKind::Face => {
                pulled.is_some_and(|pulled| (snapped - pulled).abs() <= MAGNET_STEPS * step)
            }
            SnapKind::Corner | SnapKind::EdgeMiddle | SnapKind::RoundCentre => true,
        }
    }

    pub fn holds_on_track(&self, off: f64, snapped: f64, pulled: Option<f64>, step: f64) -> bool {
        let reach = MAGNET_STEPS * step;
        off <= reach && pulled.is_some_and(|pulled| (snapped - pulled).abs() <= reach)
    }

    pub fn along(&self, base: Point3, direction: Vector3) -> Option<f64> {
        match self.normal {
            Some(normal) => {
                let facing = direction.dot(normal);
                (facing.abs() >= SQUARE_ENOUGH).then(|| (self.point - base).dot(normal) / facing)
            }
            None => Some((self.point - base).dot(direction)),
        }
    }

    pub fn on_plane(&self, plane: &Plane) -> Option<Point2> {
        self.pinned_point().map(|point| plane.to_local(point))
    }

    pub fn pinned_point(&self) -> Option<Point3> {
        match self.kind {
            SnapKind::Face => None,
            SnapKind::Corner | SnapKind::EdgeMiddle | SnapKind::RoundCentre => Some(self.point),
        }
    }
}

pub struct Probe<'a> {
    pub model: &'a Model,
    pub feature: FeatureId,
    pub built: &'a BuiltScene,
    pub view: &'a View,
    pub cursor: Vector2,
    pub pixels_per_point: f32,
    pub guide: Guide,
}

impl Probe<'_> {
    pub fn snap(&self) -> Option<Snap> {
        let ray = self.view.ray_through(self.cursor)?;
        let hits = self
            .built
            .scene
            .hits_through(self.view, self.cursor, self.pixels_per_point);
        let reached = self.built.picks.within_reach(&hits);
        let depth = |at: Point3| (at - ray.origin()).dot(ray.direction());
        let front = reached
            .iter()
            .filter(|(pickable, _)| self.hides_behind(*pickable))
            .map(|(_, hit)| depth(hit.position))
            .fold(f64::INFINITY, f64::min);
        let slack = DEPTH_SLACK * front.abs().max(1.0);
        reached
            .iter()
            .filter(|(_, hit)| depth(hit.position) <= front + slack)
            .filter_map(|(pickable, hit)| self.snap_to(*pickable, hit.position, &ray))
            .min_by_key(|snap| snap.kind.rank())
    }

    fn snap_to(&self, pickable: Pickable, at: Point3, ray: &Ray) -> Option<Snap> {
        match pickable {
            Pickable::Vertex { body, vertex } => {
                let source = self.body(body)?;
                let id = bodies::find_vertex(source, vertex)?;
                Some(Snap {
                    kind: SnapKind::Corner,
                    point: source.solid.vertex(id)?.point(),
                    normal: None,
                })
            }
            Pickable::Edge { body, edge } => self.edge(self.body(body)?, edge),
            Pickable::BlendEdge { feature, edge } => self.edge(before(self.model, feature)?, edge),
            Pickable::Face { body, face } => self.face(self.body(body)?, face, at, ray),
            Pickable::ShellFace { feature, face } => {
                self.face(before(self.model, feature)?, face, at, ray)
            }
            _ => None,
        }
    }

    fn hides_behind(&self, pickable: Pickable) -> bool {
        match pickable {
            Pickable::Face { body, face } => self
                .body(body)
                .is_some_and(|source| bodies::find_face(source, face).is_some()),
            Pickable::ShellFace { feature, face } => before(self.model, feature)
                .is_some_and(|source| bodies::find_face(source, face).is_some()),
            _ => false,
        }
    }

    fn body(&self, body: FeatureId) -> Option<&SolidResult> {
        let owner = self.model.document().feature(self.feature)?;
        let evaluation = self.model.evaluation();
        if owner.body() != Some(body) {
            return bodies::shown(evaluation, body);
        }
        match owner.kind {
            FeatureKind::Move(_) => None,
            _ => before(self.model, self.feature),
        }
    }

    fn edge(&self, source: &SolidResult, name: EdgeName) -> Option<Snap> {
        let id = bodies::find_edge(source, name)?;
        let edge = source.solid.edge(id)?;
        if let Curve::Circle(circle) = edge.curve() {
            return Some(Snap {
                kind: SnapKind::RoundCentre,
                point: circle.center(),
                normal: None,
            });
        }
        let middle = edge.curve().point(edge.interval().middle());
        let seen = self.view.project(middle)?;
        let reach = MIDDLE_REACH_POINTS * f64::from(self.pixels_per_point);
        (seen.distance(self.cursor) <= reach).then_some(Snap {
            kind: SnapKind::EdgeMiddle,
            point: middle,
            normal: None,
        })
    }

    fn face(
        &self,
        source: &SolidResult,
        key: bodies::FaceKey,
        at: Point3,
        ray: &Ray,
    ) -> Option<Snap> {
        let Guide::Line { base, direction } = self.guide else {
            return None;
        };
        let id = bodies::find_face(source, key)?;
        let Some(plane) = face_plane(&source.solid, id) else {
            return Some(Snap {
                kind: SnapKind::Face,
                point: meeting_curved(source, id, base, direction, at)?,
                normal: None,
            });
        };
        let meets = ray
            .intersect_plane(&plane)
            .map_or(at, |along| ray.at(along));
        Some(Snap {
            kind: SnapKind::Face,
            point: meets,
            normal: Some(plane.normal()),
        })
    }
}

fn meeting_curved(
    source: &SolidResult,
    face: FaceId,
    base: Point3,
    direction: Vector3,
    near: Point3,
) -> Option<Point3> {
    let line = Line::new(base, direction).ok()?;
    let along = source
        .bounding_box()?
        .corners()
        .map(|corner| (corner - base).dot(line.direction()));
    let low = along.iter().copied().fold(f64::INFINITY, f64::min);
    let high = along.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let range = Interval::new(low - LINE_MARGIN, high + LINE_MARGIN)?;
    let surface = source.solid.face(face)?.surface();
    let found = intersect_curve_surface(&Curve::Line(line), range, surface, None).ok()?;
    found
        .points
        .iter()
        .filter(|hit| {
            matches!(
                source.solid.point_in_face(face, hit.uv),
                Some(FaceContainment::Inside | FaceContainment::OnBoundary)
            )
        })
        .map(|hit| hit.point)
        .min_by(|first, second| first.distance(near).total_cmp(&second.distance(near)))
}

pub fn before(model: &Model, feature: FeatureId) -> Option<&SolidResult> {
    let evaluation = model.evaluation();
    bodies::input(evaluation, feature).or_else(|| {
        let body = model.document().feature(feature)?.body()?;
        evaluation.body_result_seen_by(feature, body)
    })
}
