use std::{cell::OnceCell, collections::BTreeSet};

use caditor_geometry::{Aabb, Point3};

use super::{EdgeId, FaceId, ShellId, Solid, ValidationError, validate::face_edges};
use crate::{
    box_tree::BoxTree, intersect::boxes_overlap, sense::Sense, tessellation::triangles_contain,
    tolerance::LINEAR_RESOLUTION,
};

const EDGE_PROBES: usize = 16;
const MAX_PROBES: usize = 1024;
const DEVIATION_ALLOWANCE: f64 = 2.0;

#[derive(Debug, Clone)]
pub(super) struct ShellMesh {
    pub(super) triangles: Vec<[Point3; 3]>,
    pub(super) faces: Vec<FaceId>,
    pub(super) corners: Vec<Point3>,
}

#[derive(Debug)]
pub(super) struct Lump<'a> {
    id: ShellId,
    sense: Sense,
    mesh: &'a ShellMesh,
    bounds: Aabb,
    feet: Vec<OnceCell<Option<Point3>>>,
}

#[derive(Debug, Clone, Copy)]
enum Probe {
    At(Point3),
    Within(usize),
}

enum Placement {
    Inside,
    Outside,
    Touching,
    Undecided,
}

enum Depth {
    Known {
        depth: i64,
        lump: Option<ShellId>,
        void: Option<ShellId>,
    },
    Touching(ShellId),
    Unknown,
}

enum Contact {
    Untested,
    Only(ShellId),
    Mixed,
}

impl<'a> Lump<'a> {
    pub(super) fn new(id: ShellId, sense: Sense, mesh: &'a ShellMesh) -> Option<Self> {
        let bounds = Aabb::from_points(mesh.triangles.iter().flatten().copied())?;
        Some(Self {
            id,
            sense,
            mesh,
            bounds,
            feet: mesh.triangles.iter().map(|_| OnceCell::new()).collect(),
        })
    }

    fn material(&self) -> i64 {
        match self.sense {
            Sense::Same => 1,
            Sense::Reversed => -1,
        }
    }

    fn expected_depth(&self) -> i64 {
        match self.sense {
            Sense::Same => 0,
            Sense::Reversed => 1,
        }
    }

    fn centroid(&self, index: usize) -> Option<Point3> {
        self.mesh
            .triangles
            .get(index)
            .map(|[a, b, c]| (*a + *b + *c) / 3.0)
    }

    fn foot(&self, solid: &Solid, index: usize) -> Option<Point3> {
        *self.feet.get(index)?.get_or_init(|| {
            let centroid = self.centroid(index)?;
            let surface = solid.face(*self.mesh.faces.get(index)?)?.surface();
            Some(surface.point_at(surface.project(centroid, None))).filter(|foot| foot.is_finite())
        })
    }

    fn deviation(&self, solid: &Solid, index: usize) -> f64 {
        self.centroid(index)
            .zip(self.foot(solid, index))
            .map_or(0.0, |(centroid, foot)| foot.distance(centroid))
    }

    fn place(&self, solid: &Solid, point: Point3) -> Placement {
        if !boxes_overlap(&self.bounds, &Aabb::from_point(point), LINEAR_RESOLUTION) {
            return Placement::Outside;
        }
        let nearest = self
            .mesh
            .triangles
            .iter()
            .enumerate()
            .map(|(index, triangle)| (closest_on_triangle(point, triangle).distance(point), index))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((distance, index)) = nearest else {
            return Placement::Outside;
        };
        let band = LINEAR_RESOLUTION + DEVIATION_ALLOWANCE * self.deviation(solid, index);
        if distance.is_nan() || distance <= band {
            return Placement::Touching;
        }
        match triangles_contain(&self.mesh.triangles, point) {
            Some(true) => Placement::Inside,
            Some(false) => Placement::Outside,
            None => Placement::Undecided,
        }
    }

    fn probes(&self, solid: &Solid) -> Vec<Probe> {
        let faces = solid.shell(self.id).map_or(&[][..], |shell| shell.faces());
        let edges: BTreeSet<EdgeId> = faces
            .iter()
            .filter_map(|face| solid.face(*face))
            .filter_map(|face| face_edges(solid, face).ok())
            .flatten()
            .collect();
        let along_edges = edges
            .into_iter()
            .filter_map(|edge| solid.edge(edge))
            .flat_map(|edge| {
                (1..EDGE_PROBES).map(move |step| {
                    let fraction = step as f64 / EDGE_PROBES as f64;
                    Probe::At(edge.curve().point(edge.interval().at(fraction)))
                })
            });
        let within_faces = (0..self.mesh.triangles.len()).map(Probe::Within);
        self.mesh
            .corners
            .iter()
            .map(|corner| Probe::At(*corner))
            .chain(along_edges)
            .chain(within_faces)
            .collect()
    }

    fn rough(&self, probe: Probe) -> Option<Point3> {
        match probe {
            Probe::At(point) => Some(point),
            Probe::Within(index) => self.centroid(index),
        }
    }

    fn exact(&self, solid: &Solid, probe: Probe) -> Option<Point3> {
        match probe {
            Probe::At(point) => Some(point),
            Probe::Within(index) => self.foot(solid, index),
        }
    }

    fn depth_at(&self, solid: &Solid, others: &[&Lump], probe: Point3) -> Depth {
        let mut depth = 0;
        let (mut lump, mut void) = (None, None);
        for other in others {
            match other.place(solid, probe) {
                Placement::Inside => {
                    depth += other.material();
                    match other.sense {
                        Sense::Same => lump = Some(other.id),
                        Sense::Reversed => void = Some(other.id),
                    }
                }
                Placement::Outside => {}
                Placement::Touching => return Depth::Touching(other.id),
                Placement::Undecided => return Depth::Unknown,
            }
        }
        Depth::Known { depth, lump, void }
    }

    fn misplaced(&self, lump: Option<ShellId>, void: Option<ShellId>) -> ValidationError {
        let reached = match (self.sense, lump, void) {
            (Sense::Same, Some(other), _)
            | (Sense::Same, None, Some(other))
            | (Sense::Reversed, Some(_), Some(other))
            | (Sense::Reversed, Some(other), None) => other,
            (_, None, _) => return ValidationError::VoidOutside(self.id),
        };
        ValidationError::LumpsOverlap {
            shell: self.id,
            other: reached,
        }
    }

    fn check_against(&self, solid: &Solid, others: &[&Lump]) -> Result<(), ValidationError> {
        let (near, apart): (Vec<Probe>, Vec<Probe>) =
            self.probes(solid).into_iter().partition(|probe| {
                self.rough(*probe).is_some_and(|point| {
                    others.iter().any(|other| {
                        boxes_overlap(&other.bounds, &Aabb::from_point(point), LINEAR_RESOLUTION)
                    })
                })
            });
        if !apart.is_empty() && self.sense == Sense::Reversed {
            return Err(ValidationError::VoidOutside(self.id));
        }
        let mut decided = !apart.is_empty();
        let mut contact = Contact::Untested;
        let stride = near.len().div_ceil(MAX_PROBES).max(1);
        let exact = near
            .into_iter()
            .step_by(stride)
            .filter_map(|probe| self.exact(solid, probe));
        for probe in exact {
            match self.depth_at(solid, others, probe) {
                Depth::Known { depth, lump, void } => {
                    if depth != self.expected_depth() {
                        return Err(self.misplaced(lump, void));
                    }
                    decided = true;
                }
                Depth::Touching(other) => {
                    contact = match contact {
                        Contact::Untested => Contact::Only(other),
                        Contact::Only(only) if only == other => Contact::Only(only),
                        _ => Contact::Mixed,
                    };
                }
                Depth::Unknown => contact = Contact::Mixed,
            }
        }
        match (decided, contact) {
            (true, _) => Ok(()),
            (false, Contact::Only(other)) => Err(ValidationError::LumpsCoincide {
                shell: self.id,
                other,
            }),
            (false, _) if self.sense == Sense::Reversed => {
                Err(ValidationError::VoidOutside(self.id))
            }
            (false, _) => Ok(()),
        }
    }
}

pub(super) fn check(solid: &Solid, lumps: &[Lump]) -> Result<(), ValidationError> {
    let tree = BoxTree::new(lumps.iter().map(|lump| lump.bounds));
    let voids_first = lumps
        .iter()
        .filter(|lump| lump.sense == Sense::Reversed)
        .chain(lumps.iter().filter(|lump| lump.sense == Sense::Same));
    for lump in voids_first {
        let others: Vec<&Lump> = tree
            .overlapping(&lump.bounds, LINEAR_RESOLUTION)
            .into_iter()
            .filter_map(|index| lumps.get(index))
            .filter(|other| other.id != lump.id)
            .collect();
        if others.is_empty() && lump.sense == Sense::Same {
            continue;
        }
        lump.check_against(solid, &others)?;
    }
    Ok(())
}

fn closest_on_triangle(point: Point3, [a, b, c]: &[Point3; 3]) -> Point3 {
    let (ab, ac, ap) = (*b - *a, *c - *a, point - *a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return *a;
    }
    let bp = point - *b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return *b;
    }
    let cp = point - *c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return *c;
    }
    let toward_c = d1 * d4 - d3 * d2;
    if toward_c <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return *a + ab * (d1 / (d1 - d3));
    }
    let toward_b = d5 * d2 - d1 * d6;
    if toward_b <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return *a + ac * (d2 / (d2 - d6));
    }
    let toward_a = d3 * d6 - d5 * d4;
    if toward_a <= 0.0 && d4 >= d3 && d5 >= d6 {
        return *b + (*c - *b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let total = toward_a + toward_b + toward_c;
    let inside = *a + ab * (toward_b / total) + ac * (toward_c / total);
    if inside.is_finite() { inside } else { *a }
}
