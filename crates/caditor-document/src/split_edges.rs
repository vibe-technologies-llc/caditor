use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Point3, Vector3};
use caditor_kernel::{
    Curve, Edge, EdgeId, EdgeNaming, EdgeReference, FaceId, FaceName, FaceOrigin, Solid, VertexId,
    VertexName,
};
use caditor_sketch::EntityId;

use crate::{
    describe::edge_faces,
    document::{Document, FeatureId},
    edge_run::EdgeRun,
    pieces::{Resolution, pieces_of_one_edge},
    tolerance::POSITION_TOLERANCE,
};

const ANGLE_RESOLUTION: f64 = 1e-9;

type PieceEdges<'a> = Vec<(EdgeId, &'a Edge)>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Followed {
    Resolved(Resolution<EdgeId>),
    PartlyLost(Vec<EdgeId>),
}

impl Followed {
    pub(crate) fn resolution(self) -> Resolution<EdgeId> {
        match self {
            Self::Resolved(resolution) => resolution,
            Self::PartlyLost(found) => Resolution::Tied(found),
        }
    }

    pub(crate) fn is_widened(&self) -> bool {
        matches!(
            self,
            Self::Resolved(Resolution::Pieces(_)) | Self::PartlyLost(_)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unrun {
    Missing,
    Tied,
    Gapped,
    PartlyTraced,
}

pub(crate) struct SplitEdges<'a> {
    document: &'a Document,
    solid: &'a Solid,
    naming: EdgeNaming,
    around: BTreeMap<VertexId, BTreeSet<FaceId>>,
}

impl<'a> SplitEdges<'a> {
    pub(crate) fn new(document: &'a Document, solid: &'a Solid) -> Self {
        let mut around: BTreeMap<VertexId, BTreeSet<FaceId>> = BTreeMap::new();
        for (id, edge) in solid.edges() {
            let faces = edge_faces(solid, id);
            for vertex in [edge.start(), edge.end()] {
                around
                    .entry(vertex)
                    .or_default()
                    .extend(faces.iter().copied());
            }
        }
        Self {
            document,
            solid,
            naming: EdgeNaming::new(solid),
            around,
        }
    }

    pub(crate) fn follow(&self, reference: &EdgeReference) -> Followed {
        let resolution = Resolution::of(reference.resolve_in(&self.naming), |pieces| {
            pieces_of_one_edge(self.solid, pieces)
        });
        let Resolution::One(found) = resolution else {
            return Followed::Resolved(resolution);
        };
        let Some(along) = SplitCurve::of(self.document, reference) else {
            return Followed::Resolved(Resolution::One(found));
        };
        match self.widen(&along, reference, found) {
            Some(followed) => followed,
            None => Followed::Resolved(Resolution::One(found)),
        }
    }

    pub(crate) fn run(&self, reference: &EdgeReference) -> Result<EdgeRun<'a>, Unrun> {
        let pieces = match self.follow(reference) {
            Followed::Resolved(Resolution::One(found)) => {
                return self
                    .solid
                    .edge(found)
                    .map(|edge| EdgeRun::single(found, edge))
                    .ok_or(Unrun::Missing);
            }
            Followed::Resolved(Resolution::Pieces(pieces)) => pieces,
            Followed::Resolved(Resolution::Tied(_)) => return Err(Unrun::Tied),
            Followed::Resolved(Resolution::Missing) => return Err(Unrun::Missing),
            Followed::PartlyLost(_) => return Err(Unrun::PartlyTraced),
        };
        let leading = reference.resolve_in(&self.naming).ok();
        EdgeRun::chained(self.solid, &pieces, leading).ok_or(Unrun::Gapped)
    }

    fn widen(
        &self,
        along: &SplitCurve,
        reference: &EdgeReference,
        found: EdgeId,
    ) -> Option<Followed> {
        let edge = self.solid.edge(found)?;
        let shared = self.shared_face(along, found)?;
        let mut wanted: Vec<VertexName> = reference.ends().to_vec();
        let mut open = Vec::new();
        for vertex in [edge.start(), edge.end()] {
            let name = self.end_name(along, shared, vertex);
            match wanted.iter().position(|end| *end == name) {
                Some(index) => {
                    wanted.remove(index);
                }
                None => open.push(vertex),
            }
        }
        if wanted.is_empty() || open.is_empty() || edge.is_closed() {
            return None;
        }

        let (candidates, strays): (PieceEdges<'_>, PieceEdges<'_>) = self
            .solid
            .edges()
            .filter(|(id, _)| *id != found && self.shared_face(along, *id) == Some(shared))
            .partition(|(id, _)| pieces_of_one_edge(self.solid, &[found, *id]));
        let mut widened = vec![found];
        let mut lost = false;
        for vertex in open {
            match self.walk(along, shared, edge, vertex, &candidates, &mut wanted) {
                Walk::Reached(walked) => widened.extend(walked),
                Walk::Unordered => lost = true,
                Walk::Unreached => lost |= self.reaches_any(along, shared, &strays, &wanted),
            }
        }
        widened.sort_unstable();
        widened.dedup();
        Some(if lost {
            Followed::PartlyLost(widened)
        } else if widened.len() > 1 {
            Followed::Resolved(Resolution::Pieces(widened))
        } else {
            Followed::Resolved(Resolution::One(found))
        })
    }

    fn walk(
        &self,
        along: &SplitCurve,
        shared: FaceId,
        edge: &Edge,
        vertex: VertexId,
        candidates: &[(EdgeId, &Edge)],
        wanted: &mut Vec<VertexName>,
    ) -> Walk {
        let Some(from) = self.solid.vertex(vertex).map(|vertex| vertex.point()) else {
            return Walk::Unreached;
        };
        let Some(travel) = Travel::of(edge.curve(), from, midpoint(edge)) else {
            return if candidates.is_empty() {
                Walk::Unreached
            } else {
                Walk::Unordered
            };
        };
        let mut ahead: Vec<(f64, EdgeId, &Edge)> = candidates
            .iter()
            .filter_map(|(id, candidate)| {
                Some((travel.distance(midpoint(candidate))?, *id, *candidate))
            })
            .collect();
        ahead.sort_by(|first, second| first.0.total_cmp(&second.0));
        let mut walked = Vec::new();
        for (_, id, candidate) in ahead {
            walked.push(id);
            if let Some(index) = self.reached(along, shared, candidate, wanted) {
                wanted.remove(index);
                return Walk::Reached(walked);
            }
        }
        Walk::Unreached
    }

    fn reached(
        &self,
        along: &SplitCurve,
        shared: FaceId,
        candidate: &Edge,
        wanted: &[VertexName],
    ) -> Option<usize> {
        [candidate.start(), candidate.end()]
            .into_iter()
            .find_map(|end| {
                let name = self.end_name(along, shared, end);
                wanted.iter().position(|wanted| *wanted == name)
            })
    }

    fn reaches_any(
        &self,
        along: &SplitCurve,
        shared: FaceId,
        edges: &[(EdgeId, &Edge)],
        wanted: &[VertexName],
    ) -> bool {
        edges
            .iter()
            .any(|(_, edge)| self.reached(along, shared, edge, wanted).is_some())
    }

    fn shared_face(&self, along: &SplitCurve, edge: EdgeId) -> Option<FaceId> {
        let faces = edge_faces(self.solid, edge);
        let [first, second] = faces.as_slice() else {
            return None;
        };
        match (
            self.is_piece_side(along, *first),
            self.is_piece_side(along, *second),
        ) {
            (true, false) => Some(*second),
            (false, true) => Some(*first),
            _ => None,
        }
    }

    fn is_piece_side(&self, along: &SplitCurve, face: FaceId) -> bool {
        self.solid
            .face(face)
            .and_then(|face| face.origin())
            .is_some_and(|origin| along.made(origin))
    }

    fn end_name(&self, along: &SplitCurve, shared: FaceId, vertex: VertexId) -> VertexName {
        let names = self
            .around
            .get(&vertex)
            .into_iter()
            .flatten()
            .filter_map(|face| {
                if *face == shared {
                    return Some(along.other);
                }
                if self.is_piece_side(along, *face) {
                    return Some(along.side);
                }
                self.solid.face(*face).map(|face| face.name())
            });
        VertexName::of_faces(names)
    }
}

enum Walk {
    Reached(Vec<EdgeId>),
    Unreached,
    Unordered,
}

struct SplitCurve {
    feature: u64,
    pieces: BTreeSet<u64>,
    side: FaceName,
    other: FaceName,
}

impl SplitCurve {
    fn of(document: &Document, reference: &EdgeReference) -> Option<Self> {
        let [first, second] = reference.faces();
        let [first_origin, second_origin] = reference.origins();
        let sides = [(first, first_origin), (second, second_origin)];
        sides
            .iter()
            .enumerate()
            .find_map(|(index, (side, origin))| {
                let FaceOrigin::Side { feature, entity } = (*origin)? else {
                    return None;
                };
                let pieces = pieces_of(document, feature, entity)?;
                let (other, other_origin) = sides.get(1 - index)?;
                let split = Self {
                    feature,
                    pieces,
                    side: *side,
                    other: *other,
                };
                if other_origin.is_some_and(|origin| split.made(origin)) {
                    return None;
                }
                Some(split)
            })
    }

    fn made(&self, origin: FaceOrigin) -> bool {
        matches!(
            origin,
            FaceOrigin::Side { feature, entity }
                if feature == self.feature && self.pieces.contains(&entity)
        )
    }
}

fn pieces_of(document: &Document, feature: u64, entity: u64) -> Option<BTreeSet<u64>> {
    let solid = document
        .feature(FeatureId::from_raw(feature))?
        .kind
        .solid()?;
    let sketch = document.feature(solid.sketch())?.kind.sketch()?;
    let pieces: BTreeSet<u64> = sketch
        .pieces_of(EntityId::from_raw(entity))
        .into_iter()
        .map(EntityId::raw)
        .collect();
    (pieces.len() > 1).then_some(pieces)
}

fn midpoint(edge: &Edge) -> Point3 {
    let interval = edge.interval();
    edge.curve()
        .point(0.5 * (interval.start() + interval.end()))
}

enum Travel {
    Straight {
        from: Point3,
        ahead: Vector3,
    },
    Around {
        centre: Point3,
        normal: Vector3,
        from: Vector3,
        sense: f64,
    },
}

impl Travel {
    fn of(curve: &Curve, from: Point3, behind: Point3) -> Option<Self> {
        match curve {
            Curve::Line(_) => Some(Self::Straight {
                from,
                ahead: (from - behind).try_normalize()?,
            }),
            Curve::Circle(circle) => {
                Self::around(circle.center(), circle.frame().normal(), from, behind)
            }
            Curve::Ellipse(ellipse) => {
                Self::around(ellipse.center(), ellipse.frame().normal(), from, behind)
            }
            _ => None,
        }
    }

    fn around(centre: Point3, normal: Vector3, from: Point3, behind: Point3) -> Option<Self> {
        let start = from - centre;
        let angle = signed_angle(normal, start, behind - centre);
        (angle.abs() > ANGLE_RESOLUTION).then(|| Self::Around {
            centre,
            normal,
            from: start,
            sense: -angle.signum(),
        })
    }

    fn distance(&self, point: Point3) -> Option<f64> {
        let distance = match self {
            Self::Straight { from, ahead } => (point - *from).dot(*ahead),
            Self::Around {
                centre,
                normal,
                from,
                sense,
            } => (sense * signed_angle(*normal, *from, point - *centre))
                .rem_euclid(std::f64::consts::TAU),
        };
        let resolution = match self {
            Self::Straight { .. } => POSITION_TOLERANCE,
            Self::Around { .. } => ANGLE_RESOLUTION,
        };
        (distance > resolution).then_some(distance)
    }
}

fn signed_angle(normal: Vector3, from: Vector3, to: Vector3) -> f64 {
    normal.dot(from.cross(to)).atan2(from.dot(to))
}
