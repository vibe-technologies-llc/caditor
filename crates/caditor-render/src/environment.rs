use std::{
    f32::consts::{PI, TAU},
    sync::atomic::{AtomicU64, Ordering},
};

use glam::Vec3;
use half::f16;

pub const MAX_ENVIRONMENT_WIDTH: u32 = 4096;
pub const LEAST_ENVIRONMENT_WIDTH: u32 = 16;
pub const IRRADIANCE_WIDTH: u32 = 32;
pub const IRRADIANCE_HEIGHT: u32 = IRRADIANCE_WIDTH / 2;
const IRRADIANCE_SOURCE_WIDTH: usize = 64;
const LARGEST_RADIANCE: f32 = 60_000.0;
const SHAPE_TOLERANCE_PERCENT: u32 = 2;
const CHANNELS: usize = 3;
const RGBA_HALF_BYTES: usize = 8;

static NEXT_IDENTITY: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EnvironmentError {
    #[error(
        "the picture is {width} by {height} pixels, but an environment is an equirectangular picture, twice as wide as it is high"
    )]
    NotEquirectangular { width: u32, height: u32 },
    #[error(
        "the picture is {width} by {height} pixels, too small to surround the model: it needs at least {least} pixels across"
    )]
    TooSmall { width: u32, height: u32, least: u32 },
    #[error("a picture of {width} by {height} pixels needs {expected} values and holds {found}")]
    WrongSize {
        width: u32,
        height: u32,
        expected: usize,
        found: usize,
    },
}

pub struct Environment {
    identity: u64,
    width: u32,
    height: u32,
    levels: Vec<Vec<u8>>,
    irradiance: Vec<u8>,
}

impl std::fmt::Debug for Environment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Environment")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("levels", &self.levels.len())
            .finish()
    }
}

impl PartialEq for Environment {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

impl Eq for Environment {}

impl Environment {
    pub fn new(width: u32, height: u32, rgb: Vec<f32>) -> Result<Self, EnvironmentError> {
        let expected = value_count(width, height);
        if expected != rgb.len() {
            return Err(EnvironmentError::WrongSize {
                width,
                height,
                expected,
                found: rgb.len(),
            });
        }
        if width < LEAST_ENVIRONMENT_WIDTH || height == 0 {
            return Err(EnvironmentError::TooSmall {
                width,
                height,
                least: LEAST_ENVIRONMENT_WIDTH,
            });
        }
        if width.abs_diff(height.saturating_mul(2)) * 100 > width * SHAPE_TOLERANCE_PERCENT {
            return Err(EnvironmentError::NotEquirectangular { width, height });
        }
        let source = Raster {
            width: width as usize,
            height: height as usize,
            rgb: rgb.into_iter().map(finite_radiance).collect(),
        };
        let kept_width = kept_width(width) as usize;
        let kept_height = kept_width / 2;
        let base = if (source.width, source.height) == (kept_width, kept_height) {
            source
        } else {
            source.resampled(kept_width, kept_height)
        };
        let irradiance = irradiance_of(&base);
        let mut chain = vec![base];
        while let Some(last) = chain.last().filter(|last| last.height > 1) {
            let next = last.halved();
            chain.push(next);
        }
        let (kept_width, kept_height) = (kept_width as u32, kept_height as u32);
        Ok(Self {
            identity: NEXT_IDENTITY.fetch_add(1, Ordering::Relaxed),
            width: kept_width,
            height: kept_height,
            levels: chain.iter().map(Raster::half_floats).collect(),
            irradiance,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    pub fn held_bytes(&self) -> usize {
        self.levels.iter().map(Vec::len).sum::<usize>() + self.irradiance.len()
    }

    pub(crate) fn level(&self, level: usize) -> Option<(&[u8], u32, u32)> {
        let shift = u32::try_from(level).ok()?;
        let bytes = self.levels.get(level)?;
        Some((
            bytes,
            self.width.checked_shr(shift)?.max(1),
            self.height.checked_shr(shift)?.max(1),
        ))
    }

    pub(crate) fn irradiance(&self) -> &[u8] {
        &self.irradiance
    }

    pub fn radiance_at(&self, level: usize, x: u32, y: u32) -> Option<[f32; 3]> {
        let (bytes, width, height) = self.level(level)?;
        texel_of(bytes, width, height, x, y)
    }

    pub fn irradiance_at(&self, x: u32, y: u32) -> Option<[f32; 3]> {
        texel_of(&self.irradiance, IRRADIANCE_WIDTH, IRRADIANCE_HEIGHT, x, y)
    }
}

fn value_count(width: u32, height: u32) -> usize {
    usize::try_from(width)
        .unwrap_or(usize::MAX)
        .saturating_mul(usize::try_from(height).unwrap_or(usize::MAX))
        .saturating_mul(CHANNELS)
}

fn kept_width(width: u32) -> u32 {
    width.min(MAX_ENVIRONMENT_WIDTH) & !1
}

fn texel_of(bytes: &[u8], width: u32, height: u32, x: u32, y: u32) -> Option<[f32; 3]> {
    if x >= width || y >= height {
        return None;
    }
    let at = (y as usize * width as usize + x as usize) * RGBA_HALF_BYTES;
    let texel = bytes.get(at..at + RGBA_HALF_BYTES)?;
    let channel = |index: usize| {
        let pair = texel.get(index * 2..index * 2 + 2)?;
        Some(f16::from_le_bytes([*pair.first()?, *pair.get(1)?]).to_f32())
    };
    Some([channel(0)?, channel(1)?, channel(2)?])
}

pub fn direction_of(u: f32, v: f32) -> Vec3 {
    let heading = (0.5 - u) * TAU;
    let polar = v * PI;
    let level = polar.sin();
    Vec3::new(level * heading.cos(), level * heading.sin(), polar.cos())
}

fn finite_radiance(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, LARGEST_RADIANCE)
    }
}

struct Raster {
    width: usize,
    height: usize,
    rgb: Vec<f32>,
}

impl Raster {
    fn halved(&self) -> Self {
        self.resampled((self.width / 2).max(1), (self.height / 2).max(1))
    }

    fn resampled(&self, width: usize, height: usize) -> Self {
        let across = self.across(width);
        let mut rgb = Vec::with_capacity(width * height * CHANNELS);
        let row_length = width * CHANNELS;
        for y in 0..height {
            let (first, last) = covered(y, height, self.height);
            let mut sum = vec![0.0_f32; row_length];
            for row in across
                .chunks_exact(row_length)
                .skip(first)
                .take(last - first)
            {
                for (total, value) in sum.iter_mut().zip(row) {
                    *total += value;
                }
            }
            let count = (last - first) as f32;
            rgb.extend(sum.into_iter().map(|total| total / count));
        }
        Self { width, height, rgb }
    }

    fn across(&self, width: usize) -> Vec<f32> {
        let row_length = self.width * CHANNELS;
        let mut rgb = Vec::with_capacity(width * self.height * CHANNELS);
        for row in self.rgb.chunks_exact(row_length) {
            for x in 0..width {
                let (first, last) = covered(x, width, self.width);
                let mut sum = [0.0_f32; CHANNELS];
                let covering = row
                    .get(first * CHANNELS..last * CHANNELS)
                    .unwrap_or_default();
                for texel in covering.as_chunks::<CHANNELS>().0 {
                    for (total, value) in sum.iter_mut().zip(texel) {
                        *total += value;
                    }
                }
                let count = (last - first) as f32;
                rgb.extend(sum.into_iter().map(|total| total / count));
            }
        }
        rgb
    }

    fn half_floats(&self) -> Vec<u8> {
        half_floats(&self.rgb)
    }
}

fn covered(index: usize, count: usize, source: usize) -> (usize, usize) {
    let first = (index * source / count).min(source - 1);
    let last = ((index + 1) * source)
        .div_ceil(count)
        .clamp(first + 1, source);
    (first, last)
}

fn half_floats(rgb: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(rgb.len() / CHANNELS * RGBA_HALF_BYTES);
    for texel in rgb.as_chunks::<CHANNELS>().0 {
        for channel in texel.iter().copied().chain([1.0]) {
            bytes.extend_from_slice(&f16::from_f32(channel).to_le_bytes());
        }
    }
    bytes
}

struct Patch {
    direction: Vec3,
    solid_angle: f32,
    radiance: Vec3,
}

fn irradiance_of(base: &Raster) -> Vec<u8> {
    let height = IRRADIANCE_SOURCE_WIDTH / 2;
    let small = base.resampled(IRRADIANCE_SOURCE_WIDTH, height);
    let step = TAU / IRRADIANCE_SOURCE_WIDTH as f32 * (PI / height as f32);
    let patches: Vec<Patch> = small
        .rgb
        .as_chunks::<CHANNELS>()
        .0
        .iter()
        .enumerate()
        .map(|(at, texel)| {
            let (x, y) = (at % IRRADIANCE_SOURCE_WIDTH, at / IRRADIANCE_SOURCE_WIDTH);
            let direction = direction_of(
                (x as f32 + 0.5) / IRRADIANCE_SOURCE_WIDTH as f32,
                (y as f32 + 0.5) / height as f32,
            );
            Patch {
                direction,
                solid_angle: step * (1.0 - direction.z * direction.z).max(0.0).sqrt(),
                radiance: Vec3::from_array(*texel),
            }
        })
        .collect();
    let mut rgb = Vec::with_capacity((IRRADIANCE_WIDTH * IRRADIANCE_HEIGHT) as usize * CHANNELS);
    for y in 0..IRRADIANCE_HEIGHT {
        for x in 0..IRRADIANCE_WIDTH {
            let normal = direction_of(
                (x as f32 + 0.5) / IRRADIANCE_WIDTH as f32,
                (y as f32 + 0.5) / IRRADIANCE_HEIGHT as f32,
            );
            let (mut gathered, mut cosine_total) = (Vec3::ZERO, 0.0_f32);
            for patch in &patches {
                let weight = normal.dot(patch.direction).max(0.0) * patch.solid_angle;
                gathered += patch.radiance * weight;
                cosine_total += weight;
            }
            rgb.extend((gathered / cosine_total.max(f32::MIN_POSITIVE)).to_array());
        }
    }
    half_floats(&rgb)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(width: u32, colour: [f32; 3]) -> Environment {
        let rgb = (0..width * width / 2).flat_map(|_| colour).collect();
        Environment::new(width, width / 2, rgb).unwrap()
    }

    fn painted(width: u32, paint: impl Fn(f32, f32) -> [f32; 3]) -> Environment {
        let height = width / 2;
        let rgb = (0..height)
            .flat_map(|y| {
                let paint = &paint;
                (0..width).flat_map(move |x| {
                    paint(
                        (x as f32 + 0.5) / width as f32,
                        (y as f32 + 0.5) / height as f32,
                    )
                })
            })
            .collect();
        Environment::new(width, height, rgb).unwrap()
    }

    fn near(found: [f32; 3], expected: [f32; 3], tolerance: f32) {
        for (found, expected) in found.into_iter().zip(expected) {
            assert!(
                (found - expected).abs() <= tolerance,
                "{found:?} against {expected:?}"
            );
        }
    }

    #[test]
    fn a_picture_keeps_its_radiance_above_one_and_halves_down_to_a_single_row() {
        let environment = uniform(64, [4.0, 0.5, 0.25]);

        assert_eq!((environment.width(), environment.height()), (64, 32));
        assert_eq!(environment.level_count(), 6);
        near(
            environment.radiance_at(0, 10, 10).unwrap(),
            [4.0, 0.5, 0.25],
            1e-3,
        );
        near(
            environment.radiance_at(5, 1, 0).unwrap(),
            [4.0, 0.5, 0.25],
            1e-3,
        );
        assert_eq!(environment.level(5).map(|(_, w, h)| (w, h)), Some((2, 1)));
        assert_eq!(environment.radiance_at(0, 64, 0), None);
    }

    #[test]
    fn each_level_averages_the_blocks_of_the_one_before() {
        let environment = painted(32, |u, _| if u < 0.5 { [2.0; 3] } else { [0.0; 3] });

        near(environment.radiance_at(0, 3, 4).unwrap(), [2.0; 3], 1e-3);
        near(environment.radiance_at(0, 20, 4).unwrap(), [0.0; 3], 1e-3);
        near(environment.radiance_at(1, 4, 2).unwrap(), [2.0; 3], 1e-3);
        near(environment.radiance_at(4, 0, 0).unwrap(), [2.0; 3], 1e-3);
        near(environment.radiance_at(4, 1, 0).unwrap(), [0.0; 3], 1e-3);
    }

    #[test]
    fn a_wider_picture_is_shrunk_to_the_widest_texture_and_odd_widths_to_even_ones() {
        assert_eq!(kept_width(MAX_ENVIRONMENT_WIDTH * 2), MAX_ENVIRONMENT_WIDTH);
        assert_eq!(kept_width(1001), 1000);
        assert_eq!(kept_width(64), 64);
    }

    #[test]
    fn shrinking_by_a_fraction_averages_what_each_new_texel_covers() {
        let width = 96;
        let height = 48;
        let rgb = (0..width * height)
            .flat_map(|at| [(at % width) as f32, 1.0, 2.0])
            .collect();
        let raster = Raster { width, height, rgb };

        let smaller = raster.resampled(40, 20);
        let first = smaller.rgb.get(..3).unwrap();
        let last = smaller.rgb.get(39 * 3..40 * 3).unwrap();

        assert_eq!((smaller.width, smaller.height), (40, 20));
        assert!(first[0] > 0.5 && first[0] < 3.5, "{first:?}");
        assert!(last[0] > 91.0 && last[0] < 95.0, "{last:?}");
        assert_eq!(&first[1..], &[1.0, 2.0]);
    }

    #[test]
    fn a_picture_a_few_pixels_off_twice_as_wide_is_stretched_to_fit() {
        let rgb = vec![1.0; 100 * 49 * CHANNELS];
        let environment = Environment::new(100, 49, rgb).unwrap();

        assert_eq!((environment.width(), environment.height()), (100, 50));
    }

    #[test]
    fn odd_pictures_are_refused_with_what_is_wrong_with_them() {
        let refused = |width: u32, height: u32, count: usize| {
            Environment::new(width, height, vec![1.0; count]).unwrap_err()
        };

        assert_eq!(
            refused(64, 64, 64 * 64 * CHANNELS),
            EnvironmentError::NotEquirectangular {
                width: 64,
                height: 64
            }
        );
        assert_eq!(
            refused(8, 4, 8 * 4 * CHANNELS),
            EnvironmentError::TooSmall {
                width: 8,
                height: 4,
                least: LEAST_ENVIRONMENT_WIDTH
            }
        );
        assert_eq!(
            refused(64, 32, 10),
            EnvironmentError::WrongSize {
                width: 64,
                height: 32,
                expected: 64 * 32 * CHANNELS,
                found: 10
            }
        );
    }

    #[test]
    fn values_a_half_float_cannot_hold_are_made_finite() {
        let mut rgb = vec![1.0; 32 * 16 * CHANNELS];
        rgb[0] = f32::NAN;
        rgb[1] = f32::INFINITY;
        rgb[2] = -5.0;
        let environment = Environment::new(32, 16, rgb).unwrap();

        let first = environment.radiance_at(0, 0, 0).unwrap();
        assert_eq!(first[0], 0.0);
        assert!(first[1].is_finite() && first[1] > 1000.0);
        assert_eq!(first[2], 0.0);
    }

    #[test]
    fn the_picture_is_wound_with_its_middle_toward_x_and_the_top_toward_z() {
        let toward = |u: f32, v: f32| direction_of(u, v);

        assert!(toward(0.5, 0.5).distance(Vec3::X) < 1e-5);
        assert!(toward(0.25, 0.5).distance(Vec3::Y) < 1e-5);
        assert!(toward(0.75, 0.5).distance(Vec3::NEG_Y) < 1e-5);
        assert!(toward(0.3, 0.0).distance(Vec3::Z) < 1e-5);
        assert!(toward(0.3, 1.0).distance(Vec3::NEG_Z) < 1e-5);
    }

    #[test]
    fn a_uniform_sky_lights_every_normal_with_its_own_radiance() {
        let environment = uniform(128, [0.5, 1.0, 2.0]);

        for (x, y) in [(0, 0), (7, 3), (16, 8), (31, 15)] {
            near(
                environment.irradiance_at(x, y).unwrap(),
                [0.5, 1.0, 2.0],
                5e-3,
            );
        }
    }

    #[test]
    fn a_bright_upper_half_lights_up_facing_normals_fully_sideways_ones_by_half_and_down_ones_not_at_all()
     {
        let environment = painted(128, |_, v| if v < 0.5 { [1.0; 3] } else { [0.0; 3] });
        let at = |direction: Vec3| {
            let (u, v) = (
                0.5 - direction.y.atan2(direction.x) / TAU,
                direction.z.clamp(-1.0, 1.0).acos() / PI,
            );
            let x = (u * IRRADIANCE_WIDTH as f32) as u32 % IRRADIANCE_WIDTH;
            let y = ((v * IRRADIANCE_HEIGHT as f32) as u32).min(IRRADIANCE_HEIGHT - 1);
            environment.irradiance_at(x, y).unwrap()
        };

        let up = at(Vec3::Z);
        let side = at(Vec3::X);
        let down = at(Vec3::NEG_Z);

        assert!(up[0] > 0.93, "{up:?}");
        assert!((side[0] - 0.5).abs() < 0.1, "{side:?}");
        assert!(down[0] < 0.07, "{down:?}");
    }

    #[test]
    fn a_light_on_one_side_lights_the_normals_toward_it_more_than_those_turned_away() {
        let environment = painted(128, |u, v| {
            if (0.4..0.6).contains(&u) && (0.4..0.6).contains(&v) {
                [50.0; 3]
            } else {
                [0.05; 3]
            }
        });
        let facing = environment.irradiance_at(16, 8).unwrap();
        let away = environment.irradiance_at(0, 8).unwrap();

        assert!(facing[0] > away[0] * 10.0, "{facing:?} {away:?}");
    }

    #[test]
    fn two_environments_are_equal_only_when_they_are_the_same_one() {
        let first = uniform(32, [1.0; 3]);
        let second = uniform(32, [1.0; 3]);

        assert_eq!(first, first);
        assert_ne!(first, second);
    }
}
