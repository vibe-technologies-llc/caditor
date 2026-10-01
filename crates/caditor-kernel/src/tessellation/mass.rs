use std::f64::consts::EULER_GAMMA;

use caditor_geometry::{Aabb, Point3, Vector3};

use crate::{tessellation::Mesh, topology::FaceId};

const RAY_DIRECTIONS: [Vector3; 3] = [
    Vector3::new(
        EULER_GAMMA,
        0.618_033_988_749_894_8,
        0.533_146_667_914_491_7,
    ),
    Vector3::new(
        -0.414_213_562_373_095,
        0.732_050_807_568_877_2,
        0.271_828_182_845_904_5,
    ),
    Vector3::new(
        0.353_553_390_593_273_8,
        -0.462_910_049_886_275_7,
        -0.813_008_130_081_300_8,
    ),
];
const EDGE_MARGIN: f64 = 1e-9;
const PARALLEL_EPSILON: f64 = 1e-14;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MassProperties {
    pub volume: f64,
    pub area: f64,
    pub centroid: Point3,
}

impl MassProperties {
    pub(crate) fn of(triangles: &[[Point3; 3]]) -> Self {
        let reference = Aabb::from_points(triangles.iter().flatten().copied())
            .map_or(Point3::ZERO, |bounds| bounds.center());
        let mut volume = 0.0;
        let mut area = 0.0;
        let mut moment = Vector3::ZERO;
        for [a, b, c] in triangles {
            let (a, b, c) = (*a - reference, *b - reference, *c - reference);
            let signed = a.dot(b.cross(c)) / 6.0;
            volume += signed;
            moment += (a + b + c) * (signed / 4.0);
            area += 0.5 * (b - a).cross(c - a).length();
        }
        let centroid = if volume.abs() > f64::MIN_POSITIVE {
            reference + moment / volume
        } else {
            reference
        };
        Self {
            volume,
            area,
            centroid,
        }
    }
}

impl Mesh {
    pub fn mass_properties(&self) -> MassProperties {
        self.mass_properties_where(|_| true)
    }

    pub fn mass_properties_where(&self, keep: impl Fn(FaceId) -> bool) -> MassProperties {
        let triangles: Vec<[Point3; 3]> = self.face_triangles(keep).collect();
        MassProperties::of(&triangles)
    }

    pub fn contains(&self, point: Point3, keep: impl Fn(FaceId) -> bool) -> Option<bool> {
        RAY_DIRECTIONS
            .iter()
            .find_map(|direction| parity(point, *direction, self.face_triangles(&keep)))
    }
}

pub(crate) fn triangles_contain(triangles: &[[Point3; 3]], point: Point3) -> Option<bool> {
    RAY_DIRECTIONS
        .iter()
        .find_map(|direction| parity(point, *direction, triangles.iter().copied()))
}

fn parity(
    point: Point3,
    direction: Vector3,
    triangles: impl IntoIterator<Item = [Point3; 3]>,
) -> Option<bool> {
    let mut crossings = 0usize;
    for triangle in triangles {
        match crossing(point, direction, &triangle) {
            Crossing::Miss => {}
            Crossing::Hit => crossings += 1,
            Crossing::Ambiguous => return None,
        }
    }
    Some(crossings % 2 == 1)
}

enum Crossing {
    Miss,
    Hit,
    Ambiguous,
}

fn crossing(origin: Point3, direction: Vector3, [a, b, c]: &[Point3; 3]) -> Crossing {
    let (first, second) = (*b - *a, *c - *a);
    let across = direction.cross(second);
    let determinant = first.dot(across);
    if determinant.abs() <= PARALLEL_EPSILON * first.length() * second.length() {
        return Crossing::Miss;
    }
    let inverse = 1.0 / determinant;
    let offset = origin - *a;
    let u = offset.dot(across) * inverse;
    let turned = offset.cross(first);
    let v = direction.dot(turned) * inverse;
    let distance = second.dot(turned) * inverse;
    let outside =
        u < -EDGE_MARGIN || v < -EDGE_MARGIN || u + v > 1.0 + EDGE_MARGIN || distance < 0.0;
    if outside {
        return Crossing::Miss;
    }
    let near_edge = u < EDGE_MARGIN || v < EDGE_MARGIN || u + v > 1.0 - EDGE_MARGIN;
    if near_edge || distance <= EDGE_MARGIN {
        Crossing::Ambiguous
    } else {
        Crossing::Hit
    }
}
