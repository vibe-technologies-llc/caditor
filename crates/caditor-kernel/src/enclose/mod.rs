#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Plane, Point2, Point3, Vector3};
use thiserror::Error;

use crate::{
    interrupt::{self, Interrupted},
    naming::{EdgeName, FaceName, FaceOrigin},
    sense::Sense,
    surface::{PlaneSurface, Surface},
    tolerance::LINEAR_RESOLUTION,
    topology::{
        BuildError, CoedgeId, EdgeId, FaceId, ShellId, Solid, SolidBuilder, VertexId,
        inside_polygon, signed_area,
    },
};

const SAMPLES_PER_EDGE: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bounds {
    Cavity,
    Material,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Enclosure {
    pub solid: Solid,
    pub bounds: Bounds,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum EnclosureError {
    #[error("no face is chosen")]
    NoFaces,
    #[error("face {0:?} does not exist")]
    UnknownFace(FaceId),
    #[error("the open edges of the chosen faces do not run in separate closed loops")]
    OpenBoundary { edges: Vec<EdgeId> },
    #[error("an opening of the chosen faces does not lie in one plane")]
    NotFlat { edges: Vec<EdgeId> },
    #[error("openings of the chosen faces in one plane wind so no flat face can close them")]
    Openings { edges: Vec<EdgeId> },
    #[error("the chosen faces and the faces closing them make no valid solid: {0}")]
    Build(BuildError),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

impl EnclosureError {
    fn built(error: BuildError) -> Self {
        match error.interrupted() {
            Some(interrupted) => Self::Cancelled(interrupted),
            None => Self::Build(error),
        }
    }
}

#[derive(Debug, Clone)]
struct Opening {
    coedges: Vec<(EdgeId, Sense)>,
    plane: Plane,
    points: Vec<Point3>,
}

#[derive(Debug, Clone)]
struct Cap {
    plane: Plane,
    loops: Vec<Vec<(EdgeId, Sense)>>,
    name: FaceName,
    origin: Option<FaceOrigin>,
}

pub fn enclose_faces(
    solid: &Solid,
    faces: &[FaceId],
    feature: u64,
) -> Result<Enclosure, EnclosureError> {
    let chosen: BTreeSet<FaceId> = faces.iter().copied().collect();
    if chosen.is_empty() {
        return Err(EnclosureError::NoFaces);
    }
    if let Some(unknown) = chosen.iter().find(|face| solid.face(**face).is_none()) {
        return Err(EnclosureError::UnknownFace(*unknown));
    }
    let open = open_coedges(solid, &chosen);
    let openings = chained(solid, &open)?
        .into_iter()
        .map(|chain| opening(solid, &chosen, chain))
        .collect::<Result<Vec<_>, _>>()?;
    let caps = caps(solid, &chosen, openings, feature)?;
    let outward = assemble(solid, &chosen, &caps, Sense::Same)?;
    if outward
        .inside_out()
        .map_err(|error| EnclosureError::built(BuildError::Invalid(error.into())))?
    {
        let inward = assemble(solid, &chosen, &caps, Sense::Reversed)?;
        return Ok(Enclosure {
            solid: inward.build().map_err(EnclosureError::built)?,
            bounds: Bounds::Cavity,
        });
    }
    Ok(Enclosure {
        solid: outward.build().map_err(EnclosureError::built)?,
        bounds: Bounds::Material,
    })
}

fn chosen_coedges<'a>(
    solid: &'a Solid,
    chosen: &'a BTreeSet<FaceId>,
) -> impl Iterator<Item = CoedgeId> + 'a {
    chosen
        .iter()
        .filter_map(|face| solid.face(*face))
        .flat_map(|face| face.loops().iter())
        .filter_map(|face_loop| solid.face_loop(*face_loop))
        .flat_map(|face_loop| face_loop.coedges().iter().copied())
}

fn open_coedges(solid: &Solid, chosen: &BTreeSet<FaceId>) -> Vec<CoedgeId> {
    chosen_coedges(solid, chosen)
        .filter(|coedge| {
            let Some(edge) = solid
                .coedge(*coedge)
                .and_then(|data| solid.edge(data.edge()))
            else {
                return false;
            };
            edge.coedges().iter().any(|other| {
                solid
                    .coedge_face(*other)
                    .is_some_and(|face| !chosen.contains(&face))
            })
        })
        .collect()
}

fn chained(solid: &Solid, open: &[CoedgeId]) -> Result<Vec<Vec<CoedgeId>>, EnclosureError> {
    let mut starting: BTreeMap<VertexId, Vec<CoedgeId>> = BTreeMap::new();
    for coedge in open {
        if let Some((start, _)) = solid.coedge_vertices(*coedge) {
            starting.entry(start).or_default().push(*coedge);
        }
    }
    let edges_of = |coedges: &[CoedgeId]| -> Vec<EdgeId> {
        coedges
            .iter()
            .filter_map(|coedge| solid.coedge(*coedge).map(|data| data.edge()))
            .collect()
    };
    if let Some(crowded) = starting.values().find(|coedges| coedges.len() > 1) {
        return Err(EnclosureError::OpenBoundary {
            edges: edges_of(crowded),
        });
    }
    let mut used = BTreeSet::new();
    let mut chains = Vec::new();
    for first in open {
        if used.contains(first) {
            continue;
        }
        let Some((start, mut at)) = solid.coedge_vertices(*first) else {
            continue;
        };
        used.insert(*first);
        let mut chain = vec![*first];
        while at != start {
            interrupt::check()?;
            let next = starting
                .get(&at)
                .and_then(|coedges| coedges.first())
                .copied()
                .filter(|next| !used.contains(next));
            let Some(next) = next else {
                return Err(EnclosureError::OpenBoundary {
                    edges: edges_of(&chain),
                });
            };
            used.insert(next);
            chain.push(next);
            at = solid.coedge_vertices(next).map_or(start, |(_, end)| end);
        }
        chains.push(chain);
    }
    Ok(chains)
}

fn opening(
    solid: &Solid,
    chosen: &BTreeSet<FaceId>,
    chain: Vec<CoedgeId>,
) -> Result<Opening, EnclosureError> {
    let coedges: Vec<(EdgeId, Sense)> = chain
        .iter()
        .rev()
        .filter_map(|coedge| solid.coedge(*coedge))
        .map(|coedge| (coedge.edge(), coedge.sense().reversed()))
        .collect();
    let edges: Vec<EdgeId> = coedges.iter().map(|(edge, _)| *edge).collect();
    let points = loop_points(solid, &coedges);
    let neighbour_planes = chain
        .iter()
        .filter_map(|coedge| neighbour_face(solid, chosen, *coedge))
        .filter_map(|face| match solid.face(face)?.surface() {
            Surface::Plane(plane) => Some(*plane.frame()),
            _ => None,
        });
    let plane = neighbour_planes
        .chain(fitted_plane(&points))
        .find(|plane| holds(plane, &points))
        .ok_or(EnclosureError::NotFlat { edges })?;
    Ok(Opening {
        coedges,
        plane,
        points,
    })
}

fn neighbour_face(solid: &Solid, chosen: &BTreeSet<FaceId>, coedge: CoedgeId) -> Option<FaceId> {
    let edge = solid.edge(solid.coedge(coedge)?.edge())?;
    edge.coedges()
        .iter()
        .filter_map(|other| solid.coedge_face(*other))
        .find(|face| !chosen.contains(face))
}

fn loop_points(solid: &Solid, coedges: &[(EdgeId, Sense)]) -> Vec<Point3> {
    let mut points = Vec::with_capacity(coedges.len() * SAMPLES_PER_EDGE);
    for (edge, sense) in coedges {
        let Some(edge) = solid.edge(*edge) else {
            continue;
        };
        let interval = edge.interval();
        let (from, to) = match sense {
            Sense::Same => (interval.start(), interval.end()),
            Sense::Reversed => (interval.end(), interval.start()),
        };
        for step in 0..SAMPLES_PER_EDGE {
            let fraction = step as f64 / SAMPLES_PER_EDGE as f64;
            points.push(edge.curve().point(from + (to - from) * fraction));
        }
    }
    points
}

fn fitted_plane(points: &[Point3]) -> Option<Plane> {
    let count = points.len();
    let centre = points.iter().fold(Vector3::ZERO, |sum, point| sum + *point) / count.max(1) as f64;
    let normal =
        points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .fold(Vector3::ZERO, |sum, (a, b)| {
                sum + Vector3::new(
                    (a.y - b.y) * (a.z + b.z),
                    (a.z - b.z) * (a.x + b.x),
                    (a.x - b.x) * (a.y + b.y),
                )
            });
    Plane::new(centre, normal)
}

fn holds(plane: &Plane, points: &[Point3]) -> bool {
    !points.is_empty()
        && points
            .iter()
            .all(|point| plane.signed_distance(*point).abs() <= LINEAR_RESOLUTION)
}

fn caps(
    solid: &Solid,
    chosen: &BTreeSet<FaceId>,
    openings: Vec<Opening>,
    feature: u64,
) -> Result<Vec<Cap>, EnclosureError> {
    let mut groups: Vec<Vec<Opening>> = Vec::new();
    for opening in openings {
        match groups.iter_mut().find(|group| {
            group
                .first()
                .is_some_and(|first| holds(&first.plane, &opening.points))
        }) {
            Some(group) => group.push(opening),
            None => groups.push(vec![opening]),
        }
    }
    let mut caps = Vec::new();
    for group in groups {
        caps.extend(group_caps(solid, chosen, group, feature)?);
    }
    Ok(caps)
}

struct Outline {
    opening: Opening,
    polygon: Vec<Point2>,
    area: f64,
}

fn group_caps(
    solid: &Solid,
    chosen: &BTreeSet<FaceId>,
    group: Vec<Opening>,
    feature: u64,
) -> Result<Vec<Cap>, EnclosureError> {
    let Some(frame) = group.first().map(|first| first.plane) else {
        return Ok(Vec::new());
    };
    let mut outlines: Vec<Outline> = group
        .into_iter()
        .map(|opening| {
            let polygon: Vec<Point2> = opening
                .points
                .iter()
                .map(|point| frame.to_local(*point))
                .collect();
            let area = signed_area(&polygon);
            Outline {
                opening,
                polygon,
                area,
            }
        })
        .collect();
    outlines.sort_by(|a, b| b.area.abs().total_cmp(&a.area.abs()));
    let mut placed: Vec<(usize, Option<usize>)> = Vec::new();
    let mut faces: Vec<(f64, Vec<usize>)> = Vec::new();
    for (index, outline) in outlines.iter().enumerate() {
        let probe = outline.polygon.first().copied().unwrap_or_default();
        let containers: Vec<usize> = placed
            .iter()
            .map(|(container, _)| *container)
            .filter(|container| {
                outlines
                    .get(*container)
                    .is_some_and(|other| inside_polygon(&other.polygon, probe))
            })
            .collect();
        let innermost = containers.last().copied();
        let owner = if containers.len() % 2 == 1 {
            innermost.and_then(|container| {
                placed
                    .iter()
                    .find(|(placed, _)| *placed == container)
                    .and_then(|(_, face)| *face)
            })
        } else {
            None
        };
        match owner {
            Some(face) => {
                let Some((sign, members)) = faces.get_mut(face) else {
                    continue;
                };
                if outline.area * *sign >= 0.0 {
                    return Err(EnclosureError::Openings {
                        edges: outline
                            .opening
                            .coedges
                            .iter()
                            .map(|(edge, _)| *edge)
                            .collect(),
                    });
                }
                members.push(index);
                placed.push((index, None));
            }
            None => {
                placed.push((index, Some(faces.len())));
                faces.push((outline.area.signum(), vec![index]));
            }
        }
    }
    let mut caps = Vec::with_capacity(faces.len());
    for (sign, members) in faces {
        let normal = frame.normal() * sign;
        let plane = Plane::new(frame.origin(), normal)
            .ok_or(EnclosureError::Openings { edges: Vec::new() })?;
        let loops: Vec<Vec<(EdgeId, Sense)>> = members
            .iter()
            .filter_map(|member| outlines.get(*member))
            .map(|outline| outline.opening.coedges.clone())
            .collect();
        let edge_names: Vec<EdgeName> = loops
            .iter()
            .flatten()
            .filter_map(|(edge, _)| solid.edge(*edge).map(|edge| edge.name()))
            .collect();
        let origin = loops
            .iter()
            .flatten()
            .find_map(|(edge, _)| {
                let edge = solid.edge(*edge)?;
                edge.coedges()
                    .iter()
                    .filter_map(|coedge| solid.coedge_face(*coedge))
                    .find(|face| !chosen.contains(face))
            })
            .and_then(|face| solid.face(face))
            .and_then(|face| face.origin());
        caps.push(Cap {
            plane,
            loops,
            name: FaceName::closure(feature, edge_names),
            origin,
        });
    }
    Ok(caps)
}

fn assemble(
    solid: &Solid,
    chosen: &BTreeSet<FaceId>,
    caps: &[Cap],
    sense: Sense,
) -> Result<SolidBuilder, EnclosureError> {
    let mut builder = SolidBuilder::new();
    let shell = builder.shell().map_err(EnclosureError::built)?;
    let mut vertices: BTreeMap<VertexId, VertexId> = BTreeMap::new();
    let mut edges: BTreeMap<EdgeId, EdgeId> = BTreeMap::new();
    let used_edges: BTreeSet<EdgeId> = chosen_coedges(solid, chosen)
        .filter_map(|coedge| solid.coedge(coedge).map(|data| data.edge()))
        .collect();
    for edge_id in used_edges {
        interrupt::check()?;
        let Some(edge) = solid.edge(edge_id) else {
            continue;
        };
        let mut copied = |vertex: VertexId| -> Result<VertexId, EnclosureError> {
            if let Some(known) = vertices.get(&vertex) {
                return Ok(*known);
            }
            let point = solid
                .vertex(vertex)
                .map(|vertex| vertex.point())
                .ok_or(EnclosureError::Build(BuildError::UnknownVertex(vertex)))?;
            let made = builder.vertex(point).map_err(EnclosureError::built)?;
            vertices.insert(vertex, made);
            Ok(made)
        };
        let start = copied(edge.start())?;
        let end = copied(edge.end())?;
        let made = builder
            .edge(edge.curve().clone(), edge.interval(), start, end)
            .map_err(EnclosureError::built)?;
        builder
            .set_edge_name(made, edge.name())
            .map_err(EnclosureError::built)?;
        edges.insert(edge_id, made);
    }
    for face_id in chosen {
        interrupt::check()?;
        copy_face(solid, *face_id, (shell, sense), &edges, &mut builder)?;
    }
    for cap in caps {
        interrupt::check()?;
        add_cap(cap, (shell, sense), &edges, &mut builder)?;
    }
    Ok(builder)
}

fn copy_face(
    solid: &Solid,
    face_id: FaceId,
    (shell, sense): (ShellId, Sense),
    edges: &BTreeMap<EdgeId, EdgeId>,
    builder: &mut SolidBuilder,
) -> Result<(), EnclosureError> {
    let face = solid
        .face(face_id)
        .ok_or(EnclosureError::UnknownFace(face_id))?;
    let made = builder
        .face(shell, face.surface().clone(), face.sense().combined(sense))
        .map_err(EnclosureError::built)?;
    builder
        .set_face_name(made, face.name())
        .map_err(EnclosureError::built)?;
    if let Some(origin) = face.origin() {
        builder
            .set_face_origin(made, origin)
            .map_err(EnclosureError::built)?;
    }
    for loop_id in face.loops() {
        let Some(face_loop) = solid.face_loop(*loop_id) else {
            continue;
        };
        let mut coedges = Vec::with_capacity(face_loop.coedges().len());
        for coedge in face_loop.coedges() {
            let Some(coedge) = solid.coedge(*coedge) else {
                continue;
            };
            let edge = edges
                .get(&coedge.edge())
                .copied()
                .ok_or(EnclosureError::Build(BuildError::UnknownEdge(
                    coedge.edge(),
                )))?;
            coedges.push(match sense {
                Sense::Same => (edge, coedge.sense(), coedge.pcurve().clone()),
                Sense::Reversed => (edge, coedge.sense().reversed(), coedge.pcurve().reversed()),
            });
        }
        if sense == Sense::Reversed {
            coedges.reverse();
        }
        let count = coedges.len();
        let made_loop = builder
            .add_loop_with_pcurves(made, coedges)
            .map_err(EnclosureError::built)?;
        if sense == Sense::Same {
            builder.settle(made_loop, &vec![true; count]);
        }
    }
    Ok(())
}

fn add_cap(
    cap: &Cap,
    (shell, sense): (ShellId, Sense),
    edges: &BTreeMap<EdgeId, EdgeId>,
    builder: &mut SolidBuilder,
) -> Result<(), EnclosureError> {
    let surface = PlaneSurface::new(cap.plane)
        .map_err(|error| EnclosureError::built(BuildError::Geometry(error)))?;
    let made = builder
        .face(shell, Surface::Plane(surface), sense)
        .map_err(EnclosureError::built)?;
    builder
        .set_face_name(made, cap.name)
        .map_err(EnclosureError::built)?;
    if let Some(origin) = cap.origin {
        builder
            .set_face_origin(made, origin)
            .map_err(EnclosureError::built)?;
    }
    for cap_loop in &cap.loops {
        let mut coedges = cap_loop
            .iter()
            .map(|(edge, edge_sense)| {
                let made = edges
                    .get(edge)
                    .copied()
                    .ok_or(EnclosureError::Build(BuildError::UnknownEdge(*edge)))?;
                Ok(match sense {
                    Sense::Same => (made, *edge_sense),
                    Sense::Reversed => (made, edge_sense.reversed()),
                })
            })
            .collect::<Result<Vec<_>, EnclosureError>>()?;
        if sense == Sense::Reversed {
            coedges.reverse();
        }
        builder
            .add_loop(made, &coedges)
            .map_err(EnclosureError::built)?;
    }
    Ok(())
}
