use std::{
    cmp::{Ordering, Reverse},
    collections::{BTreeMap, BTreeSet},
};

use caditor_geometry::{Plane, Point2, Point3, RigidTransform, Rotation3, Vector3};
use caditor_kernel::{Circle, Curve, LINEAR_RESOLUTION, Line, PlaneSurface, Surface};

use crate::read::{
    conform::LooseBody,
    offset::{Fit, OffsetRefusal, offset},
};

const CLEAN: f64 = 0.25 * LINEAR_RESOLUTION;
const EDGE_SAMPLES: usize = 16;
const PARALLEL_ANGLE: f64 = 1e-2;
const MOST_FACES_AT_A_CORNER: usize = 64;
const MOST_OUTLINE_SAMPLES: usize = 256;

#[derive(Debug, Clone, Default)]
pub(crate) struct Snapped {
    pub surfaces: BTreeMap<u64, Surface>,
    pub flat: usize,
    pub curved: usize,
    pub farthest: f64,
}

#[derive(Default)]
struct Meeting {
    samples: Vec<Point3>,
    along_edge: bool,
}

struct Snapper<'b, 'a> {
    body: &'b LooseBody<'a>,
    faces: BTreeMap<u64, Surface>,
    round_outlines: BTreeMap<u64, Vec<Point3>>,
    extents: BTreeMap<u64, [Point3; 4]>,
    meetings: BTreeMap<(u64, u64), Meeting>,
}

enum Centre {
    Axis(Point3, Vector3),
    Point(Point3),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Standing {
    tangent: usize,
    neighbours: usize,
}

struct RoundMove {
    surface: Surface,
    moved: f64,
    kept: f64,
}

impl RoundMove {
    fn fits_written_edges(&self, other: &Self) -> Ordering {
        if (self.kept - other.kept).abs() <= CLEAN {
            Ordering::Equal
        } else {
            self.kept.total_cmp(&other.kept)
        }
    }
}

struct RoundPair<'m> {
    first: u64,
    second: u64,
    meeting: &'m Meeting,
}

pub(crate) fn snap(body: &LooseBody<'_>) -> Snapped {
    let mut snapper = Snapper::of(body);

    let rounds = snapper.round_faces();
    snapper.faces.extend(
        rounds
            .iter()
            .map(|(face, (surface, _))| (*face, surface.clone())),
    );
    let tangent = snapper.tangent_planes();
    let mut planes: BTreeMap<u64, Plane> = snapper.planes().collect();
    planes.extend(tangent.iter().map(|(face, (plane, _))| (*face, *plane)));
    let mut moves: BTreeMap<u64, (Plane, f64)> = tangent;
    moves.extend(snapper.coplanar_planes(&planes, &moves));

    let farthest = moves
        .values()
        .map(|(_, moved)| *moved)
        .chain(rounds.values().map(|(_, moved)| *moved))
        .fold(0.0, f64::max);
    let curved = rounds.len();
    let mut surfaces: BTreeMap<u64, Surface> = moves
        .into_iter()
        .filter_map(|(face, (plane, _))| {
            PlaneSurface::new(plane)
                .ok()
                .map(|surface| (face, Surface::Plane(surface)))
        })
        .collect();
    let flat = surfaces.len();
    surfaces.extend(
        rounds
            .into_iter()
            .map(|(face, (surface, _))| (face, surface)),
    );
    Snapped {
        surfaces,
        flat,
        curved,
        farthest,
    }
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
            .iter()
            .filter_map(|(face, outline)| {
                let plane = body.faces.get(face).and_then(plane_of)?;
                Some((*face, extent(&plane, outline)?))
            })
            .collect();
        let round_outlines = outlines
            .iter()
            .filter(|(face, _)| body.faces.get(face).is_some_and(is_round))
            .map(|(face, outline)| (*face, thinned(outline)))
            .collect();
        Self {
            body,
            faces: body.faces.clone(),
            round_outlines,
            extents,
            meetings,
        }
    }

    fn planes(&self) -> impl Iterator<Item = (u64, Plane)> + '_ {
        self.faces
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
                    self.faces.get(&face).and_then(plane_of),
                    self.faces
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

    fn round_faces(&self) -> BTreeMap<u64, (Surface, f64)> {
        let mut standing: BTreeMap<u64, Standing> = BTreeMap::new();
        let mut pairs = Vec::new();
        for ((first, second), meeting) in &self.meetings {
            if !meeting.along_edge {
                continue;
            }
            let (Some(first_surface), Some(second_surface)) =
                (self.faces.get(first), self.faces.get(second))
            else {
                continue;
            };
            let tangent = is_rigid(first_surface)
                && is_rigid(second_surface)
                && meeting
                    .samples
                    .get(meeting.samples.len() / 2)
                    .is_some_and(|middle| tangent_at(first_surface, second_surface, *middle));
            for face in [*first, *second] {
                let face = standing.entry(face).or_default();
                face.neighbours += 1;
                face.tangent += usize::from(tangent);
            }
            if tangent && is_round(first_surface) && is_round(second_surface) {
                pairs.push(RoundPair {
                    first: *first,
                    second: *second,
                    meeting,
                });
            }
        }

        let mut held = BTreeSet::new();
        for pair in &pairs {
            if let (Some(first), Some(second)) =
                (self.faces.get(&pair.first), self.faces.get(&pair.second))
                && misfit(first, second, &pair.meeting.samples) <= CLEAN
            {
                held.extend([pair.first, pair.second]);
            }
        }

        let rank = |face: u64| {
            (
                standing.get(&face).copied().unwrap_or_default(),
                Reverse(face),
            )
        };
        let mut moved: BTreeMap<u64, (Surface, f64)> = BTreeMap::new();
        for pair in &pairs {
            let current = |face: u64| {
                moved
                    .get(&face)
                    .map(|(surface, _)| surface)
                    .or_else(|| self.faces.get(&face))
            };
            let (Some(first), Some(second)) = (current(pair.first), current(pair.second)) else {
                continue;
            };
            if misfit(first, second, &pair.meeting.samples) <= CLEAN {
                continue;
            }
            let chosen = [(pair.first, second, first), (pair.second, first, second)]
                .into_iter()
                .filter(|(follower, _, _)| {
                    !held.contains(follower) && !moved.contains_key(follower)
                })
                .filter_map(|(follower, onto, written)| {
                    self.round_onto(follower, written, onto, &pair.meeting.samples)
                        .map(|snapped| (follower, snapped))
                })
                .min_by(|(first, first_move), (second, second_move)| {
                    first_move
                        .fits_written_edges(second_move)
                        .then(rank(*first).cmp(&rank(*second)))
                });
            if let Some((follower, snapped)) = chosen {
                let master = if follower == pair.first {
                    pair.second
                } else {
                    pair.first
                };
                moved.insert(follower, (snapped.surface, snapped.moved));
                held.insert(master);
            }
        }
        moved
    }

    fn round_onto(
        &self,
        face: u64,
        written: &Surface,
        onto: &Surface,
        shared: &[Point3],
    ) -> Option<RoundMove> {
        let feet = feet_on(onto, shared);
        let touch = feet.get(feet.len() / 2)?;
        let outline = self.round_outlines.get(&face)?;
        let on_face = feet_on(written, outline);
        candidates(written, onto, *touch)
            .into_iter()
            .filter(|candidate| feet.iter().all(|foot| candidate.distance(*foot) <= CLEAN))
            .filter_map(|surface| {
                let moved = farthest_from(&surface, &on_face);
                let kept = farthest_from(&surface, outline);
                (moved > CLEAN && moved <= self.body.allowance).then_some(RoundMove {
                    surface,
                    moved,
                    kept,
                })
            })
            .min_by(|first, second| {
                first
                    .kept
                    .total_cmp(&second.kept)
                    .then(first.moved.total_cmp(&second.moved))
            })
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

pub(crate) fn seam_through(
    surface: &Surface,
    samples: &[Point3],
    from: Point3,
    to: Point3,
) -> Option<Curve> {
    let seam: Curve = match surface {
        Surface::Cylinder(_) | Surface::Cone(_) => {
            let line = Line::through(from, to).ok()?;
            if surface.distance(from.lerp(to, 0.5)) > CLEAN {
                return None;
            }
            line.into()
        }
        Surface::Sphere(sphere) => {
            let middle = samples.get(samples.len() / 2)?;
            let frame = sphere.frame();
            let radial = radial_of(frame, surface.project(*middle, None).x);
            let meridian =
                Plane::from_frame(sphere.center(), radial.cross(frame.normal()), radial)?;
            Circle::new(meridian, sphere.radius()).ok()?.into()
        }
        Surface::Torus(torus) => {
            let frame = torus.frame();
            let at = surface.project(from, None);
            let circle = if runs_along_tube(surface, samples) {
                let radial = radial_of(frame, at.x);
                let centre = frame.origin() + radial * torus.major_radius();
                let tube = Plane::from_frame(centre, radial.cross(frame.normal()), radial)?;
                Circle::new(tube, torus.minor_radius())
            } else {
                let (sin, cos) = at.y.sin_cos();
                let centre = frame.origin() + frame.normal() * (torus.minor_radius() * sin);
                let ring = Plane::from_frame(centre, frame.normal(), frame.x_axis())?;
                Circle::new(ring, torus.major_radius() + torus.minor_radius() * cos)
            };
            circle.ok()?.into()
        }
        _ => return None,
    };
    Some(seam)
}

fn thinned(points: &[Point3]) -> Vec<Point3> {
    let stride = points.len().div_ceil(MOST_OUTLINE_SAMPLES).max(1);
    points.iter().step_by(stride).copied().collect()
}

fn radial_of(frame: &Plane, u: f64) -> Vector3 {
    let (sin, cos) = u.sin_cos();
    frame.x_axis() * cos + frame.y_axis() * sin
}

fn runs_along_tube(surface: &Surface, samples: &[Point3]) -> bool {
    let mut hint = None;
    let mut low = Point2::splat(f64::INFINITY);
    let mut high = Point2::splat(f64::NEG_INFINITY);
    for point in samples {
        let uv = surface.project(*point, hint);
        hint = Some(uv);
        low = low.min(uv);
        high = high.max(uv);
    }
    let spread = high - low;
    spread.y > spread.x
}

fn is_round(surface: &Surface) -> bool {
    centre_of(surface).is_some()
}

fn is_rigid(surface: &Surface) -> bool {
    matches!(surface, Surface::Plane(_)) || is_round(surface)
}

fn centre_of(surface: &Surface) -> Option<Centre> {
    let axis = |frame: &Plane| Some(Centre::Axis(frame.origin(), frame.normal()));
    match surface {
        Surface::Cylinder(cylinder) => axis(cylinder.frame()),
        Surface::Cone(cone) => axis(cone.frame()),
        Surface::Torus(torus) => axis(torus.frame()),
        Surface::Sphere(sphere) => Some(Centre::Point(sphere.center())),
        _ => None,
    }
}

fn tangent_at(first: &Surface, second: &Surface, at: Point3) -> bool {
    let normal = |surface: &Surface| {
        let uv = surface.project(at, None);
        surface.normal(uv.x, uv.y)
    };
    normal(first)
        .zip(normal(second))
        .is_some_and(|(first, second)| first.dot(second).abs() >= PARALLEL_ANGLE.cos())
}

fn feet_on(surface: &Surface, points: &[Point3]) -> Vec<Point3> {
    let mut hint = None;
    points
        .iter()
        .map(|point| {
            let foot = surface.project(*point, hint);
            hint = Some(foot);
            surface.point_at(foot)
        })
        .collect()
}

fn farthest_from(surface: &Surface, points: &[Point3]) -> f64 {
    points
        .iter()
        .map(|point| surface.distance(*point))
        .fold(0.0, f64::max)
}

fn misfit(follower: &Surface, master: &Surface, shared: &[Point3]) -> f64 {
    farthest_from(follower, &feet_on(master, shared))
}

fn candidates(written: &Surface, onto: &Surface, touch: Point3) -> Vec<Surface> {
    let aligned = coaxial(written, onto);
    let mut found = Vec::new();
    for base in std::iter::once(written).chain(aligned.as_ref()) {
        let uv = base.project(touch, None);
        let gap = touch - base.point_at(uv);
        if let Some(normal) = base.normal(uv.x, uv.y)
            && let Ok(grown) = offset(base, gap.dot(normal), &EXACT)
        {
            found.push(grown);
        }
        if let Some(shifted) =
            RigidTransform::translation(gap).and_then(|shift| base.transformed(&shift).ok())
        {
            found.push(shifted);
        }
    }
    found.extend(aligned);
    found
}

const EXACT: Fit<'static> = Fit {
    tolerance: CLEAN,
    charge: &no_charge,
};

fn no_charge(_: usize) -> Result<(), OffsetRefusal> {
    Ok(())
}

fn coaxial(written: &Surface, onto: &Surface) -> Option<Surface> {
    let foot_on_axis = |point: Point3, origin: Point3, direction: Vector3| {
        origin + direction * (point - origin).dot(direction)
    };
    let transform = match (centre_of(written)?, centre_of(onto)?) {
        (Centre::Axis(origin, direction), Centre::Axis(onto_origin, onto_direction)) => {
            let alignment = direction.dot(onto_direction);
            if alignment.abs() < PARALLEL_ANGLE.cos() {
                return None;
            }
            let rotation =
                Rotation3::from_rotation_arc(direction, onto_direction * alignment.signum());
            let foot = foot_on_axis(origin, onto_origin, onto_direction);
            RigidTransform::new(rotation, foot - rotation * origin)
        }
        (Centre::Axis(origin, direction), Centre::Point(centre)) => {
            RigidTransform::translation(centre - foot_on_axis(centre, origin, direction))
        }
        (Centre::Point(centre), Centre::Axis(onto_origin, onto_direction)) => {
            RigidTransform::translation(foot_on_axis(centre, onto_origin, onto_direction) - centre)
        }
        (Centre::Point(centre), Centre::Point(onto_centre)) => {
            RigidTransform::translation(onto_centre - centre)
        }
    }?;
    written.transformed(&transform).ok()
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
