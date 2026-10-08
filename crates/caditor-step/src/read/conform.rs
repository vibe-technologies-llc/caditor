use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Point2, Point3, Vector2, Vector3};
use caditor_kernel::{
    BSpline, BSplineSurface, BendError, BendTarget, Curve, Interval, LINEAR_RESOLUTION, SideBend,
    Surface, SurfaceSide,
};

use crate::read::{
    graph::{Problem, Read},
    topology::settle_on,
};

const CLEAN: f64 = 0.25 * LINEAR_RESOLUTION;
const BEND_TOLERANCE: f64 = 2.5e-8;
const STUDY_SAMPLES: usize = 32;
const TANGENT_ANGLE: f64 = 1e-2;
const TABLE_PER_SPAN: usize = 8;
const TABLE_MINIMUM: usize = 64;
const NEWTON_STEPS: usize = 40;
const NEWTON_SETTLED: f64 = 1e-14;
const NEWTON_CLOSE: f64 = 1e-10;
const LINE_SIDE: f64 = 1e-6;
const PIN_SNAP: f64 = 1e-3;
const DOMAIN_SLACK: f64 = 1e-12;
const SIDES: [SurfaceSide; 4] = [
    SurfaceSide::VStart,
    SurfaceSide::VEnd,
    SurfaceSide::UStart,
    SurfaceSide::UEnd,
];

pub(crate) struct LooseEdge {
    pub id: u64,
    pub ends: [u64; 2],
    pub curve: Curve,
    pub interval: Interval,
    pub faces: Vec<u64>,
}

pub(crate) struct LooseBody<'a> {
    pub faces: BTreeMap<u64, Surface>,
    pub edges: Vec<LooseEdge>,
    pub vertices: BTreeMap<u64, Point3>,
    pub vertex_faces: &'a BTreeMap<u64, BTreeSet<u64>>,
    pub allowance: f64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Conformed {
    pub surfaces: BTreeMap<u64, Surface>,
    pub vertices: BTreeMap<u64, Point3>,
    pub edges: BTreeMap<u64, (Curve, Interval)>,
    pub faces_bent: usize,
    pub farthest: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Line {
    fixed_u: bool,
    value: f64,
}

impl Line {
    fn at(self, along: f64) -> Point2 {
        if self.fixed_u {
            Point2::new(self.value, along)
        } else {
            Point2::new(along, self.value)
        }
    }

    fn along(self, uv: Point2) -> f64 {
        if self.fixed_u { uv.y } else { uv.x }
    }

    fn across(self, uv: Point2) -> f64 {
        if self.fixed_u { uv.x } else { uv.y }
    }
}

struct Study {
    faces: [u64; 2],
    feet: [Vec<Point2>; 2],
    lines: [Option<Line>; 2],
    angle: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Bent { master: u64, slave: u64 },
    Traced,
}

struct FaceLine {
    line: Line,
    edges: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bound {
    Lower,
    Upper,
}

struct Placed {
    slave: u64,
    side: SurfaceSide,
    stations: [f64; 2],
}

pub(crate) fn conform(body: &LooseBody<'_>) -> Read<Conformed> {
    let studies: Vec<Option<Study>> = body.edges.iter().map(|edge| study(body, edge)).collect();
    let lines = face_lines(body, &studies);
    let order = ordered(body, &studies)?;
    let rank: BTreeMap<u64, usize> = order
        .iter()
        .enumerate()
        .map(|(index, face)| (*face, index))
        .collect();
    let roles: Vec<Role> = studies
        .iter()
        .map(|study| {
            study
                .as_ref()
                .map_or(Role::Traced, |study| role(body, study, &rank))
        })
        .collect();
    let vertices = placed_vertices(body, &lines)?;
    let mut bender = Bender {
        body,
        studies: &studies,
        lines: &lines,
        roles: &roles,
        vertices: &vertices,
        surfaces: body.faces.clone(),
        placed: BTreeMap::new(),
        bent: BTreeSet::new(),
        farthest: 0.0,
    };
    for face in &order {
        bender.bend_face(*face)?;
    }
    let mut edges = BTreeMap::new();
    for (index, placed) in &bender.placed {
        let Some(edge) = body.edges.get(*index) else {
            continue;
        };
        let Some(Surface::BSpline(slave)) = bender.surfaces.get(&placed.slave) else {
            continue;
        };
        let curve = slave
            .side(placed.side)
            .ok_or_else(|| Problem::new(placed.slave, "lost its side while it was bent"))?;
        edges.insert(edge.id, oriented(curve, placed.stations, edge.id)?);
    }
    let Bender {
        surfaces,
        bent,
        farthest,
        ..
    } = bender;
    let surfaces: BTreeMap<u64, Surface> = surfaces
        .into_iter()
        .filter(|(face, _)| bent.contains(face))
        .collect();
    Ok(Conformed {
        faces_bent: surfaces.len(),
        surfaces,
        vertices,
        edges,
        farthest,
    })
}

fn oriented(curve: BSpline<Point3>, [start, end]: [f64; 2], id: u64) -> Read<(Curve, Interval)> {
    let backwards = || Problem::new(id, "runs backwards along its bent face");
    if start < end {
        let interval = Interval::new(start, end).ok_or_else(backwards)?;
        return Ok((Curve::BSpline(curve), interval));
    }
    let domain = curve.domain();
    let pivot = domain.start() + domain.end();
    let interval = Interval::new(pivot - start, pivot - end).ok_or_else(backwards)?;
    Ok((Curve::BSpline(curve.reversed()), interval))
}

fn spline_of(surface: Option<&Surface>) -> Option<&BSplineSurface> {
    match surface {
        Some(Surface::BSpline(spline)) => Some(spline),
        _ => None,
    }
}

fn feet_on(surface: &Surface, points: &[Point3]) -> Vec<Point2> {
    let mut hint = None;
    points
        .iter()
        .map(|point| {
            let uv = surface.project(*point, hint);
            hint = Some(uv);
            uv
        })
        .collect()
}

fn study(body: &LooseBody<'_>, edge: &LooseEdge) -> Option<Study> {
    let [first, second] = edge.faces.as_slice() else {
        return None;
    };
    if first == second || edge.ends[0] == edge.ends[1] {
        return None;
    }
    let samples: Vec<Point3> = edge
        .interval
        .split(STUDY_SAMPLES)
        .map(|parameter| edge.curve.point(parameter))
        .collect();
    let (a, b) = (body.faces.get(first)?, body.faces.get(second)?);
    let feet = [feet_on(a, &samples), feet_on(b, &samples)];
    let angle = feet[0]
        .iter()
        .zip(&feet[1])
        .filter_map(|(on_a, on_b)| {
            let normals = (a.normal(on_a.x, on_a.y)?, b.normal(on_b.x, on_b.y)?);
            Some(normals.0.cross(normals.1).length().min(1.0).asin())
        })
        .fold(0.0, f64::max);
    let lines = [
        line_of(a, &feet[0], body.allowance),
        line_of(b, &feet[1], body.allowance),
    ];
    Some(Study {
        faces: [*first, *second],
        feet,
        lines,
        angle,
    })
}

fn line_of(surface: &Surface, feet: &[Point2], allowance: f64) -> Option<Line> {
    let Surface::BSpline(spline) = surface else {
        return None;
    };
    let mut best: Option<(f64, Line)> = None;
    for fixed_u in [true, false] {
        let (domain, closed) = if fixed_u {
            (spline.u_domain(), spline.u_period().is_some())
        } else {
            (spline.v_domain(), spline.v_period().is_some())
        };
        if closed || feet.is_empty() {
            continue;
        }
        let probe = Line {
            fixed_u,
            value: 0.0,
        };
        let mean = feet.iter().map(|uv| probe.across(*uv)).sum::<f64>() / feet.len() as f64;
        let deviation = |line: Line| {
            feet.iter()
                .map(|uv| {
                    surface
                        .point_at(line.at(line.along(*uv)))
                        .distance(surface.point_at(*uv))
                })
                .fold(0.0, f64::max)
        };
        let chosen = [domain.start(), domain.end(), mean]
            .into_iter()
            .map(|value| Line { fixed_u, value })
            .map(|line| (deviation(line), line))
            .find(|(worst, _)| *worst <= allowance);
        if let Some((worst, line)) = chosen
            && best.is_none_or(|(known, _)| worst < known)
        {
            best = Some((worst, line));
        }
    }
    best.map(|(_, line)| line)
}

fn face_lines(body: &LooseBody<'_>, studies: &[Option<Study>]) -> BTreeMap<u64, Vec<FaceLine>> {
    let mut lines: BTreeMap<u64, Vec<FaceLine>> = BTreeMap::new();
    for (index, study) in studies.iter().enumerate() {
        let Some(study) = study else {
            continue;
        };
        for slot in 0..2 {
            let (Some(face), Some(Some(line)), Some(feet)) = (
                study.faces.get(slot),
                study.lines.get(slot),
                study.feet.get(slot),
            ) else {
                continue;
            };
            let Some(surface) = body.faces.get(face) else {
                continue;
            };
            let middle = feet.get(feet.len() / 2).map_or(0.0, |uv| line.along(*uv));
            let known = lines.entry(*face).or_default();
            let same = known.iter_mut().find(|known| {
                known.line.fixed_u == line.fixed_u
                    && surface
                        .point_at(known.line.at(middle))
                        .distance(surface.point_at(line.at(middle)))
                        <= body.allowance
            });
            match same {
                Some(known) => known.edges.push(index),
                None => known.push(FaceLine {
                    line: *line,
                    edges: vec![index],
                }),
            }
        }
    }
    lines
}

fn slot_of(study: &Study, face: u64) -> Option<usize> {
    study.faces.iter().position(|known| *known == face)
}

fn ordered(body: &LooseBody<'_>, studies: &[Option<Study>]) -> Read<Vec<u64>> {
    let splines: BTreeSet<u64> = body
        .faces
        .iter()
        .filter(|(_, surface)| matches!(surface, Surface::BSpline(_)))
        .map(|(face, _)| *face)
        .collect();
    let mut after: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
    let mut waiting: BTreeMap<u64, usize> = splines.iter().map(|face| (*face, 0)).collect();
    for study in studies.iter().flatten() {
        if study.angle > TANGENT_ANGLE {
            continue;
        }
        let [a, b] = study.faces;
        if !(splines.contains(&a) && splines.contains(&b)) {
            continue;
        }
        let (master, slave) = match study.lines {
            [None, Some(_)] => (a, b),
            [Some(_), None] => (b, a),
            _ => continue,
        };
        if after.entry(master).or_default().insert(slave)
            && let Some(count) = waiting.get_mut(&slave)
        {
            *count += 1;
        }
    }
    let size = |face: u64| {
        spline_of(body.faces.get(&face)).map_or(0, |spline| spline.control_points().len())
    };
    let mut ready: BTreeSet<(usize, u64)> = waiting
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(face, _)| (size(*face), *face))
        .collect();
    let mut order = Vec::with_capacity(splines.len());
    while let Some((_, face)) = ready.pop_first() {
        order.push(face);
        for next in after.get(&face).into_iter().flatten() {
            if let Some(count) = waiting.get_mut(next) {
                *count -= 1;
                if *count == 0 {
                    ready.insert((size(*next), *next));
                }
            }
        }
    }
    if order.len() < splines.len() {
        let stuck = splines
            .iter()
            .find(|face| !order.contains(face))
            .copied()
            .unwrap_or_default();
        return Err(Problem::new(
            stuck,
            "touches other curved faces in a ring where each would have to give way to the next",
        ));
    }
    Ok(order)
}

fn role(body: &LooseBody<'_>, study: &Study, rank: &BTreeMap<u64, usize>) -> Role {
    if study.angle > TANGENT_ANGLE {
        return Role::Traced;
    }
    let [a, b] = study.faces;
    let rigid = |face: u64| spline_of(body.faces.get(&face)).is_none();
    match (rigid(a), rigid(b), study.lines) {
        (true, true, _) => Role::Traced,
        (true, false, [_, Some(_)]) => Role::Bent {
            master: a,
            slave: b,
        },
        (false, true, [Some(_), _]) => Role::Bent {
            master: b,
            slave: a,
        },
        (false, false, [Some(_), Some(_)]) => {
            let first = rank.get(&a).copied().unwrap_or(usize::MAX);
            let second = rank.get(&b).copied().unwrap_or(usize::MAX);
            if first <= second {
                Role::Bent {
                    master: a,
                    slave: b,
                }
            } else {
                Role::Bent {
                    master: b,
                    slave: a,
                }
            }
        }
        (false, false, [None, Some(_)]) => Role::Bent {
            master: a,
            slave: b,
        },
        (false, false, [Some(_), None]) => Role::Bent {
            master: b,
            slave: a,
        },
        _ => Role::Traced,
    }
}

fn placed_vertices(
    body: &LooseBody<'_>,
    lines: &BTreeMap<u64, Vec<FaceLine>>,
) -> Read<BTreeMap<u64, Point3>> {
    let on_lines = |face: u64, vertex: u64| {
        lines.get(&face).is_some_and(|lines| {
            lines.iter().any(|line| {
                line.edges.iter().any(|index| {
                    body.edges
                        .get(*index)
                        .is_some_and(|edge| edge.ends.contains(&vertex))
                })
            })
        })
    };
    let mut placed = BTreeMap::new();
    for (vertex, start) in &body.vertices {
        let rigid: Vec<Surface> = body
            .vertex_faces
            .get(vertex)
            .into_iter()
            .flatten()
            .filter(|face| spline_of(body.faces.get(face)).is_none() || !on_lines(**face, *vertex))
            .filter_map(|face| body.faces.get(face).cloned())
            .collect();
        let (point, _) = settle_on(*start, &rigid);
        let off = rigid
            .iter()
            .map(|surface| surface.distance(point))
            .fold(0.0, f64::max);
        if off > CLEAN {
            return Err(Problem::new(
                *vertex,
                format!("lies {off} mm from faces that cannot be bent to meet it"),
            ));
        }
        if point.distance(*start) > body.allowance {
            return Err(Problem::new(
                *vertex,
                "would have to move farther than the file's precision",
            ));
        }
        placed.insert(*vertex, point);
    }
    Ok(placed)
}

enum Map {
    Rigid {
        surface: Surface,
        table: Vec<(f64, Point2)>,
        shift: [Vector2; 2],
    },
    Line {
        spline: BSplineSurface,
        line: Line,
        table: Vec<(f64, f64)>,
        shift: [f64; 2],
    },
    Patch {
        spline: BSplineSurface,
        table: Vec<(f64, Point2)>,
        shift: [Vector2; 2],
    },
}

struct Strict {
    map: Map,
    side: BSpline<Point3>,
    range: Interval,
    settled: [Vector3; 2],
}

impl Strict {
    fn point(&self, along: f64) -> Option<Point3> {
        let blend = ((along - self.range.start()) / self.range.length()).clamp(0.0, 1.0);
        self.on_master(along, blend)
            .map(|point| point + self.settled[0].lerp(self.settled[1], blend))
    }

    fn on_master(&self, along: f64, blend: f64) -> Option<Point3> {
        let position = self.side.point(along);
        match &self.map {
            Map::Rigid {
                surface,
                table,
                shift,
            } => {
                let hint = nearest(table, along)?;
                let uv = surface.project(position, Some(hint));
                Some(surface.point_at(uv + shift[0].lerp(shift[1], blend)))
            }
            Map::Line {
                spline,
                line,
                table,
                shift,
            } => {
                let start = nearest(table, along)?;
                let raw = closest_on_line(spline, *line, position, start)?;
                let moved = raw + shift[0] + (shift[1] - shift[0]) * blend;
                inside(spline, line.at(moved)).map(|uv| spline.extended_evaluate(uv.x, uv.y).point)
            }
            Map::Patch {
                spline,
                table,
                shift,
            } => {
                let start = nearest(table, along)?;
                let raw = closest_on_patch(spline, position, start)?;
                inside(spline, raw + shift[0].lerp(shift[1], blend))
                    .map(|uv| spline.extended_evaluate(uv.x, uv.y).point)
            }
        }
    }

    fn crossings(&self) -> Vec<f64> {
        let mut knots = Vec::new();
        let blend =
            |along: f64| ((along - self.range.start()) / self.range.length()).clamp(0.0, 1.0);
        match &self.map {
            Map::Rigid { .. } => {}
            Map::Line {
                spline,
                line,
                table,
                shift,
            } => {
                let values: Vec<(f64, f64)> = table
                    .iter()
                    .map(|(along, t)| {
                        (*along, t + shift[0] + (shift[1] - shift[0]) * blend(*along))
                    })
                    .collect();
                let lines = if line.fixed_u {
                    spline.v_knots()
                } else {
                    spline.u_knots()
                };
                knots.extend(crossed(&values, lines));
            }
            Map::Patch {
                spline,
                table,
                shift,
            } => {
                let moved: Vec<(f64, Point2)> = table
                    .iter()
                    .map(|(along, uv)| (*along, *uv + shift[0].lerp(shift[1], blend(*along))))
                    .collect();
                let us: Vec<(f64, f64)> = moved.iter().map(|(along, uv)| (*along, uv.x)).collect();
                let vs: Vec<(f64, f64)> = moved.iter().map(|(along, uv)| (*along, uv.y)).collect();
                knots.extend(crossed(&us, spline.u_knots()));
                knots.extend(crossed(&vs, spline.v_knots()));
            }
        }
        knots
    }
}

struct Keep {
    side: BSpline<Point3>,
    range: Interval,
    shift: [Vector3; 2],
}

impl Keep {
    fn point(&self, along: f64) -> Option<Point3> {
        let blend = if self.range.length() > 0.0 {
            ((along - self.range.start()) / self.range.length()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Some(self.side.point(along) + self.shift[0].lerp(self.shift[1], blend))
    }
}

enum Goal {
    Strict(Box<Strict>),
    Keep(Keep),
}

impl Goal {
    fn point(&self, along: f64) -> Option<Point3> {
        match self {
            Self::Strict(strict) => strict.point(along),
            Self::Keep(keep) => keep.point(along),
        }
    }

    fn range(&self) -> Interval {
        match self {
            Self::Strict(strict) => strict.range,
            Self::Keep(keep) => keep.range,
        }
    }
}

fn nearest<T: Copy>(table: &[(f64, T)], along: f64) -> Option<T> {
    let after = table.partition_point(|(known, _)| *known <= along);
    let candidates = [after.checked_sub(1), Some(after)];
    candidates
        .into_iter()
        .flatten()
        .filter_map(|index| table.get(index))
        .min_by(|a, b| (a.0 - along).abs().total_cmp(&(b.0 - along).abs()))
        .map(|(_, value)| *value)
}

fn crossed(values: &[(f64, f64)], knots: &[f64]) -> Vec<f64> {
    let mut found = Vec::new();
    let mut distinct: Vec<(f64, usize)> = Vec::new();
    for knot in knots {
        match distinct.last_mut() {
            Some((value, count)) if *value == *knot => *count += 1,
            _ => distinct.push((*knot, 1)),
        }
    }
    for (knot, multiplicity) in distinct {
        for pair in values.windows(2) {
            let [(a_along, a), (b_along, b)] = pair else {
                continue;
            };
            if (a - knot) * (b - knot) < 0.0 {
                let along = a_along + (b_along - a_along) * (knot - a) / (b - a);
                found.extend(std::iter::repeat_n(along, multiplicity));
            }
        }
    }
    found
}

fn inside(spline: &BSplineSurface, uv: Point2) -> Option<Point2> {
    let fits = |value: f64, domain: Interval| {
        let slack = DOMAIN_SLACK * (1.0 + domain.length());
        (domain.start() - slack <= value && value <= domain.end() + slack)
            .then(|| domain.clamp(value))
    };
    Some(Point2::new(
        fits(uv.x, spline.u_domain())?,
        fits(uv.y, spline.v_domain())?,
    ))
}

fn closest_on_line(spline: &BSplineSurface, line: Line, point: Point3, start: f64) -> Option<f64> {
    let mut along = start;
    let mut last_step = f64::INFINITY;
    for _ in 0..NEWTON_STEPS {
        let uv = line.at(along);
        let at = spline.extended_evaluate(uv.x, uv.y);
        let (first, second) = if line.fixed_u {
            (at.dv, at.dvv)
        } else {
            (at.du, at.duu)
        };
        let offset = at.point - point;
        let curvature = first.dot(first) + offset.dot(second);
        if curvature.is_nan() || curvature <= 0.0 {
            return None;
        }
        let step = offset.dot(first) / curvature;
        along -= step;
        last_step = step.abs();
        if last_step <= NEWTON_SETTLED * (1.0 + along.abs()) {
            break;
        }
    }
    (along.is_finite() && last_step <= NEWTON_CLOSE * (1.0 + along.abs())).then_some(along)
}

fn closest_on_patch(spline: &BSplineSurface, point: Point3, start: Point2) -> Option<Point2> {
    let mut uv = start;
    let mut last_step = f64::INFINITY;
    for _ in 0..NEWTON_STEPS {
        let at = spline.extended_evaluate(uv.x, uv.y);
        let offset = at.point - point;
        let gradient = Vector2::new(at.du.dot(offset), at.dv.dot(offset));
        let uu = at.du.dot(at.du) + at.duu.dot(offset);
        let uv_term = at.du.dot(at.dv) + at.duv.dot(offset);
        let vv = at.dv.dot(at.dv) + at.dvv.dot(offset);
        let determinant = uu * vv - uv_term * uv_term;
        if !(determinant > 0.0 && uu > 0.0) {
            return None;
        }
        let step = Vector2::new(
            (vv * gradient.x - uv_term * gradient.y) / determinant,
            (uu * gradient.y - uv_term * gradient.x) / determinant,
        );
        uv -= step;
        last_step = step.length();
        if last_step <= NEWTON_SETTLED * (1.0 + uv.length()) {
            break;
        }
    }
    (uv.is_finite() && last_step <= NEWTON_CLOSE * (1.0 + uv.length())).then_some(uv)
}

struct Bender<'b, 'a> {
    body: &'b LooseBody<'a>,
    studies: &'b [Option<Study>],
    lines: &'b BTreeMap<u64, Vec<FaceLine>>,
    roles: &'b [Role],
    vertices: &'b BTreeMap<u64, Point3>,
    surfaces: BTreeMap<u64, Surface>,
    placed: BTreeMap<usize, Placed>,
    bent: BTreeSet<u64>,
    farthest: f64,
}

struct Fitting {
    line: usize,
    side: SurfaceSide,
}

impl Bender<'_, '_> {
    fn edge_vertices(&self, index: usize) -> Option<[u64; 2]> {
        self.body.edges.get(index).map(|edge| edge.ends)
    }

    fn line_vertices(&self, line: &FaceLine) -> BTreeSet<u64> {
        line.edges
            .iter()
            .filter_map(|index| self.edge_vertices(*index))
            .flatten()
            .collect()
    }

    fn is_slave(&self, face: u64, index: usize) -> bool {
        matches!(self.roles.get(index), Some(Role::Bent { slave, .. }) if *slave == face)
    }

    fn bend_face(&mut self, face: u64) -> Read<()> {
        let Some(spline) = spline_of(self.surfaces.get(&face)).cloned() else {
            return Ok(());
        };
        let Some(lines) = self.lines.get(&face) else {
            return Ok(());
        };
        let surface = Surface::BSpline(spline.clone());
        let fits: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                line.edges.iter().any(|index| self.is_slave(face, *index))
                    || self.line_vertices(line).iter().any(|vertex| {
                        self.vertices.get(vertex).is_some_and(|point| {
                            off_line(&surface, line.line, *point) > BEND_TOLERANCE
                        })
                    })
            })
            .map(|(index, _)| index)
            .collect();
        if fits.is_empty() {
            return Ok(());
        }
        let mut bounds = Vec::with_capacity(fits.len());
        for index in &fits {
            let Some(line) = lines.get(*index) else {
                continue;
            };
            bounds.push((*index, self.bound_of(face, &spline, line)?));
        }
        let (restricted, sides) = restricted(face, &spline, lines, &bounds)?;
        let mut current = restricted;
        let mut corners = [false; 4];
        for side in SIDES {
            let Some(fitting) = sides.iter().find(|fitting| fitting.side == side) else {
                continue;
            };
            let Some(line) = lines.get(fitting.line) else {
                continue;
            };
            current =
                self.bend_side(face, current, (lines, line), fitting, &sides, &mut corners)?;
        }
        self.surfaces.insert(face, Surface::BSpline(current));
        self.bent.insert(face);
        Ok(())
    }

    fn bound_of(&self, face: u64, spline: &BSplineSurface, line: &FaceLine) -> Read<Bound> {
        let domain = if line.line.fixed_u {
            spline.u_domain()
        } else {
            spline.v_domain()
        };
        let tolerance = LINE_SIDE * domain.length();
        if line.line.value <= domain.start() + tolerance {
            return Ok(Bound::Lower);
        }
        if line.line.value >= domain.end() - tolerance {
            return Ok(Bound::Upper);
        }
        let (mut above, mut below) = (0usize, 0usize);
        for (index, study) in self.studies.iter().enumerate() {
            let Some(study) = study else {
                continue;
            };
            if line.edges.contains(&index) {
                continue;
            }
            let Some(feet) = slot_of(study, face).and_then(|slot| study.feet.get(slot)) else {
                continue;
            };
            for uv in feet {
                let across = line.line.across(*uv);
                if across > line.line.value + tolerance {
                    above += 1;
                } else if across < line.line.value - tolerance {
                    below += 1;
                }
            }
        }
        match (above > 0, below > 0) {
            (true, false) => Ok(Bound::Lower),
            (false, true) => Ok(Bound::Upper),
            _ => Err(Problem::new(
                face,
                "has an edge along a line of its surface that does not bound it",
            )),
        }
    }

    fn bend_side(
        &mut self,
        face: u64,
        surface: BSplineSurface,
        (lines, line): (&[FaceLine], &FaceLine),
        fitting: &Fitting,
        sides: &[Fitting],
        corners: &mut [bool; 4],
    ) -> Read<BSplineSurface> {
        let side = fitting.side;
        let curve = surface
            .side(side)
            .ok_or_else(|| Problem::new(face, "has a side that cannot be followed"))?;
        let domain = curve.domain();
        let stations = self.stations(&curve, lines, line, side, sides);
        let mut ranges: Vec<(Interval, usize)> = Vec::new();
        for index in &line.edges {
            let Some([start, end]) = self.edge_vertices(*index) else {
                continue;
            };
            let (Some(a), Some(b)) = (stations.get(&start), stations.get(&end)) else {
                continue;
            };
            let range = Interval::new(a.min(*b), a.max(*b))
                .ok_or_else(|| Problem::new(face, "has an edge of no length along its side"))?;
            ranges.push((range, *index));
            if self.is_slave(face, *index) {
                self.placed.insert(
                    *index,
                    Placed {
                        slave: face,
                        side,
                        stations: [*a, *b],
                    },
                );
            }
        }
        ranges.sort_by(|a, b| a.0.start().total_cmp(&b.0.start()));
        if ranges
            .windows(2)
            .any(|pair| matches!(pair, [a, b] if b.0.start() < a.0.end()))
        {
            return Err(Problem::new(face, "has edges that overlap along its side"));
        }
        let pins: Vec<(f64, Point3)> = stations
            .iter()
            .filter_map(|(vertex, along)| self.vertices.get(vertex).map(|point| (*along, *point)))
            .collect();
        let shift_at = |along: f64| -> Vector3 {
            pins.iter()
                .find(|(known, _)| *known == along)
                .map_or(Vector3::ZERO, |(_, point)| *point - curve.point(along))
        };
        let mut goals: Vec<Goal> = Vec::new();
        let mut cursor = domain.start();
        let mut previous = shift_at(domain.start());
        for (range, index) in &ranges {
            let (start, end) = (shift_at(range.start()), shift_at(range.end()));
            if range.start() > cursor
                && let Some(gap) = Interval::new(cursor, range.start())
            {
                goals.push(Goal::Keep(Keep {
                    side: curve.clone(),
                    range: gap,
                    shift: [
                        if cursor == domain.start() {
                            start
                        } else {
                            previous
                        },
                        start,
                    ],
                }));
            }
            let goal = match self.roles.get(*index) {
                Some(Role::Bent { master, slave }) if *slave == face => Goal::Strict(Box::new(
                    self.strict(face, *master, *index, &curve, *range, &pins)?,
                )),
                _ => Goal::Keep(Keep {
                    side: curve.clone(),
                    range: *range,
                    shift: [start, end],
                }),
            };
            goals.push(goal);
            cursor = range.end();
            previous = end;
        }
        if cursor < domain.end()
            && let Some(gap) = Interval::new(cursor, domain.end())
        {
            goals.push(Goal::Keep(Keep {
                side: curve.clone(),
                range: gap,
                shift: [previous, previous],
            }));
        }
        let knots: Vec<f64> = goals
            .iter()
            .filter_map(|goal| match goal {
                Goal::Strict(strict) => Some(strict.crossings()),
                Goal::Keep(_) => None,
            })
            .flatten()
            .collect();
        let mut moved = 0.0f64;
        for goal in &goals {
            if let Goal::Strict(strict) = goal {
                for along in strict_samples(strict) {
                    if let Some(target) = strict.point(along) {
                        moved = moved.max(target.distance(curve.point(along)));
                    }
                }
            }
        }
        for (along, point) in &pins {
            moved = moved.max(point.distance(curve.point(*along)));
        }
        if moved > self.body.allowance {
            return Err(Problem::new(
                face,
                "would have to bend farther than the file's precision to meet its neighbours",
            ));
        }
        self.farthest = self.farthest.max(moved);
        let closures: Vec<Box<dyn Fn(f64) -> Option<Point3> + '_>> = goals
            .iter()
            .map(|goal| Box::new(move |along: f64| goal.point(along)) as Box<_>)
            .collect();
        let targets: Vec<BendTarget<'_>> = goals
            .iter()
            .zip(&closures)
            .map(|(goal, closure)| BendTarget {
                range: goal.range(),
                point: closure.as_ref(),
                strict: matches!(goal, Goal::Strict(_)),
            })
            .collect();
        let held = corner_slots(side).map(|slot| corners.get(slot).copied().unwrap_or(false));
        let bend = SideBend {
            side,
            targets,
            pins: pins.clone(),
            knots,
            held,
        };
        let bent = surface
            .bent_side(&bend, BEND_TOLERANCE)
            .map_err(|error| bend_problem(face, &error))?;
        for slot in corner_slots(side) {
            if let Some(corner) = corners.get_mut(slot) {
                *corner = true;
            }
        }
        Ok(bent.surface)
    }

    fn stations(
        &self,
        curve: &BSpline<Point3>,
        lines: &[FaceLine],
        line: &FaceLine,
        side: SurfaceSide,
        sides: &[Fitting],
    ) -> BTreeMap<u64, f64> {
        let domain = curve.domain();
        let shape = Curve::BSpline(curve.clone());
        let mut stations = BTreeMap::new();
        for vertex in self.line_vertices(line) {
            let Some(point) = self.vertices.get(&vertex) else {
                continue;
            };
            let corner = sides
                .iter()
                .filter(|other| other.side.along_u() != side.along_u())
                .find(|other| {
                    lines
                        .get(other.line)
                        .is_some_and(|other| self.line_vertices(other).contains(&vertex))
                })
                .map(|other| {
                    if other.side.at_start() {
                        domain.start()
                    } else {
                        domain.end()
                    }
                });
            let along = match corner {
                Some(end) => end,
                None => snapped(curve, shape.closest_parameter(*point, domain)),
            };
            stations.insert(vertex, along);
        }
        stations
    }

    fn strict(
        &self,
        face: u64,
        master: u64,
        index: usize,
        curve: &BSpline<Point3>,
        range: Interval,
        pins: &[(f64, Point3)],
    ) -> Read<Strict> {
        let missing = || Problem::new(face, "could not be matched to the face it meets");
        let surface = self.surfaces.get(&master).cloned().ok_or_else(missing)?;
        let pinned = |along: f64| {
            pins.iter()
                .find(|(known, _)| *known == along)
                .map(|(_, point)| *point)
                .unwrap_or_else(|| curve.point(along))
        };
        let ends = [pinned(range.start()), pinned(range.end())];
        let stations = table_stations(curve, range);
        let line = self
            .studies
            .get(index)
            .and_then(Option::as_ref)
            .and_then(|study| {
                let slot = slot_of(study, master)?;
                let line = (*study.lines.get(slot)?)?;
                let lines = self.lines.get(&master)?;
                lines
                    .iter()
                    .find(|known| known.edges.contains(&index))
                    .map(|known| Line {
                        fixed_u: line.fixed_u,
                        value: known.line.value,
                    })
            });
        let map = match (&surface, line) {
            (Surface::BSpline(spline), Some(line)) => {
                let first = curve.point(range.start());
                let seed = line.along(surface.project(first, None));
                let mut previous =
                    closest_on_line(spline, line, first, seed).ok_or_else(missing)?;
                let mut table = Vec::with_capacity(stations.len());
                for along in &stations {
                    previous = closest_on_line(spline, line, curve.point(*along), previous)
                        .ok_or_else(missing)?;
                    table.push((*along, previous));
                }
                let raw = [
                    table.first().map(|(_, t)| *t).ok_or_else(missing)?,
                    table.last().map(|(_, t)| *t).ok_or_else(missing)?,
                ];
                let wanted = [
                    closest_on_line(spline, line, ends[0], raw[0]).ok_or_else(missing)?,
                    closest_on_line(spline, line, ends[1], raw[1]).ok_or_else(missing)?,
                ];
                Map::Line {
                    spline: spline.clone(),
                    line,
                    table,
                    shift: [wanted[0] - raw[0], wanted[1] - raw[1]],
                }
            }
            (Surface::BSpline(spline), None) => {
                let first = curve.point(range.start());
                let mut previous = closest_on_patch(spline, first, surface.project(first, None))
                    .ok_or_else(missing)?;
                let mut table = Vec::with_capacity(stations.len());
                for along in &stations {
                    previous = closest_on_patch(spline, curve.point(*along), previous)
                        .ok_or_else(missing)?;
                    table.push((*along, previous));
                }
                let raw = [
                    table.first().map(|(_, uv)| *uv).ok_or_else(missing)?,
                    table.last().map(|(_, uv)| *uv).ok_or_else(missing)?,
                ];
                let wanted = [
                    closest_on_patch(spline, ends[0], raw[0]).ok_or_else(missing)?,
                    closest_on_patch(spline, ends[1], raw[1]).ok_or_else(missing)?,
                ];
                Map::Patch {
                    spline: spline.clone(),
                    table,
                    shift: [wanted[0] - raw[0], wanted[1] - raw[1]],
                }
            }
            (rigid, _) => {
                let mut hint = None;
                let mut table = Vec::with_capacity(stations.len());
                for along in &stations {
                    let uv = rigid.project(curve.point(*along), hint);
                    hint = Some(uv);
                    table.push((*along, uv));
                }
                let raw = [
                    table.first().map(|(_, uv)| *uv).ok_or_else(missing)?,
                    table.last().map(|(_, uv)| *uv).ok_or_else(missing)?,
                ];
                let wanted = [
                    rigid.project(ends[0], Some(raw[0])),
                    rigid.project(ends[1], Some(raw[1])),
                ];
                Map::Rigid {
                    surface: rigid.clone(),
                    table,
                    shift: [wanted[0] - raw[0], wanted[1] - raw[1]],
                }
            }
        };
        let mut strict = Strict {
            map,
            side: curve.clone(),
            range,
            settled: [Vector3::ZERO; 2],
        };
        let reached = [
            strict.on_master(range.start(), 0.0).ok_or_else(missing)?,
            strict.on_master(range.end(), 1.0).ok_or_else(missing)?,
        ];
        let settled = [ends[0] - reached[0], ends[1] - reached[1]];
        if settled.iter().any(|offset| offset.length() > CLEAN) {
            return Err(Problem::new(
                face,
                "meets a corner that lies off the face it is bent onto",
            ));
        }
        strict.settled = settled;
        Ok(strict)
    }
}

fn table_stations(curve: &BSpline<Point3>, range: Interval) -> Vec<f64> {
    let mut breaks: Vec<f64> = curve
        .breakpoints()
        .into_iter()
        .filter(|knot| *knot > range.start() && *knot < range.end())
        .chain([range.start(), range.end()])
        .collect();
    breaks.sort_by(f64::total_cmp);
    breaks.dedup();
    let per_span = TABLE_PER_SPAN.max(TABLE_MINIMUM / breaks.len().max(1));
    let mut stations = Vec::with_capacity(breaks.len() * per_span);
    for pair in breaks.windows(2) {
        let [start, end] = pair else {
            continue;
        };
        for step in 0..per_span {
            stations.push(start + (end - start) * step as f64 / per_span as f64);
        }
    }
    stations.push(range.end());
    stations
}

fn strict_samples(strict: &Strict) -> Vec<f64> {
    match &strict.map {
        Map::Rigid { table, .. } | Map::Patch { table, .. } => {
            table.iter().map(|(along, _)| *along).collect()
        }
        Map::Line { table, .. } => table.iter().map(|(along, _)| *along).collect(),
    }
}

fn snapped(curve: &BSpline<Point3>, along: f64) -> f64 {
    let breaks = curve.breakpoints();
    let after = breaks.partition_point(|knot| *knot <= along);
    let below = after
        .checked_sub(1)
        .and_then(|index| breaks.get(index))
        .copied();
    let above = breaks.get(after).copied();
    let (Some(below), Some(above)) = (below, above) else {
        return along;
    };
    let reach = PIN_SNAP * (above - below);
    if along - below <= reach {
        below
    } else if above - along <= reach {
        above
    } else {
        along
    }
}

fn off_line(surface: &Surface, line: Line, point: Point3) -> f64 {
    let Surface::BSpline(spline) = surface else {
        return 0.0;
    };
    let seed = line.along(surface.project(point, None));
    let domain = if line.fixed_u {
        spline.v_domain()
    } else {
        spline.u_domain()
    };
    closest_on_line(spline, line, point, seed)
        .map(|along| line.at(domain.clamp(along)))
        .map_or(f64::INFINITY, |uv| surface.point_at(uv).distance(point))
}

fn corner_slots(side: SurfaceSide) -> [usize; 2] {
    match side {
        SurfaceSide::VStart => [0, 1],
        SurfaceSide::VEnd => [2, 3],
        SurfaceSide::UStart => [0, 2],
        SurfaceSide::UEnd => [1, 3],
    }
}

fn restricted(
    face: u64,
    spline: &BSplineSurface,
    lines: &[FaceLine],
    bounds: &[(usize, Bound)],
) -> Read<(BSplineSurface, Vec<Fitting>)> {
    let (mut u, mut v) = (spline.u_domain(), spline.v_domain());
    let mut sides: Vec<Fitting> = Vec::with_capacity(bounds.len());
    for (index, bound) in bounds {
        let Some(line) = lines.get(*index) else {
            continue;
        };
        let domain = if line.line.fixed_u { &mut u } else { &mut v };
        let tolerance = LINE_SIDE * domain.length();
        let value = line.line.value;
        let narrowed = match bound {
            Bound::Lower if value <= domain.start() + tolerance => Some(*domain),
            Bound::Upper if value >= domain.end() - tolerance => Some(*domain),
            Bound::Lower => Interval::new(value, domain.end()),
            Bound::Upper => Interval::new(domain.start(), value),
        };
        *domain = narrowed.ok_or_else(|| {
            Problem::new(
                face,
                "has edges along lines of its surface that leave no room between",
            )
        })?;
        let side = match (line.line.fixed_u, bound) {
            (true, Bound::Lower) => SurfaceSide::UStart,
            (true, Bound::Upper) => SurfaceSide::UEnd,
            (false, Bound::Lower) => SurfaceSide::VStart,
            (false, Bound::Upper) => SurfaceSide::VEnd,
        };
        if sides.iter().any(|known| known.side == side) {
            return Err(Problem::new(
                face,
                "has two separate edges along lines that would both have to bound it on one side",
            ));
        }
        sides.push(Fitting { line: *index, side });
    }
    let piece = if u == spline.u_domain() && v == spline.v_domain() {
        spline.clone()
    } else {
        spline
            .restricted(u, v)
            .ok_or_else(|| Problem::new(face, "could not be cut down to its edges"))?
    };
    Ok((piece, sides))
}

fn bend_problem(face: u64, error: &BendError) -> Problem {
    match error {
        BendError::Cancelled => {
            Problem::new(face, "was not bent, because the import was cancelled")
        }
        other => Problem::new(
            face,
            format!("could not be bent to meet its neighbours ({other})"),
        ),
    }
}
