use std::collections::{BTreeMap, BTreeSet, VecDeque};

use caditor_geometry::{Point2, Vector2};
use thiserror::Error;

use crate::{
    sense::Sense,
    surface::Surface,
    tessellation::TessellationError,
    tolerance::{LINEAR_RESOLUTION, SamplingTolerance},
    topology::{CoedgeId, EdgeId, Face, FaceId, LoopId, ShellId, Solid, VertexId},
};

const EDGE_CHECK_SAMPLES: usize = 16;
const PARAMETER_MATCH: f64 = 1e-9;
const POLE_MATCH: f64 = 1e-9;
const VALIDATION_COARSENESS: [f64; 3] = [20.0, 1.0, 0.1];
const VOID_PROBES: usize = 8;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ValidationError {
    #[error("the solid has no shells")]
    NoShells,
    #[error("shell {0:?} has no faces")]
    EmptyShell(ShellId),
    #[error("face {0:?} has no loops")]
    FaceWithoutLoops(FaceId),
    #[error("loop {0:?} has no coedges")]
    EmptyLoop(LoopId),
    #[error("the solid refers to an entity it does not contain")]
    MissingEntity,
    #[error("vertex {0:?} is not used by any edge")]
    UnusedVertex(VertexId),
    #[error("edge {0:?} has no length")]
    ZeroLengthEdge(EdgeId),
    #[error("vertex {vertex:?} lies {distance} from the end of edge {edge:?}")]
    VertexOffEdge {
        edge: EdgeId,
        vertex: VertexId,
        distance: f64,
    },
    #[error("edge {edge:?} is used by {uses} coedges instead of two")]
    EdgeUseCount { edge: EdgeId, uses: usize },
    #[error("both coedges of edge {0:?} run the same way")]
    EdgeSenses(EdgeId),
    #[error("edge {0:?} joins faces of different shells")]
    EdgeAcrossShells(EdgeId),
    #[error("coedge {coedge:?} of loop {face_loop:?} does not start where the previous one ends")]
    LoopBreak { face_loop: LoopId, coedge: CoedgeId },
    #[error("the pcurve of coedge {0:?} does not span its edge")]
    PcurveEnds(CoedgeId),
    #[error("edge {edge:?} lies {distance} off the surface of face {face:?}")]
    EdgeOffSurface {
        edge: EdgeId,
        face: FaceId,
        distance: f64,
    },
    #[error("the pcurve of coedge {coedge:?} strays {distance} from its edge")]
    PcurveOffEdge { coedge: CoedgeId, distance: f64 },
    #[error("the pcurve of coedge {0:?} does not continue from the previous one")]
    PcurveGap(CoedgeId),
    #[error("loop {face_loop:?} winds the wrong way around its face (signed area {area})")]
    LoopOrientation { face_loop: LoopId, area: f64 },
    #[error("inner loop {0:?} lies outside the outer loop of its face")]
    InnerLoopOutside(LoopId),
    #[error("face {face:?} is not connected to the rest of shell {shell:?}")]
    ShellDisconnected { shell: ShellId, face: FaceId },
    #[error(
        "shell {shell:?} has Euler characteristic {characteristic}, which no closed surface has"
    )]
    EulerPoincare { shell: ShellId, characteristic: i64 },
    #[error("the solid could not be tessellated: {0}")]
    Tessellation(#[from] TessellationError),
    #[error("shell {0:?} encloses no volume")]
    EmptyVolume(ShellId),
    #[error("shell {0:?} faces inward but lies inside no outward shell")]
    VoidOutside(ShellId),
}

type Checked<T> = Result<T, ValidationError>;

pub(crate) fn validate(solid: &Solid) -> Checked<()> {
    structure(solid)?;
    vertices_used(solid)?;
    edges(solid)?;
    edge_uses(solid)?;
    loop_chains(solid)?;
    coedge_geometry(solid)?;
    face_domains(solid)?;
    shells(solid)?;
    volumes(solid)
}

fn face_of(solid: &Solid, id: FaceId) -> Checked<&Face> {
    solid.face(id).ok_or(ValidationError::MissingEntity)
}

fn structure(solid: &Solid) -> Checked<()> {
    if solid.shells().next().is_none() {
        return Err(ValidationError::NoShells);
    }
    for (id, shell) in solid.shells() {
        if shell.faces().is_empty() {
            return Err(ValidationError::EmptyShell(id));
        }
    }
    for (id, face) in solid.faces() {
        if face.loops().is_empty() {
            return Err(ValidationError::FaceWithoutLoops(id));
        }
        for loop_id in face.loops() {
            let face_loop = solid
                .face_loop(*loop_id)
                .ok_or(ValidationError::MissingEntity)?;
            if face_loop.coedges().is_empty() {
                return Err(ValidationError::EmptyLoop(*loop_id));
            }
            if face_loop
                .coedges()
                .iter()
                .any(|coedge| solid.coedge(*coedge).is_none())
            {
                return Err(ValidationError::MissingEntity);
            }
        }
    }
    Ok(())
}

fn vertices_used(solid: &Solid) -> Checked<()> {
    let used: BTreeSet<VertexId> = solid
        .edges()
        .flat_map(|(_, edge)| [edge.start(), edge.end()])
        .collect();
    match solid.vertices().find(|(id, _)| !used.contains(id)) {
        Some((id, _)) => Err(ValidationError::UnusedVertex(id)),
        None => Ok(()),
    }
}

fn edges(solid: &Solid) -> Checked<()> {
    for (id, edge) in solid.edges() {
        let length = edge.curve().length(edge.interval());
        if length.is_nan() || length <= LINEAR_RESOLUTION {
            return Err(ValidationError::ZeroLengthEdge(id));
        }
        for (vertex, parameter) in [
            (edge.start(), edge.interval().start()),
            (edge.end(), edge.interval().end()),
        ] {
            let point = solid
                .vertex(vertex)
                .ok_or(ValidationError::MissingEntity)?
                .point();
            let distance = edge.curve().point(parameter).distance(point);
            if distance.is_nan() || distance > LINEAR_RESOLUTION {
                return Err(ValidationError::VertexOffEdge {
                    edge: id,
                    vertex,
                    distance,
                });
            }
        }
    }
    Ok(())
}

fn edge_uses(solid: &Solid) -> Checked<()> {
    for (id, edge) in solid.edges() {
        let [first, second] = edge.coedges() else {
            return Err(ValidationError::EdgeUseCount {
                edge: id,
                uses: edge.coedges().len(),
            });
        };
        let (Some(first), Some(second)) = (solid.coedge(*first), solid.coedge(*second)) else {
            return Err(ValidationError::MissingEntity);
        };
        if first.sense() == second.sense() {
            return Err(ValidationError::EdgeSenses(id));
        }
        let shell_of = |owner: LoopId| -> Checked<ShellId> {
            let face = solid
                .face_loop(owner)
                .ok_or(ValidationError::MissingEntity)?
                .face();
            Ok(face_of(solid, face)?.shell())
        };
        if shell_of(first.owner())? != shell_of(second.owner())? {
            return Err(ValidationError::EdgeAcrossShells(id));
        }
    }
    Ok(())
}

fn loop_chains(solid: &Solid) -> Checked<()> {
    for (id, face_loop) in solid.loops() {
        let coedges = face_loop.coedges();
        for (index, coedge) in coedges.iter().enumerate() {
            let Some(next) = coedges.get((index + 1) % coedges.len()) else {
                continue;
            };
            let (_, end) = solid
                .coedge_vertices(*coedge)
                .ok_or(ValidationError::MissingEntity)?;
            let (start, _) = solid
                .coedge_vertices(*next)
                .ok_or(ValidationError::MissingEntity)?;
            if end != start {
                return Err(ValidationError::LoopBreak {
                    face_loop: id,
                    coedge: *next,
                });
            }
        }
    }
    Ok(())
}

fn same_parameter(a: f64, b: f64) -> bool {
    (a - b).abs() <= PARAMETER_MATCH * (1.0 + a.abs().max(b.abs()))
}

fn coedge_geometry(solid: &Solid) -> Checked<()> {
    for (id, coedge) in solid.coedges() {
        let edge = solid
            .edge(coedge.edge())
            .ok_or(ValidationError::MissingEntity)?;
        let face_id = solid
            .coedge_face(id)
            .ok_or(ValidationError::MissingEntity)?;
        let surface = face_of(solid, face_id)?.surface();
        let (start, end) = solid
            .coedge_parameters(id)
            .ok_or(ValidationError::MissingEntity)?;
        let pcurve = coedge.pcurve();
        if !same_parameter(pcurve.start_parameter(), start)
            || !same_parameter(pcurve.end_parameter(), end)
        {
            return Err(ValidationError::PcurveEnds(id));
        }
        let curve = edge.curve();
        for step in 0..=EDGE_CHECK_SAMPLES {
            let parameter = start + (end - start) * step as f64 / EDGE_CHECK_SAMPLES as f64;
            let point = curve.point(parameter);
            let uv = surface.project(point, Some(pcurve.uv_at(parameter)));
            let distance = surface.point_at(uv).distance(point);
            if distance.is_nan() || distance > LINEAR_RESOLUTION {
                return Err(ValidationError::EdgeOffSurface {
                    edge: coedge.edge(),
                    face: face_id,
                    distance,
                });
            }
        }
        let samples = pcurve.samples();
        let at_samples = samples.iter().map(|sample| {
            (
                surface
                    .point_at(sample.uv)
                    .distance(curve.point(sample.parameter)),
                LINEAR_RESOLUTION,
            )
        });
        let between_samples = samples.windows(2).filter_map(|pair| match pair {
            [a, b] => {
                let middle = surface.point_at(a.uv.lerp(b.uv, 0.5));
                let expected = curve.point(0.5 * (a.parameter + b.parameter));
                Some((
                    middle.distance(expected),
                    pcurve.tolerance() + LINEAR_RESOLUTION,
                ))
            }
            _ => None,
        });
        if let Some((distance, _)) = at_samples
            .chain(between_samples)
            .find(|(distance, allowed)| distance.is_nan() || distance > allowed)
        {
            return Err(ValidationError::PcurveOffEdge {
                coedge: id,
                distance,
            });
        }
    }
    Ok(())
}

pub(crate) fn continues(surface: &Surface, end: Point2, start: Point2) -> bool {
    let derivatives = surface.evaluate(end.x, end.y);
    let gap = start - end;
    let spatial = gap.x.abs() * derivatives.du.length() + gap.y.abs() * derivatives.dv.length();
    if spatial <= LINEAR_RESOLUTION && gap.abs().max_element() <= periodic_limit(surface) {
        return true;
    }
    let same_pole = matches!(
        (surface.pole_at(end), surface.pole_at(start)),
        (Some(a), Some(b)) if (a.v - b.v).abs() <= POLE_MATCH
    );
    same_pole && gap.y.abs() <= POLE_MATCH * (1.0 + end.y.abs())
}

fn periodic_limit(surface: &Surface) -> f64 {
    let periods = [surface.u_period(), surface.v_period()];
    0.5 * periods.into_iter().flatten().fold(f64::INFINITY, f64::min)
}

fn loop_polygon(solid: &Solid, loop_id: LoopId) -> Checked<Vec<Point2>> {
    let face_loop = solid
        .face_loop(loop_id)
        .ok_or(ValidationError::MissingEntity)?;
    let mut polygon = Vec::new();
    for coedge in face_loop.coedges() {
        let coedge = solid
            .coedge(*coedge)
            .ok_or(ValidationError::MissingEntity)?;
        polygon.extend(coedge.pcurve().samples().iter().map(|sample| sample.uv));
    }
    Ok(polygon)
}

pub(crate) fn signed_area(polygon: &[Point2]) -> f64 {
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

pub(crate) fn inside_polygon(polygon: &[Point2], point: Point2) -> bool {
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

fn face_domains(solid: &Solid) -> Checked<()> {
    for (_, face) in solid.faces() {
        let surface = face.surface();
        for (position, loop_id) in face.loops().iter().enumerate() {
            let face_loop = solid
                .face_loop(*loop_id)
                .ok_or(ValidationError::MissingEntity)?;
            let coedges = face_loop.coedges();
            for (index, coedge) in coedges.iter().enumerate() {
                let Some(next) = coedges.get((index + 1) % coedges.len()) else {
                    continue;
                };
                let (Some(current), Some(following)) = (solid.coedge(*coedge), solid.coedge(*next))
                else {
                    return Err(ValidationError::MissingEntity);
                };
                if !continues(surface, current.pcurve().end(), following.pcurve().start()) {
                    return Err(ValidationError::PcurveGap(*next));
                }
            }
            let area = signed_area(&loop_polygon(solid, *loop_id)?);
            let outer = if position == 0 {
                Sense::Same
            } else {
                Sense::Reversed
            };
            let expected = outer.combined(face.sense()).sign();
            if area.is_nan() || area * expected <= 0.0 {
                return Err(ValidationError::LoopOrientation {
                    face_loop: *loop_id,
                    area,
                });
            }
        }
        let Some(outer) = face.outer_loop() else {
            continue;
        };
        let outer = loop_polygon(solid, outer)?;
        for inner in face.inner_loops() {
            let probe = loop_polygon(solid, *inner)?
                .first()
                .copied()
                .unwrap_or(Point2::ZERO);
            let shifts = |period: Option<f64>| match period {
                Some(period) => vec![-period, 0.0, period],
                None => vec![0.0],
            };
            let inside = shifts(surface.u_period()).into_iter().any(|du| {
                shifts(surface.v_period())
                    .into_iter()
                    .any(|dv| inside_polygon(&outer, probe + Vector2::new(du, dv)))
            });
            if !inside {
                return Err(ValidationError::InnerLoopOutside(*inner));
            }
        }
    }
    Ok(())
}

fn face_edges(solid: &Solid, face: &Face) -> Checked<Vec<EdgeId>> {
    let mut edges = Vec::new();
    for loop_id in face.loops() {
        let face_loop = solid
            .face_loop(*loop_id)
            .ok_or(ValidationError::MissingEntity)?;
        for coedge in face_loop.coedges() {
            edges.push(
                solid
                    .coedge(*coedge)
                    .ok_or(ValidationError::MissingEntity)?
                    .edge(),
            );
        }
    }
    Ok(edges)
}

fn shells(solid: &Solid) -> Checked<()> {
    for (shell_id, shell) in solid.shells() {
        let faces = shell.faces();
        let mut reached: BTreeSet<FaceId> = BTreeSet::new();
        let mut pending: VecDeque<FaceId> = faces.first().copied().into_iter().collect();
        while let Some(face_id) = pending.pop_front() {
            if !reached.insert(face_id) {
                continue;
            }
            for edge in face_edges(solid, face_of(solid, face_id)?)? {
                let edge = solid.edge(edge).ok_or(ValidationError::MissingEntity)?;
                for coedge in edge.coedges() {
                    let neighbour = solid
                        .coedge_face(*coedge)
                        .ok_or(ValidationError::MissingEntity)?;
                    if !reached.contains(&neighbour) {
                        pending.push_back(neighbour);
                    }
                }
            }
        }
        if let Some(face) = faces.iter().find(|face| !reached.contains(face)) {
            return Err(ValidationError::ShellDisconnected {
                shell: shell_id,
                face: *face,
            });
        }
        let mut edges: BTreeSet<EdgeId> = BTreeSet::new();
        let mut rings = 0i64;
        for face_id in faces {
            let face = face_of(solid, *face_id)?;
            edges.extend(face_edges(solid, face)?);
            rings += face.loops().len() as i64 - 1;
        }
        let characteristic =
            vertex_fans(solid, faces)? - edges.len() as i64 + faces.len() as i64 - rings;
        if characteristic % 2 != 0 || characteristic > 2 {
            return Err(ValidationError::EulerPoincare {
                shell: shell_id,
                characteristic,
            });
        }
    }
    Ok(())
}

fn vertex_fans(solid: &Solid, faces: &[FaceId]) -> Checked<i64> {
    let mut leaving: Vec<CoedgeId> = Vec::new();
    let mut corner_after: BTreeMap<CoedgeId, usize> = BTreeMap::new();
    for face_id in faces {
        for loop_id in face_of(solid, *face_id)?.loops() {
            let coedges = solid
                .face_loop(*loop_id)
                .ok_or(ValidationError::MissingEntity)?
                .coedges();
            for (index, arriving) in coedges.iter().enumerate() {
                let next = coedges
                    .get((index + 1) % coedges.len())
                    .ok_or(ValidationError::MissingEntity)?;
                corner_after.insert(*arriving, leaving.len());
                leaving.push(*next);
            }
        }
    }
    let mut visited = vec![false; leaving.len()];
    let mut fans = 0;
    for start in 0..leaving.len() {
        let mut corner = start;
        if visited.get(corner).copied().unwrap_or(true) {
            continue;
        }
        fans += 1;
        while let Some(seen) = visited.get_mut(corner)
            && !*seen
        {
            *seen = true;
            let coedge = *leaving.get(corner).ok_or(ValidationError::MissingEntity)?;
            let edge = solid
                .coedge(coedge)
                .and_then(|coedge| solid.edge(coedge.edge()))
                .ok_or(ValidationError::MissingEntity)?;
            let mate = edge
                .coedges()
                .iter()
                .find(|other| **other != coedge)
                .ok_or(ValidationError::MissingEntity)?;
            corner = *corner_after
                .get(mate)
                .ok_or(ValidationError::MissingEntity)?;
        }
    }
    Ok(fans)
}

fn volumes(solid: &Solid) -> Checked<()> {
    let extent = solid.bounding_box().map_or(1.0, |bounds| bounds.diagonal());
    let mut result = Ok(());
    for coarseness in VALIDATION_COARSENESS {
        result = volumes_at(solid, &SamplingTolerance::for_extent(extent * coarseness));
        if !matches!(result, Err(ValidationError::VoidOutside(_))) {
            break;
        }
    }
    result
}

fn volumes_at(solid: &Solid, tolerance: &SamplingTolerance) -> Checked<()> {
    let mesh = solid.tessellate(tolerance)?;
    let mut outward = Vec::new();
    let mut inward = Vec::new();
    for (id, _) in solid.shells() {
        let in_shell = |face: FaceId| solid.face(face).is_some_and(|face| face.shell() == id);
        let volume = mesh.mass_properties_where(in_shell).volume;
        if !volume.is_finite() || volume == 0.0 {
            return Err(ValidationError::EmptyVolume(id));
        }
        if volume > 0.0 {
            outward.push(id);
        } else {
            inward.push(id);
        }
    }
    for void in inward {
        let probes: Vec<_> = mesh
            .faces()
            .iter()
            .filter(|face| {
                solid
                    .face(face.face)
                    .is_some_and(|face| face.shell() == void)
            })
            .flat_map(|face| {
                mesh.triangles()
                    .get(face.triangles.clone())
                    .unwrap_or_default()
            })
            .filter_map(|triangle| mesh.corner_points(*triangle))
            .map(|[a, b, c]| (a + b + c) / 3.0)
            .take(VOID_PROBES)
            .collect();
        let enclosed = probes.iter().find_map(|probe| {
            let answers: Vec<Option<bool>> = outward
                .iter()
                .map(|shell| {
                    mesh.contains(*probe, |face| {
                        solid.face(face).is_some_and(|face| face.shell() == *shell)
                    })
                })
                .collect();
            if answers.contains(&None) {
                None
            } else {
                Some(answers.contains(&Some(true)))
            }
        });
        if enclosed != Some(true) {
            return Err(ValidationError::VoidOutside(void));
        }
    }
    Ok(())
}
