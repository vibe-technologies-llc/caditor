use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::FRAC_PI_2,
};

use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{
    BSpline, BuildError, Circle, CrossingCheck, Curve, EdgeId, FaceId, FacetedError,
    IntersectionCurve, Interval, LINEAR_RESOLUTION, MeshQuality, PlaneSurface, Sense, ShellId,
    Solid, SolidBuilder, Surface, ValidationError, VertexId, faceted_solids,
};

use crate::read::{
    conform::{Conformed, LooseBody, LooseEdge, conform},
    geometry::Geometry,
    graph::{Entity, Graph, Problem, Read, friendly},
    loose::{exact_kind, farthest, met_at_ends},
};

const LOOP_SAMPLES: usize = 16;
const CHECK_SAMPLES: usize = 32;
const HEAL_SAMPLES: usize = 48;
const CLEAN: f64 = 0.25 * LINEAR_RESOLUTION;
const VERTEX_ITERATIONS: usize = 30;
const VERTEX_DAMPING: f64 = 1e-12;
const COARSER_STEPS: i32 = 2;
const COARSER_FACTOR: f64 = 2.0;
const MAX_VERTEX_DAMPING: f64 = 1e6;
const DAMPING_GROWTH: f64 = 10.0;
const DAMPING_RELIEF: f64 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Healing {
    Exact,
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
    vertices: BTreeMap<u64, VertexId>,
    edges: BTreeMap<u64, (EdgeId, bool)>,
    curves: BTreeMap<EdgeId, (Curve, Interval)>,
    corners: BTreeMap<[u64; 3], VertexId>,
    sides: BTreeMap<(VertexId, VertexId), EdgeId>,
    face_entities: Vec<u64>,
    healed: usize,
    loosest: f64,
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
                    .ok_or_else(|| Problem::new(id, "has no closed shell, so it is not a solid"))?;
                (first, Vec::new(), closed.collect())
            }
            other => {
                return Err(Problem::new(
                    id,
                    format!("is a {}, which caditor cannot import yet", friendly(other)),
                ));
            }
        };
        Ok(Self {
            outer,
            lumps,
            voids,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Built {
    pub solid: Solid,
    pub healed: usize,
    pub unchecked: Option<[u64; 2]>,
    pub faceted: bool,
    pub bent: Option<Bending>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Bending {
    pub faces: usize,
    pub farthest: f64,
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
            vertices: BTreeMap::new(),
            edges: BTreeMap::new(),
            curves: BTreeMap::new(),
            corners: BTreeMap::new(),
            sides: BTreeMap::new(),
            face_entities: Vec::new(),
            healed: 0,
            loosest: 0.0,
            conformed: Conformed::default(),
        }
    }

    pub fn solid(mut self, id: u64, shells: &SolidShells) -> Read<Built> {
        let graph = self.geometry.graph;
        let SolidShells {
            outer,
            lumps,
            voids,
        } = shells;
        let mut shells = vec![self.plan_shell(*outer, false)?];
        for lump in lumps {
            shells.push(self.plan_shell(*lump, false)?);
        }
        for void in voids.iter().copied() {
            let void_entity = graph.entity(void)?;
            shells.push(match void_entity.kind() {
                "ORIENTED_CLOSED_SHELL" => {
                    let fields = void_entity.record("ORIENTED_CLOSED_SHELL")?;
                    let orientation = fields.logical(3)?;
                    self.plan_shell(fields.reference(2)?, !orientation)?
                }
                _ => self.plan_shell(void, true)?,
            });
        }
        if self.healing == Healing::Bent {
            self.conform(id)?;
        }
        for shell in &shells {
            self.build_shell(shell)?;
        }
        let healed = self.healed;
        let precision = self.geometry.units.precision;
        let faceted = self.healing == Healing::Faceted;
        let solid = if faceted {
            let allowance = precision.unwrap_or(LINEAR_RESOLUTION);
            if self.loosest > allowance {
                return Err(Problem::new(
                    id,
                    format!(
                        "has faces {} mm apart, more than the file's precision of {} mm",
                        short(self.loosest),
                        short(allowance)
                    ),
                ));
            }
            facets(&self.builder).map_err(|reason| Problem::new(id, reason))?
        } else {
            self.builder
                .build()
                .map_err(|error| Problem::new(id, describe_build(&error, precision)))?
        };
        let check = if faceted {
            CrossingCheck::Clear
        } else {
            solid.find_crossing().map_err(|_| {
                Problem::new(id, "was not checked, because the import was cancelled")
            })?
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
                let reason = match entities(crossing.faces) {
                    [Some(first), Some(second)] if first == second => format!(
                        "has face #{first} whose edges cross each other, so it does not enclose one \
                         volume"
                    ),
                    [Some(first), Some(second)] => format!(
                        "has faces #{first} and #{second} that cross each other, so it does not \
                         enclose one volume"
                    ),
                    _ => "has faces that cross each other, so it does not enclose one volume"
                        .to_owned(),
                };
                return Err(Problem::new(id, reason));
            }
        };
        let bent = (self.healing == Healing::Bent).then_some(Bending {
            faces: self.conformed.faces_bent,
            farthest: self.conformed.farthest,
        });
        Ok(Built {
            solid,
            healed,
            unchecked,
            faceted,
            bent,
        })
    }

    fn conform(&mut self, id: u64) -> Read<()> {
        let allowance =
            self.geometry.units.precision.ok_or_else(|| {
                Problem::new(id, "declares no precision to bend its faces within")
            })?;
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
                    format!("is a {} where a closed shell belongs", friendly(other)),
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
                    format!("is a {} where a face belongs", friendly(other)),
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
        let (surface, transposed) = upright(self.geometry.surface(fields.reference(2)?)?);
        let graph = self.geometry.graph;
        for bound in &bounds {
            let loop_id = graph.entity(*bound)?.fields()?.reference(1)?;
            let loop_entity = graph.entity(loop_id)?;
            if loop_entity.kind() != "EDGE_LOOP" {
                continue;
            }
            for oriented in loop_entity.fields()?.references(1)? {
                let edge = graph
                    .entity(oriented)?
                    .record("ORIENTED_EDGE")?
                    .reference(3)?;
                self.edge_faces.entry(edge).or_default().insert(entity.id);
                let edge_fields = graph.entity(edge)?.record("EDGE_CURVE")?;
                for vertex in [edge_fields.reference(1)?, edge_fields.reference(2)?] {
                    self.vertex_faces
                        .entry(vertex)
                        .or_default()
                        .insert(entity.id);
                }
            }
        }
        self.faces.insert(
            entity.id,
            FacePlan {
                surface,
                same_sense: fields.logical(3)? != transposed,
                bounds,
            },
        );
        Ok(())
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
            None => return Err(Problem::new(id, "is missing from its shell")),
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
                .ok_or_else(|| {
                    Problem::new(
                        id,
                        "has no edges around it, which caditor cannot import yet",
                    )
                })?;
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

    fn describe(&self, error: &BuildError) -> String {
        describe_build(error, self.geometry.units.precision)
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
        let (points, orientation) =
            outline.ok_or_else(|| Problem::new(face, "has no polygon to take its plane from"))?;
        let plane = polygon_plane(&points, orientation, self.geometry.units.uncertainty())
            .ok_or_else(|| Problem::new(face, "has a polygon that encloses no area"))?;
        PlaneSurface::new(plane)
            .map(Surface::from)
            .map_err(|error| Problem::new(face, format!("is not a usable plane ({error})")))
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
            return Err(Problem::new(id, "has fewer than three corners"));
        }
        let mut coedges = Vec::with_capacity(corners.len());
        for (index, from) in corners.iter().enumerate() {
            let to = corners
                .get((index + 1) % corners.len())
                .copied()
                .ok_or_else(|| Problem::new(id, "has fewer than three corners"))?;
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
                    format!("is a {}, which caditor cannot import yet", friendly(other)),
                ));
            }
        };
        let mut coedges = Vec::with_capacity(edges.len());
        for oriented in edges {
            let oriented_fields = graph.entity(oriented)?.record("ORIENTED_EDGE")?;
            let orientation = oriented_fields.logical(4)?;
            let (edge, edge_reversed) = self.edge(oriented_fields.reference(3)?)?;
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
                let (point, gap) = settle_on(point, &surfaces);
                self.note_repair(gap);
                point
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
            return None;
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
                .ok_or_else(|| Problem::new(id, "uses a vertex that is missing"))
        };
        let (from, to) = (point(start)?, point(end)?);
        parameter_range(curve, from, to, start == end)
            .ok_or_else(|| Problem::new(id, "runs backwards along its curve"))
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

fn parameter_range(curve: &Curve, from: Point3, to: Point3, closed: bool) -> Option<Interval> {
    let domain = curve.domain();
    let search = domain
        .bounded()
        .unwrap_or_else(|| Interval::new(-1e12, 1e12).unwrap_or(Interval::UNIT));
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

pub(crate) fn settle_on(point: Point3, surfaces: &[Surface]) -> (Point3, Option<f64>) {
    let worst = |at: Point3| {
        surfaces
            .iter()
            .map(|surface| surface.distance(at))
            .fold(0.0, f64::max)
    };
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
        let Some((next, next_worst)) =
            damped_step(system, (current, current_worst), &mut damping, worst)
        else {
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
            if next_worst < current_worst {
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

fn facets(builder: &SolidBuilder) -> Result<Solid, String> {
    let mut last = String::new();
    let coarse = MeshQuality::COARSE;
    let coarser = (1..=COARSER_STEPS).filter_map(|step| {
        let scale = COARSER_FACTOR.powi(step);
        MeshQuality::new(coarse.chord_fraction() * scale, coarse.angle() * scale)
    });
    for quality in [MeshQuality::SMOOTH, coarse].into_iter().chain(coarser) {
        let mesh = builder
            .unvalidated_mesh(&quality)
            .map_err(|error| format!("could not be meshed ({error})"))?;
        match faceted_solids(&mesh) {
            Ok(mut built) if built.solids.len() == 1 => {
                return built
                    .solids
                    .pop()
                    .ok_or_else(|| "did not close into one solid".to_owned());
            }
            Ok(_) => return Err("did not close into one solid".to_owned()),
            Err(FacetedError::TooDetailed { faces }) => {
                last = format!("would need {faces} flat faces, too many to import");
            }
            Err(error) => return Err(format!("did not close into a solid ({error})")),
        }
    }
    Err(last)
}

fn upright(surface: Surface) -> (Surface, bool) {
    match surface {
        Surface::BSpline(spline) => {
            let last_column = spline.columns().saturating_sub(1);
            let last_row = spline.rows().saturating_sub(1);
            let sideways = spline.degenerate_column(0) || spline.degenerate_column(last_column);
            let upright = spline.degenerate_row(0) || spline.degenerate_row(last_row);
            if sideways && !upright {
                (Surface::BSpline(spline.transposed()), true)
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

fn describe_build(error: &BuildError, precision: Option<f64>) -> String {
    match error {
        BuildError::VertexOffCurve { distance, .. } => format!(
            "has a vertex {} mm away from the end of its edge",
            short(*distance)
        ),
        BuildError::ZeroLengthEdge => "has an edge of no length".to_owned(),
        BuildError::IntervalOutsideDomain => {
            "has an edge that runs past the end of its curve".to_owned()
        }
        BuildError::Pcurve { .. } => {
            "has an edge that could not be followed across its face".to_owned()
        }
        BuildError::Invalid(ValidationError::EdgeOffSurface { distance, .. }) => match precision {
            Some(precision) if *distance <= precision => format!(
                "has faces that meet only within {} mm, which the file's precision of {} mm \
                     allows, but caditor needs them to meet within {} mm, so the model would \
                     have to be exported again with a finer precision",
                short(*distance),
                short(precision),
                short(LINEAR_RESOLUTION)
            ),
            _ => format!(
                "has faces that meet only within {} mm, and caditor needs them to meet within \
                     {} mm",
                short(*distance),
                short(LINEAR_RESOLUTION)
            ),
        },
        BuildError::Invalid(validation) => {
            format!("does not make a closed, valid solid ({validation})")
        }
        other => format!("could not be rebuilt ({other})"),
    }
}

pub(crate) fn short(value: f64) -> String {
    let digits = (1.0 - value.log10().floor()).clamp(0.0, 12.0) as usize;
    let written = format!("{value:.digits$}");
    if written.contains('.') {
        written
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    } else {
        written
    }
}

#[cfg(test)]
mod tests {
    use caditor_kernel::BSplineSurface;

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
        Surface::BSpline(
            BSplineSurface::new(3, 3, knots.clone(), knots, size, points, Some(weights)).unwrap(),
        )
    }

    fn on_every_sample(surface: &Surface, samples: &[Point3]) -> bool {
        samples
            .iter()
            .all(|point| surface.distance(*point) <= CLEAN)
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
