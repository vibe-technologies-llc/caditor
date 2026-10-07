use std::{
    collections::{BTreeMap, BTreeSet},
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use caditor_document::{Document, Evaluation, FeatureId, FeatureKind, FeatureResult, SolidResult};
pub use caditor_document::{describe_origin, origin_feature};
use caditor_geometry::{Aabb, Point3};
use caditor_kernel::{
    Curve, EdgeId, EdgeName, EdgeReference, FaceId, FaceName, FaceOrigin, FaceReference,
    MassProperties, Mesh, Solid, Surface, VertexId, VertexName,
};
use caditor_render::{MeshFace, MeshPoint, ShadedMesh};

use crate::{
    blend_tools::{self, ChosenEdges},
    model::Waker,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VertexKey {
    pub name: VertexName,
    pub occurrence: u32,
}

pub fn vertex_keys(body: &SolidResult) -> Vec<(VertexId, VertexKey)> {
    let names = body.names();
    body.solid
        .vertices()
        .filter_map(|(id, _)| {
            let name = names.vertex_name(id)?;
            let occurrence = names
                .vertices_named(name)
                .iter()
                .position(|same| *same == id)?;
            Some((
                id,
                VertexKey {
                    name,
                    occurrence: u32::try_from(occurrence).ok()?,
                },
            ))
        })
        .collect()
}

pub fn find_vertex(body: &SolidResult, key: VertexKey) -> Option<VertexId> {
    let occurrence = usize::try_from(key.occurrence).ok()?;
    body.names()
        .vertices_named(key.name)
        .get(occurrence)
        .copied()
}

pub fn shown(evaluation: &Evaluation, body: FeatureId) -> Option<&SolidResult> {
    evaluation.body_result(body)?.solid()
}

pub fn input(evaluation: &Evaluation, feature: FeatureId) -> Option<&SolidResult> {
    evaluation.body_before(feature)?.solid()
}

pub fn find_face(body: &SolidResult, key: FaceKey) -> Option<FaceId> {
    let occurrence = usize::try_from(key.occurrence).ok()?;
    body.names().faces_named(key.name).get(occurrence).copied()
}

pub fn find_edge(body: &SolidResult, name: EdgeName) -> Option<EdgeId> {
    body.names().edge_named(name)
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

pub fn is_seam(solid: &Solid, edge: EdgeId) -> bool {
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyVertex {
    pub key: VertexKey,
    pub position: Point3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MassAccuracy {
    Exact,
    Mesh { chord: f64, volume_within: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyMass {
    pub properties: MassProperties,
    pub accuracy: MassAccuracy,
    pub size: Option<[f64; 3]>,
}

impl BodyMass {
    pub fn of(solid: &Solid, mesh: &Mesh) -> Self {
        let curved: BTreeSet<FaceId> = solid
            .faces()
            .filter(|(_, face)| !matches!(face.surface(), Surface::Plane(_)))
            .map(|(id, _)| id)
            .collect();
        let straight = solid
            .edges()
            .all(|(_, edge)| matches!(edge.curve(), Curve::Line(_)));
        let accuracy = if curved.is_empty() && straight {
            MassAccuracy::Exact
        } else {
            let curved_area = mesh
                .mass_properties_where(|face| curved.contains(&face))
                .area;
            MassAccuracy::Mesh {
                chord: mesh.chord(),
                volume_within: curved_area * mesh.chord(),
            }
        };
        let size = Aabb::from_points(mesh.positions().iter().copied()).map(|bounds| {
            let extent = bounds.max() - bounds.min();
            [extent.x, extent.y, extent.z]
        });
        Self {
            properties: mesh.mass_properties(),
            accuracy,
            size,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BodyMesh {
    source: Arc<FeatureResult>,
    pub mesh: Arc<ShadedMesh>,
    pub faces: Vec<BodyFace>,
    pub edges: Vec<BodyEdge>,
    pub vertices: Vec<BodyVertex>,
    pub mass: BodyMass,
}

impl BodyMesh {
    fn of(source: &Arc<FeatureResult>) -> Option<Self> {
        let solid = source.solid()?;
        Some(Self::build(source, solid, solid.mesh()?))
    }

    fn build(source: &Arc<FeatureResult>, body: &SolidResult, mesh: &Mesh) -> Self {
        let solid = &body.solid;
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
        let vertices = vertex_keys(body)
            .into_iter()
            .filter_map(|(id, key)| {
                Some(BodyVertex {
                    key,
                    position: solid.vertex(id)?.point(),
                })
            })
            .collect();
        Self {
            source: Arc::clone(source),
            mesh: Arc::new(ShadedMesh::new(shaded)),
            faces,
            edges,
            vertices,
            mass: BodyMass::of(solid, mesh),
        }
    }

    pub fn vertex_position(&self, key: VertexKey) -> Option<Point3> {
        self.vertices
            .iter()
            .find(|vertex| vertex.key == key)
            .map(|vertex| vertex.position)
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
    pub before: Arc<BodyMesh>,
    pub choice: OpenChoice,
}

#[derive(Debug, Clone, Default)]
pub struct BodyMeshes {
    bodies: BTreeMap<FeatureId, Arc<BodyMesh>>,
    open: Option<BodyBefore>,
    generation: u64,
}

impl BodyMeshes {
    pub fn body_before(&self) -> Option<&BodyBefore> {
        self.open.as_ref()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn changed(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn update_open(
        &mut self,
        document: &Document,
        evaluation: &Evaluation,
        meshing: &BodyMeshing,
        feature: Option<FeatureId>,
    ) {
        let shown_before = self.open.clone();
        let previous = self
            .open
            .take()
            .filter(|open| Some(open.feature) == feature);
        self.open = feature.and_then(|feature| {
            let input = evaluation.body_before(feature)?;
            let solid = input.solid()?;
            let kind = document.feature(feature).map(|owner| &owner.kind);
            let (before, choice) = match (previous, meshing.lookup(input)) {
                (Some(open), _) if Arc::ptr_eq(&open.before.source, input) => {
                    let choice = OpenChoice::of(kind, &solid.solid, Some(open.choice));
                    (open.before, choice)
                }
                (_, Converted::Ready(mesh)) => {
                    (Arc::clone(mesh), OpenChoice::of(kind, &solid.solid, None))
                }
                (Some(open), Converted::Pending) => {
                    (open.before, OpenChoice::of(kind, &solid.solid, None))
                }
                (_, Converted::Pending | Converted::Missing) => return None,
            };
            Some(BodyBefore {
                feature,
                body: solid.body,
                before,
                choice,
            })
        });
        let same = match (&shown_before, &self.open) {
            (Some(shown), Some(open)) => {
                shown.feature == open.feature
                    && shown.body == open.body
                    && Arc::ptr_eq(&shown.before, &open.before)
                    && shown.choice == open.choice
            }
            (None, None) => true,
            (Some(_), None) | (None, Some(_)) => false,
        };
        if !same {
            self.changed();
        }
    }

    pub fn update(&mut self, evaluation: &Evaluation, meshing: &BodyMeshing) {
        let mut previous = std::mem::take(&mut self.bodies);
        let mut changed = false;
        for (body, _) in evaluation.bodies() {
            let Some(result) = evaluation.body_result(body) else {
                continue;
            };
            let held = previous.remove(&body);
            let current = match &held {
                Some(cached) if Arc::ptr_eq(&cached.source, result) => held.clone(),
                _ => match meshing.lookup(result) {
                    Converted::Ready(mesh) => Some(Arc::clone(mesh)),
                    Converted::Pending => held.clone(),
                    Converted::Missing => None,
                },
            };
            changed |= match (&held, &current) {
                (Some(held), Some(current)) => !Arc::ptr_eq(held, current),
                (None, None) => false,
                (Some(_), None) | (None, Some(_)) => true,
            };
            if let Some(current) = current {
                self.bodies.insert(body, current);
            }
        }
        if changed || !previous.is_empty() {
            self.changed();
        }
    }

    pub fn get(&self, body: FeatureId) -> Option<&BodyMesh> {
        self.bodies.get(&body).map(Arc::as_ref)
    }

    pub fn iter(&self) -> impl Iterator<Item = (FeatureId, &BodyMesh)> {
        self.bodies
            .iter()
            .map(|(body, mesh)| (*body, mesh.as_ref()))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Converted<'a> {
    Ready(&'a Arc<BodyMesh>),
    Pending,
    Missing,
}

struct Conversion {
    source: Arc<FeatureResult>,
    mesh: Option<Arc<BodyMesh>>,
}

struct Converter {
    jobs: Sender<Arc<FeatureResult>>,
    done: Receiver<Conversion>,
}

impl Converter {
    fn spawn(wake: Waker) -> Option<Self> {
        let (jobs, queue) = mpsc::channel::<Arc<FeatureResult>>();
        let (sender, done) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("body meshes".to_owned())
            .spawn(move || {
                while let Ok(source) = queue.recv() {
                    if sender.send(convert(source)).is_err() {
                        break;
                    }
                    wake();
                }
            });
        match spawned {
            Ok(_) => Some(Self { jobs, done }),
            Err(error) => {
                log::error!("could not start the body mesh worker: {error}");
                None
            }
        }
    }
}

fn convert(source: Arc<FeatureResult>) -> Conversion {
    let mesh = panic::catch_unwind(AssertUnwindSafe(|| BodyMesh::of(&source)))
        .unwrap_or_else(|_| {
            log::error!("preparing a body mesh for display panicked");
            None
        })
        .map(Arc::new);
    Conversion { source, mesh }
}

#[derive(Default)]
pub struct BodyMeshing {
    converter: Option<Converter>,
    pending: Vec<Arc<FeatureResult>>,
    converted: Vec<Conversion>,
}

impl BodyMeshing {
    pub fn lookup(&self, source: &Arc<FeatureResult>) -> Converted<'_> {
        let conversion = self
            .converted
            .iter()
            .find(|conversion| Arc::ptr_eq(&conversion.source, source));
        match (conversion, source.solid()) {
            (Some(conversion), _) => conversion
                .mesh
                .as_ref()
                .map_or(Converted::Missing, Converted::Ready),
            (None, Some(solid)) if !solid.mesh_failed() => Converted::Pending,
            (None, Some(_) | None) => Converted::Missing,
        }
    }

    pub fn is_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn request(&mut self, source: &Arc<FeatureResult>, wake: impl FnOnce() -> Waker) {
        let meshed = source.solid().is_some_and(|solid| solid.mesh().is_some());
        let known = self
            .pending
            .iter()
            .chain(self.converted.iter().map(|conversion| &conversion.source))
            .any(|known| Arc::ptr_eq(known, source));
        if !meshed || known {
            return;
        }
        if self.converter.is_none() {
            self.converter = Converter::spawn(wake());
        }
        let sent = self
            .converter
            .as_ref()
            .is_some_and(|converter| converter.jobs.send(Arc::clone(source)).is_ok());
        if sent {
            self.pending.push(Arc::clone(source));
        } else {
            log::error!("no body mesh worker, so the mesh is prepared on the UI thread");
            self.converter = None;
            self.converted.push(convert(Arc::clone(source)));
        }
    }

    pub fn poll(&mut self) -> bool {
        let Some(converter) = &self.converter else {
            return false;
        };
        let mut arrived = false;
        while let Ok(conversion) = converter.done.try_recv() {
            self.pending
                .retain(|pending| !Arc::ptr_eq(pending, &conversion.source));
            self.converted.push(conversion);
            arrived = true;
        }
        arrived
    }

    pub fn retain(&mut self, keep: impl Fn(&Arc<FeatureResult>) -> bool) {
        self.pending.retain(|pending| keep(pending));
        self.converted.retain(|conversion| keep(&conversion.source));
    }
}

pub fn describe_face_id(document: &Document, body: &SolidResult, face: FaceId) -> String {
    let Some(definition) = body.solid.face(face) else {
        return "Missing face".to_owned();
    };
    let text = describe_origin(document, definition.origin());
    let parts = body.names().faces_named(definition.name());
    let occurrence = parts.iter().position(|part| *part == face).unwrap_or(0);
    if parts.len() > 1 {
        format!(
            "{text}, part {} of {}",
            occurrence.saturating_add(1),
            parts.len()
        )
    } else {
        text
    }
}

pub fn describe_face(document: &Document, body: &SolidResult, key: FaceKey) -> String {
    match find_face(body, key) {
        Some(face) => describe_face_id(document, body, face),
        None => "Missing face".to_owned(),
    }
}

pub fn describe_edge(document: &Document, body: &SolidResult, name: EdgeName) -> String {
    match find_edge(body, name) {
        Some(edge) => describe_edge_id(document, body, edge),
        None => "Missing edge".to_owned(),
    }
}

pub fn describe_edge_id(document: &Document, body: &SolidResult, edge: EdgeId) -> String {
    match edge_faces(&body.solid, edge).as_slice() {
        [first, second] => format!(
            "Edge between {} and {}",
            describe_face_id(document, body, *first),
            describe_face_id(document, body, *second)
        ),
        _ => "Edge".to_owned(),
    }
}

pub fn describe_vertex(document: &Document, body: &SolidResult, key: VertexKey) -> String {
    let Some(vertex) = find_vertex(body, key) else {
        return "Missing vertex".to_owned();
    };
    let mut faces: Vec<FaceId> = body
        .solid
        .edges()
        .filter(|(_, edge)| edge.start() == vertex || edge.end() == vertex)
        .flat_map(|(id, _)| edge_faces(&body.solid, id))
        .collect();
    faces.sort_unstable();
    faces.dedup();
    let described: Vec<String> = faces
        .into_iter()
        .map(|face| describe_face_id(document, body, face))
        .collect();
    match described.as_slice() {
        [] => "Vertex".to_owned(),
        [only] => format!("Vertex of {only}"),
        [rest @ .., last] => format!("Vertex where {} and {last} meet", rest.join(", ")),
    }
}

pub fn face_origin(body: &SolidResult, key: FaceKey) -> Option<FaceOrigin> {
    body.solid.face(find_face(body, key)?)?.origin()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use caditor_document::{CancelToken, Edit, ModelEvaluator, Recompute, Transaction};
    use caditor_expression::Expression;

    use super::*;
    use crate::samples::Sample;

    const TIMEOUT: Duration = Duration::from_secs(20);

    fn no_wake() -> Waker {
        Box::new(|| {})
    }

    fn evaluate(recompute: &mut Recompute, document: &Document) -> Evaluation {
        recompute.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
    }

    fn only_body(evaluation: &Evaluation) -> (FeatureId, Arc<FeatureResult>) {
        let (body, _) = evaluation.bodies().next().unwrap();
        (body, Arc::clone(evaluation.body_result(body).unwrap()))
    }

    fn wait_for(meshing: &mut BodyMeshing) {
        let deadline = Instant::now() + TIMEOUT;
        while meshing.is_pending() {
            assert!(
                Instant::now() < deadline,
                "the body mesh was never prepared"
            );
            meshing.poll();
            thread::yield_now();
        }
    }

    #[test]
    fn body_meshes_are_prepared_on_a_worker_and_shown_once_they_arrive() {
        let document = Sample::Spool.document().unwrap();
        let evaluation = evaluate(&mut Recompute::default(), &document);
        let (body, source) = only_body(&evaluation);
        let mut meshing = BodyMeshing::default();
        let mut meshes = BodyMeshes::default();

        meshing.request(&source, no_wake);
        meshing.request(&source, no_wake);
        meshes.update(&evaluation, &meshing);

        assert!(meshing.is_pending());
        assert_eq!(meshing.pending.len(), 1);
        assert!(matches!(meshing.lookup(&source), Converted::Pending));
        assert!(meshes.get(body).is_none());

        wait_for(&mut meshing);
        meshes.update(&evaluation, &meshing);
        let shown = meshes.get(body).unwrap();
        let direct = BodyMesh::of(&source).unwrap();

        assert_eq!(shown.faces, direct.faces);
        assert_eq!(shown.edges, direct.edges);
        assert_eq!(shown.mesh.face_count(), direct.mesh.face_count());
        assert_eq!(shown.bounds(), direct.bounds());
    }

    #[test]
    fn faces_and_edges_are_found_by_key_and_name_through_the_cached_index() {
        for sample in Sample::ALL {
            let document = sample.document().unwrap();
            let evaluation = evaluate(&mut Recompute::default(), &document);
            let (body, _) = only_body(&evaluation);
            let shown = super::shown(&evaluation, body).unwrap();

            for (id, key) in face_keys(&shown.solid) {
                assert_eq!(find_face(shown, key), Some(id));
                let parts = shown.names().faces_named(key.name).len();
                let described = describe_face_id(&document, shown, id);
                assert_eq!(
                    described.ends_with(&format!("part {} of {parts}", key.occurrence + 1)),
                    parts > 1,
                    "{described}"
                );
            }
            for (id, edge) in shown.solid.edges() {
                let found = find_edge(shown, edge.name()).unwrap();
                assert_eq!(shown.solid.edge(found).unwrap().name(), edge.name());
                assert!(found <= id);
            }
            let missing = FaceKey {
                name: FaceName::NONE,
                occurrence: 0,
            };
            assert_eq!(find_face(shown, missing), None);
        }
    }

    #[test]
    fn the_generation_moves_only_when_a_shown_mesh_arrives_or_goes() {
        let document = Sample::Spool.document().unwrap();
        let evaluation = evaluate(&mut Recompute::default(), &document);
        let (_, source) = only_body(&evaluation);
        let mut meshing = BodyMeshing::default();
        let mut meshes = BodyMeshes::default();

        meshes.update(&evaluation, &meshing);
        let pending = meshes.generation();
        meshing.request(&source, no_wake);
        wait_for(&mut meshing);
        meshes.update(&evaluation, &meshing);
        let arrived = meshes.generation();
        meshes.update(&evaluation, &meshing);
        let again = meshes.generation();
        meshes.update(&Evaluation::default(), &meshing);
        let gone = meshes.generation();

        assert_eq!(pending, BodyMeshes::default().generation());
        assert_ne!(arrived, pending);
        assert_eq!(again, arrived);
        assert_ne!(gone, again);
    }

    #[test]
    fn a_changed_body_keeps_its_previous_mesh_until_the_new_one_is_ready() {
        let mut document = Sample::Spool.document().unwrap();
        let mut recompute = Recompute::default();
        let first = evaluate(&mut recompute, &document);
        let (body, old) = only_body(&first);
        let mut meshing = BodyMeshing::default();
        let mut meshes = BodyMeshes::default();
        meshing.request(&old, no_wake);
        wait_for(&mut meshing);
        meshes.update(&first, &meshing);
        let old_bounds = meshes.get(body).unwrap().bounds().unwrap();

        let height = document.parameter_named("height").unwrap().id();
        document
            .apply(Transaction::single(
                "Taller",
                Edit::SetParameterExpression {
                    id: height,
                    expression: Expression::parse_stored("70 mm").unwrap(),
                },
            ))
            .unwrap();
        let second = evaluate(&mut recompute, &document);
        let (_, new) = only_body(&second);
        meshing.retain(|source| Arc::ptr_eq(source, &new));
        meshing.request(&new, no_wake);
        meshes.update(&second, &meshing);

        assert!(!Arc::ptr_eq(&old, &new));
        assert!(matches!(meshing.lookup(&old), Converted::Pending));
        assert!(Arc::ptr_eq(&meshes.bodies[&body].source, &old));

        wait_for(&mut meshing);
        meshes.update(&second, &meshing);
        let taller = meshes.get(body).unwrap().bounds().unwrap();

        assert!(Arc::ptr_eq(&meshes.bodies[&body].source, &new));
        assert!(taller.diagonal() > old_bounds.diagonal() + 10.0);
    }
}
