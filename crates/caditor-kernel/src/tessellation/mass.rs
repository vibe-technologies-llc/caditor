use std::f64::consts::{EULER_GAMMA, PI};

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

pub type SecondMoment = [[f64; 3]; 3];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MassProperties {
    pub volume: f64,
    pub area: f64,
    pub centroid: Point3,
    pub second_moment: SecondMoment,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Moments {
    pub(crate) area: f64,
    pub(crate) volume: f64,
    pub(crate) first: Vector3,
    pub(crate) second: SecondMoment,
}

impl Moments {
    pub(crate) fn of_triangles(triangles: &[[Point3; 3]], reference: Point3) -> Self {
        let mut moments = Self::default();
        for [a, b, c] in triangles {
            let (a, b, c) = (*a - reference, *b - reference, *c - reference);
            let signed = a.dot(b.cross(c)) / 6.0;
            moments.volume += signed;
            moments.first += (a + b + c) * (signed / 4.0);
            moments.area += 0.5 * (b - a).cross(c - a).length();
            let sum = a + b + c;
            for corner in [a, b, c, sum] {
                add_outer(&mut moments.second, corner, corner, signed / 20.0);
            }
        }
        moments
    }

    pub(crate) fn add(&mut self, other: &Self) {
        self.area += other.area;
        self.volume += other.volume;
        self.first += other.first;
        for (row, other_row) in self.second.iter_mut().zip(other.second) {
            for (entry, other_entry) in row.iter_mut().zip(other_row) {
                *entry += other_entry;
            }
        }
    }

    pub(crate) fn about(self, reference: Point3) -> MassProperties {
        let offset = if self.volume.abs() > f64::MIN_POSITIVE {
            self.first / self.volume
        } else {
            Vector3::ZERO
        };
        let mut second = self.second;
        add_outer(&mut second, offset, offset, -self.volume);
        MassProperties {
            volume: self.volume,
            area: self.area,
            centroid: reference + offset,
            second_moment: second,
        }
    }
}

impl MassProperties {
    pub(crate) fn of(triangles: &[[Point3; 3]]) -> Self {
        let reference = Aabb::from_points(triangles.iter().flatten().copied())
            .map_or(Point3::ZERO, |bounds| bounds.center());
        Moments::of_triangles(triangles, reference).about(reference)
    }

    pub fn second_moment_about(&self, point: Point3) -> SecondMoment {
        let mut second = self.second_moment;
        let offset = self.centroid - point;
        add_outer(&mut second, offset, offset, self.volume);
        second
    }

    pub fn inertia(second_moment: &SecondMoment) -> SecondMoment {
        let [[xx, xy, xz], [_, yy, yz], [_, _, zz]] = *second_moment;
        [
            [yy + zz, -xy, -xz],
            [-xy, xx + zz, -yz],
            [-xz, -yz, xx + yy],
        ]
    }

    pub fn principal_moments(inertia: &SecondMoment) -> [f64; 3] {
        let [[a, d, e], [_, b, f], [_, _, c]] = *inertia;
        let off_diagonal = d * d + e * e + f * f;
        let mut moments = if off_diagonal <= f64::EPSILON * (a * a + b * b + c * c) {
            [a, b, c]
        } else {
            let mean = (a + b + c) / 3.0;
            let spread = (((a - mean).powi(2)
                + (b - mean).powi(2)
                + (c - mean).powi(2)
                + 2.0 * off_diagonal)
                / 6.0)
                .sqrt();
            let scaled = |value: f64| (value - mean) / spread;
            let (sa, sb, sc) = (scaled(a), scaled(b), scaled(c));
            let (sd, se, sf) = (d / spread, e / spread, f / spread);
            let half_determinant = (sa * (sb * sc - sf * sf) - sd * (sd * sc - sf * se)
                + se * (sd * sf - sb * se))
                / 2.0;
            let angle = half_determinant.clamp(-1.0, 1.0).acos() / 3.0;
            let largest = mean + 2.0 * spread * angle.cos();
            let smallest = mean + 2.0 * spread * (angle + 2.0 * PI / 3.0).cos();
            [smallest, 3.0 * mean - largest - smallest, largest]
        };
        moments.sort_by(f64::total_cmp);
        moments
    }
}

fn add_outer(sum: &mut SecondMoment, first: Vector3, second: Vector3, weight: f64) {
    let (first, second) = (first.to_array(), second.to_array());
    for (row, along) in sum.iter_mut().zip(first) {
        for (entry, across) in row.iter_mut().zip(second) {
            *entry += weight * along * across;
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
