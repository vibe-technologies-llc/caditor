use std::ops::RangeInclusive;

use caditor_file::Settings;
use caditor_render::LightDirection;

use crate::preferences::Choice;

const FRAME_KEY: &str = "rendering.light_follows";
const AZIMUTH_KEY: &str = "rendering.azimuth";
const ELEVATION_KEY: &str = "rendering.elevation";
const STRENGTH_KEY: &str = "rendering.strength";
const TEMPERATURE_KEY: &str = "rendering.temperature";
const ENVIRONMENT_KEY: &str = "rendering.environment";
const BOUNCE_KEY: &str = "rendering.bounce";
const EXPOSURE_KEY: &str = "rendering.exposure";
const SHADOWS_KEY: &str = "rendering.shadows";
pub const AZIMUTH: RangeInclusive<f64> = -180.0..=180.0;
pub const ELEVATION: RangeInclusive<f64> = 0.0..=90.0;
pub const STRENGTH: RangeInclusive<f64> = 0.0..=10.0;
pub const TEMPERATURE: RangeInclusive<f64> = 1800.0..=12000.0;
pub const ENVIRONMENT: RangeInclusive<f64> = 0.0..=2.0;
pub const BOUNCE: RangeInclusive<f64> = 0.0..=3.0;
pub const EXPOSURE: RangeInclusive<f64> = -4.0..=4.0;
const LUMINANCE: [f64; 3] = [0.2126, 0.7152, 0.0722];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LightFrame {
    #[default]
    View,
    Model,
}

impl Choice for LightFrame {
    const ALL: &'static [Self] = &[Self::View, Self::Model];

    fn key(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::Model => "model",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::View => "The view",
            Self::Model => "The model",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::View => {
                "The light turns with the camera, so the side you look at stays lit as you orbit"
            }
            Self::Model => {
                "The light stays put like the sun, Z up, so orbiting shows the model's shaded side"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lighting {
    pub frame: LightFrame,
    pub azimuth: f64,
    pub elevation: f64,
    pub strength: f64,
    pub temperature: f64,
    pub environment: f64,
    pub bounce: f64,
    pub exposure: f64,
    pub shadows: bool,
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            frame: LightFrame::View,
            azimuth: -35.0,
            elevation: 40.0,
            strength: 3.0,
            temperature: 5600.0,
            environment: 1.0,
            bounce: 1.0,
            exposure: 0.0,
            shadows: true,
        }
    }
}

impl Lighting {
    pub fn from_settings(raw: &Settings) -> Self {
        let defaults = Self::default();
        let number = |key, range: RangeInclusive<f64>, default: f64| {
            raw.number(key)
                .map_or(default, |value| value.clamp(*range.start(), *range.end()))
        };
        Self {
            frame: LightFrame::read(raw, FRAME_KEY),
            azimuth: number(AZIMUTH_KEY, AZIMUTH, defaults.azimuth),
            elevation: number(ELEVATION_KEY, ELEVATION, defaults.elevation),
            strength: number(STRENGTH_KEY, STRENGTH, defaults.strength),
            temperature: number(TEMPERATURE_KEY, TEMPERATURE, defaults.temperature),
            environment: number(ENVIRONMENT_KEY, ENVIRONMENT, defaults.environment),
            bounce: number(BOUNCE_KEY, BOUNCE, defaults.bounce),
            exposure: number(EXPOSURE_KEY, EXPOSURE, defaults.exposure),
            shadows: raw.flag(SHADOWS_KEY).unwrap_or(defaults.shadows),
        }
    }

    pub fn write(&self, settings: &mut Settings) {
        settings.set_text(FRAME_KEY, self.frame.key());
        settings.set_number(AZIMUTH_KEY, self.azimuth);
        settings.set_number(ELEVATION_KEY, self.elevation);
        settings.set_number(STRENGTH_KEY, self.strength);
        settings.set_number(TEMPERATURE_KEY, self.temperature);
        settings.set_number(ENVIRONMENT_KEY, self.environment);
        settings.set_number(BOUNCE_KEY, self.bounce);
        settings.set_number(EXPOSURE_KEY, self.exposure);
        settings.set_flag(SHADOWS_KEY, self.shadows);
    }

    pub fn render(&self) -> caditor_render::Lighting {
        let azimuth = self.azimuth.to_radians();
        let elevation = self.elevation.to_radians();
        caditor_render::Lighting {
            direction: match self.frame {
                LightFrame::View => LightDirection::View { azimuth, elevation },
                LightFrame::Model => LightDirection::Model { azimuth, elevation },
            },
            colour: light_colour(self.temperature).map(|channel| channel as f32),
            strength: self.strength as f32,
            environment: self.environment as f32,
            exposure: self.exposure as f32,
            shadows: self.shadows,
            bounce: self.bounce as f32,
        }
    }

    pub fn swatch(&self) -> [u8; 3] {
        let brightest = light_colour(self.temperature)
            .into_iter()
            .fold(f64::MIN_POSITIVE, f64::max);
        light_colour(self.temperature).map(|channel| encoded(channel / brightest))
    }
}

pub fn light_colour(kelvin: f64) -> [f64; 3] {
    let hundreds = kelvin.clamp(*TEMPERATURE.start(), *TEMPERATURE.end()) / 100.0;
    let red = if hundreds <= 66.0 {
        1.0
    } else {
        1.292_936_186 * (hundreds - 60.0).powf(-0.133_204_759_2)
    };
    let green = if hundreds <= 66.0 {
        0.390_081_578_8 * hundreds.ln() - 0.631_841_443_8
    } else {
        1.129_890_861 * (hundreds - 60.0).powf(-0.075_514_849_2)
    };
    let blue = if hundreds >= 66.0 {
        1.0
    } else if hundreds <= 19.0 {
        0.0
    } else {
        0.543_206_789_1 * (hundreds - 10.0).ln() - 1.196_254_089
    };
    let linear = [red, green, blue].map(|channel| decoded(channel.clamp(0.0, 1.0)));
    let luminance: f64 = linear
        .iter()
        .zip(LUMINANCE)
        .map(|(channel, weight)| channel * weight)
        .sum();
    linear.map(|channel| channel / luminance.max(f64::MIN_POSITIVE))
}

fn decoded(channel: f64) -> f64 {
    if channel <= 0.040_45 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn encoded(channel: f64) -> u8 {
    let channel = channel.clamp(0.0, 1.0);
    let srgb = if channel <= 0.003_130_8 {
        channel * 12.92
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lighting_round_trips_and_odd_or_missing_keys_read_as_the_defaults() {
        let lighting = Lighting {
            frame: LightFrame::Model,
            azimuth: 120.0,
            elevation: 15.0,
            strength: 5.5,
            temperature: 3200.0,
            environment: 1.25,
            bounce: 0.0,
            exposure: -1.5,
            shadows: false,
        };
        let mut settings = Settings::default();
        lighting.write(&mut settings);

        let mut odd = Settings::default();
        odd.set_text(FRAME_KEY, "sideways");
        odd.set_number(ELEVATION_KEY, 400.0);
        odd.set_text(STRENGTH_KEY, "bright");

        assert_eq!(Lighting::from_settings(&settings), lighting);
        assert_eq!(
            Lighting::from_settings(&Settings::default()),
            Lighting::default()
        );
        let read = Lighting::from_settings(&odd);
        assert_eq!(read.frame, LightFrame::View);
        assert_eq!(read.elevation, *ELEVATION.end());
        assert_eq!(read.strength, Lighting::default().strength);
    }

    #[test]
    fn a_warm_light_is_red_a_cool_one_blue_and_each_has_unit_luminance() {
        let warm = light_colour(2000.0);
        let daylight = light_colour(6500.0);
        let cool = light_colour(11000.0);
        let luminance = |colour: [f64; 3]| -> f64 {
            colour
                .iter()
                .zip(LUMINANCE)
                .map(|(channel, weight)| channel * weight)
                .sum()
        };

        assert!(warm[0] > warm[2] * 3.0, "{warm:?}");
        assert!(cool[2] > cool[0], "{cool:?}");
        for channel in daylight {
            assert!((channel - 1.0).abs() < 0.1, "{daylight:?}");
        }
        for colour in [warm, daylight, cool] {
            assert!((luminance(colour) - 1.0).abs() < 1e-9, "{colour:?}");
        }
    }

    #[test]
    fn the_light_follows_the_frame_chosen_and_turns_degrees_into_radians() {
        let lighting = Lighting {
            frame: LightFrame::Model,
            azimuth: 90.0,
            elevation: 30.0,
            ..Lighting::default()
        };
        let LightDirection::Model { azimuth, elevation } = lighting.render().direction else {
            panic!("a model light stays in the model frame");
        };

        assert!((azimuth - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        assert!((elevation - std::f64::consts::FRAC_PI_6).abs() < 1e-12);
        assert!(matches!(
            Lighting::default().render().direction,
            LightDirection::View { .. }
        ));
    }
}
