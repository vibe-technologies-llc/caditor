use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use caditor_document::{Document, Evaluation, FeatureId, FeatureKind, FeatureResult};
pub use caditor_document::{describe_origin, origin_feature};
use caditor_geometry::{Aabb, Point3};
use caditor_kernel::{
    EdgeId, EdgeName, EdgeReference, FaceId, FaceName, FaceOrigin, FaceReference, Mesh, Solid,
    Surface,
};
use caditor_render::{MeshFace, MeshPoint, ShadedMesh};

use crate::{
    blend_tools::{self, ChosenEdges},
    shell_tools,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FaceKey {
    pub name: FaceName,
    pub occurrence: u32,
}

pub fn face_keys(solid: &Solid) -> Vec<(FaceId, FaceKey)> {
    let mut seen: BTreeMap<FaceName, u32> = BTreeMap::new();
    solid
        .faces()
        .map(|(id, face)| {
            let count = seen.entry(face.name()).or_insert(0);
            let key = FaceKey {
                name: face.name(),
                occurrence: *count,
            };
            *count = count.saturating_add(1);
            (id, key)
        })
        .collect()
}

pub fn input_solid(evaluation: &Evaluation, feature: FeatureId) -> Option<&Solid> {
    Some(&evaluation.body_before(feature)?.solid()?.solid)
}

pub fn find_face(solid: &Solid, key: FaceKey) -> Option<FaceId> {
    face_keys(solid)
        .into_iter()
        .find_map(|(id, candidate)| (candidate == key).then_some(id))
}

pub fn find_edge(solid: &Solid, name: EdgeName) -> Option<EdgeId> {
    solid
        .edges()
        .find_map(|(id, edge)| (edge.name() == name).then_some(id))
}

fn edge_faces(solid: &Solid, edge: EdgeId) -> Vec<FaceId> {
    let Some(edge) = solid.edge(edge) else {
        return Vec::new();
    };
    let mut faces: Vec<FaceId> = edge
        .coedges()
        .iter()
        .filter_map(|coedge| solid.coedge_face(*coedge))
        .collect();
    faces.dedup();
    faces
}

fn is_seam(solid: &Solid, edge: EdgeId) -> bool {
    edge_faces(solid, edge).len() == 1
}

#[derive(Debug, Clone, PartialEq)]
pub struct BodyFace {
    pub key: FaceKey,
    pub flat: bool,
    pub bounds: Option<Aabb>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BodyEdge {
    pub name: EdgeName,
    pub points: Vec<Point3>,
}

#[derive(Debug, Clone)]
pub struct BodyMesh {
    source: Arc<FeatureResult>,
    pub mesh: Arc<ShadedMesh>,
    pub faces: Vec<BodyFace>,
    pub edges: Vec<BodyEdge>,
}

impl BodyMesh {
    fn build(source: &Arc<FeatureResult>, solid: &Solid, mesh: &Mesh) -> Self {
        let keys: BTreeMap<FaceId, FaceKey> = face_keys(solid).into_iter().collect();
        let mut faces = Vec::new();
        let mut shaded = Vec::new();
        for face in mesh.faces() {
            let Some(key) = keys.get(&face.face).copied() else {
                continue;
            };
            let mut corners: BTreeMap<u32, u32> = BTreeMap::new();
            let mut drawn = MeshFace::default();
            for triangle in mesh
                .triangles()
                .get(face.triangles.clone())
                .unwrap_or_default()
            {
                let [a, b, c] =
                    triangle.map(|vertex| local_corner(mesh, vertex, &mut corners, &mut drawn));
                if let (Some(a), Some(b), Some(c)) = (a, b, c) {
                    drawn.triangles.push([a, b, c]);
                }
            }
            faces.push(BodyFace {
                key,
                flat: solid
                    .face(face.face)
                    .is_some_and(|face| matches!(face.surface(), Surface::Plane(_))),
                bounds: Aabb::from_points(drawn.points.iter().map(|point| point.position)),
            });
            shaded.push(drawn);
        }
        let edges = mesh
            .edges()
            .iter()
            .filter(|polyline| !is_seam(solid, polyline.edge))
            .filter_map(|polyline| {
                Some(BodyEdge {
                    name: solid.edge(polyline.edge)?.name(),
                    points: polyline
                        .positions
                        .iter()
                        .filter_map(|position| mesh.position(*position))
                        .collect(),
                })
            })
            .collect();
        Self {
            source: Arc::clone(source),
            mesh: Arc::new(ShadedMesh::new(shaded)),
            faces,
            edges,
        }
    }

    pub fn bounds(&self) -> Option<Aabb> {
        self.mesh.bounds()
    }

    pub fn face_bounds(&self, key: FaceKey) -> Option<Aabb> {
        self.faces
            .iter()
            .find(|face| face.key == key)
            .and_then(|face| face.bounds)
    }

    pub fn edge_points(&self, name: EdgeName) -> Option<&[Point3]> {
        self.edges
            .iter()
            .find(|edge| edge.name == name)
            .map(|edge| edge.points.as_slice())
    }
}

fn local_corner(
    mesh: &Mesh,
    vertex: u32,
    corners: &mut BTreeMap<u32, u32>,
    drawn: &mut MeshFace,
) -> Option<u32> {
    if let Some(index) = corners.get(&vertex) {
        return Some(*index);
    }
    let source = mesh.vertices().get(vertex as usize)?;
    let index = u32::try_from(drawn.points.len()).ok()?;
    drawn.points.push(MeshPoint {
        position: mesh.position(source.position)?,
        normal: source.normal,
    });
    corners.insert(vertex, index);
    Some(index)
}

#[derive(Debug, Clone, PartialEq)]
pub enum OpenChoice {
    Edges {
        references: Vec<EdgeReference>,
        chosen: ChosenEdges,
    },
    Faces {
        references: Vec<FaceReference>,
        opened: BTreeSet<FaceKey>,
    },
    Nothing,
}

impl OpenChoice {
    fn of(kind: Option<&FeatureKind>, solid: &Solid, previous: Option<Self>) -> Self {
        match kind {
            Some(FeatureKind::Blend(blend)) => match previous {
                Some(Self::Edges { references, chosen }) if references == blend.edges => {
                    Self::Edges { references, chosen }
                }
                _ => Self::Edges {
                    references: blend.edges.clone(),
                    chosen: blend_tools::chosen_edges(solid, blend),
                },
            },
            Some(FeatureKind::Shell(shell)) => match previous {
                Some(Self::Faces { references, opened }) if references == shell.open => {
                    Self::Faces { references, opened }
                }
                _ => Self::Faces {
                    references: shell.open.clone(),
                    opened: shell_tools::opened_faces(solid, shell),
                },
            },
            Some(_) | None => Self::Nothing,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BodyBefore {
    pub feature: FeatureId,
    pub body: FeatureId,
    pub before: BodyMesh,
    pub choice: OpenChoice,
}

#[derive(Debug, Clone, Default)]
pub struct BodyMeshes {
    bodies: BTreeMap<FeatureId, BodyMesh>,
    open: Option<BodyBefore>,
}

impl BodyMeshes {
    pub fn body_before(&self) -> Option<&BodyBefore> {
        self.open.as_ref()
    }

    pub fn update_open(
        &mut self,
        document: &Document,
        evaluation: &Evaluation,
        feature: Option<FeatureId>,
    ) {
        let previous = self
            .open
            .take()
            .filter(|open| Some(open.feature) == feature);
        self.open = feature.and_then(|feature| {
            let input = evaluation.body_before(feature)?;
            let solid = input.solid()?;
            let kind = document.feature(feature).map(|owner| &owner.kind);
            let (before, choice) = match previous {
                Some(open) if Arc::ptr_eq(&open.before.source, input) => {
                    let choice = OpenChoice::of(kind, &solid.solid, Some(open.choice));
                    (open.before, choice)
                }
                _ => (
                    BodyMesh::build(input, &solid.solid, solid.mesh()?),
                    OpenChoice::of(kind, &solid.solid, None),
                ),
            };
            Some(BodyBefore {
                feature,
                body: solid.body,
                before,
                choice,
            })
        });
    }

    pub fn update(&mut self, evaluation: &Evaluation) {
        let mut next = BTreeMap::new();
        for (body, _) in evaluation.bodies() {
            let Some(result) = evaluation.body_result(body) else {
                continue;
            };
            let previous = self.bodies.remove(&body);
            let current = match previous {
                Some(cached) if Arc::ptr_eq(&cached.source, result) => Some(cached),
                previous => match result.solid() {
                    Some(solid) if solid.is_meshed() => solid
                        .mesh()
                        .map(|mesh| BodyMesh::build(result, &solid.solid, mesh)),
                    Some(_) | None => previous,
                },
            };
            if let Some(current) = current {
                next.insert(body, current);
            }
        }
        self.bodies = next;
    }

    pub fn get(&self, body: FeatureId) -> Option<&BodyMesh> {
        self.bodies.get(&body)
    }

    pub fn iter(&self) -> impl Iterator<Item = (FeatureId, &BodyMesh)> {
        self.bodies.iter().map(|(body, mesh)| (*body, mesh))
    }
}

pub fn describe_face_id(document: &Document, solid: &Solid, face: FaceId) -> String {
    let Some(definition) = solid.face(face) else {
        return "Missing face".to_owned();
    };
    let text = describe_origin(document, definition.origin());
    let occurrence = face_keys(solid)
        .into_iter()
        .find_map(|(id, key)| (id == face).then_some(key.occurrence))
        .unwrap_or(0);
    let parts = solid
        .faces()
        .filter(|(_, other)| other.name() == definition.name())
        .count();
    if parts > 1 {
        format!("{text}, part {} of {parts}", occurrence.saturating_add(1))
    } else {
        text
    }
}

pub fn describe_face(document: &Document, solid: &Solid, key: FaceKey) -> String {
    match find_face(solid, key) {
        Some(face) => describe_face_id(document, solid, face),
        None => "Missing face".to_owned(),
    }
}

pub fn describe_edge(document: &Document, solid: &Solid, name: EdgeName) -> String {
    match find_edge(solid, name) {
        Some(edge) => describe_edge_id(document, solid, edge),
        None => "Missing edge".to_owned(),
    }
}

pub fn describe_edge_id(document: &Document, solid: &Solid, edge: EdgeId) -> String {
    match edge_faces(solid, edge).as_slice() {
        [first, second] => format!(
            "Edge between {} and {}",
            describe_face_id(document, solid, *first),
            describe_face_id(document, solid, *second)
        ),
        _ => "Edge".to_owned(),
    }
}

pub fn face_origin(solid: &Solid, key: FaceKey) -> Option<FaceOrigin> {
    solid.face(find_face(solid, key)?)?.origin()
}
