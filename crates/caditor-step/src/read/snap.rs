use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{LINEAR_RESOLUTION, PlaneSurface, Surface};

use crate::read::conform::LooseBody;

const CLEAN: f64 = 0.25 * LINEAR_RESOLUTION;
const EDGE_SAMPLES: usize = 16;
const PARALLEL_ANGLE: f64 = 1e-2;
const MOST_FACES_AT_A_CORNER: usize = 64;

#[derive(Debug, Clone, Default)]
pub(crate) struct Snapped {
    pub surfaces: BTreeMap<u64, Surface>,
    pub farthest: f64,
}

#[derive(Default)]
struct Meeting {
    samples: Vec<Point3>,
    along_edge: bool,
}

struct Snapper<'b, 'a> {
    body: &'b LooseBody<'a>,
    extents: BTreeMap<u64, [Point3; 4]>,
    meetings: BTreeMap<(u64, u64), Meeting>,
}

pub(crate) fn snap(body: &LooseBody<'_>) -> Snapped {
    let snapper = Snapper::of(body);

    let tangent = snapper.tangent_planes();
    let mut planes: BTreeMap<u64, Plane> = snapper.planes().collect();
    planes.extend(tangent.iter().map(|(face, (plane, _))| (*face, *plane)));
    let mut moves: BTreeMap<u64, (Plane, f64)> = tangent;
    moves.extend(snapper.coplanar_planes(&planes, &moves));

    let farthest = moves.values().map(|(_, moved)| *moved).fold(0.0, f64::max);
    let surfaces = moves
        .into_iter()
        .filter_map(|(face, (plane, _))| {
            PlaneSurface::new(plane)
                .ok()
                .map(|surface| (face, Surface::Plane(surface)))
        })
        .collect();
    Snapped { surfaces, farthest }
}

impl<'b, 'a> Snapper<'b, 'a> {
    fn of(body: &'b LooseBody<'a>) -> Self {
        let mut outlines: BTreeMap<u64, Vec<Point3>> = BTreeMap::new();
        let mut meetings: BTreeMap<(u64, u64), Meeting> = BTreeMap::new();
        for edge in &body.edges {
            let samples: Vec<Point3> = edge
                .interval
                .split(EDGE_SAMPLES)
                .map(|parameter| edge.curve.point(parameter))
                .collect();
            for face in &edge.faces {
                outlines
                    .entry(*face)
                    .or_default()
                    .extend(samples.iter().copied());
            }
            if edge.faces.len() > MOST_FACES_AT_A_CORNER {
                continue;
            }
            for pair in pairs(&edge.faces) {
                let meeting = meetings.entry(pair).or_default();
                meeting.samples.extend(samples.iter().copied());
                meeting.along_edge = true;
            }
        }
        for (vertex, point) in &body.vertices {
            let Some(faces) = body.vertex_faces.get(vertex) else {
                continue;
            };
            if faces.len() > MOST_FACES_AT_A_CORNER {
                continue;
            }
            let faces: Vec<u64> = faces.iter().copied().collect();
            for pair in pairs(&faces) {
                meetings.entry(pair).or_default().samples.push(*point);
            }
        }
        let extents = outlines
            .into_iter()
            .filter_map(|(face, outline)| {
                let plane = body.faces.get(&face).and_then(plane_of)?;
                Some((face, extent(&plane, &outline)?))
            })
            .collect();
        Self {
            body,
            extents,
            meetings,
        }
    }

    fn planes(&self) -> impl Iterator<Item = (u64, Plane)> + '_ {
        self.body
            .faces
            .iter()
            .filter_map(|(face, surface)| plane_of(surface).map(|plane| (*face, plane)))
    }

    fn tangent_planes(&self) -> BTreeMap<u64, (Plane, f64)> {
        let mut snapped = BTreeMap::new();
        for ((a, b), meeting) in &self.meetings {
            if !meeting.along_edge {
                continue;
            }
            for (face, partner) in [(*a, *b), (*b, *a)] {
                if snapped.contains_key(&face) {
                    continue;
                }
                let (Some(plane), Some(master)) = (
                    self.body.faces.get(&face).and_then(plane_of),
                    self.body
                        .faces
                        .get(&partner)
                        .filter(|surface| touches_planes_along_curves(surface)),
                ) else {
                    continue;
                };
                let Some(tangent) = tangent_plane(&plane, master, &meeting.samples) else {
                    continue;
                };
                if let Some(moved) = self.move_of(face, &tangent) {
                    snapped.insert(face, (tangent, moved));
                }
            }
        }
        snapped
    }

    fn coplanar_planes(
        &self,
        planes: &BTreeMap<u64, Plane>,
        fixed: &BTreeMap<u64, (Plane, f64)>,
    ) -> BTreeMap<u64, (Plane, f64)> {
        let mut groups = Groups::default();
        let mut misfits: BTreeMap<u64, f64> = BTreeMap::new();
        for ((a, b), meeting) in &self.meetings {
            let (Some(first), Some(second)) = (planes.get(a), planes.get(b)) else {
                continue;
            };
            if first.normal().dot(second.normal()).abs() < PARALLEL_ANGLE.cos() {
                continue;
            }
            let gap = meeting
                .samples
                .iter()
                .map(|point| {
                    first
                        .signed_distance(*point)
                        .abs()
                        .max(second.signed_distance(*point).abs())
                })
                .fold(0.0, f64::max);
            let movable = self.move_of(*a, second).is_some() || self.move_of(*b, first).is_some();
            if gap > CLEAN && movable {
                groups.join(*a, *b);
                for (face, plane) in [(*a, first), (*b, second)] {
                    let misfit = misfits.entry(face).or_default();
                    *misfit = meeting
                        .samples
                        .iter()
                        .map(|point| plane.signed_distance(*point).abs())
                        .fold(*misfit, f64::max);
                }
            }
        }
        let mut snapped = BTreeMap::new();
        for members in groups.all() {
            let Some(master) = master_of(&members, &misfits, fixed) else {
                continue;
            };
            let Some(onto) = planes.get(&master) else {
                continue;
            };
            for member in members
                .iter()
                .filter(|member| **member != master && !fixed.contains_key(member))
            {
                let Some(plane) = planes.get(member) else {
                    continue;
                };
                let Some(coplanar) = reframed(plane, onto.origin(), onto.normal()) else {
                    continue;
                };
                if let Some(moved) = self.move_of(*member, &coplanar) {
                    snapped.insert(*member, (coplanar, moved));
                }
            }
        }
        snapped
    }

    fn move_of(&self, face: u64, to: &Plane) -> Option<f64> {
        let moved = self
            .extents
            .get(&face)?
            .iter()
            .map(|corner| to.signed_distance(*corner).abs())
            .fold(0.0, f64::max);
        (moved > CLEAN && moved <= self.body.allowance).then_some(moved)
    }
}

fn master_of(
    members: &BTreeSet<u64>,
    misfits: &BTreeMap<u64, f64>,
    fixed: &BTreeMap<u64, (Plane, f64)>,
) -> Option<u64> {
    if let Some(fixed) = members.iter().find(|member| fixed.contains_key(member)) {
        return Some(*fixed);
    }
    members
        .iter()
        .map(|member| {
            (
                misfits.get(member).copied().unwrap_or(f64::INFINITY),
                *member,
            )
        })
        .min_by(|(first, _), (second, _)| first.total_cmp(second))
        .map(|(_, member)| member)
}

fn pairs(faces: &[u64]) -> impl Iterator<Item = (u64, u64)> + '_ {
    faces.iter().enumerate().flat_map(move |(index, first)| {
        faces
            .iter()
            .skip(index + 1)
            .filter(move |second| *second != first)
            .map(move |second| (*first.min(second), *first.max(second)))
    })
}

fn extent(plane: &Plane, outline: &[Point3]) -> Option<[Point3; 4]> {
    let mut local = outline.iter().map(|point| plane.to_local(*point));
    let first = local.next()?;
    let (low, high) = local.fold((first, first), |(low, high), uv| {
        (low.min(uv), high.max(uv))
    });
    Some(
        [
            Point2::new(low.x, low.y),
            Point2::new(high.x, low.y),
            Point2::new(high.x, high.y),
            Point2::new(low.x, high.y),
        ]
        .map(|corner| plane.to_world(corner)),
    )
}

fn plane_of(surface: &Surface) -> Option<Plane> {
    match surface {
        Surface::Plane(plane) => Some(*plane.frame()),
        _ => None,
    }
}

fn touches_planes_along_curves(surface: &Surface) -> bool {
    matches!(
        surface,
        Surface::Cylinder(_) | Surface::Cone(_) | Surface::Torus(_)
    )
}

fn tangent_plane(plane: &Plane, master: &Surface, shared: &[Point3]) -> Option<Plane> {
    let middle = shared.get(shared.len() / 2)?;
    let touch = master.project(*middle, None);
    if master.pole_at(touch).is_some() {
        return None;
    }
    let normal = master.normal(touch.x, touch.y)?;
    if normal.dot(plane.normal()).abs() < PARALLEL_ANGLE.cos() {
        return None;
    }
    let tangent = reframed(plane, master.point_at(touch), normal)?;
    let mut hint = None;
    let mut gap: f64 = 0.0;
    for point in shared {
        let foot = master.project(*point, hint);
        hint = Some(foot);
        let on_master = master.point_at(foot);
        if tangent.signed_distance(on_master).abs() > CLEAN {
            return None;
        }
        gap = gap.max(plane.signed_distance(on_master).abs());
    }
    (gap > CLEAN).then_some(tangent)
}

fn reframed(plane: &Plane, through: Point3, normal: Vector3) -> Option<Plane> {
    let facing = if normal.dot(plane.normal()) < 0.0 {
        -normal
    } else {
        normal
    };
    let origin = plane.origin() - facing * (plane.origin() - through).dot(facing);
    Plane::with_x_axis(origin, facing, plane.x_axis())
}

#[derive(Default)]
struct Groups {
    parent: BTreeMap<u64, u64>,
    size: BTreeMap<u64, usize>,
}

impl Groups {
    fn root(&self, mut face: u64) -> u64 {
        while let Some(parent) = self.parent.get(&face).filter(|parent| **parent != face) {
            face = *parent;
        }
        face
    }

    fn join(&mut self, first: u64, second: u64) {
        let (first, second) = (self.root(first), self.root(second));
        if first == second {
            return;
        }
        let size = |face: u64| self.size.get(&face).copied().unwrap_or(1);
        let (larger, smaller) = if size(first) >= size(second) {
            (first, second)
        } else {
            (second, first)
        };
        let joined = size(larger) + size(smaller);
        self.parent.entry(larger).or_insert(larger);
        self.parent.insert(smaller, larger);
        self.size.insert(larger, joined);
    }

    fn all(&self) -> Vec<BTreeSet<u64>> {
        let mut groups: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
        for face in self.parent.keys() {
            groups.entry(self.root(*face)).or_default().insert(*face);
        }
        groups.into_values().collect()
    }
}
