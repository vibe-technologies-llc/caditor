#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use caditor_geometry::{Aabb, Plane, Point2, Point3, Vector3};
use thiserror::Error;

use crate::{
    interrupt::{self, Interrupted},
    sense::Sense,
    surface::{PlaneSurface, Surface},
    tolerance::LINEAR_RESOLUTION,
    topology::{BuildError, EdgeId, FaceId, Solid, SolidBuilder, VertexId},
};

pub const MAX_FACETED_FACES: usize = 20_000;
pub const MAX_FILLED_HOLE_EDGES: usize = 64;
const WELD_FRACTION: f64 = 1e-9;
const NOISE_FRACTION: f64 = 4e-7;
const FLAT_ANGLE_COSINE: f64 = 0.999_999_995;
const SNAP_WEIGHT: f64 = 1e-6;
const SNAP_RESIDUAL: f64 = 0.1 * LINEAR_RESOLUTION;
const COLLINEAR_FRACTION: f64 = 1e-9;
const MERGE_ATTEMPTS: usize = 8;
const VOID_VOLUME_FRACTION: f64 = 1e-12;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriangleMesh {
    pub positions: Vec<Point3>,
    pub triangles: Vec<[usize; 3]>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FaceMesh {
    pub mesh: TriangleMesh,
    pub faces: Vec<FaceId>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeshRepairs {
    pub welded: usize,
    pub degenerate: usize,
    pub duplicate: usize,
    pub flipped: usize,
    pub inverted_shells: usize,
    pub filled_holes: usize,
    pub open_shells: usize,
    pub tangled_shells: usize,
    pub sealed_voids: usize,
    pub kept_triangles: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FacetedSolids {
    pub solids: Vec<Solid>,
    pub repairs: MeshRepairs,
    pub faces: usize,
    pub sources: Vec<Vec<Vec<usize>>>,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum FacetedError {
    #[error("it holds no triangles")]
    NoTriangles,
    #[error(
        "no part of it is a closed surface ({open_edges} of its edges border only one \
         triangle), so it does not enclose a solid"
    )]
    NotClosed { open_edges: usize },
    #[error(
        "it has {faces} faces even after its flat areas are joined, more than the \
         {MAX_FACETED_FACES} caditor takes from one mesh; reduce its triangles in a mesh editor"
    )]
    TooDetailed { faces: usize },
    #[error("a solid could not be built from it: {0}")]
    Build(BuildError),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

pub fn faceted_solids(mesh: &TriangleMesh) -> Result<FacetedSolids, FacetedError> {
    let mut repairs = MeshRepairs::default();
    let (welded, welded_sources) = weld(mesh, &mut repairs)?;
    let (mut triangles, cleaned) = clean(&welded.positions, &welded.triangles, &mut repairs);
    if triangles.is_empty() {
        return Err(FacetedError::NoTriangles);
    }
    let mut sources: Vec<Option<usize>> = cleaned
        .into_iter()
        .map(|kept| welded_sources.get(kept).copied())
        .collect();
    repairs.filled_holes = fill_holes(&mut triangles);
    sources.resize(triangles.len(), None);
    let positions = welded.positions;
    let shells = shells(&positions, &triangles, &sources, &mut repairs)?;
    if shells.is_empty() {
        let open_edges = open_edge_count(&triangles);
        return Err(FacetedError::NotClosed { open_edges });
    }
    let noise = noise_of(&positions);
    let mut solids = Vec::with_capacity(shells.len());
    let mut face_sources = Vec::with_capacity(shells.len());
    let mut faces = 0usize;
    for shell in &shells {
        interrupt::check()?;
        let room = MAX_FACETED_FACES.saturating_sub(faces);
        let built = shell_solid(&positions, shell, noise, room)?;
        faces += built.faces;
        if built.kept_triangles {
            repairs.kept_triangles += 1;
        }
        solids.push(built.solid);
        face_sources.push(built.sources);
    }
    Ok(FacetedSolids {
        solids,
        repairs,
        faces,
        sources: face_sources,
    })
}

fn weld(
    mesh: &TriangleMesh,
    repairs: &mut MeshRepairs,
) -> Result<(TriangleMesh, Vec<usize>), FacetedError> {
    let finite: Vec<Point3> = mesh
        .positions
        .iter()
        .copied()
        .filter(|point| point.is_finite())
        .collect();
    let bounds = Aabb::from_points(finite).ok_or(FacetedError::NoTriangles)?;
    let grid = (bounds.diagonal() * WELD_FRACTION).max(f64::MIN_POSITIVE);
    let mut cells: BTreeMap<[i64; 3], usize> = BTreeMap::new();
    let mut positions = Vec::new();
    let mut renumbered: Vec<Option<usize>> = Vec::with_capacity(mesh.positions.len());
    for point in &mesh.positions {
        if !point.is_finite() {
            renumbered.push(None);
            continue;
        }
        let key = (*point / grid).round().to_array().map(|value| value as i64);
        let index = *cells.entry(key).or_insert_with(|| {
            positions.push(*point);
            positions.len() - 1
        });
        renumbered.push(Some(index));
    }
    repairs.welded = mesh.positions.len().saturating_sub(positions.len());
    let mut triangles = Vec::with_capacity(mesh.triangles.len());
    let mut sources = Vec::with_capacity(mesh.triangles.len());
    for (source, corners) in mesh.triangles.iter().enumerate() {
        let mapped = corners.map(|corner| renumbered.get(corner).copied().flatten());
        match mapped {
            [Some(a), Some(b), Some(c)] => {
                triangles.push([a, b, c]);
                sources.push(source);
            }
            _ => repairs.degenerate += 1,
        }
    }
    Ok((
        TriangleMesh {
            positions,
            triangles,
        },
        sources,
    ))
}

fn point(positions: &[Point3], index: usize) -> Point3 {
    positions.get(index).copied().unwrap_or(Point3::ZERO)
}

fn area_vector(positions: &[Point3], triangle: [usize; 3]) -> Vector3 {
    let [a, b, c] = triangle.map(|index| point(positions, index));
    (b - a).cross(c - a)
}

fn clean(
    positions: &[Point3],
    triangles: &[[usize; 3]],
    repairs: &mut MeshRepairs,
) -> (Vec<[usize; 3]>, Vec<usize>) {
    let scale =
        Aabb::from_points(positions.iter().copied()).map_or(0.0, |bounds| bounds.diagonal());
    let tiny = (scale * WELD_FRACTION).powi(2);
    let mut seen = BTreeSet::new();
    let mut kept = Vec::with_capacity(triangles.len());
    let mut kept_indices = Vec::with_capacity(triangles.len());
    for (index, triangle) in triangles.iter().enumerate() {
        let [a, b, c] = *triangle;
        if a == b || b == c || a == c || area_vector(positions, *triangle).length() <= tiny {
            repairs.degenerate += 1;
            continue;
        }
        let mut key = *triangle;
        key.sort_unstable();
        if !seen.insert(key) {
            repairs.duplicate += 1;
            continue;
        }
        kept.push(*triangle);
        kept_indices.push(index);
    }
    (kept, kept_indices)
}

fn half_edges(triangle: [usize; 3]) -> [(usize, usize); 3] {
    let [a, b, c] = triangle;
    [(a, b), (b, c), (c, a)]
}

fn undirected(edge: (usize, usize)) -> (usize, usize) {
    (edge.0.min(edge.1), edge.0.max(edge.1))
}

fn edge_users(triangles: &[[usize; 3]]) -> BTreeMap<(usize, usize), Vec<usize>> {
    let mut users: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (index, triangle) in triangles.iter().enumerate() {
        for edge in half_edges(*triangle) {
            users.entry(undirected(edge)).or_default().push(index);
        }
    }
    users
}

fn open_edge_count(triangles: &[[usize; 3]]) -> usize {
    edge_users(triangles)
        .values()
        .filter(|users| users.len() == 1)
        .count()
}

fn fill_holes(triangles: &mut Vec<[usize; 3]>) -> usize {
    let mut directed: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for triangle in triangles.iter() {
        for edge in half_edges(*triangle) {
            *directed.entry(edge).or_default() += 1;
        }
    }
    let mut needed: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for ((from, to), count) in &directed {
        let reverse = directed.get(&(*to, *from)).copied().unwrap_or(0);
        if *count == 1 && reverse == 0 {
            needed.entry(*to).or_default().push(*from);
        }
    }
    let mut used: BTreeSet<usize> = BTreeSet::new();
    let mut filled = 0;
    let starts: Vec<usize> = needed.keys().copied().collect();
    for start in starts {
        if used.contains(&start) {
            continue;
        }
        let mut hole = vec![start];
        let mut at = start;
        let closed = loop {
            let Some([next]) = needed.get(&at).map(Vec::as_slice) else {
                break false;
            };
            if *next == start {
                break true;
            }
            if hole.contains(next) || hole.len() > MAX_FILLED_HOLE_EDGES {
                break false;
            }
            hole.push(*next);
            at = *next;
        };
        if !closed || hole.len() < 3 {
            continue;
        }
        used.extend(hole.iter().copied());
        let Some((first, rest)) = hole.split_first() else {
            continue;
        };
        for pair in rest.windows(2) {
            if let [b, c] = pair {
                triangles.push([*first, *b, *c]);
            }
        }
        filled += 1;
    }
    filled
}

struct Shell {
    triangles: Vec<[usize; 3]>,
    sources: Vec<Option<usize>>,
}

fn signed_volume(positions: &[Point3], triangles: &[[usize; 3]]) -> f64 {
    triangles
        .iter()
        .map(|triangle| {
            let [a, b, c] = triangle.map(|index| point(positions, index));
            a.dot(b.cross(c)) / 6.0
        })
        .sum()
}

fn shells(
    positions: &[Point3],
    triangles: &[[usize; 3]],
    sources: &[Option<usize>],
    repairs: &mut MeshRepairs,
) -> Result<Vec<Shell>, FacetedError> {
    let users = edge_users(triangles);
    let mut component = vec![usize::MAX; triangles.len()];
    let mut oriented: Vec<[usize; 3]> = triangles.to_vec();
    let mut shells: Vec<(Vec<usize>, bool, bool)> = Vec::new();
    for seed in 0..triangles.len() {
        if component.get(seed) != Some(&usize::MAX) {
            continue;
        }
        interrupt::check()?;
        let id = shells.len();
        let mut members = vec![seed];
        let mut open = false;
        let mut tangled = false;
        if let Some(slot) = component.get_mut(seed) {
            *slot = id;
        }
        let mut queue = VecDeque::from([seed]);
        while let Some(current) = queue.pop_front() {
            let Some(triangle) = oriented.get(current).copied() else {
                continue;
            };
            for edge in half_edges(triangle) {
                let Some(sharing) = users.get(&undirected(edge)) else {
                    continue;
                };
                if sharing.len() != 2 {
                    open = true;
                }
                for &other in sharing.iter().filter(|other| **other != current) {
                    let Some(neighbour) = oriented.get(other).copied() else {
                        continue;
                    };
                    let agrees = half_edges(neighbour).contains(&(edge.1, edge.0));
                    let assigned = component.get(other).copied().unwrap_or(usize::MAX);
                    if assigned == usize::MAX {
                        if !agrees && let Some(slot) = oriented.get_mut(other) {
                            slot.swap(1, 2);
                            repairs.flipped += 1;
                        }
                        if let Some(slot) = component.get_mut(other) {
                            *slot = id;
                        }
                        members.push(other);
                        queue.push_back(other);
                    } else if !agrees && sharing.len() == 2 {
                        tangled = true;
                    }
                }
            }
        }
        shells.push((members, open, tangled));
    }
    let mut closed: Vec<(Shell, f64)> = Vec::new();
    for (members, open, tangled) in shells {
        if open {
            repairs.open_shells += 1;
            continue;
        }
        if tangled {
            repairs.tangled_shells += 1;
            continue;
        }
        let shell = Shell {
            triangles: members
                .iter()
                .filter_map(|index| oriented.get(*index).copied())
                .collect(),
            sources: members
                .iter()
                .map(|index| sources.get(*index).copied().flatten())
                .collect(),
        };
        let volume = signed_volume(positions, &shell.triangles);
        closed.push((shell, volume));
    }
    let largest = closed
        .iter()
        .map(|(_, volume)| volume.abs())
        .fold(0.0, f64::max);
    let boxes: Vec<Option<Aabb>> = closed
        .iter()
        .map(|(shell, _)| {
            Aabb::from_points(
                shell
                    .triangles
                    .iter()
                    .flatten()
                    .map(|index| point(positions, *index)),
            )
        })
        .collect();
    let mut kept = Vec::new();
    for (index, (shell, volume)) in closed.iter().enumerate() {
        if volume.abs() <= largest * VOID_VOLUME_FRACTION {
            repairs.tangled_shells += 1;
            continue;
        }
        let inside_another = boxes.get(index).copied().flatten().is_some_and(|inner| {
            boxes
                .iter()
                .zip(&closed)
                .enumerate()
                .any(|(other, (outer, (_, outer_volume)))| {
                    other != index
                        && outer_volume.abs() > volume.abs()
                        && outer.is_some_and(|outer| encloses(&outer, &inner))
                })
        });
        if *volume < 0.0 && inside_another {
            repairs.sealed_voids += 1;
            continue;
        }
        let triangles = if *volume < 0.0 {
            repairs.inverted_shells += 1;
            shell
                .triangles
                .iter()
                .map(|[a, b, c]| [*a, *c, *b])
                .collect()
        } else {
            shell.triangles.clone()
        };
        kept.push(Shell {
            triangles,
            sources: shell.sources.clone(),
        });
    }
    Ok(kept)
}

fn encloses(outer: &Aabb, inner: &Aabb) -> bool {
    outer.min().cmple(inner.min()).all() && inner.max().cmple(outer.max()).all()
}

fn noise_of(positions: &[Point3]) -> f64 {
    let magnitude = positions
        .iter()
        .map(|point| point.abs().max_element())
        .fold(0.0, f64::max);
    (magnitude * NOISE_FRACTION).max(LINEAR_RESOLUTION)
}

struct Built {
    solid: Solid,
    faces: usize,
    sources: Vec<Vec<usize>>,
    kept_triangles: bool,
}

#[derive(Clone)]
struct Region {
    triangles: Vec<usize>,
    normal: Vector3,
    offset: f64,
}

impl Region {
    fn merged(&self) -> bool {
        self.triangles.len() > 1
    }
}

struct Layout {
    region_of: Vec<usize>,
    regions: Vec<Region>,
}

fn shell_solid(
    positions: &[Point3],
    shell: &Shell,
    noise: f64,
    room: usize,
) -> Result<Built, FacetedError> {
    let triangles = &shell.triangles;
    let mut split: BTreeSet<usize> = BTreeSet::new();
    let everything: BTreeSet<usize> = (0..triangles.len()).collect();
    let mut attempt = 0;
    loop {
        interrupt::check()?;
        let keeping_triangles = attempt >= MERGE_ATTEMPTS;
        if keeping_triangles {
            split.clone_from(&everything);
        }
        let layout = layout(positions, triangles, &split, noise);
        if layout.regions.len() > room {
            return Err(FacetedError::TooDetailed {
                faces: layout.regions.len() + MAX_FACETED_FACES - room,
            });
        }
        let snapped = match snap(positions, triangles, &layout) {
            Ok(snapped) => snapped,
            Err(unsnapped) => {
                split.extend(unsnapped);
                attempt += 1;
                continue;
            }
        };
        match assemble(&snapped, triangles, &layout) {
            Ok(solid) => {
                let sources = layout
                    .regions
                    .iter()
                    .map(|region| {
                        region
                            .triangles
                            .iter()
                            .filter_map(|triangle| shell.sources.get(*triangle).copied().flatten())
                            .collect()
                    })
                    .collect();
                return Ok(Built {
                    solid,
                    faces: layout.regions.len(),
                    sources,
                    kept_triangles: keeping_triangles,
                });
            }
            Err(Assembly::Region(region)) if !keeping_triangles => {
                if let Some(region) = layout.regions.get(region) {
                    split.extend(region.triangles.iter().copied());
                }
                attempt += 1;
            }
            Err(Assembly::Build(_)) if !keeping_triangles => attempt = MERGE_ATTEMPTS,
            Err(Assembly::Region(_)) => {
                return Err(FacetedError::Build(BuildError::EmptyLoop));
            }
            Err(Assembly::Build(error)) => {
                return Err(match error.interrupted() {
                    Some(interrupted) => FacetedError::Cancelled(interrupted),
                    None => FacetedError::Build(error),
                });
            }
        }
    }
}

fn layout(
    positions: &[Point3],
    triangles: &[[usize; 3]],
    split: &BTreeSet<usize>,
    noise: f64,
) -> Layout {
    let users = edge_users(triangles);
    let normals: Vec<Vector3> = triangles
        .iter()
        .map(|triangle| area_vector(positions, *triangle).normalize_or_zero())
        .collect();
    let mut region_of = vec![usize::MAX; triangles.len()];
    let mut regions = Vec::new();
    for seed in 0..triangles.len() {
        if region_of.get(seed) != Some(&usize::MAX) {
            continue;
        }
        let id = regions.len();
        let normal = normals.get(seed).copied().unwrap_or(Vector3::Z);
        let anchor = triangles
            .get(seed)
            .map_or(Point3::ZERO, |triangle| point(positions, triangle[0]));
        let mut members = vec![seed];
        if let Some(slot) = region_of.get_mut(seed) {
            *slot = id;
        }
        if !split.contains(&seed) {
            let mut queue = VecDeque::from([seed]);
            while let Some(current) = queue.pop_front() {
                let Some(triangle) = triangles.get(current).copied() else {
                    continue;
                };
                for edge in half_edges(triangle) {
                    let Some(sharing) = users.get(&undirected(edge)) else {
                        continue;
                    };
                    for &other in sharing {
                        if region_of.get(other) != Some(&usize::MAX) || split.contains(&other) {
                            continue;
                        }
                        let flat = normals
                            .get(other)
                            .is_some_and(|other| other.dot(normal) >= FLAT_ANGLE_COSINE);
                        let on_plane = triangles.get(other).is_some_and(|corners| {
                            corners.iter().all(|corner| {
                                (point(positions, *corner) - anchor).dot(normal).abs() <= noise
                            })
                        });
                        if flat && on_plane {
                            if let Some(slot) = region_of.get_mut(other) {
                                *slot = id;
                            }
                            members.push(other);
                            queue.push_back(other);
                        }
                    }
                }
            }
        }
        let summed: Vector3 = members
            .iter()
            .filter_map(|index| triangles.get(*index))
            .map(|triangle| area_vector(positions, *triangle))
            .sum();
        let fitted = summed.normalize_or_zero();
        let corners: BTreeSet<usize> = members
            .iter()
            .filter_map(|index| triangles.get(*index))
            .flatten()
            .copied()
            .collect();
        let offset = corners
            .iter()
            .map(|corner| fitted.dot(point(positions, *corner)))
            .sum::<f64>()
            / corners.len().max(1) as f64;
        regions.push(Region {
            triangles: members,
            normal: fitted,
            offset,
        });
    }
    Layout { region_of, regions }
}

fn snap(
    positions: &[Point3],
    triangles: &[[usize; 3]],
    layout: &Layout,
) -> Result<Vec<Point3>, BTreeSet<usize>> {
    let mut planes_at: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (index, triangle) in triangles.iter().enumerate() {
        let region = layout.region_of.get(index).copied().unwrap_or(usize::MAX);
        if layout.regions.get(region).is_some_and(Region::merged) {
            for corner in triangle {
                planes_at.entry(*corner).or_default().insert(region);
            }
        }
    }
    let mut snapped = positions.to_vec();
    let mut failed = BTreeSet::new();
    for (corner, regions) in &planes_at {
        let original = point(positions, *corner);
        let planes: Vec<(Vector3, f64)> = regions
            .iter()
            .filter_map(|region| layout.regions.get(*region))
            .map(|region| (region.normal, region.offset))
            .collect();
        let moved = closest_on_planes(original, &planes);
        let fits = planes
            .iter()
            .all(|(normal, offset)| (normal.dot(moved) - offset).abs() <= SNAP_RESIDUAL);
        if fits && let Some(slot) = snapped.get_mut(*corner) {
            *slot = moved;
        } else {
            for region in regions {
                if let Some(region) = layout.regions.get(*region) {
                    failed.extend(region.triangles.iter().copied());
                }
            }
        }
    }
    if failed.is_empty() {
        Ok(snapped)
    } else {
        Err(failed)
    }
}

fn closest_on_planes(original: Point3, planes: &[(Vector3, f64)]) -> Point3 {
    let mut rows = [
        Vector3::X * SNAP_WEIGHT,
        Vector3::Y * SNAP_WEIGHT,
        Vector3::Z * SNAP_WEIGHT,
    ];
    let mut right = original * SNAP_WEIGHT;
    for (normal, offset) in planes {
        rows[0] += *normal * normal.x;
        rows[1] += *normal * normal.y;
        rows[2] += *normal * normal.z;
        right += *normal * *offset;
    }
    let [r0, r1, r2] = rows;
    let determinant = r0.dot(r1.cross(r2));
    if determinant.abs() <= f64::MIN_POSITIVE {
        return original;
    }
    let solved =
        (r1.cross(r2) * right.x + r2.cross(r0) * right.y + r0.cross(r1) * right.z) / determinant;
    if solved.is_finite() { solved } else { original }
}

enum Assembly {
    Region(usize),
    Build(BuildError),
}

struct Chain {
    vertices: Vec<usize>,
}

fn assemble(
    positions: &[Point3],
    triangles: &[[usize; 3]],
    layout: &Layout,
) -> Result<Solid, Assembly> {
    let mut owner: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for (index, triangle) in triangles.iter().enumerate() {
        for edge in half_edges(*triangle) {
            owner.insert(edge, index);
        }
    }
    let region_at = |triangle: usize| {
        layout
            .region_of
            .get(triangle)
            .copied()
            .unwrap_or(usize::MAX)
    };
    let mut boundary: Vec<BTreeMap<usize, Vec<usize>>> =
        vec![BTreeMap::new(); layout.regions.len()];
    let mut touching: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    let mut degree: BTreeMap<usize, usize> = BTreeMap::new();
    for (index, triangle) in triangles.iter().enumerate() {
        let region = region_at(index);
        for (from, to) in half_edges(*triangle) {
            let twin = owner.get(&(to, from)).copied().map(region_at);
            if twin == Some(region) {
                continue;
            }
            if let Some(map) = boundary.get_mut(region) {
                map.entry(from).or_default().push(to);
            }
            touching.entry(from).or_default().insert(region);
            *degree.entry(from).or_default() += 1;
        }
    }
    let scale =
        Aabb::from_points(positions.iter().copied()).map_or(1.0, |bounds| bounds.diagonal());
    let mut incoming: Vec<BTreeMap<usize, Vec<usize>>> = vec![BTreeMap::new(); boundary.len()];
    for (region, map) in boundary.iter().enumerate() {
        for (from, targets) in map {
            for to in targets {
                if let Some(slot) = incoming.get_mut(region) {
                    slot.entry(*to).or_default().push(*from);
                }
            }
        }
    }
    let removable = |vertex: usize| -> bool {
        let Some(regions) = touching.get(&vertex) else {
            return false;
        };
        if regions.len() != 2 || degree.get(&vertex) != Some(&2) {
            return false;
        }
        if !regions
            .iter()
            .all(|region| layout.regions.get(*region).is_some_and(Region::merged))
        {
            return false;
        }
        let Some(first) = regions.first() else {
            return false;
        };
        let next = boundary.get(*first).and_then(|map| map.get(&vertex));
        let previous = incoming.get(*first).and_then(|map| map.get(&vertex));
        let (Some([next]), Some([previous])) =
            (next.map(Vec::as_slice), previous.map(Vec::as_slice))
        else {
            return false;
        };
        let (a, v, b) = (
            point(positions, *previous),
            point(positions, vertex),
            point(positions, *next),
        );
        let along = (b - a).normalize_or_zero();
        let offset = v - a;
        offset.cross(along).length() <= COLLINEAR_FRACTION * scale
            && offset.dot(along) > 0.0
            && (b - v).dot(along) > 0.0
    };
    let removed: BTreeSet<usize> = touching
        .keys()
        .copied()
        .filter(|vertex| removable(*vertex))
        .collect();
    let mut builder = SolidBuilder::new();
    let mut vertices: BTreeMap<usize, VertexId> = BTreeMap::new();
    let mut edges: BTreeMap<Vec<usize>, EdgeId> = BTreeMap::new();
    let shell = builder.shell().map_err(Assembly::Build)?;
    for (index, region) in layout.regions.iter().enumerate() {
        let Some(map) = boundary.get(index) else {
            continue;
        };
        if map.values().any(|targets| targets.len() != 1) {
            return Err(Assembly::Region(index));
        }
        let loops = region_loops(map, &removed).ok_or(Assembly::Region(index))?;
        let plane = region_plane(positions, triangles, region).ok_or(Assembly::Region(index))?;
        let area = |chains: &[Chain]| -> f64 {
            let points: Vec<Point2> = chains
                .iter()
                .flat_map(|chain| {
                    chain
                        .vertices
                        .iter()
                        .take(chain.vertices.len().saturating_sub(1))
                })
                .map(|vertex| plane.to_local(point(positions, *vertex)))
                .collect();
            crate::topology::signed_area(&points)
        };
        let mut ordered: Vec<(f64, Vec<Chain>)> = loops
            .into_iter()
            .map(|chains| (area(&chains), chains))
            .collect();
        ordered.sort_by(|a, b| b.0.total_cmp(&a.0));
        let outer = ordered.iter().filter(|(area, _)| *area > 0.0).count();
        if outer != 1 {
            return Err(Assembly::Region(index));
        }
        let surface = PlaneSurface::new(plane)
            .map(Surface::from)
            .map_err(|error| Assembly::Build(BuildError::Geometry(error)))?;
        let face = builder
            .face(shell, surface, Sense::Same)
            .map_err(Assembly::Build)?;
        for (_, chains) in ordered {
            let mut coedges = Vec::with_capacity(chains.len());
            for chain in chains {
                let reversed: Vec<usize> = chain.vertices.iter().rev().copied().collect();
                let forward = chain.vertices.as_slice() <= reversed.as_slice();
                let key = if forward {
                    chain.vertices.clone()
                } else {
                    reversed
                };
                let (Some(start), Some(end)) = (key.first().copied(), key.last().copied()) else {
                    return Err(Assembly::Region(index));
                };
                let edge = match edges.get(&key) {
                    Some(found) => *found,
                    None => {
                        let mut corner = |vertex: usize,
                                          builder: &mut SolidBuilder|
                         -> Result<VertexId, Assembly> {
                            if let Some(existing) = vertices.get(&vertex) {
                                return Ok(*existing);
                            }
                            let id = builder
                                .vertex(point(positions, vertex))
                                .map_err(Assembly::Build)?;
                            vertices.insert(vertex, id);
                            Ok(id)
                        };
                        let from = corner(start, &mut builder)?;
                        let to = corner(end, &mut builder)?;
                        let edge = builder.line_edge(from, to).map_err(Assembly::Build)?;
                        edges.insert(key, edge);
                        edge
                    }
                };
                let sense = if forward {
                    Sense::Same
                } else {
                    Sense::Reversed
                };
                coedges.push((edge, sense));
            }
            builder.add_loop(face, &coedges).map_err(Assembly::Build)?;
        }
    }
    builder.build().map_err(Assembly::Build)
}

fn region_plane(positions: &[Point3], triangles: &[[usize; 3]], region: &Region) -> Option<Plane> {
    if region.merged() {
        return Plane::new(region.normal * region.offset, region.normal);
    }
    let triangle = *triangles.get(*region.triangles.first()?)?;
    let normal = area_vector(positions, triangle).try_normalize()?;
    Plane::new(point(positions, triangle[0]), normal)
}

fn region_loops(
    map: &BTreeMap<usize, Vec<usize>>,
    removed: &BTreeSet<usize>,
) -> Option<Vec<Vec<Chain>>> {
    let next = |vertex: usize| {
        map.get(&vertex)
            .and_then(|targets| targets.first().copied())
    };
    let mut unused: BTreeSet<usize> = map.keys().copied().collect();
    let mut loops = Vec::new();
    while let Some(&start) = unused.iter().find(|vertex| !removed.contains(vertex)) {
        let mut chains = Vec::new();
        let mut chain = vec![start];
        let mut at = start;
        loop {
            unused.remove(&at);
            let to = next(at)?;
            chain.push(to);
            if !removed.contains(&to) {
                chains.push(Chain {
                    vertices: std::mem::replace(&mut chain, vec![to]),
                });
            }
            if to == start {
                break;
            }
            if !unused.contains(&to) {
                return None;
            }
            at = to;
        }
        loops.push(chains);
    }
    if unused.is_empty() { Some(loops) } else { None }
}
