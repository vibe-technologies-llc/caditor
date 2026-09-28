use caditor_geometry::{Aabb, Aabb2, Point2, Point3, Vector2, Vector3};

use crate::{
    curve::{Curve, Line},
    intersect::{boxes_overlap, intersect_curve_surface, patch_bounds},
    interval::Interval,
    sense::Sense,
    surface::Surface,
    tolerance::{LINEAR_RESOLUTION, PCURVE_TOLERANCE},
    topology::{CoedgeId, EdgeId, FaceId, LoopId, Solid, VertexId},
};

const TOLERANCE: f64 = LINEAR_RESOLUTION;
const NEAR_BOUNDARY: f64 = 8.0 * PCURVE_TOLERANCE;
const GRAZING_COSINE: f64 = 1e-4;
const COINCIDENT_SINE: f64 = 1e-6;
const POLE_NUDGE: f64 = 1e-7;
const RELATIVE_POLE_NUDGE: f64 = 1e-6;
const RAY_REACH_MARGIN: f64 = 1.0;
const PERIOD_SHIFTS: [f64; 5] = [0.0, -1.0, 1.0, -2.0, 2.0];
const POLE_PROBES: usize = 5;
const RAY_DIRECTIONS: [[f64; 3]; 12] = [
    [0.573_462, 0.612_378, 0.544_223],
    [-0.408_248, 0.707_817, 0.576_479],
    [0.267_261, -0.534_522, 0.801_784],
    [0.801_784, 0.267_261, -0.534_522],
    [-0.639_602, -0.426_401, 0.639_602],
    [0.371_391, 0.928_477, 0.017_391],
    [-0.912_871, 0.182_574, -0.365_148],
    [0.123_091, -0.861_640, -0.492_366],
    [0.696_311, -0.696_311, 0.174_078],
    [-0.235_702, -0.235_702, -0.942_809],
    [0.980_581, -0.078_446, 0.179_605],
    [-0.057_735, 0.519_615, -0.852_332],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointClass {
    Inside,
    Outside,
    OnBoundary(FaceId),
    Undecided,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceContainment {
    Inside,
    Outside,
    OnBoundary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryClass {
    Inside,
    Outside,
    Coincident { face: FaceId, sense: Sense },
    Touching(FaceId),
    Undecided,
}

#[derive(Debug, Clone)]
struct BoundaryCoedge {
    id: CoedgeId,
    edge: EdgeId,
    face_loop: LoopId,
    bounds: Aabb,
}

#[derive(Debug, Clone)]
struct FaceData {
    id: FaceId,
    polygons: Vec<Vec<Point2>>,
    uv_box: Aabb2,
    bounds: Aabb,
    boundary: Vec<BoundaryCoedge>,
}

#[derive(Debug, Clone)]
pub struct SolidClassifier<'a> {
    solid: &'a Solid,
    faces: Vec<FaceData>,
    positions: Vec<Option<usize>>,
    bounds: Option<Aabb>,
}

fn positions_of(faces: &[FaceData]) -> Vec<Option<usize>> {
    let size = faces
        .iter()
        .map(|data| data.id.index() + 1)
        .max()
        .unwrap_or(0);
    let mut positions = vec![None; size];
    for (position, data) in faces.iter().enumerate() {
        if let Some(slot) = positions.get_mut(data.id.index()) {
            *slot = Some(position);
        }
    }
    positions
}

fn face_data(solid: &Solid, id: FaceId) -> Option<FaceData> {
    let face = solid.face(id)?;
    let mut polygons = Vec::with_capacity(face.loops().len());
    let mut uses: Vec<EdgeId> = Vec::new();
    for loop_id in face.loops() {
        let face_loop = solid.face_loop(*loop_id)?;
        let mut polygon = Vec::new();
        for coedge in face_loop.coedges() {
            let coedge = solid.coedge(*coedge)?;
            polygon.extend(coedge.pcurve().samples().iter().map(|sample| sample.uv));
            uses.push(coedge.edge());
        }
        polygons.push(polygon);
    }
    let uv_box = Aabb2::from_points(polygons.iter().flatten().copied())?;
    let mut boundary = Vec::new();
    for loop_id in face.loops() {
        let face_loop = solid.face_loop(*loop_id)?;
        for coedge_id in face_loop.coedges() {
            let coedge = solid.coedge(*coedge_id)?;
            let seam = uses.iter().filter(|edge| **edge == coedge.edge()).count() > 1;
            if seam {
                continue;
            }
            let edge = solid.edge(coedge.edge())?;
            boundary.push(BoundaryCoedge {
                id: *coedge_id,
                edge: coedge.edge(),
                face_loop: *loop_id,
                bounds: edge.curve().bounding_box(edge.interval()),
            });
        }
    }
    Some(FaceData {
        id,
        polygons,
        uv_box,
        bounds: patch_bounds(face.surface(), uv_box).expanded(TOLERANCE),
        boundary,
    })
}

fn crossings(polygon: &[Point2], point: Point2) -> bool {
    let count = polygon.len();
    let mut inside = false;
    for (index, a) in polygon.iter().enumerate() {
        let Some(b) = polygon.get((index + 1) % count) else {
            continue;
        };
        if (a.y > point.y) != (b.y > point.y) {
            let crossing = a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if point.x < crossing {
                inside = !inside;
            }
        }
    }
    inside
}

fn shifts(period: Option<f64>) -> Vec<f64> {
    match period {
        Some(period) => PERIOD_SHIFTS.iter().map(|shift| shift * period).collect(),
        None => vec![0.0],
    }
}

fn contains_point(bounds: &Aabb, point: Point3, margin: f64) -> bool {
    boxes_overlap(bounds, &Aabb::from_point(point), margin)
}

impl<'a> SolidClassifier<'a> {
    pub fn new(solid: &'a Solid) -> Self {
        let faces = solid
            .faces()
            .filter_map(|(id, _)| face_data(solid, id))
            .collect::<Vec<_>>();
        let bounds = faces.iter().map(|face| face.bounds).reduce(Aabb::union);
        Self {
            solid,
            positions: positions_of(&faces),
            faces,
            bounds,
        }
    }

    fn data(&self, face: FaceId) -> Option<&FaceData> {
        let position = (*self.positions.get(face.index())?)?;
        self.faces.get(position)
    }

    pub(crate) fn face_uv_box(&self, face: FaceId) -> Option<Aabb2> {
        self.data(face).map(|data| data.uv_box)
    }

    fn outward_normal(&self, face: FaceId, uv: Point2) -> Option<Vector3> {
        let face = self.solid.face(face)?;
        let surface = face.surface();
        let normal = surface.normal(uv.x, uv.y).or_else(|| {
            let nudged = Point2::new(uv.x, surface.v_domain().clamp(uv.y + POLE_NUDGE));
            surface
                .normal(nudged.x, nudged.y)
                .or_else(|| surface.normal(uv.x, surface.v_domain().clamp(uv.y - POLE_NUDGE)))
        })?;
        Some(normal * face.sense().sign())
    }

    pub fn point_in_face(&self, face: FaceId, uv: Point2) -> Option<FaceContainment> {
        let data = self.data(face)?;
        let surface = self.solid.face(face)?.surface();
        let point = surface.point_at(uv);
        let mut nearest: Option<(f64, &BoundaryCoedge, f64)> = None;
        for coedge in &data.boundary {
            if !contains_point(&coedge.bounds, point, NEAR_BOUNDARY) {
                continue;
            }
            let edge = self.solid.edge(coedge.edge)?;
            let parameter = edge.curve().closest_parameter(point, edge.interval());
            let distance = edge.curve().point(parameter).distance(point);
            if nearest.is_none_or(|(best, _, _)| distance < best) {
                nearest = Some((distance, coedge, parameter));
            }
        }
        match nearest {
            Some((distance, _, _)) if distance <= TOLERANCE => Some(FaceContainment::OnBoundary),
            Some((distance, coedge, parameter)) if distance <= NEAR_BOUNDARY => {
                let inside = self.side_of(face, uv, point, coedge, parameter)?;
                Some(if inside {
                    FaceContainment::Inside
                } else {
                    FaceContainment::Outside
                })
            }
            _ => Some(if self.inside_polygons(data, surface, uv) {
                FaceContainment::Inside
            } else {
                FaceContainment::Outside
            }),
        }
    }

    fn inside_polygons(&self, data: &FaceData, surface: &Surface, uv: Point2) -> bool {
        let probes = match surface.pole_at(uv) {
            Some(pole) => {
                let domain = surface.v_domain();
                let span = domain.end() - domain.start();
                let nudge = if span.is_finite() {
                    POLE_NUDGE.min(span * RELATIVE_POLE_NUDGE)
                } else {
                    POLE_NUDGE
                };
                let from_start = (pole.v - domain.start()).abs();
                let from_end = (pole.v - domain.end()).abs();
                let inward = if from_start <= from_end {
                    nudge
                } else {
                    -nudge
                };
                let u_range = Interval::new(data.uv_box.min().x, data.uv_box.max().x)
                    .unwrap_or(Interval::UNIT);
                (0..POLE_PROBES)
                    .map(|index| {
                        Point2::new(
                            u_range.at((index as f64 + 0.5) / POLE_PROBES as f64),
                            pole.v + inward,
                        )
                    })
                    .collect()
            }
            None => vec![uv],
        };
        let slack = data
            .uv_box
            .expanded(1e-9 * (1.0 + data.uv_box.size().max_element()));
        probes.into_iter().any(|probe| {
            shifts(surface.u_period()).into_iter().any(|du| {
                shifts(surface.v_period()).into_iter().any(|dv| {
                    let shifted = probe + Vector2::new(du, dv);
                    slack.contains(shifted)
                        && data
                            .polygons
                            .iter()
                            .filter(|polygon| crossings(polygon, shifted))
                            .count()
                            % 2
                            == 1
                })
            })
        })
    }

    fn coedge_tangent(&self, coedge: CoedgeId, parameter: f64) -> Option<Vector3> {
        let data = self.solid.coedge(coedge)?;
        let edge = self.solid.edge(data.edge())?;
        Some(edge.curve().evaluate(parameter).first * data.sense().sign())
    }

    fn left_of(&self, normal: Vector3, tangent: Vector3, from: Point3, point: Point3) -> bool {
        normal.cross(tangent).dot(point - from) > 0.0
    }

    fn side_of(
        &self,
        face: FaceId,
        uv: Point2,
        point: Point3,
        coedge: &BoundaryCoedge,
        parameter: f64,
    ) -> Option<bool> {
        let normal = self.outward_normal(face, uv)?;
        let edge = self.solid.edge(coedge.edge)?;
        let foot = edge.curve().point(parameter);
        let (start, end) = self.solid.coedge_parameters(coedge.id)?;
        let (from, to) = self.solid.coedge_vertices(coedge.id)?;
        let at_vertex = if parameter == start {
            Some(from)
        } else if parameter == end {
            Some(to)
        } else {
            None
        };
        let Some(vertex) = at_vertex else {
            let tangent = self.coedge_tangent(coedge.id, parameter)?;
            return Some(self.left_of(normal, tangent, foot, point));
        };
        let (incoming, outgoing) = self.corner(coedge, vertex)?;
        let tangent_in = self.coedge_tangent(incoming.0, incoming.1)?;
        let tangent_out = self.coedge_tangent(outgoing.0, outgoing.1)?;
        let left_in = self.left_of(normal, tangent_in, foot, point);
        let left_out = self.left_of(normal, tangent_out, foot, point);
        let convex = tangent_in.cross(tangent_out).dot(normal) >= 0.0;
        Some(if convex {
            left_in && left_out
        } else {
            left_in || left_out
        })
    }

    fn corner(
        &self,
        coedge: &BoundaryCoedge,
        vertex: VertexId,
    ) -> Option<((CoedgeId, f64), (CoedgeId, f64))> {
        let face_loop = self.solid.face_loop(coedge.face_loop)?;
        let data = self
            .faces
            .iter()
            .find(|data| data.boundary.iter().any(|entry| entry.id == coedge.id))?;
        let boundary: Vec<CoedgeId> = face_loop
            .coedges()
            .iter()
            .copied()
            .filter(|id| data.boundary.iter().any(|entry| entry.id == *id))
            .collect();
        let ending = boundary.iter().copied().find(|id| {
            self.solid
                .coedge_vertices(*id)
                .is_some_and(|(_, end)| end == vertex)
        })?;
        let starting = boundary.iter().copied().find(|id| {
            self.solid
                .coedge_vertices(*id)
                .is_some_and(|(start, _)| start == vertex)
        })?;
        let (_, ending_parameter) = self.solid.coedge_parameters(ending)?;
        let (starting_parameter, _) = self.solid.coedge_parameters(starting)?;
        Some(((ending, ending_parameter), (starting, starting_parameter)))
    }

    fn on_face(&self, data: &FaceData, point: Point3) -> Option<Point2> {
        if !contains_point(&data.bounds, point, TOLERANCE) {
            return None;
        }
        let surface = self.solid.face(data.id)?.surface();
        let uv = surface.project(point, Some(data.uv_box.center()));
        if surface.point_at(uv).distance(point) > TOLERANCE {
            return None;
        }
        match self.point_in_face(data.id, uv)? {
            FaceContainment::Outside => None,
            FaceContainment::Inside | FaceContainment::OnBoundary => Some(uv),
        }
    }

    pub fn classify(&self, point: Point3) -> PointClass {
        let Some(bounds) = self.bounds else {
            return PointClass::Outside;
        };
        if !point.is_finite() || !contains_point(&bounds, point, TOLERANCE) {
            return PointClass::Outside;
        }
        if let Some(data) = self
            .faces
            .iter()
            .find(|data| self.on_face(data, point).is_some())
        {
            return PointClass::OnBoundary(data.id);
        }
        let reach = bounds
            .corners()
            .iter()
            .map(|corner| corner.distance(point))
            .fold(0.0, f64::max)
            + RAY_REACH_MARGIN;
        let mut fallback = None;
        for [x, y, z] in RAY_DIRECTIONS {
            let direction = Vector3::new(x, y, z).normalize();
            match self.cast(point, direction, reach) {
                Cast::Decided(class) => return class,
                Cast::Ambiguous(guess) => {
                    if fallback.is_none() {
                        fallback = guess;
                    }
                }
            }
        }
        fallback.unwrap_or(PointClass::Undecided)
    }

    fn cast(&self, origin: Point3, direction: Vector3, reach: f64) -> Cast {
        let Ok(line) = Line::new(origin, direction) else {
            return Cast::Ambiguous(None);
        };
        let curve = Curve::Line(line);
        let mut nearest: Option<(f64, f64)> = None;
        let mut ambiguous = false;
        for data in &self.faces {
            let Some(window) = crate::intersect::line_window(origin, direction, &data.bounds)
            else {
                continue;
            };
            let (low, high) = (window.start().max(0.0), window.end().min(reach));
            let Some(range) = Interval::new(low, high) else {
                continue;
            };
            let Some(face) = self.solid.face(data.id) else {
                continue;
            };
            let Ok(found) =
                intersect_curve_surface(&curve, range, face.surface(), Some(data.uv_box))
            else {
                ambiguous = true;
                continue;
            };
            if !found.overlaps.is_empty() {
                ambiguous = true;
                continue;
            }
            for hit in found.points {
                let containment = self.point_in_face(data.id, hit.uv);
                match containment {
                    Some(FaceContainment::Outside) => {}
                    Some(FaceContainment::Inside) if !hit.tangent && hit.parameter > TOLERANCE => {
                        let Some(normal) = self.outward_normal(data.id, hit.uv) else {
                            ambiguous = true;
                            continue;
                        };
                        let facing = normal.dot(direction);
                        if facing.abs() <= GRAZING_COSINE {
                            ambiguous = true;
                        }
                        if nearest.is_none_or(|(distance, _)| hit.parameter < distance) {
                            nearest = Some((hit.parameter, facing));
                        }
                    }
                    _ => ambiguous = true,
                }
            }
        }
        let class = match nearest {
            Some((_, facing)) if facing > 0.0 => PointClass::Inside,
            _ => PointClass::Outside,
        };
        if ambiguous {
            Cast::Ambiguous(Some(class))
        } else {
            Cast::Decided(class)
        }
    }

    pub fn classify_boundary_point(&self, point: Point3, normal: Vector3) -> BoundaryClass {
        let mut touching = None;
        for data in &self.faces {
            let Some(uv) = self.on_face(data, point) else {
                continue;
            };
            let Some(outward) = self.outward_normal(data.id, uv) else {
                touching.get_or_insert(data.id);
                continue;
            };
            let Some(given) = normal.try_normalize() else {
                touching.get_or_insert(data.id);
                continue;
            };
            if outward.cross(given).length() <= COINCIDENT_SINE {
                return BoundaryClass::Coincident {
                    face: data.id,
                    sense: Sense::from_sign(outward.dot(given)),
                };
            }
            touching.get_or_insert(data.id);
        }
        if let Some(face) = touching {
            return BoundaryClass::Touching(face);
        }
        match self.classify(point) {
            PointClass::Inside => BoundaryClass::Inside,
            PointClass::Outside => BoundaryClass::Outside,
            PointClass::OnBoundary(face) => BoundaryClass::Touching(face),
            PointClass::Undecided => BoundaryClass::Undecided,
        }
    }
}

enum Cast {
    Decided(PointClass),
    Ambiguous(Option<PointClass>),
}

impl Solid {
    pub fn classifier(&self) -> SolidClassifier<'_> {
        SolidClassifier::new(self)
    }

    pub fn classify_point(&self, point: Point3) -> PointClass {
        self.classifier().classify(point)
    }

    pub fn point_in_face(&self, face: FaceId, uv: Point2) -> Option<FaceContainment> {
        let data = face_data(self, face)?;
        let faces = vec![data];
        SolidClassifier {
            solid: self,
            positions: positions_of(&faces),
            faces,
            bounds: None,
        }
        .point_in_face(face, uv)
    }

    pub fn classify_boundary_point(&self, point: Point3, normal: Vector3) -> BoundaryClass {
        self.classifier().classify_boundary_point(point, normal)
    }
}
