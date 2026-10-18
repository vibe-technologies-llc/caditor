use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    num::NonZeroUsize,
    panic::{self, AssertUnwindSafe},
    slice,
    sync::{
        Arc, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use caditor_document::{
    Clipped, Document, EdgeGroup, Evaluation, FeatureId, FeatureKind, FeatureResult, FeatureState,
    Resolution, SolidResult, openable_face,
};
pub use caditor_document::{describe_origin, origin_feature};
use caditor_geometry::{Aabb, Point3, RigidTransform};
use caditor_kernel::{
    EdgeId, EdgeName, EdgeReference, FaceId, FaceName, FaceOrigin, FaceReference, FaceTriangles,
    Interrupted, MassProperties, MeasureError, Mesh, Solid, SolidMass, VertexId, VertexName,
    check_interrupt, extent, interruptible, mass_properties,
};
use caditor_render::{MeshFace, MeshPoint, MeshSource, PolylineSource, ShadedMesh};
use parking_lot::{Condvar, Mutex};

use crate::{
    blend_tools::{self, ChosenEdges},
    delete_face_tools, emboss_tools,
    model::Waker,
    move_face_tools, offset_face_tools,
    selection::Pickable,
    shell_tools, split_face_tools,
};

const SHOWN_TOGETHER_FOR: Duration = Duration::from_millis(750);

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

#[derive(Debug, Clone, Copy)]
pub enum ShownCut<'a> {
    Clipping,
    Shown(&'a Arc<FeatureResult>),
    Hidden,
}

pub fn clipping_body<'a>(
    document: &Document,
    evaluation: &'a Evaluation,
    feature: FeatureId,
    cut: &FeatureResult,
) -> Option<&'a Arc<FeatureResult>> {
    let cuts_one_body = document.feature(feature)?.bodies().len() == 1;
    cuts_one_body
        .then(|| evaluation.body_state_seen_by(feature, cut.solid()?.body))
        .flatten()
}

pub fn shown_cut(cut: &Arc<FeatureResult>, clipping: bool) -> ShownCut<'_> {
    let Some(solid) = cut.solid().filter(|_| clipping) else {
        return ShownCut::Shown(cut);
    };
    match solid.clipped() {
        None => ShownCut::Clipping,
        Some(Clipped::Within(clipped)) => ShownCut::Shown(clipped),
        Some(Clipped::Outside) => ShownCut::Hidden,
        Some(Clipped::Whole) => ShownCut::Shown(cut),
    }
}

pub fn shown_cuts<'a>(
    document: &Document,
    evaluation: &Evaluation,
    feature: FeatureId,
    cuts: &'a [Arc<FeatureResult>],
) -> Vec<ShownCut<'a>> {
    cuts.iter()
        .map(|cut| {
            let clipping = clipping_body(document, evaluation, feature, cut).is_some();
            shown_cut(cut, clipping)
        })
        .collect()
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
    pub openable: bool,
    pub bounds: Option<Aabb>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BodyEdge {
    pub name: EdgeName,
    polyline: usize,
}

impl BodyEdge {
    pub fn polyline(&self) -> usize {
        self.polyline
    }
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
    pub fn of(solid: &Solid, mesh: &Mesh) -> Result<Self, Interrupted> {
        Self::with_face_areas(solid, mesh).map(|(mass, _)| mass)
    }

    fn with_face_areas(
        solid: &Solid,
        mesh: &Mesh,
    ) -> Result<(Self, BTreeMap<FaceId, f64>), Interrupted> {
        let within = |meshed: &dyn Fn(FaceId) -> bool| MassAccuracy::Mesh {
            chord: mesh.chord(),
            volume_within: mesh.mass_properties_where(meshed).area * mesh.chord(),
        };
        let (properties, mut accuracy, face_areas) = match mass_properties(solid, mesh) {
            Ok(SolidMass {
                properties,
                face_areas,
                meshed_faces,
            }) if meshed_faces.is_empty() => (properties, MassAccuracy::Exact, face_areas),
            Ok(SolidMass {
                properties,
                face_areas,
                meshed_faces,
            }) => {
                let meshed: BTreeSet<FaceId> = meshed_faces.into_iter().collect();
                let accuracy = within(&|face| meshed.contains(&face));
                (properties, accuracy, face_areas)
            }
            Err(MeasureError::Cancelled(interrupted)) => return Err(interrupted),
            Err(error) => {
                log::warn!("mass properties are taken from the display mesh: {error}");
                (mesh.mass_properties(), within(&|_| true), BTreeMap::new())
            }
        };
        let bounds = match extent(solid) {
            Ok(Some(bounds)) => Some(bounds),
            Err(MeasureError::Cancelled(interrupted)) => return Err(interrupted),
            Ok(None) | Err(_) => {
                if accuracy == MassAccuracy::Exact {
                    accuracy = within(&|_| false);
                }
                Aabb::from_points(mesh.positions().iter().copied())
            }
        };
        let size = bounds.map(|bounds| {
            let extent = bounds.max() - bounds.min();
            [extent.x, extent.y, extent.z]
        });
        let mass = Self {
            properties,
            accuracy,
            size,
        };
        Ok((mass, face_areas))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MassReading {
    pub mass: BodyMass,
    face_areas: BTreeMap<FaceKey, f64>,
}

impl MassReading {
    fn of(source: &FeatureResult) -> Result<Option<Self>, Interrupted> {
        let Some((body, mesh)) = source.solid().and_then(|body| Some((body, body.mesh()?))) else {
            return Ok(None);
        };
        let (mass, areas) = BodyMass::with_face_areas(&body.solid, mesh)?;
        let keys: BTreeMap<FaceId, FaceKey> = face_keys(&body.solid).into_iter().collect();
        let face_areas = areas
            .into_iter()
            .filter_map(|(face, area)| Some((*keys.get(&face)?, area)))
            .collect();
        Ok(Some(Self { mass, face_areas }))
    }
}

struct MassSlot {
    source: Arc<FeatureResult>,
    reading: OnceLock<Option<MassReading>>,
    asked: AtomicBool,
    stopped: Arc<AtomicBool>,
    pool: Weak<Pool>,
}

impl fmt::Debug for MassSlot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MassSlot")
            .field("reading", &self.reading.get())
            .finish_non_exhaustive()
    }
}

impl MassSlot {
    fn reading(self: &Arc<Self>) -> Option<&MassReading> {
        if let Some(reading) = self.reading.get() {
            return reading.as_ref();
        }
        if !self.asked.swap(true, Ordering::AcqRel) {
            let queued = self
                .pool
                .upgrade()
                .is_some_and(|pool| pool.push(Task::Measure(Arc::clone(self))));
            if !queued {
                self.measure();
            }
        }
        self.reading.get().and_then(Option::as_ref)
    }

    fn measure(&self) {
        if self.reading.get().is_some() {
            return;
        }
        let stopped = Arc::clone(&self.stopped);
        #[cfg(test)]
        let hold = self
            .pool
            .upgrade()
            .map(|pool| pool.holds.lock().measures.clone())
            .unwrap_or_default();
        let measured = interruptible(Arc::new(move || stopped.load(Ordering::Acquire)), || {
            panic::catch_unwind(AssertUnwindSafe(|| {
                #[cfg(test)]
                hold.wait()?;
                MassReading::of(&self.source)
            }))
        });
        let reading = match measured {
            Ok(Ok(reading)) => reading,
            Ok(Err(Interrupted)) => return,
            Err(_) => {
                log::error!("working out a body's mass properties panicked");
                None
            }
        };
        let _ = self.reading.set(reading);
    }

    fn is_stopped(&self) -> bool {
        self.reading.get().is_none() && self.stopped.load(Ordering::Acquire)
    }

    fn is_pending(&self) -> bool {
        self.reading.get().is_none() && !self.stopped.load(Ordering::Acquire)
    }

    fn stop(&self) {
        if self.reading.get().is_none() {
            self.stopped.store(true, Ordering::Release);
        }
    }

    fn resume(&self) {
        if self.stopped.swap(false, Ordering::AcqRel) {
            self.asked.store(false, Ordering::Release);
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
    polylines: Arc<DisplayedMesh>,
    mass: Arc<MassSlot>,
}

fn is_closed(solid: &Solid) -> bool {
    solid.edges().all(|(_, edge)| edge.coedges().len() >= 2)
}

impl BodyMesh {
    fn of(source: &Arc<FeatureResult>, pool: Weak<Pool>) -> Option<Self> {
        let solid = source.solid()?;
        Some(Self::build(source, solid, solid.mesh()?, pool))
    }

    fn build(
        source: &Arc<FeatureResult>,
        body: &SolidResult,
        mesh: &Mesh,
        pool: Weak<Pool>,
    ) -> Self {
        let solid = &body.solid;
        let keys: BTreeMap<FaceId, FaceKey> = face_keys(solid).into_iter().collect();
        let shown: Vec<&FaceTriangles> = mesh
            .faces()
            .iter()
            .filter(|face| keys.contains_key(&face.face))
            .collect();
        let faces = shown
            .iter()
            .filter_map(|face| {
                Some(BodyFace {
                    key: *keys.get(&face.face)?,
                    openable: openable_face(solid, face.face),
                    bounds: face_bounds(mesh, face),
                })
            })
            .collect();
        let shared = (shown.len() == mesh.faces().len())
            .then(|| {
                ShadedMesh::shared(
                    Arc::new(DisplayedMesh(Arc::clone(source))),
                    shown.iter().map(|face| face.triangles.end),
                )
            })
            .flatten();
        let shaded = shared.unwrap_or_else(|| copied_mesh(mesh, &shown));
        let edges = mesh
            .edges()
            .iter()
            .enumerate()
            .filter(|(_, polyline)| !is_seam(solid, polyline.edge))
            .filter_map(|(index, polyline)| {
                Some(BodyEdge {
                    name: solid.edge(polyline.edge)?.name(),
                    polyline: index,
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
            mesh: Arc::new(shaded.with_closed(is_closed(solid))),
            faces,
            edges,
            vertices,
            polylines: Arc::new(DisplayedMesh(Arc::clone(source))),
            mass: Arc::new(MassSlot {
                source: Arc::clone(source),
                reading: OnceLock::new(),
                asked: AtomicBool::new(false),
                stopped: Arc::default(),
                pool,
            }),
        }
    }

    pub fn mass(&self) -> Option<&BodyMass> {
        self.mass.reading().map(|reading| &reading.mass)
    }

    pub fn mass_pending(&self) -> bool {
        self.mass.is_pending()
    }

    pub fn mass_stopped(&self) -> bool {
        self.mass.is_stopped()
    }

    pub fn stop_mass(&self) {
        self.mass.stop();
    }

    pub fn resume_mass(&self) {
        self.mass.resume();
    }

    pub fn source(&self) -> &Arc<FeatureResult> {
        &self.source
    }

    pub fn polylines(&self) -> Arc<dyn PolylineSource> {
        Arc::clone(&self.polylines) as Arc<dyn PolylineSource>
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

    pub fn face_area(&self, key: FaceKey) -> Option<f64> {
        self.mass.reading()?.face_areas.get(&key).copied()
    }

    pub fn face_bounds(&self, key: FaceKey) -> Option<Aabb> {
        self.faces
            .iter()
            .find(|face| face.key == key)
            .and_then(|face| face.bounds)
    }

    pub fn edge_points(&self, name: EdgeName) -> Option<EdgePoints<'_>> {
        self.edges
            .iter()
            .find(|edge| edge.name == name)
            .map(|edge| self.points(edge))
    }

    pub fn points(&self, edge: &BodyEdge) -> EdgePoints<'_> {
        let mesh = self.source.solid().and_then(SolidResult::mesh);
        let positions = mesh
            .and_then(|mesh| mesh.edges().get(edge.polyline))
            .map_or(&[][..], |polyline| polyline.positions.as_slice());
        EdgePoints {
            mesh,
            positions: positions.iter(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EdgePoints<'a> {
    mesh: Option<&'a Mesh>,
    positions: slice::Iter<'a, u32>,
}

impl EdgePoints<'_> {
    pub fn segments(self) -> impl Iterator<Item = (Point3, Point3)> {
        self.clone().zip(self.skip(1))
    }
}

impl Iterator for EdgePoints<'_> {
    type Item = Point3;

    fn next(&mut self) -> Option<Point3> {
        let mesh = self.mesh?;
        self.positions
            .by_ref()
            .find_map(|position| mesh.position(*position))
    }
}

#[derive(Debug)]
struct DisplayedMesh(Arc<FeatureResult>);

impl DisplayedMesh {
    fn mesh(&self) -> Option<&Mesh> {
        self.0.solid()?.mesh()
    }
}

impl MeshSource for DisplayedMesh {
    fn triangles(&self) -> &[[u32; 3]] {
        self.mesh().map_or(&[], Mesh::triangles)
    }

    fn point_count(&self) -> usize {
        self.mesh().map_or(0, |mesh| mesh.vertices().len())
    }

    fn point(&self, index: u32) -> Option<MeshPoint> {
        let mesh = self.mesh()?;
        let vertex = mesh.vertices().get(index as usize)?;
        Some(MeshPoint {
            position: mesh.position(vertex.position)?,
            normal: vertex.normal.vector(),
        })
    }
}

impl PolylineSource for DisplayedMesh {
    fn polyline(&self, index: usize) -> &[u32] {
        self.mesh()
            .and_then(|mesh| mesh.edges().get(index))
            .map_or(&[], |polyline| polyline.positions.as_slice())
    }

    fn point(&self, index: u32) -> Option<Point3> {
        self.mesh()?.position(index)
    }
}

fn face_corners<'a>(mesh: &'a Mesh, face: &FaceTriangles) -> impl Iterator<Item = u32> + 'a {
    mesh.triangles()
        .get(face.triangles.clone())
        .unwrap_or_default()
        .iter()
        .flatten()
        .copied()
}

fn face_bounds(mesh: &Mesh, face: &FaceTriangles) -> Option<Aabb> {
    Aabb::from_points(face_corners(mesh, face).filter_map(|corner| {
        let vertex = mesh.vertices().get(corner as usize)?;
        mesh.position(vertex.position)
    }))
}

fn copied_mesh(mesh: &Mesh, faces: &[&FaceTriangles]) -> ShadedMesh {
    ShadedMesh::new(faces.iter().map(|face| {
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
        drawn
    }))
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
        normal: source.normal.vector(),
    });
    corners.insert(vertex, index);
    Some(index)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceChoice {
    Opening,
    Moving { tangent: bool },
    Deleting,
    Embossing,
    Splitting,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OpenChoice {
    Edges {
        references: Vec<EdgeReference>,
        groups: Vec<EdgeGroup>,
        chosen: ChosenEdges,
    },
    Faces {
        references: Vec<FaceReference>,
        opened: BTreeSet<FaceKey>,
        choice: FaceChoice,
    },
    Nothing,
}

impl OpenChoice {
    fn of(
        document: &Document,
        kind: Option<&FeatureKind>,
        solid: &Solid,
        previous: Option<Self>,
    ) -> Self {
        match kind {
            Some(FeatureKind::Blend(blend)) => match previous {
                Some(Self::Edges {
                    references,
                    groups,
                    chosen,
                }) if references == blend.edges && groups == blend.groups => Self::Edges {
                    references,
                    groups,
                    chosen,
                },
                _ => Self::Edges {
                    references: blend.edges.clone(),
                    groups: blend.groups.clone(),
                    chosen: blend_tools::chosen_edges(document, solid, blend),
                },
            },
            Some(FeatureKind::Shell(shell)) => match previous {
                Some(Self::Faces {
                    references,
                    opened,
                    choice: FaceChoice::Opening,
                }) if references == shell.open => Self::Faces {
                    references,
                    opened,
                    choice: FaceChoice::Opening,
                },
                _ => Self::Faces {
                    references: shell.open.clone(),
                    opened: shell_tools::opened_faces(solid, shell),
                    choice: FaceChoice::Opening,
                },
            },
            Some(FeatureKind::OffsetFace(offset)) => match previous {
                Some(Self::Faces {
                    references,
                    opened,
                    choice: FaceChoice::Moving { tangent },
                }) if references == offset.faces && tangent == offset.tangent => Self::Faces {
                    references,
                    opened,
                    choice: FaceChoice::Moving { tangent },
                },
                _ => Self::Faces {
                    references: offset.faces.clone(),
                    opened: offset_face_tools::moved_faces(solid, offset),
                    choice: FaceChoice::Moving {
                        tangent: offset.tangent,
                    },
                },
            },
            Some(FeatureKind::MoveFace(moving)) => match previous {
                Some(Self::Faces {
                    references,
                    opened,
                    choice: FaceChoice::Moving { tangent: false },
                }) if references == moving.faces => Self::Faces {
                    references,
                    opened,
                    choice: FaceChoice::Moving { tangent: false },
                },
                _ => Self::Faces {
                    references: moving.faces.clone(),
                    opened: move_face_tools::moved_faces(solid, moving),
                    choice: FaceChoice::Moving { tangent: false },
                },
            },
            Some(FeatureKind::DeleteFace(deletion)) => match previous {
                Some(Self::Faces {
                    references,
                    opened,
                    choice: FaceChoice::Deleting,
                }) if references == deletion.faces => Self::Faces {
                    references,
                    opened,
                    choice: FaceChoice::Deleting,
                },
                _ => Self::Faces {
                    references: deletion.faces.clone(),
                    opened: delete_face_tools::deleted_faces(solid, deletion),
                    choice: FaceChoice::Deleting,
                },
            },
            Some(FeatureKind::Emboss(emboss)) => match previous {
                Some(Self::Faces {
                    references,
                    opened,
                    choice: FaceChoice::Embossing,
                }) if references == emboss.faces => Self::Faces {
                    references,
                    opened,
                    choice: FaceChoice::Embossing,
                },
                _ => Self::Faces {
                    references: emboss.faces.clone(),
                    opened: emboss_tools::chosen_faces(solid, emboss),
                    choice: FaceChoice::Embossing,
                },
            },
            Some(FeatureKind::SplitFace(split)) => match previous {
                Some(Self::Faces {
                    references,
                    opened,
                    choice: FaceChoice::Splitting,
                }) if references == split.faces => Self::Faces {
                    references,
                    opened,
                    choice: FaceChoice::Splitting,
                },
                _ => Self::Faces {
                    references: split.faces.clone(),
                    opened: split_face_tools::chosen_faces(solid, split),
                    choice: FaceChoice::Splitting,
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

#[derive(Debug, Clone, Copy, Default)]
pub struct OpenDraft<'a> {
    pub evaluation: Option<&'a Evaluation>,
    pub result: Option<(FeatureId, &'a Arc<FeatureResult>)>,
    pub cuts: Option<&'a [Arc<FeatureResult>]>,
    pub moved: Option<(FeatureId, RigidTransform)>,
}

#[derive(Debug, Clone)]
pub struct DraftShown {
    pub body: FeatureId,
    pub mesh: Arc<BodyMesh>,
    pub computed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct BodyMeshes {
    bodies: BTreeMap<FeatureId, Arc<BodyMesh>>,
    open: Option<BodyBefore>,
    cuts: Vec<Arc<BodyMesh>>,
    cuts_of: Option<FeatureId>,
    draft: Option<DraftShown>,
    moved: Option<(FeatureId, RigidTransform)>,
    generation: u64,
    updated_for: Option<(u64, u64)>,
}

impl BodyMeshes {
    pub fn body_before(&self) -> Option<&BodyBefore> {
        self.open.as_ref()
    }

    pub fn cuts(&self) -> &[Arc<BodyMesh>] {
        &self.cuts
    }

    pub fn draft(&self) -> Option<&DraftShown> {
        self.draft.as_ref()
    }

    pub fn moved(&self) -> Option<(FeatureId, RigidTransform)> {
        self.moved
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn only(&self, kept: &[FeatureId]) -> Self {
        Self {
            bodies: self
                .bodies
                .iter()
                .filter(|(body, _)| kept.contains(body))
                .map(|(body, mesh)| (*body, Arc::clone(mesh)))
                .collect(),
            generation: self.generation,
            ..Self::default()
        }
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
        draft: OpenDraft<'_>,
    ) {
        self.show_draft(meshing, feature, draft);
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
                    let choice = OpenChoice::of(document, kind, &solid.solid, Some(open.choice));
                    (open.before, choice)
                }
                (_, Converted::Ready(mesh)) => (
                    Arc::clone(mesh),
                    OpenChoice::of(document, kind, &solid.solid, None),
                ),
                (Some(open), Converted::Pending) => (
                    open.before,
                    OpenChoice::of(document, kind, &solid.solid, None),
                ),
                (_, Converted::Pending | Converted::Missing) => return None,
            };
            Some(BodyBefore {
                feature,
                body: solid.body,
                before,
                choice,
            })
        });
        let wanted = match (feature, draft.cuts) {
            (Some(feature), Some(drafted)) => shown_cuts(
                document,
                draft.evaluation.unwrap_or(evaluation),
                feature,
                drafted,
            ),
            (Some(feature), None) => {
                shown_cuts(document, evaluation, feature, evaluation.cuts(feature))
            }
            (None, _) => Vec::new(),
        };
        let pending = wanted.iter().any(|cut| match cut {
            ShownCut::Clipping => true,
            ShownCut::Shown(cut) => matches!(meshing.lookup(cut), Converted::Pending),
            ShownCut::Hidden => false,
        });
        let cuts: Vec<Arc<BodyMesh>> = if pending && self.cuts_of == feature {
            self.cuts.clone()
        } else {
            wanted
                .iter()
                .filter_map(|cut| match cut {
                    ShownCut::Shown(cut) => match meshing.lookup(cut) {
                        Converted::Ready(mesh) => Some(Arc::clone(mesh)),
                        Converted::Pending | Converted::Missing => None,
                    },
                    ShownCut::Clipping | ShownCut::Hidden => None,
                })
                .collect()
        };
        let same_cuts = cuts.len() == self.cuts.len()
            && cuts
                .iter()
                .zip(&self.cuts)
                .all(|(new, old)| Arc::ptr_eq(new, old));
        self.cuts = cuts;
        self.cuts_of = feature;
        if !same_cuts {
            self.changed();
        }
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

    fn show_draft(
        &mut self,
        meshing: &BodyMeshing,
        feature: Option<FeatureId>,
        draft: OpenDraft<'_>,
    ) {
        let computed = feature.is_some_and(|feature| {
            draft.evaluation.is_some_and(|evaluation| {
                !evaluation.is_pending(feature)
                    && evaluation
                        .feature(feature)
                        .is_some_and(|status| status.state == FeatureState::UpToDate)
            })
        });
        let shown = match draft
            .result
            .map(|(body, result)| (body, meshing.lookup(result)))
        {
            Some((body, Converted::Ready(mesh))) => Some(DraftShown {
                body,
                mesh: Arc::clone(mesh),
                computed,
            }),
            Some((body, Converted::Pending)) => {
                self.draft.clone().filter(|shown| shown.body == body)
            }
            Some((_, Converted::Missing)) | None => None,
        };
        let same_draft = match (&shown, &self.draft) {
            (Some(new), Some(old)) => {
                new.body == old.body
                    && Arc::ptr_eq(&new.mesh, &old.mesh)
                    && new.computed == old.computed
            }
            (None, None) => true,
            (Some(_), None) | (None, Some(_)) => false,
        };
        self.draft = shown;
        let same_move = self.moved == draft.moved;
        self.moved = draft.moved;
        if !same_draft || !same_move {
            self.changed();
        }
    }

    pub fn update_when_changed(
        &mut self,
        (evaluation, evaluated): (&Evaluation, u64),
        meshing: &BodyMeshing,
    ) {
        let basis = (evaluated, meshing.conversions());
        if self.updated_for != Some(basis) {
            self.update(evaluation, meshing);
            self.updated_for = Some(basis);
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

enum Task {
    Convert(Arc<FeatureResult>),
    Measure(Arc<MassSlot>),
}

#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub struct Hold(Arc<AtomicBool>);

#[cfg(test)]
impl Hold {
    pub fn holding() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }

    pub fn release(&self) {
        self.0.store(false, Ordering::Release);
    }

    pub fn wait(&self) -> Result<(), Interrupted> {
        while self.0.load(Ordering::Acquire) {
            check_interrupt()?;
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub struct Holds {
    pub converts: Hold,
    pub measures: Hold,
}

enum Done {
    Converted(Conversion),
    Measured,
}

#[derive(Default)]
struct Tasks {
    converts: VecDeque<(Arc<FeatureResult>, u64)>,
    measures: VecDeque<Arc<MassSlot>>,
    measuring: usize,
    idle: usize,
    started: usize,
    closed: bool,
}

impl Tasks {
    fn waiting(&self) -> usize {
        self.converts.len() + self.measures.len()
    }

    fn take(&mut self) -> Option<Taken> {
        self.converts
            .pop_front()
            .map(|(source, run)| Taken::Convert(source, run))
            .or_else(|| self.measures.pop_front().map(Taken::Measure))
    }
}

enum Taken {
    Convert(Arc<FeatureResult>, u64),
    Measure(Arc<MassSlot>),
}

struct Pool {
    tasks: Mutex<Tasks>,
    ready: Condvar,
    limit: usize,
    done: Sender<Done>,
    wake: Mutex<Waker>,
    stops: AtomicU64,
    #[cfg(test)]
    holds: Mutex<Holds>,
}

impl Pool {
    fn push(self: &Arc<Self>, task: Task) -> bool {
        let mut tasks = self.tasks.lock();
        if tasks.closed {
            return false;
        }
        let measure = matches!(task, Task::Measure(_));
        match task {
            Task::Convert(source) => {
                let run = self.stops.load(Ordering::Acquire);
                tasks.converts.push_back((source, run));
            }
            Task::Measure(slot) => {
                tasks.measures.push_back(slot);
                tasks.measuring += 1;
            }
        }
        let spawn = tasks.waiting() > tasks.idle && tasks.started < self.limit;
        if spawn {
            tasks.started += 1;
        }
        drop(tasks);
        self.ready.notify_one();
        if !spawn {
            return true;
        }
        let pool = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("body meshes".to_owned())
            .spawn(move || pool.serve());
        match spawned {
            Ok(_) => true,
            Err(error) => {
                log::error!("could not start a body mesh worker: {error}");
                let mut tasks = self.tasks.lock();
                tasks.started = tasks.started.saturating_sub(1);
                let served = tasks.started > 0;
                if measure && !served {
                    tasks.measuring = tasks.measuring.saturating_sub(1);
                }
                served
            }
        }
    }

    fn serve(self: &Arc<Self>) {
        loop {
            let task = {
                let mut tasks = self.tasks.lock();
                loop {
                    if tasks.closed {
                        return;
                    }
                    if let Some(task) = tasks.take() {
                        break task;
                    }
                    tasks.idle += 1;
                    self.ready.wait(&mut tasks);
                    tasks.idle -= 1;
                }
            };
            let done = match task {
                Taken::Convert(source, run) => match self.convert(source, run) {
                    Some(conversion) => Done::Converted(conversion),
                    None => continue,
                },
                Taken::Measure(slot) => {
                    slot.measure();
                    Done::Measured
                }
            };
            if self.done.send(done).is_err() {
                return;
            }
            (*self.wake.lock())();
        }
    }

    fn convert(self: &Arc<Self>, source: Arc<FeatureResult>, run: u64) -> Option<Conversion> {
        let pool = Arc::downgrade(self);
        let stopped = Arc::new(move || {
            pool.upgrade()
                .is_none_or(|pool| pool.stops.load(Ordering::Acquire) != run)
        });
        #[cfg(test)]
        let hold = self.holds.lock().converts.clone();
        interruptible(stopped, || {
            #[cfg(test)]
            hold.wait().ok()?;
            check_interrupt().ok()?;
            Some(convert(source, Arc::downgrade(self)))
        })
    }

    fn stop_converting(&self) {
        self.stops.fetch_add(1, Ordering::AcqRel);
        self.tasks.lock().converts.clear();
    }

    fn close(&self) {
        self.tasks.lock().closed = true;
        self.ready.notify_all();
    }
}

struct Converter {
    pool: Arc<Pool>,
    done: Receiver<Done>,
}

impl Converter {
    fn new(wake: Waker) -> Self {
        let (done, received) = mpsc::channel();
        let limit = thread::available_parallelism()
            .map_or(1, NonZeroUsize::get)
            .saturating_sub(1)
            .max(1);
        Self {
            pool: Arc::new(Pool {
                tasks: Mutex::new(Tasks::default()),
                ready: Condvar::new(),
                limit,
                done,
                wake: Mutex::new(wake),
                stops: AtomicU64::new(0),
                #[cfg(test)]
                holds: Mutex::default(),
            }),
            done: received,
        }
    }
}

impl Drop for Converter {
    fn drop(&mut self) {
        self.pool.close();
    }
}

fn convert(source: Arc<FeatureResult>, pool: Weak<Pool>) -> Conversion {
    let mesh = panic::catch_unwind(AssertUnwindSafe(|| BodyMesh::of(&source, pool)))
        .unwrap_or_else(|_| {
            log::error!("preparing a body mesh for display panicked");
            None
        })
        .map(Arc::new);
    Conversion { source, mesh }
}

fn address(source: &Arc<FeatureResult>) -> usize {
    Arc::as_ptr(source).addr()
}

#[derive(Default)]
pub struct BodyMeshing {
    converter: Option<Converter>,
    pending: BTreeMap<usize, Arc<FeatureResult>>,
    stopped: BTreeMap<usize, Arc<FeatureResult>>,
    held: BTreeMap<usize, Conversion>,
    converted: BTreeMap<usize, Conversion>,
    shown_at: Option<Instant>,
    measured: u64,
    conversions: u64,
    #[cfg(test)]
    holds: Holds,
}

impl BodyMeshing {
    pub fn lookup(&self, source: &Arc<FeatureResult>) -> Converted<'_> {
        match (self.converted.get(&address(source)), source.solid()) {
            (Some(conversion), _) => conversion
                .mesh
                .as_ref()
                .map_or(Converted::Missing, Converted::Ready),
            (None, Some(solid)) if !solid.mesh_failed() => Converted::Pending,
            (None, Some(_) | None) => Converted::Missing,
        }
    }

    pub fn is_pending(&self) -> bool {
        !self.pending.is_empty() || !self.held.is_empty()
    }

    pub fn preparing(&self) -> Option<(usize, usize)> {
        let waiting = self.pending.len() + self.held.len();
        (waiting > 0).then(|| {
            let shown = self.converted.len();
            (shown, shown + waiting + self.stopped.len())
        })
    }

    pub fn stopped(&self) -> Option<(usize, usize)> {
        let shown = self.converted.len();
        (!self.stopped.is_empty() && self.preparing().is_none())
            .then(|| (shown, shown + self.stopped.len()))
    }

    pub fn stop(&mut self) {
        if let Some(converter) = &self.converter {
            converter.pool.stop_converting();
        }
        self.stopped.append(&mut self.pending);
        if !self.held.is_empty() {
            self.converted.append(&mut self.held);
            self.conversions = self.conversions.wrapping_add(1);
        }
    }

    pub fn resume(&mut self, wake: impl Fn() -> Waker) {
        let stopped = std::mem::take(&mut self.stopped);
        for source in stopped.values() {
            self.request(source, &wake);
        }
    }

    #[cfg(test)]
    pub fn hold(&mut self, holds: &Holds) {
        self.holds = holds.clone();
        if let Some(converter) = &self.converter {
            *converter.pool.holds.lock() = holds.clone();
        }
    }

    pub fn masses_measured(&self) -> u64 {
        self.measured
    }

    pub fn conversions(&self) -> u64 {
        self.conversions
    }

    #[cfg(test)]
    pub fn is_measuring(&self) -> bool {
        self.converter
            .as_ref()
            .is_some_and(|converter| converter.pool.tasks.lock().measuring > 0)
    }

    pub fn request(&mut self, source: &Arc<FeatureResult>, wake: impl FnOnce() -> Waker) {
        let meshed = source.solid().is_some_and(|solid| solid.mesh().is_some());
        let at = address(source);
        let known = self.pending.contains_key(&at)
            || self.stopped.contains_key(&at)
            || self.held.contains_key(&at)
            || self.converted.contains_key(&at);
        if !meshed || known {
            return;
        }
        #[cfg(test)]
        let holds = &self.holds;
        let converter = self.converter.get_or_insert_with(|| {
            let converter = Converter::new(wake());
            #[cfg(test)]
            converter.pool.holds.lock().clone_from(holds);
            converter
        });
        if converter.pool.push(Task::Convert(Arc::clone(source))) {
            self.pending.insert(at, Arc::clone(source));
        } else {
            log::error!("no body mesh worker, so the mesh is prepared on the UI thread");
            self.converter = None;
            self.converted
                .insert(at, convert(Arc::clone(source), Weak::new()));
            self.conversions = self.conversions.wrapping_add(1);
        }
    }

    pub fn poll(&mut self) -> bool {
        let Some(converter) = &self.converter else {
            return false;
        };
        let mut arrived = false;
        while let Ok(done) = converter.done.try_recv() {
            match done {
                Done::Converted(conversion) => {
                    let at = address(&conversion.source);
                    if self.pending.remove(&at).is_some() || self.stopped.remove(&at).is_some() {
                        self.held.insert(at, conversion);
                    }
                }
                Done::Measured => {
                    let mut tasks = converter.pool.tasks.lock();
                    tasks.measuring = tasks.measuring.saturating_sub(1);
                    self.measured = self.measured.wrapping_add(1);
                    arrived = true;
                }
            }
        }
        let due = self.pending.is_empty()
            || self
                .shown_at
                .is_none_or(|shown| shown.elapsed() >= SHOWN_TOGETHER_FOR);
        if due && !self.held.is_empty() {
            self.converted.append(&mut self.held);
            self.conversions = self.conversions.wrapping_add(1);
            self.shown_at = Some(Instant::now());
            arrived = true;
        }
        arrived
    }

    pub fn retain(&mut self, keep: impl Fn(&Arc<FeatureResult>) -> bool) {
        self.pending.retain(|_, pending| keep(pending));
        self.stopped.retain(|_, stopped| keep(stopped));
        self.held.retain(|_, conversion| keep(&conversion.source));
        let before = self.converted.len();
        self.converted
            .retain(|_, conversion| keep(&conversion.source));
        if self.converted.len() != before {
            self.conversions = self.conversions.wrapping_add(1);
        }
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

pub fn face_pickables(
    feature: FeatureId,
    solid: &Solid,
    resolution: Option<&Resolution<FaceId>>,
) -> Vec<Pickable> {
    let Some(found) = resolution.map(Resolution::found) else {
        return Vec::new();
    };
    face_keys(solid)
        .into_iter()
        .filter(|(face, _)| found.contains(face))
        .map(|(_, face)| Pickable::ShellFace { feature, face })
        .collect()
}

pub fn made_by(body: FeatureId, solid: &Solid, feature: FeatureId) -> Vec<Pickable> {
    let made: BTreeSet<FaceId> = solid
        .faces()
        .filter(|(_, face)| face.origin().map(origin_feature) == Some(feature))
        .map(|(id, _)| id)
        .collect();
    let faces = face_keys(solid)
        .into_iter()
        .filter(|(id, _)| made.contains(id))
        .map(|(_, face)| Pickable::Face { body, face });
    let edges = solid
        .edges()
        .filter(|(id, _)| {
            let faces = edge_faces(solid, *id);
            faces.len() > 1 && faces.iter().any(|face| made.contains(face))
        })
        .map(|(_, edge)| Pickable::Edge {
            body,
            edge: edge.name(),
        });
    faces.chain(edges).collect()
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
        let direct = BodyMesh::of(&source, Weak::new()).unwrap();

        assert_eq!(shown.faces, direct.faces);
        assert_eq!(shown.edges, direct.edges);
        assert_eq!(shown.mesh.face_count(), direct.mesh.face_count());
        assert_eq!(shown.bounds(), direct.bounds());
    }

    #[test]
    fn a_body_mesh_reads_the_kernel_mesh_rather_than_holding_a_copy_of_it() {
        for sample in Sample::ALL {
            let document = sample.document().unwrap();
            let evaluation = evaluate(&mut Recompute::default(), &document);
            let (_, source) = only_body(&evaluation);
            let mesh = source.solid().unwrap().mesh().unwrap();
            let faces: Vec<&FaceTriangles> = mesh.faces().iter().collect();

            let shared = ShadedMesh::shared(
                Arc::new(DisplayedMesh(Arc::clone(&source))),
                faces.iter().map(|face| face.triangles.end),
            );
            let shown = BodyMesh::of(&source, Weak::new()).unwrap();

            let shared = shared.unwrap_or_else(|| panic!("{sample:?} is not laid out by face"));
            assert_eq!(shared, copied_mesh(mesh, &faces), "{sample:?}");
            assert_eq!(*shown.mesh, shared, "{sample:?}");
            assert_eq!(shown.faces.len(), faces.len());
        }
    }

    #[test]
    fn body_edges_read_their_points_from_the_kernel_mesh() {
        for sample in Sample::ALL {
            let document = sample.document().unwrap();
            let evaluation = evaluate(&mut Recompute::default(), &document);
            let (_, source) = only_body(&evaluation);
            let solid = &source.solid().unwrap().solid;
            let mesh = source.solid().unwrap().mesh().unwrap();

            let shown = BodyMesh::of(&source, Weak::new()).unwrap();
            let polylines: Vec<_> = mesh
                .edges()
                .iter()
                .filter(|polyline| !is_seam(solid, polyline.edge))
                .collect();

            assert_eq!(shown.edges.len(), polylines.len(), "{sample:?}");
            for (edge, polyline) in shown.edges.iter().zip(polylines) {
                let expected: Vec<Point3> = polyline
                    .positions
                    .iter()
                    .map(|position| mesh.position(*position).unwrap())
                    .collect();
                let points: Vec<Point3> = shown.points(edge).collect();
                let segments: Vec<(Point3, Point3)> = shown.points(edge).segments().collect();

                assert_eq!(edge.name, solid.edge(polyline.edge).unwrap().name());
                assert_eq!(points, expected, "{sample:?}");
                assert_eq!(segments.len(), expected.len() - 1);
                assert_eq!(
                    shown.edge_points(edge.name).unwrap().collect::<Vec<_>>(),
                    expected
                );
            }
        }
    }

    #[test]
    fn the_meshes_of_valid_solids_are_closed_so_a_section_caps_them() {
        for sample in Sample::ALL {
            let document = sample.document().unwrap();
            let evaluation = evaluate(&mut Recompute::default(), &document);
            let (_, source) = only_body(&evaluation);

            let mesh = BodyMesh::of(&source, Weak::new()).unwrap();

            assert!(mesh.mesh.is_closed(), "{sample:?}");
        }
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
