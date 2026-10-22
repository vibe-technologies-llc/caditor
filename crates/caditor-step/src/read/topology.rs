use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    f64::consts::FRAC_PI_2,
};

use caditor_expression::format_decimals;
use caditor_geometry::{Aabb, Plane, Point2, Point3, Vector3};
use caditor_kernel::{
    BSpline, BuildError, Circle, CrossingCheck, Curve, EdgeId, FaceId, FaceMesh, FacetedError,
    IntersectionCurve, Interval, LINEAR_RESOLUTION, MeshQuality, PlaneSurface, Sense, ShellId,
    Solid, SolidBuilder, Surface, TriangleMesh, VertexId, faceted_solids,
    mass_properties_tessellating,
};

use crate::read::{
    conform::{Conformed, LooseBody, LooseEdge, conform},
    fault::Fault,
    geometry::{Crossings, Geometry},
    graph::{Entity, Graph, Problem, Read},
    loose::{exact_kind, farthest, met_at_ends},
    snap::{seam_through, snap},
};

const LOOP_SAMPLES: usize = 16;
const SPLINE_EXTENSION: f64 = 0.1;
const CHECK_SAMPLES: usize = 32;
const FACETED_REACH: f64 = 4.0;
const HEAL_SAMPLES: usize = 48;
const CLEAN: f64 = 0.25 * LINEAR_RESOLUTION;
const VERTEX_ITERATIONS: usize = 30;
const VERTEX_DAMPING: f64 = 1e-12;
const COARSER_STEPS: i32 = 2;
const COARSER_FACTOR: f64 = 2.0;
const MAX_VERTEX_DAMPING: f64 = 1e6;
const DAMPING_GROWTH: f64 = 10.0;
const DAMPING_RELIEF: f64 = 0.1;
const WELD_FRACTION: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Healing {
    Exact,
    Extended,
    Snapped,
    Bent,
    Faceted,
}

pub(crate) struct Topology<'g, 'a> {
    geometry: &'g Geometry<'a>,
    healing: Healing,
    builder: SolidBuilder,
    faces: BTreeMap<u64, FacePlan>,
    edge_faces: BTreeMap<u64, BTreeSet<u64>>,
    vertex_faces: BTreeMap<u64, BTreeSet<u64>>,
    vertex_edges: BTreeMap<u64, BTreeSet<u64>>,
    vertices: BTreeMap<u64, VertexId>,
    edges: BTreeMap<u64, (EdgeId, bool)>,
    curves: BTreeMap<EdgeId, (Curve, Interval)>,
    corners: BTreeMap<[u64; 3], VertexId>,
    sides: BTreeMap<(VertexId, VertexId), EdgeId>,
    fins: BTreeSet<u64>,
    face_entities: Vec<u64>,
    left_out: BTreeSet<u64>,
    reversed_voids: Option<BTreeSet<u64>>,
    healed: usize,
    loosest: f64,
    snapping: Snapping,
    conformed: Conformed,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SolidShells {
    outer: u64,
    lumps: Vec<u64>,
    voids: Vec<u64>,
}

impl SolidShells {
    pub fn of(graph: &Graph<'_>, id: u64) -> Read<Self> {
        let entity = graph.entity(id)?;
        let (outer, voids, lumps) = match entity.kind() {
            "MANIFOLD_SOLID_BREP" => (
                entity.record("MANIFOLD_SOLID_BREP")?.reference(1)?,
                Vec::new(),
                Vec::new(),
            ),
            "FACETED_BREP" => (entity.fields()?.reference(1)?, Vec::new(), Vec::new()),
            "BREP_WITH_VOIDS" => {
                let fields = entity.record("BREP_WITH_VOIDS")?;
                (fields.reference(1)?, fields.references(2)?, Vec::new())
            }
            "SHELL_BASED_SURFACE_MODEL" => {
                let mut closed = closed_shells(graph, entity)?.into_iter();
                let first = closed
                    .next()
                    .ok_or_else(|| Problem::new(id, Fault::NoClosedShell))?;
                (first, Vec::new(), closed.collect())
            }
            other => {
                return Err(Problem::new(id, Fault::Unsupported(other.to_owned())));
            }
        };
        Ok(Self {
            outer,
            lumps,
            voids,
        })
    }

    pub fn all(&self) -> impl Iterator<Item = u64> + '_ {
        std::iter::once(self.outer)
            .chain(self.lumps.iter().copied())
            .chain(self.voids.iter().copied())
    }

    pub fn outer(&self) -> u64 {
        self.outer
    }

    pub fn is_void(&self, shell: u64) -> bool {
        self.voids.contains(&shell)
    }

    pub fn without(&self, left_out: &BTreeSet<u64>) -> Self {
        let kept = |shells: &[u64]| {
            shells
                .iter()
                .copied()
                .filter(|shell| !left_out.contains(shell))
                .collect()
        };
        Self {
            outer: self.outer,
            lumps: kept(&self.lumps),
            voids: kept(&self.voids),
        }
    }
}

pub(crate) struct Unreadable {
    pub shell: u64,
    pub face: u64,
    pub problem: Problem,
}

pub(crate) fn unreadable_faces(geometry: &Geometry<'_>, shells: &SolidShells) -> Vec<Unreadable> {
    let graph = geometry.graph;
    let mut found = Vec::new();
    for shell in shells.all() {
        for face in shell_faces(graph, shell).unwrap_or_default() {
            let surface = graph
                .entity(face)
                .and_then(|entity| match entity.kind() {
                    "ADVANCED_FACE" | "FACE_SURFACE" => entity.fields()?.reference(2),
                    _ => Err(Problem::new(face, Fault::NoSurface)),
                })
                .ok();
            if let Some(Err(problem)) = surface.map(|surface| geometry.surface(surface)) {
                found.push(Unreadable {
                    shell,
                    face,
                    problem,
                });
            }
        }
    }
    found
}

pub(crate) fn shell_faces(graph: Graph<'_>, shell: u64) -> Read<Vec<u64>> {
    let mut entity = graph.entity(shell)?;
    if entity.kind() == "ORIENTED_CLOSED_SHELL" {
        entity = graph.entity(entity.record("ORIENTED_CLOSED_SHELL")?.reference(2)?)?;
    }
    let mut faces = Vec::new();
    for face in entity.fields()?.references(1)? {
        let face_entity = graph.entity(face)?;
        faces.push(if face_entity.kind() == "ORIENTED_FACE" {
            face_entity.record("ORIENTED_FACE")?.reference(2)?
        } else {
            face
        });
    }
    Ok(faces)
}

#[derive(Debug, Clone)]
pub(crate) struct Built {
    pub solid: Solid,
    pub healed: usize,
    pub unchecked: Option<[u64; 2]>,
    pub faceted: Option<Faceting>,
    pub bent: Option<Bending>,
    pub snapped: Snapping,
    pub faces: Vec<Option<u64>>,
    pub lost: Option<Lost>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Faceting {
    WithinPrecision,
    Apart { loosest: f64 },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Lost {
    pub faces: Vec<u64>,
    pub problem: Problem,
    pub voids: usize,
    pub lumps: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Bending {
    pub faces: usize,
    pub farthest: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Snapping {
    pub flat: usize,
    pub curved: usize,
    pub farthest: f64,
}

impl Snapping {
    pub fn faces(&self) -> usize {
        self.flat + self.curved
    }
}

struct FacePlan {
    surface: Surface,
    same_sense: bool,
    bounds: Vec<u64>,
}

struct Bound {
    outer: bool,
    coedges: Vec<(EdgeId, Sense)>,
}

struct ShellPlan {
    id: u64,
    faces: Vec<(u64, bool)>,
}

impl<'g, 'a> Topology<'g, 'a> {
    pub fn new(geometry: &'g Geometry<'a>, healing: Healing) -> Self {
        Self {
            geometry,
            healing,
            builder: SolidBuilder::new(),
            faces: BTreeMap::new(),
            edge_faces: BTreeMap::new(),
            vertex_faces: BTreeMap::new(),
            vertex_edges: BTreeMap::new(),
            vertices: BTreeMap::new(),
            edges: BTreeMap::new(),
            curves: BTreeMap::new(),
            corners: BTreeMap::new(),
            sides: BTreeMap::new(),
            fins: BTreeSet::new(),
            face_entities: Vec::new(),
            left_out: BTreeSet::new(),
            reversed_voids: None,
            healed: 0,
            loosest: 0.0,
            snapping: Snapping::default(),
            conformed: Conformed::default(),
        }
    }

    pub fn leaving_out(mut self, faces: BTreeSet<u64>) -> Self {
        self.left_out = faces;
        self
    }

    pub fn solid(mut self, id: u64, solid_shells: &SolidShells) -> Read<Built> {
        let graph = self.geometry.graph;
        let SolidShells {
            outer,
            lumps,
            voids,
        } = solid_shells;
        let mut shells = vec![self.plan_shell(*outer, false)?];
        for lump in lumps {
            shells.push(self.plan_shell(*lump, false)?);
        }
        for void in voids.iter().copied() {
            let void_entity = graph.entity(void)?;
            let (shell, flipped) = match void_entity.kind() {
                "ORIENTED_CLOSED_SHELL" => {
                    let fields = void_entity.record("ORIENTED_CLOSED_SHELL")?;
                    (fields.reference(2)?, !fields.logical(3)?)
                }
                _ => (void, true),
            };
            let reversed = self
                .reversed_voids
                .as_ref()
                .is_some_and(|reversed| reversed.contains(&void));
            shells.push(self.plan_shell(shell, flipped != reversed)?);
        }
        if matches!(self.healing, Healing::Snapped | Healing::Bent) {
            self.snap(id)?;
        }
        if self.healing == Healing::Bent {
            self.conform(id)?;
        }
        for shell in &shells {
            self.build_shell(shell)?;
        }
        if self.reversed_voids.is_none() {
            let outward = self.outward_voids(voids, 1 + lumps.len());
            if !outward.is_empty() {
                let mut again =
                    Topology::new(self.geometry, self.healing).leaving_out(self.left_out);
                again.reversed_voids = Some(outward);
                return again.solid(id, solid_shells);
            }
        }
        let healed = self.healed;
        let precision = self.geometry.units.precision;
        let faceted = self.healing == Healing::Faceted;
        let mut faceted_faces = Vec::new();
        let mut faceting = None;
        let solid = if faceted {
            let within = precision.unwrap_or(LINEAR_RESOLUTION);
            let allowance = if within > LINEAR_RESOLUTION {
                FACETED_REACH * within
            } else {
                within
            };
            if self.loosest > allowance {
                return Err(Problem::new(
                    id,
                    Fault::FacesApart {
                        loosest: self.loosest,
                        allowance,
                    },
                ));
            }
            let Faceted { solid, faces } = facets(&self.builder, !self.left_out.is_empty())
                .map_err(|fault| Problem::new(id, fault))?;
            faceting = Some(if self.loosest > within {
                Faceting::Apart {
                    loosest: self.loosest,
                }
            } else {
                Faceting::WithinPrecision
            });
            faceted_faces = faces
                .into_iter()
                .map(|face| face.and_then(|face| self.face_entities.get(face.index()).copied()))
                .collect();
            solid
        } else {
            self.builder
                .build()
                .map_err(|error| Problem::new(id, Fault::build(error, precision)))?
        };
        let check = if faceted || self.geometry.crossings == Crossings::Trusted {
            CrossingCheck::Clear
        } else {
            solid
                .find_crossing()
                .map_err(|_| Problem::new(id, Fault::CheckCancelled))?
        };
        let entities =
            |faces: [FaceId; 2]| faces.map(|face| self.face_entities.get(face.index()).copied());
        let unchecked = match check {
            CrossingCheck::Clear => None,
            CrossingCheck::Inconclusive { faces } => match entities(faces) {
                [Some(first), Some(second)] => Some([first, second]),
                _ => Some([id, id]),
            },
            CrossingCheck::Crossing(crossing) => {
                let fault = match entities(crossing.faces) {
                    [Some(first), Some(second)] if first == second => {
                        Fault::FaceCrossesItself { face: first }
                    }
                    [Some(first), Some(second)] => Fault::FacesCross { first, second },
                    _ => Fault::SomeFacesCross,
                };
                return Err(Problem::new(id, fault));
            }
        };
        let bent = (self.healing == Healing::Bent).then_some(Bending {
            faces: self.conformed.faces_bent,
            farthest: self.conformed.farthest,
        });
        let faces = if faceted {
            faceted_faces
        } else {
            self.face_entities.into_iter().map(Some).collect()
        };
        Ok(Built {
            solid,
            healed,
            unchecked,
            faceted: faceting,
            bent,
            snapped: self.snapping,
            faces,
            lost: None,
        })
    }

    fn outward_voids(&self, voids: &[u64], first_void_shell: usize) -> BTreeSet<u64> {
        if voids.is_empty() {
            return BTreeSet::new();
        }
        let Ok(inward) = self.builder.inward_shells() else {
            return BTreeSet::new();
        };
        voids
            .iter()
            .enumerate()
            .filter(|(offset, _)| {
                !inward
                    .iter()
                    .any(|shell| shell.index() == first_void_shell + offset)
            })
            .map(|(_, void)| *void)
            .collect()
    }

    fn snap(&mut self, id: u64) -> Read<()> {
        let allowance = self
            .geometry
            .units
            .precision
            .ok_or_else(|| Problem::new(id, Fault::NoPrecisionToMove))?;
        let snapped = snap(&self.loose_body(allowance)?);
        if snapped.surfaces.is_empty() && self.healing == Healing::Snapped {
            return Err(Problem::new(id, Fault::NoFaceToSnap));
        }
        for (face, surface) in &snapped.surfaces {
            if let Some(plan) = self.faces.get_mut(face) {
                plan.surface = surface.clone();
            }
        }
        self.snapping = Snapping {
            flat: snapped.flat,
            curved: snapped.curved,
            farthest: snapped.farthest,
        };
        Ok(())
    }

    fn conform(&mut self, id: u64) -> Read<()> {
        let allowance = self
            .geometry
            .units
            .precision
            .ok_or_else(|| Problem::new(id, Fault::NoPrecisionToBend))?;
        let body = self.loose_body(allowance)?;
        let conformed = conform(&body)?;
        for (face, surface) in &conformed.surfaces {
            if let Some(plan) = self.faces.get_mut(face) {
                plan.surface = surface.clone();
            }
        }
        self.conformed = conformed;
        Ok(())
    }

    fn loose_body(&self, allowance: f64) -> Read<LooseBody<'_>> {
        let graph = self.geometry.graph;
        let mut edges = Vec::new();
        let mut vertices = BTreeMap::new();
        for (edge, faces) in &self.edge_faces {
            let fields = graph.entity(*edge)?.record("EDGE_CURVE")?;
            let (first, second) = (fields.reference(1)?, fields.reference(2)?);
            let mut point = |vertex: u64| -> Read<Point3> {
                let position = self
                    .geometry
                    .point(graph.entity(vertex)?.record("VERTEX_POINT")?.reference(1)?)?;
                vertices.insert(vertex, position);
                Ok(position)
            };
            let (first_point, second_point) = (point(first)?, point(second)?);
            let curve = self.geometry.curve(fields.reference(3)?)?;
            let ends = if fields.logical(4)? {
                [first, second]
            } else {
                [second, first]
            };
            let (from, to) = if ends[0] == first {
                (first_point, second_point)
            } else {
                (second_point, first_point)
            };
            let Some(interval) = parameter_range(&curve, from, to, ends[0] == ends[1]) else {
                continue;
            };
            edges.push(LooseEdge {
                id: *edge,
                ends,
                curve,
                interval,
                faces: faces.iter().copied().collect(),
            });
        }
        let faces = self
            .faces
            .iter()
            .map(|(face, plan)| (*face, plan.surface.clone()))
            .collect();
        Ok(LooseBody {
            faces,
            edges,
            vertices,
            vertex_faces: &self.vertex_faces,
            allowance,
        })
    }

    fn plan_shell(&mut self, id: u64, flipped: bool) -> Read<ShellPlan> {
        let graph = self.geometry.graph;
        let entity = graph.entity(id)?;
        let faces = match entity.kind() {
            "CLOSED_SHELL" | "OPEN_SHELL" => entity.fields()?.references(1)?,
            other => {
                return Err(Problem::new(
                    id,
                    Fault::ClosedShellExpected(other.to_owned()),
                ));
            }
        };
        let mut planned = Vec::with_capacity(faces.len());
        for face in faces {
            self.geometry.charge(face)?;
            let mut entity = graph.entity(face)?;
            let mut face_flipped = flipped;
            if entity.kind() == "ORIENTED_FACE" {
                let fields = entity.record("ORIENTED_FACE")?;
                face_flipped ^= !fields.logical(3)?;
                entity = graph.entity(fields.reference(2)?)?;
            }
            if self.left_out.contains(&entity.id) {
                continue;
            }
            self.plan_face(entity)?;
            planned.push((entity.id, face_flipped));
        }
        Ok(ShellPlan { id, faces: planned })
    }

    fn plan_face(&mut self, entity: Entity<'_>) -> Read<()> {
        if self.faces.contains_key(&entity.id) {
            return Ok(());
        }
        let fields = match entity.kind() {
            "ADVANCED_FACE" | "FACE_SURFACE" | "FACE" => entity.fields()?,
            other => {
                return Err(Problem::new(
                    entity.id,
                    Fault::FaceExpected(other.to_owned()),
                ));
            }
        };
        let bounds = fields.references(1)?;
        if entity.kind() == "FACE" {
            let surface = self.polygon_plane(entity.id, &bounds)?;
            self.faces.insert(
                entity.id,
                FacePlan {
                    surface,
                    same_sense: true,
                    bounds,
                },
            );
            return Ok(());
        }
        let surface_id = fields.reference(2)?;
        let reversed = self.geometry.reversed(surface_id);
        let (surface, transposed) = upright(self.geometry.surface(surface_id)?);
        let surface = match surface {
            Surface::BSpline(spline) if self.healing == Healing::Extended => Surface::from(
                spline
                    .extended(SPLINE_EXTENSION, self.geometry.units.uncertainty())
                    .unwrap_or(*spline),
            ),
            other => other,
        };
        let graph = self.geometry.graph;
        for bound in &bounds {
            let loop_id = graph.entity(*bound)?.fields()?.reference(1)?;
            let loop_entity = graph.entity(loop_id)?;
            if loop_entity.kind() != "EDGE_LOOP" {
                continue;
            }
            let mut uses = Vec::new();
            for oriented in loop_entity.fields()?.references(1)? {
                let fields = graph.entity(oriented)?.record("ORIENTED_EDGE")?;
                uses.push((fields.reference(3)?, fields.logical(4)?));
            }
            let fins = self.fins_in(&uses, &surface)?;
            for (edge, _) in uses {
                if fins.contains(&edge) {
                    continue;
                }
                self.edge_faces.entry(edge).or_default().insert(entity.id);
                let edge_fields = graph.entity(edge)?.record("EDGE_CURVE")?;
                for vertex in [edge_fields.reference(1)?, edge_fields.reference(2)?] {
                    self.vertex_faces
                        .entry(vertex)
                        .or_default()
                        .insert(entity.id);
                    self.vertex_edges.entry(vertex).or_default().insert(edge);
                }
            }
        }
        self.faces.insert(
            entity.id,
            FacePlan {
                surface,
                same_sense: fields.logical(3)? != (transposed != reversed),
                bounds,
            },
        );
        Ok(())
    }

    fn fins_in(&mut self, uses: &[(u64, bool)], surface: &Surface) -> Read<BTreeSet<u64>> {
        let mut kept: VecDeque<(u64, bool)> = VecDeque::with_capacity(uses.len());
        let mut fins = BTreeSet::new();
        let folds_back = |(edge, orientation): (u64, bool), other: Option<&(u64, bool)>| {
            other.is_some_and(|(other, other_orientation)| {
                *other == edge && *other_orientation != orientation
            })
        };
        for used in uses.iter().copied() {
            if folds_back(used, kept.back()) && !self.reaches_pole(used.0, surface)? {
                kept.pop_back();
                fins.insert(used.0);
            } else {
                kept.push_back(used);
            }
        }
        while kept.len() > 2 {
            match kept.back().copied() {
                Some(last)
                    if folds_back(last, kept.front()) && !self.reaches_pole(last.0, surface)? =>
                {
                    kept.pop_back();
                    kept.pop_front();
                    fins.insert(last.0);
                }
                _ => break,
            }
        }
        if kept.is_empty() {
            return Ok(BTreeSet::new());
        }
        self.fins.extend(fins.iter().copied());
        Ok(fins)
    }

    fn reaches_pole(&self, edge: u64, surface: &Surface) -> Read<bool> {
        let graph = self.geometry.graph;
        let fields = graph.entity(edge)?.record("EDGE_CURVE")?;
        let reach = self.geometry.units.uncertainty().max(LINEAR_RESOLUTION);
        let poles = surface.poles();
        for field in [1, 2] {
            let vertex = graph.entity(fields.reference(field)?)?;
            let point = self
                .geometry
                .point(vertex.record("VERTEX_POINT")?.reference(1)?)?;
            if poles.iter().any(|pole| pole.point.distance(point) <= reach) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn build_shell(&mut self, plan: &ShellPlan) -> Read<()> {
        let shell = self
            .builder
            .shell()
            .map_err(|error| Problem::new(plan.id, self.describe(&error)))?;
        for (face, face_flipped) in &plan.faces {
            self.build_face(shell, *face, *face_flipped)?;
        }
        Ok(())
    }

    fn build_face(&mut self, shell: ShellId, id: u64, flipped: bool) -> Read<()> {
        let (surface, same_sense, bound_ids) = match self.faces.get(&id) {
            Some(plan) => (plan.surface.clone(), plan.same_sense, plan.bounds.clone()),
            None => return Err(Problem::new(id, Fault::MissingFromShell)),
        };
        let sense = if same_sense != flipped {
            Sense::Same
        } else {
            Sense::Reversed
        };
        let mut bounds = Vec::new();
        for bound in bound_ids {
            if let Some(bound) = self.bound(bound, flipped)? {
                bounds.push(bound);
            }
        }
        if bounds.is_empty() {
            let seam = self
                .seam_around(&surface)
                .map_err(|error| Problem::new(id, self.describe(&error)))?
                .ok_or_else(|| Problem::new(id, Fault::NoEdgesAround))?;
            bounds.push(Bound {
                outer: true,
                coedges: vec![(seam, Sense::Same), (seam, Sense::Reversed)],
            });
        }
        self.order_bounds(&mut bounds, &surface, sense);
        let face = self
            .builder
            .face(shell, surface, sense)
            .map_err(|error| Problem::new(id, self.describe(&error)))?;
        if self.face_entities.len() == face.index() {
            self.face_entities.push(id);
        }
        self.loops(face, id, &bounds)
    }

    fn loops(&mut self, face: FaceId, id: u64, bounds: &[Bound]) -> Read<()> {
        for bound in bounds {
            self.builder
                .add_loop(face, &bound.coedges)
                .map_err(|error| Problem::new(id, self.describe(&error)))?;
        }
        Ok(())
    }

    fn describe(&self, error: &BuildError) -> Fault {
        Fault::build(error.clone(), self.geometry.units.precision)
    }

    fn note_repair(&mut self, gap: Option<f64>) {
        if gap.is_some_and(|gap| gap > self.geometry.units.uncertainty()) {
            self.healed += 1;
        }
    }

    fn seam_around(&mut self, surface: &Surface) -> Result<Option<EdgeId>, BuildError> {
        let seam = match surface {
            Surface::Sphere(sphere) => {
                let frame = sphere.frame();
                let meridian = Plane::from_frame(sphere.center(), -frame.y_axis(), frame.x_axis())
                    .ok_or(BuildError::NonFinitePoint)?;
                let curve: Curve = Circle::new(meridian, sphere.radius())?.into();
                let interval =
                    Interval::new(-FRAC_PI_2, FRAC_PI_2).ok_or(BuildError::NonFinitePoint)?;
                Some((curve, interval))
            }
            Surface::Revolution(revolution) if surface.poles().len() == 2 => {
                let profile = revolution.profile();
                let interval = profile
                    .domain()
                    .bounded()
                    .ok_or(BuildError::NonFinitePoint)?;
                Some((profile.clone(), interval))
            }
            Surface::BSpline(spline) if spline.u_period().is_some() => {
                let column: Vec<Point3> = (0..spline.rows())
                    .filter_map(|row| spline.control_point(0, row))
                    .collect();
                let weights: Option<Vec<f64>> = spline.weights().map(|weights| {
                    (0..spline.rows())
                        .map(|row| weights.get(row * spline.columns()).copied().unwrap_or(1.0))
                        .collect()
                });
                let degree = spline.v_degree();
                let knots = spline.v_knots().to_vec();
                let curve = match weights {
                    Some(weights) => BSpline::rational(degree, knots, column, weights)?,
                    None => BSpline::new(degree, knots, column)?,
                };
                let interval = curve.domain();
                Some((Curve::BSpline(curve), interval))
            }
            _ => None,
        };
        let Some((curve, interval)) = seam else {
            return Ok(None);
        };
        let (from, to) = (curve.point(interval.start()), curve.point(interval.end()));
        let start = self.builder.vertex(from)?;
        let end = if from.distance(to) <= LINEAR_RESOLUTION {
            start
        } else {
            self.builder.vertex(to)?
        };
        let edge = self.builder.edge(curve.clone(), interval, start, end)?;
        self.curves.insert(edge, (curve, interval));
        Ok(Some(edge))
    }

    fn polygon_plane(&self, face: u64, bounds: &[u64]) -> Read<Surface> {
        let graph = self.geometry.graph;
        let mut outline = None;
        for bound in bounds {
            let bound = graph.entity(*bound)?;
            let loop_entity = graph.entity(bound.fields()?.reference(1)?)?;
            if loop_entity.kind() == "POLY_LOOP" {
                let points = loop_entity
                    .fields()?
                    .references(1)?
                    .into_iter()
                    .map(|point| self.geometry.point(point))
                    .collect::<Read<Vec<Point3>>>()?;
                let orientation = bound.fields()?.logical(2)?;
                let outer = bound.kind() == "FACE_OUTER_BOUND";
                if outer || outline.is_none() {
                    outline = Some((points, orientation));
                }
            }
        }
        let (points, orientation) = outline.ok_or_else(|| Problem::new(face, Fault::NoPolygon))?;
        let plane = polygon_plane(&points, orientation, self.geometry.units.uncertainty())
            .ok_or_else(|| Problem::new(face, Fault::PolygonWithoutArea))?;
        PlaneSurface::new(plane)
            .map(Surface::from)
            .map_err(|error| Problem::new(face, Fault::UnusablePlane(error)))
    }

    fn corner(&mut self, point: u64) -> Read<VertexId> {
        let position = self.geometry.point(point)?;
        let key = position.to_array().map(f64::to_bits);
        if let Some(existing) = self.corners.get(&key) {
            return Ok(*existing);
        }
        let vertex = self
            .builder
            .vertex(position)
            .map_err(|error| Problem::new(point, self.describe(&error)))?;
        self.corners.insert(key, vertex);
        Ok(vertex)
    }

    fn polygon(&mut self, id: u64, points: &[u64]) -> Read<Vec<(EdgeId, Sense)>> {
        let mut corners = Vec::with_capacity(points.len());
        for point in points {
            let corner = self.corner(*point)?;
            if corners.last() != Some(&corner) {
                corners.push(corner);
            }
        }
        if corners.len() > 1 && corners.first() == corners.last() {
            corners.pop();
        }
        if corners.len() < 3 {
            return Err(Problem::new(id, Fault::FewerThanThreeCorners));
        }
        let mut coedges = Vec::with_capacity(corners.len());
        for (index, from) in corners.iter().enumerate() {
            let to = corners
                .get((index + 1) % corners.len())
                .copied()
                .ok_or_else(|| Problem::new(id, Fault::FewerThanThreeCorners))?;
            if let Some(edge) = self.sides.get(&(to, *from)) {
                coedges.push((*edge, Sense::Reversed));
                continue;
            }
            if let Some(edge) = self.sides.get(&(*from, to)) {
                coedges.push((*edge, Sense::Same));
                continue;
            }
            let edge = self
                .builder
                .line_edge(*from, to)
                .map_err(|error| Problem::new(id, self.describe(&error)))?;
            self.sides.insert((*from, to), edge);
            coedges.push((edge, Sense::Same));
        }
        Ok(coedges)
    }

    fn bound(&mut self, id: u64, flipped: bool) -> Read<Option<Bound>> {
        let graph = self.geometry.graph;
        let entity = graph.entity(id)?;
        let outer = entity.kind() == "FACE_OUTER_BOUND";
        let fields = entity.fields()?;
        let reversed = !fields.logical(2)? ^ flipped;
        let loop_entity = graph.entity(fields.reference(1)?)?;
        let edges = match loop_entity.kind() {
            "EDGE_LOOP" => loop_entity.fields()?.references(1)?,
            "VERTEX_LOOP" => return Ok(None),
            "POLY_LOOP" => {
                let points = loop_entity.fields()?.references(1)?;
                let mut coedges = self.polygon(loop_entity.id, &points)?;
                if reversed {
                    coedges.reverse();
                    for (_, sense) in &mut coedges {
                        *sense = sense.reversed();
                    }
                }
                return Ok(Some(Bound { outer, coedges }));
            }
            other => {
                return Err(Problem::new(
                    loop_entity.id,
                    Fault::Unsupported(other.to_owned()),
                ));
            }
        };
        let mut coedges = Vec::with_capacity(edges.len());
        for oriented in edges {
            let oriented_fields = graph.entity(oriented)?.record("ORIENTED_EDGE")?;
            let orientation = oriented_fields.logical(4)?;
            let edge_id = oriented_fields.reference(3)?;
            if self.fins.contains(&edge_id) {
                continue;
            }
            let (edge, edge_reversed) = self.edge(edge_id)?;
            let along = orientation != edge_reversed;
            let sense = if along != reversed {
                Sense::Same
            } else {
                Sense::Reversed
            };
            coedges.push((edge, sense));
        }
        if reversed {
            coedges.reverse();
        }
        Ok(Some(Bound { outer, coedges }))
    }

    fn surfaces_of(&self, faces: Option<&BTreeSet<u64>>) -> Vec<Surface> {
        faces
            .into_iter()
            .flatten()
            .filter_map(|face| self.faces.get(face))
            .map(|plan| plan.surface.clone())
            .collect()
    }

    fn vertex(&mut self, id: u64) -> Read<VertexId> {
        if let Some(existing) = self.vertices.get(&id) {
            return Ok(*existing);
        }
        let fields = self.geometry.graph.entity(id)?.record("VERTEX_POINT")?;
        let point = self.geometry.point(fields.reference(1)?)?;
        let surfaces = self.surfaces_of(self.vertex_faces.get(&id));
        let point = match self.conformed.vertices.get(&id) {
            Some(placed) => *placed,
            None => {
                let reach = match self.healing {
                    Healing::Faceted => worst_gap(point, &surfaces)
                        .min(self.geometry.units.precision.unwrap_or(f64::INFINITY)),
                    _ => f64::INFINITY,
                };
                let (point, gap) = settle_on(point, &surfaces, reach);
                self.note_repair(gap);
                match self.healing {
                    Healing::Faceted => self.centred_among_edge_ends(id, point),
                    _ => point,
                }
            }
        };
        if self.healing == Healing::Faceted {
            let left = surfaces
                .iter()
                .map(|surface| surface.distance(point))
                .fold(0.0, f64::max);
            self.loosest = self.loosest.max(left);
        }
        let vertex = self
            .builder
            .vertex(point)
            .map_err(|error| Problem::new(id, self.describe(&error)))?;
        self.vertices.insert(id, vertex);
        Ok(vertex)
    }

    fn centred_among_edge_ends(&mut self, id: u64, point: Point3) -> Point3 {
        let Some(precision) = self.geometry.units.precision else {
            return point;
        };
        let feet: Vec<Point3> = self
            .vertex_edges
            .get(&id)
            .into_iter()
            .flatten()
            .filter(|edge| {
                !self
                    .surfaces_of(self.edge_faces.get(edge))
                    .iter()
                    .all(exact_kind)
            })
            .filter_map(|edge| {
                let fields = self
                    .geometry
                    .graph
                    .entity(*edge)
                    .ok()?
                    .record("EDGE_CURVE")
                    .ok()?;
                self.geometry.curve(fields.reference(3).ok()?).ok()
            })
            .map(|curve| curve.point(curve.closest_parameter(point, search_range(&curve))))
            .collect();
        let farthest_foot = |at: Point3| {
            feet.iter()
                .map(|foot| foot.distance(at))
                .fold(0.0, f64::max)
        };
        if farthest_foot(point) <= precision {
            return point;
        }
        let Some(centre) = Aabb::from_points(feet.iter().copied()).map(|bounds| bounds.center())
        else {
            return point;
        };
        if farthest_foot(centre) > precision {
            return point;
        }
        self.note_repair(Some(centre.distance(point)));
        centre
    }

    fn edge(&mut self, id: u64) -> Read<(EdgeId, bool)> {
        if let Some(existing) = self.edges.get(&id) {
            return Ok(*existing);
        }
        let fields = self.geometry.graph.entity(id)?.record("EDGE_CURVE")?;
        let first = self.vertex(fields.reference(1)?)?;
        let second = self.vertex(fields.reference(2)?)?;
        let same_sense = fields.logical(4)?;
        let (start, end) = if same_sense {
            (first, second)
        } else {
            (second, first)
        };
        if let Some((curve, interval)) = self.conformed.edges.get(&id).cloned() {
            let edge = self
                .builder
                .edge(curve.clone(), interval, start, end)
                .map_err(|error| Problem::new(id, self.describe(&error)))?;
            self.curves.insert(edge, (curve, interval));
            let entry = (edge, !same_sense);
            self.edges.insert(id, entry);
            return Ok(entry);
        }
        let curve = self.geometry.curve(fields.reference(3)?)?;
        let interval = self.interval(id, &curve, start, end)?;
        let surfaces = self.surfaces_of(self.edge_faces.get(&id));
        let traceable = self.healing != Healing::Faceted || surfaces.iter().all(exact_kind);
        let healed = match traceable
            .then(|| self.heal_edge(&curve, interval, start, end, &surfaces))
            .flatten()
        {
            None if self.healing == Healing::Faceted => self.joined(&curve, interval, [start, end]),
            healed => healed,
        };
        let (curve, interval) = match healed {
            Some((healed, gap)) => {
                self.note_repair(Some(gap));
                healed
            }
            None => (curve, interval),
        };
        if self.healing == Healing::Faceted {
            let samples: Vec<Point3> = interval
                .split(CHECK_SAMPLES)
                .map(|parameter| curve.point(parameter))
                .collect();
            let left = surfaces
                .iter()
                .map(|surface| farthest(surface, &samples))
                .fold(0.0, f64::max);
            self.loosest = self.loosest.max(left);
        }
        let edge = self
            .builder
            .edge(curve.clone(), interval, start, end)
            .map_err(|error| Problem::new(id, self.describe(&error)))?;
        self.curves.insert(edge, (curve, interval));
        let entry = (edge, !same_sense);
        self.edges.insert(id, entry);
        Ok(entry)
    }

    fn heal_edge(
        &self,
        curve: &Curve,
        interval: Interval,
        start: VertexId,
        end: VertexId,
        surfaces: &[Surface],
    ) -> Option<((Curve, Interval), f64)> {
        let (from, to) = (
            self.builder.vertex_point(start)?,
            self.builder.vertex_point(end)?,
        );
        let samples: Vec<Point3> = interval
            .split(CHECK_SAMPLES)
            .map(|parameter| curve.point(parameter))
            .collect();
        let gap = surfaces
            .iter()
            .map(|surface| farthest(surface, &samples))
            .chain([
                curve.point(interval.start()).distance(from),
                curve.point(interval.end()).distance(to),
            ])
            .fold(0.0, f64::max);
        if gap <= CLEAN {
            return None;
        }
        let [first, second] = surfaces else {
            let [only] = surfaces else {
                return None;
            };
            return healed_seam(only, &samples, [from, to], start == end).map(|seam| (seam, gap));
        };
        let mut points: Vec<Point3> = interval
            .split(HEAL_SAMPLES)
            .map(|parameter| curve.point(parameter))
            .collect();
        if let Some(point) = points.first_mut() {
            *point = from;
        }
        if let Some(point) = points.last_mut() {
            *point = to;
        }
        let rebuilt =
            IntersectionCurve::through([first.clone(), second.clone()], &points, start == end)?;
        let domain = rebuilt.domain();
        let fits = rebuilt.point(domain.start()).distance(from) <= LINEAR_RESOLUTION
            && rebuilt.point(domain.end()).distance(to) <= LINEAR_RESOLUTION;
        fits.then_some(((Curve::Intersection(rebuilt), domain), gap))
    }

    fn joined(
        &self,
        curve: &Curve,
        interval: Interval,
        [start, end]: [VertexId; 2],
    ) -> Option<((Curve, Interval), f64)> {
        let ends = [
            self.builder.vertex_point(start)?,
            self.builder.vertex_point(end)?,
        ];
        let allowance = self.geometry.units.precision?;
        met_at_ends(curve, interval, ends, allowance)
    }

    fn interval(&self, id: u64, curve: &Curve, start: VertexId, end: VertexId) -> Read<Interval> {
        let point = |vertex: VertexId| {
            self.builder
                .vertex_point(vertex)
                .ok_or_else(|| Problem::new(id, Fault::MissingVertex))
        };
        let (from, to) = (point(start)?, point(end)?);
        parameter_range(curve, from, to, start == end)
            .ok_or_else(|| Problem::new(id, Fault::BackwardsAlongCurve))
    }

    fn order_bounds(&self, bounds: &mut [Bound], surface: &Surface, sense: Sense) {
        if let Some(index) = bounds.iter().position(|bound| bound.outer) {
            bounds.swap(0, index);
            return;
        }
        if bounds.len() < 2 {
            return;
        }
        let wraps = |bound: &Bound| {
            let mut edges: Vec<EdgeId> = bound.coedges.iter().map(|(edge, _)| *edge).collect();
            edges.sort();
            edges
                .windows(2)
                .any(|pair| matches!(pair, [a, b] if a == b))
        };
        if let Some(index) = bounds.iter().position(wraps) {
            bounds.swap(0, index);
            return;
        }
        let area = |bound: &Bound| -> f64 {
            let mut previous = None;
            let polygon: Vec<Point2> = bound
                .coedges
                .iter()
                .flat_map(|(edge, coedge_sense)| self.samples(*edge, *coedge_sense))
                .map(|point| {
                    let uv = surface.project(point, previous);
                    previous = Some(uv);
                    uv
                })
                .collect();
            let travel = match (polygon.first(), polygon.last()) {
                (Some(first), Some(last)) => *last - *first,
                _ => Point2::ZERO,
            };
            let wrapped = [surface.u_period(), surface.v_period()]
                .into_iter()
                .zip([travel.x, travel.y])
                .any(|(period, travel)| period.is_some_and(|period| travel.abs() > 0.5 * period));
            if wrapped {
                f64::INFINITY
            } else {
                signed_area(&polygon) * sense.sign()
            }
        };
        if let Some(index) = bounds
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| area(a).total_cmp(&area(b)))
            .map(|(index, _)| index)
        {
            bounds.swap(0, index);
        }
    }

    fn samples(&self, edge: EdgeId, sense: Sense) -> Vec<Point3> {
        let Some((curve, interval)) = self.curves.get(&edge) else {
            return Vec::new();
        };
        let mut points: Vec<_> = interval
            .split(LOOP_SAMPLES)
            .map(|parameter| curve.point(parameter))
            .collect();
        if sense == Sense::Reversed {
            points.reverse();
        }
        points.pop();
        points
    }
}

fn healed_seam(
    surface: &Surface,
    samples: &[Point3],
    [from, to]: [Point3; 2],
    closed: bool,
) -> Option<(Curve, Interval)> {
    let seam = seam_through(surface, samples, from, to)?;
    let interval = parameter_range(&seam, from, to, closed)?;
    let fits = seam.point(interval.start()).distance(from) <= LINEAR_RESOLUTION
        && seam.point(interval.end()).distance(to) <= LINEAR_RESOLUTION;
    fits.then_some((seam, interval))
}

fn parameter_range(curve: &Curve, from: Point3, to: Point3, closed: bool) -> Option<Interval> {
    let domain = curve.domain();
    let search = search_range(curve);
    let low = curve.closest_parameter(from, search);
    let high = curve.closest_parameter(to, search);
    match curve.period() {
        Some(period) => {
            let mut high = low + (high - low).rem_euclid(period);
            if closed || high - low <= period * 1e-12 {
                high = low + period;
            }
            Interval::new(low, high)
        }
        None if closed => domain.bounded(),
        None => Interval::new(low, high),
    }
}

fn search_range(curve: &Curve) -> Interval {
    curve
        .domain()
        .bounded()
        .unwrap_or_else(|| Interval::new(-1e12, 1e12).unwrap_or(Interval::UNIT))
}

fn worst_gap(point: Point3, surfaces: &[Surface]) -> f64 {
    surfaces
        .iter()
        .map(|surface| surface.distance(point))
        .fold(0.0, f64::max)
}

pub(crate) fn settle_on(point: Point3, surfaces: &[Surface], reach: f64) -> (Point3, Option<f64>) {
    let worst = |at: Point3| worst_gap(at, surfaces);
    let within_reach = |at: Point3| at.distance(point) <= reach;
    let start = worst(point);
    if start <= CLEAN || surfaces.is_empty() {
        return (point, None);
    }
    let mut current = point;
    let mut current_worst = start;
    let mut hints: Vec<Option<Point2>> = vec![None; surfaces.len()];
    let mut damping = VERTEX_DAMPING;
    for _ in 0..VERTEX_ITERATIONS {
        let system = linearised_gaps(surfaces, current, &mut hints);
        let Some((next, next_worst)) = damped_step(
            system,
            (current, current_worst),
            &mut damping,
            worst,
            within_reach,
        ) else {
            break;
        };
        current = next;
        current_worst = next_worst;
        if current_worst <= 1e-3 * LINEAR_RESOLUTION {
            break;
        }
    }
    (current, (current_worst < start).then_some(start))
}

fn linearised_gaps(
    surfaces: &[Surface],
    at: Point3,
    hints: &mut [Option<Point2>],
) -> ([[f64; 3]; 3], [f64; 3]) {
    let mut normal_matrix = [[0.0f64; 3]; 3];
    let mut right = [0.0f64; 3];
    for (surface, hint) in surfaces.iter().zip(hints.iter_mut()) {
        let uv = surface.project(at, *hint);
        *hint = Some(uv);
        let foot = surface.point_at(uv);
        let Some(normal) = surface.normal(uv.x, uv.y) else {
            continue;
        };
        let gap = normal.dot(foot - at);
        let components = normal.to_array();
        for (row, value) in normal_matrix.iter_mut().zip(components) {
            for (entry, other) in row.iter_mut().zip(components) {
                *entry += value * other;
            }
        }
        for (entry, value) in right.iter_mut().zip(components) {
            *entry += value * gap;
        }
    }
    (normal_matrix, right)
}

fn damped_step(
    (normal_matrix, right): ([[f64; 3]; 3], [f64; 3]),
    (current, current_worst): (Point3, f64),
    damping: &mut f64,
    worst: impl Fn(Point3) -> f64,
    within_reach: impl Fn(Point3) -> bool,
) -> Option<(Point3, f64)> {
    while *damping <= MAX_VERTEX_DAMPING {
        let mut damped = normal_matrix;
        for (index, row) in damped.iter_mut().enumerate() {
            if let Some(entry) = row.get_mut(index) {
                *entry += *damping;
            }
        }
        if let Some(step) = solve3(damped, right) {
            let next = current + step;
            let next_worst = worst(next);
            if next_worst < current_worst && within_reach(next) {
                *damping = (*damping * DAMPING_RELIEF).max(VERTEX_DAMPING);
                return Some((next, next_worst));
            }
        }
        *damping *= DAMPING_GROWTH;
    }
    None
}

fn solve3(matrix: [[f64; 3]; 3], right: [f64; 3]) -> Option<Vector3> {
    let [[a, b, c], [d, e, f], [g, h, i]] = matrix;
    let determinant = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if determinant.abs() < 1e-300 || !determinant.is_finite() {
        return None;
    }
    let [x, y, z] = right;
    let solution = Vector3::new(
        (x * (e * i - f * h) - b * (y * i - f * z) + c * (y * h - e * z)) / determinant,
        (a * (y * i - f * z) - x * (d * i - f * g) + c * (d * z - y * g)) / determinant,
        (a * (e * z - y * h) - b * (d * z - y * g) + x * (d * h - e * g)) / determinant,
    );
    solution.is_finite().then_some(solution)
}

struct Faceted {
    solid: Solid,
    faces: Vec<Option<FaceId>>,
}

fn facets(builder: &SolidBuilder, open: bool) -> Result<Faceted, Fault> {
    let mut last = None;
    let coarse = MeshQuality::COARSE;
    let coarser = (1..=COARSER_STEPS).filter_map(|step| {
        let scale = COARSER_FACTOR.powi(step);
        MeshQuality::new(coarse.chord_fraction() * scale, coarse.angle() * scale)
    });
    for quality in [MeshQuality::SMOOTH, coarse].into_iter().chain(coarser) {
        let FaceMesh { mesh, faces } = builder
            .unvalidated_mesh(&quality)
            .map_err(Fault::NotMeshed)?;
        let mesh = if open { closed_over(mesh) } else { mesh };
        match faceted_solids(&mesh) {
            Ok(built) if built.solids.is_empty() => return Err(Fault::NotOneSolid),
            Ok(mut built) => {
                let solid = match built.solids.len() {
                    1 => built.solids.pop().ok_or(Fault::NotOneSolid)?,
                    _ if !built.solids.iter().all(has_thickness) => {
                        return Err(Fault::PieceWithoutThickness);
                    }
                    _ => Solid::joined(built.solids).map_err(Fault::PiecesNotJoined)?,
                };
                return Ok(Faceted {
                    faces: built
                        .sources
                        .concat()
                        .iter()
                        .map(|made_of| main_face(made_of, &mesh, &faces))
                        .collect(),
                    solid,
                });
            }
            Err(FacetedError::TooDetailed { faces }) => {
                last = Some(Fault::TooManyFlatFaces { faces });
            }
            Err(error) => return Err(Fault::NotClosedIntoSolid(error)),
        }
    }
    Err(last.unwrap_or(Fault::NotOneSolid))
}

fn has_thickness(piece: &Solid) -> bool {
    mass_properties_tessellating(piece, &piece.default_tolerance()).is_ok_and(|mass| {
        3.0 * mass.properties.volume.abs() > LINEAR_RESOLUTION * mass.properties.area
    })
}

fn main_face(made_of: &[usize], mesh: &TriangleMesh, faces: &[FaceId]) -> Option<FaceId> {
    let mut areas: BTreeMap<FaceId, f64> = BTreeMap::new();
    for triangle in made_of {
        let (Some(face), Some(corners)) = (faces.get(*triangle), mesh.triangles.get(*triangle))
        else {
            continue;
        };
        let [a, b, c] = corners.map(|corner| mesh.positions.get(corner).copied());
        let area = match (a, b, c) {
            (Some(a), Some(b), Some(c)) => (b - a).cross(c - a).length(),
            _ => 0.0,
        };
        *areas.entry(*face).or_default() += area;
    }
    areas
        .into_iter()
        .max_by(|(_, first), (_, second)| first.total_cmp(second))
        .map(|(face, _)| face)
}

fn closed_over(mut mesh: TriangleMesh) -> TriangleMesh {
    let Some(first) = mesh.positions.first().copied() else {
        return mesh;
    };
    let (low, high) = mesh
        .positions
        .iter()
        .fold((first, first), |(low, high), point| {
            (low.min(*point), high.max(*point))
        });
    let cell = (high - low).length().max(LINEAR_RESOLUTION) * WELD_FRACTION;
    let mut welded: BTreeMap<[i64; 3], usize> = BTreeMap::new();
    let same: Vec<usize> = mesh
        .positions
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let key = ((*point - low) / cell)
                .round()
                .to_array()
                .map(|value| value as i64);
            *welded.entry(key).or_insert(index)
        })
        .collect();
    let corner = |index: usize| same.get(index).copied().unwrap_or(index);
    let mut sides: BTreeSet<(usize, usize)> = BTreeSet::new();
    for triangle in &mesh.triangles {
        let [a, b, c] = triangle.map(corner);
        sides.extend([(a, b), (b, c), (c, a)]);
    }
    let mut open: BTreeMap<usize, usize> = BTreeMap::new();
    for (from, to) in &sides {
        if !sides.contains(&(*to, *from)) && open.insert(*from, *to).is_some() {
            return mesh;
        }
    }
    let mut holes = Vec::new();
    while let Some((&start, _)) = open.first_key_value() {
        let mut hole = vec![start];
        let mut at = start;
        loop {
            let Some(next) = open.remove(&at) else {
                return mesh;
            };
            if next == start {
                break;
            }
            hole.push(next);
            at = next;
        }
        holes.push(hole);
    }
    for hole in holes {
        let points: Vec<Point3> = hole
            .iter()
            .filter_map(|index| mesh.positions.get(*index).copied())
            .collect();
        let centre = points.iter().fold(Point3::ZERO, |sum, point| sum + *point)
            / points.len().max(1) as f64;
        let middle = mesh.positions.len();
        mesh.positions.push(centre);
        for (index, from) in hole.iter().enumerate() {
            if let Some(to) = hole.get((index + 1) % hole.len()) {
                mesh.triangles.push([*to, *from, middle]);
            }
        }
    }
    mesh
}

fn upright(surface: Surface) -> (Surface, bool) {
    match surface {
        Surface::BSpline(spline) => {
            let last_column = spline.columns().saturating_sub(1);
            let last_row = spline.rows().saturating_sub(1);
            let sideways = spline.degenerate_column(0) || spline.degenerate_column(last_column);
            let upright = spline.degenerate_row(0) || spline.degenerate_row(last_row);
            if sideways && !upright {
                (Surface::from(spline.transposed()), true)
            } else {
                (Surface::BSpline(spline), false)
            }
        }
        other => (other, false),
    }
}

fn signed_area(polygon: &[Point2]) -> f64 {
    let count = polygon.len();
    0.5 * polygon
        .iter()
        .enumerate()
        .filter_map(|(index, point)| {
            polygon
                .get((index + 1) % count)
                .map(|next| point.perp_dot(*next))
        })
        .sum::<f64>()
}

fn closed_shells(graph: &Graph<'_>, model: Entity<'_>) -> Read<Vec<u64>> {
    let mut closed = Vec::new();
    for shell in model.fields()?.references(1)? {
        if graph.entity(shell)?.kind() == "CLOSED_SHELL" {
            closed.push(shell);
        }
    }
    Ok(closed)
}

fn polygon_plane(points: &[Point3], orientation: bool, uncertainty: f64) -> Option<Plane> {
    let first = *points.first()?;
    let mut normal = Vector3::ZERO;
    for (index, point) in points.iter().enumerate() {
        let next = points.get((index + 1) % points.len())?;
        normal += (*point - first).cross(*next - first);
    }
    let normal = if orientation { normal } else { -normal };
    let along = points
        .iter()
        .map(|point| *point - first)
        .find(|offset| offset.length() > uncertainty)?;
    Plane::with_x_axis(first, normal, along)
}

pub(crate) fn short(value: f64) -> String {
    let digits = (1.0 - value.log10().floor()).clamp(0.0, 12.0) as usize;
    format_decimals(value, digits)
}

#[cfg(test)]
mod tests {
    use caditor_kernel::{
        BSplineSurface, CoedgeId, GeometryError, Interrupted, LoopId, PcurveError,
        TessellationError, ValidationError,
    };

    use super::*;

    fn bump() -> Surface {
        let size = 7;
        let mut points = Vec::new();
        for row in 0..size {
            for column in 0..size {
                let (x, y) = (column as f64 * 5.0, row as f64 * 5.0);
                points.push(Point3::new(x, y, 4.0 * (x * 0.2).sin() * (y * 0.15).cos()));
            }
        }
        let knots: Vec<f64> = std::iter::repeat_n(0.0, 4)
            .chain((1..size - 3).map(|index| index as f64 / (size - 3) as f64))
            .chain(std::iter::repeat_n(1.0, 4))
            .collect();
        let weights = (0..size * size)
            .map(|index| 1.0 + (index % 3) as f64 * 0.3)
            .collect();
        Surface::from(
            BSplineSurface::new(3, 3, knots.clone(), knots, size, points, Some(weights)).unwrap(),
        )
    }

    fn on_every_sample(surface: &Surface, samples: &[Point3]) -> bool {
        samples
            .iter()
            .all(|point| surface.distance(*point) <= CLEAN)
    }

    fn some_ids() -> (VertexId, EdgeId, FaceId, ShellId, LoopId, CoedgeId) {
        let solid = crate::fixtures::plate_with_hole();
        (
            solid.vertices().next().unwrap().0,
            solid.edges().next().unwrap().0,
            solid.faces().next().unwrap().0,
            solid.shells().next().unwrap().0,
            solid.loops().next().unwrap().0,
            solid.coedges().next().unwrap().0,
        )
    }

    fn every_kind_of_build_error() -> Vec<BuildError> {
        let (vertex, edge, face, shell, face_loop, coedge) = some_ids();
        let validations = vec![
            ValidationError::NoShells,
            ValidationError::EmptyShell(shell),
            ValidationError::FaceWithoutLoops(face),
            ValidationError::EmptyLoop(face_loop),
            ValidationError::MissingEntity,
            ValidationError::TooManyEntities,
            ValidationError::UnusedVertex(vertex),
            ValidationError::ZeroLengthEdge(edge),
            ValidationError::VertexOffEdge {
                edge,
                vertex,
                distance: 0.5,
            },
            ValidationError::EdgeUseCount { edge, uses: 3 },
            ValidationError::EdgeSenses(edge),
            ValidationError::EdgeAcrossShells(edge),
            ValidationError::LoopBreak { face_loop, coedge },
            ValidationError::PcurveEnds(coedge),
            ValidationError::EdgeOffSurface {
                edge,
                face,
                distance: 0.5,
            },
            ValidationError::PcurveOffEdge {
                coedge,
                distance: 0.5,
            },
            ValidationError::PcurveGap(coedge),
            ValidationError::LoopOrientation {
                face_loop,
                area: -1.0,
            },
            ValidationError::InnerLoopOutside(face_loop),
            ValidationError::ShellDisconnected { shell, face },
            ValidationError::EulerPoincare {
                shell,
                characteristic: 1,
            },
            ValidationError::Tessellation(TessellationError::EdgeSampling(edge)),
            ValidationError::EmptyVolume(shell),
            ValidationError::VoidOutside(shell),
            ValidationError::LumpsOverlap {
                shell,
                other: shell,
            },
            ValidationError::LumpsCoincide {
                shell,
                other: shell,
            },
            ValidationError::DanglingEdge { edge, face },
            ValidationError::Cancelled(Interrupted),
        ];
        let mut errors: Vec<BuildError> =
            validations.into_iter().map(BuildError::Invalid).collect();
        errors.extend([
            BuildError::UnknownVertex(vertex),
            BuildError::UnknownEdge(edge),
            BuildError::UnknownFace(face),
            BuildError::UnknownShell(shell),
            BuildError::NonFinitePoint,
            BuildError::VertexOffCurve {
                vertex,
                distance: 0.5,
            },
            BuildError::IntervalOutsideDomain,
            BuildError::ZeroLengthEdge,
            BuildError::EmptyLoop,
            BuildError::TooManyEntities,
            BuildError::Pcurve {
                edge,
                error: PcurveError::NotMonotone,
            },
            BuildError::Geometry(GeometryError::DegenerateSurface),
            BuildError::Cancelled(Interrupted),
        ]);
        errors
    }

    #[test]
    fn no_note_about_a_body_that_cannot_be_rebuilt_shows_kernel_internals() {
        let errors = every_kind_of_build_error();
        let forbidden = [
            "Id(", "ShellId", "FaceId", "EdgeId", "Euler", "Poincar", "pcurve", "coedge", "{", "}",
        ];

        for precision in [None, Some(0.1), Some(0.001)] {
            for error in &errors {
                let note = Fault::build(error.clone(), precision).to_string();

                assert!(!note.is_empty());
                for word in forbidden {
                    assert!(
                        !note.contains(word),
                        "the note {note:?} for {error:?} holds {word:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_shell_that_does_not_close_up_is_worded_plainly() {
        let (_, _, _, shell, _, _) = some_ids();
        let error = BuildError::Invalid(ValidationError::EulerPoincare {
            shell,
            characteristic: 1,
        });

        let note = Fault::build(error, None).to_string();

        assert_eq!(
            note,
            "does not make a closed, valid solid: its faces do not close up into one surface without gaps or extra edges"
        );
    }

    #[test]
    fn chained_hints_decide_like_projecting_every_sample_afresh() {
        let surfaces = [
            bump(),
            Surface::Sphere(caditor_kernel::Sphere::new(Plane::XY, 6.0).unwrap()),
        ];
        let mut decided = [0, 0];
        for surface in &surfaces {
            for path in 0..40 {
                let turn = path as f64 * 0.37;
                let (start, end) = (
                    Point2::new(0.1 + 0.02 * turn.sin(), 0.2 + 0.1 * turn.cos()),
                    Point2::new(0.9 - 0.03 * turn.cos(), 0.7 + 0.2 * turn.sin()),
                );
                let (u, v) = (
                    surface.u_domain().bounded().unwrap_or(Interval::UNIT),
                    surface.v_domain().bounded().unwrap_or(Interval::UNIT),
                );
                let lift = CLEAN * [0.0, 0.4, 0.8, 1.3, 2.0][path % 5];
                let samples: Vec<Point3> = (0..=CHECK_SAMPLES)
                    .map(|index| {
                        let along = start.lerp(end, index as f64 / CHECK_SAMPLES as f64);
                        let at = surface.evaluate(u.at(along.x), v.at(along.y));
                        let bend = if index == CHECK_SAMPLES / 2 {
                            lift
                        } else {
                            0.0
                        };
                        at.point + at.normal().unwrap() * bend
                    })
                    .collect();
                let chained = farthest(surface, &samples) <= CLEAN;
                assert_eq!(chained, on_every_sample(surface, &samples), "path {path}");
                decided[usize::from(chained)] += 1;
            }
        }
        assert!(decided.iter().all(|count| *count > 10), "{decided:?}");
    }
}
