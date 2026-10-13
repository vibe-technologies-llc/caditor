use caditor_geometry::Vector3;

const SCALE: f64 = i32::MAX as f64;
const ABSENT: [i32; 2] = [-i32::MAX, -i32::MAX];
const DOWN: [i32; 2] = [i32::MAX, i32::MAX];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshNormal([i32; 2]);

impl MeshNormal {
    pub const ABSENT: Self = Self(ABSENT);

    pub fn new(direction: Vector3) -> Self {
        let length = direction.abs().element_sum();
        if !(length > 0.0 && length.is_finite()) {
            return Self::ABSENT;
        }
        let on = direction / length;
        let side = |value: f64| if value >= 0.0 { 1.0 } else { -1.0 };
        let folded = if on.z >= 0.0 {
            [on.x, on.y]
        } else {
            [
                (1.0 - on.y.abs()) * side(on.x),
                (1.0 - on.x.abs()) * side(on.y),
            ]
        };
        let packed = folded.map(snorm32);
        Self(if packed == ABSENT { DOWN } else { packed })
    }

    pub fn is_absent(self) -> bool {
        self.0 == ABSENT
    }

    pub fn vector(self) -> Vector3 {
        if self.is_absent() {
            return Vector3::ZERO;
        }
        let [x, y] = self.0.map(|value| (f64::from(value) / SCALE).max(-1.0));
        let z = 1.0 - x.abs() - y.abs();
        let lift = (-z).max(0.0);
        let unfold = |value: f64| value - lift.copysign(value);
        Vector3::new(unfold(x), unfold(y), z).normalize_or_zero()
    }
}

fn snorm32(value: f64) -> i32 {
    (value.clamp(-1.0, 1.0) * SCALE).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    const FINER_THAN_F32: f64 = 1e-8;

    fn angle(first: Vector3, second: Vector3) -> f64 {
        first.cross(second).length().atan2(first.dot(second))
    }

    #[test]
    fn a_packed_normal_is_eight_bytes_and_a_mesh_vertex_twelve() {
        assert_eq!(size_of::<MeshNormal>(), 8);
        assert_eq!(size_of::<crate::tessellation::MeshVertex>(), 12);
    }

    #[test]
    fn normals_come_back_finer_than_f32_holds_them() {
        let steps = 96;
        let mut worst = 0.0f64;
        for latitude in 0..=steps {
            for longitude in 0..steps * 2 {
                let polar = std::f64::consts::PI * f64::from(latitude) / f64::from(steps);
                let azimuth = std::f64::consts::PI * f64::from(longitude) / f64::from(steps);
                let direction = Vector3::new(
                    polar.sin() * azimuth.cos(),
                    polar.sin() * azimuth.sin(),
                    polar.cos(),
                );

                let back = MeshNormal::new(direction * 3.0).vector();

                assert!(
                    (back.length() - 1.0).abs() < 1e-12,
                    "{direction} came back {back}"
                );
                worst = worst.max(angle(back, direction));
            }
        }

        assert!(worst < FINER_THAN_F32, "{worst}");
    }

    #[test]
    fn the_axes_come_back_exactly() {
        for axis in [
            Vector3::X,
            Vector3::NEG_X,
            Vector3::Y,
            Vector3::NEG_Y,
            Vector3::Z,
            Vector3::NEG_Z,
        ] {
            assert_eq!(MeshNormal::new(axis).vector(), axis);
        }
    }

    #[test]
    fn no_direction_stays_absent_and_nothing_else_becomes_it() {
        let near_down = Vector3::new(-1e-12, -1e-12, -1.0);

        assert!(MeshNormal::new(Vector3::ZERO).is_absent());
        assert!(MeshNormal::new(Vector3::NAN).is_absent());
        assert_eq!(MeshNormal::ABSENT.vector(), Vector3::ZERO);
        assert!(!MeshNormal::new(near_down).is_absent());
        assert!(angle(MeshNormal::new(near_down).vector(), near_down) < FINER_THAN_F32);
    }
}
