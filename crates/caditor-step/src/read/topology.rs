use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::FRAC_PI_2,
};

use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{
    BSpline, BuildError, Circle, Curve, EdgeId, FaceId, IntersectionCurve, Interval,
    LINEAR_RESOLUTION, Sense, ShellId, Solid, SolidBuilder, Surface, ValidationError, VertexId,
};

use crate::read::{
    geometry::Geometry,
    graph::{Entity, Problem, Read, friendly},
};

const LOOP_SAMPLES: usize = 16;
const CHECK_SAMPLES: usize = 32;
const HEAL_SAMPLES: usize = 48;
const CLEAN: f64 = 0.25 * LINEAR_RESOLUTION;
const VERTEX_ITERATIONS: usize = 30;
const VERTEX_DAMPING: f64 = 1e-12;

pub(crate) struct Topology<'g, 'a> {
    geometry: &'g Geometry<'a>,
    builder: SolidBuilder,
    faces: BTreeMap<u64, FacePlan>,
    edge_faces: BTreeMap<u64, BTreeSet<u64>>,
    vertex_faces: BTreeMap<u64, BTreeSet<u64>>,
    vertices: BTreeMap<u64, VertexId>,
    edges: BTreeMap<u64, (EdgeId, bool)>,
    curves: BTreeMap<EdgeId, (Curve, Interval)>,
    healed: usize,
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
    pub fn new(geometry: &'g Geometry<'a>) -> Self {
        Self {
            geometry,
            builder: SolidBuilder::new(),
            faces: BTreeMap::new(),
            edge_faces: BTreeMap::new(),
            vertex_faces: BTreeMap::new(),
            vertices: BTreeMap::new(),
            edges: BTreeMap::new(),
            curves: BTreeMap::new(),
            healed: 0,
        }
    }

    pub fn solid(mut self, id: u64) -> Read<(Solid, usize)> {
        let graph = self.geometry.graph;
        let entity = graph.entity(id)?;
        let (outer, voids) = match entity.kind() {
            "MANIFOLD_SOLID_BREP" => (
                entity.record("MANIFOLD_SOLID_BREP")?.reference(1)?,
                Vec::new(),
            ),
            "BREP_WITH_VOIDS" => {
                let fields = entity.record("BREP_WITH_VOIDS")?;
                (fields.reference(1)?, fields.references(2)?)
            }
            other => {
                return Err(Problem::new(
                    id,
                    format!("is a {}, which caditor cannot import yet", friendly(other)),
                ));
            }
        };
        let mut shells = vec![self.plan_shell(outer, false)?];
        for void in voids {
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
        for shell in &shells {
            self.build_shell(shell)?;
        }
        let healed = self.healed;
        let solid = self
            .builder
            .build()
            .map_err(|error| Problem::new(id, describe_build(&error)))?;
        Ok((solid, healed))
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
            "ADVANCED_FACE" | "FACE_SURFACE" => entity.fields()?,
            other => {
                return Err(Problem::new(
                    entity.id,
                    format!("is a {} where a face belongs", friendly(other)),
                ));
            }
        };
        let (surface, transposed) = upright(self.geometry.surface(fields.reference(2)?)?);
        let bounds = fields.references(1)?;
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
            .map_err(|error| Problem::new(plan.id, describe_build(&error)))?;
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
                .map_err(|error| Problem::new(id, describe_build(&error)))?
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
            .map_err(|error| Problem::new(id, describe_build(&error)))?;
        self.loops(face, id, &bounds)
    }

    fn loops(&mut self, face: FaceId, id: u64, bounds: &[Bound]) -> Read<()> {
        for bound in bounds {
            self.builder
                .add_loop(face, &bound.coedges)
                .map_err(|error| Problem::new(id, describe_build(&error)))?;
        }
        Ok(())
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
        let start = self.builder.vertex(curve.point(interval.start()))?;
        let end = self.builder.vertex(curve.point(interval.end()))?;
        let edge = self.builder.edge(curve.clone(), interval, start, end)?;
        self.curves.insert(edge, (curve, interval));
        Ok(Some(edge))
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
        let (point, moved) = settle_on(point, &surfaces);
        if moved {
            self.healed += 1;
        }
        let vertex = self
            .builder
            .vertex(point)
            .map_err(|error| Problem::new(id, describe_build(&error)))?;
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
        let curve = self.geometry.curve(fields.reference(3)?)?;
        let same_sense = fields.logical(4)?;
        let (start, end) = if same_sense {
            (first, second)
        } else {
            (second, first)
        };
        let interval = self.interval(id, &curve, start, end)?;
        let surfaces = self.surfaces_of(self.edge_faces.get(&id));
        let (curve, interval) = match self.heal_edge(&curve, interval, start, end, &surfaces) {
            Some(healed) => {
                self.healed += 1;
                healed
            }
            None => (curve, interval),
        };
        let edge = self
            .builder
            .edge(curve.clone(), interval, start, end)
            .map_err(|error| Problem::new(id, describe_build(&error)))?;
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
    ) -> Option<(Curve, Interval)> {
        let (from, to) = (
            self.builder.vertex_point(start)?,
            self.builder.vertex_point(end)?,
        );
        let ends_fit = curve.point(interval.start()).distance(from) <= CLEAN
            && curve.point(interval.end()).distance(to) <= CLEAN;
        let on_surfaces = interval.split(CHECK_SAMPLES).all(|parameter| {
            let point = curve.point(parameter);
            surfaces
                .iter()
                .all(|surface| surface.distance(point) <= CLEAN)
        });
        if ends_fit && on_surfaces {
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
        fits.then_some((Curve::Intersection(rebuilt), domain))
    }

    fn interval(&self, id: u64, curve: &Curve, start: VertexId, end: VertexId) -> Read<Interval> {
        let point = |vertex: VertexId| {
            self.builder
                .vertex_point(vertex)
                .ok_or_else(|| Problem::new(id, "uses a vertex that is missing"))
        };
        let (from, to) = (point(start)?, point(end)?);
        let domain = curve.domain();
        let search = domain
            .bounded()
            .unwrap_or_else(|| Interval::new(-1e12, 1e12).unwrap_or(Interval::UNIT));
        let low = curve.closest_parameter(from, search);
        let high = curve.closest_parameter(to, search);
        let range = match curve.period() {
            Some(period) => {
                let mut high = low + (high - low).rem_euclid(period);
                if start == end || high - low <= period * 1e-12 {
                    high = low + period;
                }
                Interval::new(low, high)
            }
            None if start == end => domain.bounded(),
            None => Interval::new(low, high),
        };
        range.ok_or_else(|| Problem::new(id, "runs backwards along its curve"))
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

fn settle_on(point: Point3, surfaces: &[Surface]) -> (Point3, bool) {
    let worst = |at: Point3| {
        surfaces
            .iter()
            .map(|surface| surface.distance(at))
            .fold(0.0, f64::max)
    };
    let start = worst(point);
    if start <= CLEAN || surfaces.is_empty() {
        return (point, false);
    }
    let mut current = point;
    let mut current_worst = start;
    let mut hints: Vec<Option<Point2>> = vec![None; surfaces.len()];
    for _ in 0..VERTEX_ITERATIONS {
        let mut normal_matrix = [[0.0f64; 3]; 3];
        let mut right = [0.0f64; 3];
        for (surface, hint) in surfaces.iter().zip(hints.iter_mut()) {
            let uv = surface.project(current, *hint);
            *hint = Some(uv);
            let foot = surface.point_at(uv);
            let Some(normal) = surface.normal(uv.x, uv.y) else {
                continue;
            };
            let gap = normal.dot(foot - current);
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
        for (index, row) in normal_matrix.iter_mut().enumerate() {
            if let Some(entry) = row.get_mut(index) {
                *entry += VERTEX_DAMPING;
            }
        }
        let Some(step) = solve3(normal_matrix, right) else {
            break;
        };
        let next = current + step;
        let next_worst = worst(next);
        if next_worst.is_nan() || next_worst >= current_worst {
            break;
        }
        current = next;
        current_worst = next_worst;
        if current_worst <= 1e-3 * LINEAR_RESOLUTION {
            break;
        }
    }
    (current, current_worst < start)
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

pub(crate) fn describe_build(error: &BuildError) -> String {
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
        BuildError::Invalid(ValidationError::EdgeOffSurface { distance, .. }) => format!(
            "has faces that meet only within {} mm, and caditor needs them to meet within {} mm",
            short(*distance),
            short(LINEAR_RESOLUTION)
        ),
        BuildError::Invalid(validation) => {
            format!("does not make a closed, valid solid ({validation})")
        }
        other => format!("could not be rebuilt ({other})"),
    }
}

fn short(value: f64) -> String {
    let digits = (1.0 - value.log10().floor()).clamp(0.0, 12.0) as usize;
    format!("{value:.digits$}")
}
