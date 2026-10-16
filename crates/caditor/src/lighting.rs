use std::{ops::RangeInclusive, sync::Arc};

use caditor_document::Rgb;
use caditor_file::Settings;
use caditor_render::{Backdrop, Color, Environment, LightDirection, Surroundings};

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
const BACKDROP_TURN_KEY: &str = "rendering.backdrop_turn";
const BACKDROP_EXPOSURE_KEY: &str = "rendering.backdrop_exposure";
const BACKDROP_COLOUR_KEY: &str = "rendering.backdrop_colour";
const FLOOR_KEY: &str = "rendering.floor";
const OCCLUSION_KEY: &str = "rendering.occlusion";
pub const AZIMUTH: RangeInclusive<f64> = -180.0..=180.0;
pub const ELEVATION: RangeInclusive<f64> = 0.0..=90.0;
pub const STRENGTH: RangeInclusive<f64> = 0.0..=10.0;
pub const TEMPERATURE: RangeInclusive<f64> = 1800.0..=12000.0;
pub const ENVIRONMENT: RangeInclusive<f64> = 0.0..=2.0;
pub const BOUNCE: RangeInclusive<f64> = 0.0..=3.0;
pub const EXPOSURE: RangeInclusive<f64> = -4.0..=4.0;
pub const OCCLUSION: RangeInclusive<f64> = 0.0..=2.0;
pub const BACKDROP_TURN: RangeInclusive<f64> = -180.0..=180.0;
pub const BACKDROP_EXPOSURE: RangeInclusive<f64> = -4.0..=4.0;
const LUMINANCE: [f64; 3] = [0.2126, 0.7152, 0.0722];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LightFrame {
    View,
    #[default]
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
    pub occlusion: f64,
    pub exposure: f64,
    pub shadows: bool,
    pub backdrop_turn: f64,
    pub backdrop_exposure: f64,
    pub backdrop_colour: Option<Rgb>,
    pub floor: bool,
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            frame: LightFrame::Model,
            azimuth: -80.0,
            elevation: 45.0,
            strength: 3.0,
            temperature: 5600.0,
            environment: 1.0,
            bounce: 1.0,
            occlusion: 1.0,
            exposure: 0.0,
            shadows: true,
            backdrop_turn: 0.0,
            backdrop_exposure: 0.0,
            backdrop_colour: None,
            floor: false,
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
            occlusion: number(OCCLUSION_KEY, OCCLUSION, defaults.occlusion),
            exposure: number(EXPOSURE_KEY, EXPOSURE, defaults.exposure),
            shadows: raw.flag(SHADOWS_KEY).unwrap_or(defaults.shadows),
            backdrop_turn: number(BACKDROP_TURN_KEY, BACKDROP_TURN, defaults.backdrop_turn),
            backdrop_exposure: number(
                BACKDROP_EXPOSURE_KEY,
                BACKDROP_EXPOSURE,
                defaults.backdrop_exposure,
            ),
            backdrop_colour: raw.text(BACKDROP_COLOUR_KEY).and_then(Rgb::from_hex),
            floor: raw.flag(FLOOR_KEY).unwrap_or(defaults.floor),
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
        settings.set_number(OCCLUSION_KEY, self.occlusion);
        settings.set_number(EXPOSURE_KEY, self.exposure);
        settings.set_flag(SHADOWS_KEY, self.shadows);
        settings.set_number(BACKDROP_TURN_KEY, self.backdrop_turn);
        settings.set_number(BACKDROP_EXPOSURE_KEY, self.backdrop_exposure);
        match self.backdrop_colour {
            Some(colour) => settings.set_text(BACKDROP_COLOUR_KEY, &colour.hex()),
            None => settings.remove(BACKDROP_COLOUR_KEY),
        }
        settings.set_flag(FLOOR_KEY, self.floor);
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
            occlusion: self.occlusion as f32,
            surroundings: self.surroundings(),
        }
    }

    pub fn surroundings(&self) -> Surroundings {
        Surroundings {
            backdrop: self
                .backdrop_colour
                .map(|colour| Color::from_rgb8(colour.red, colour.green, colour.blue)),
            floor: self.floor,
        }
    }

    pub fn backdrop(&self, environment: Arc<Environment>) -> Backdrop {
        Backdrop {
            environment,
            turn: self.backdrop_turn.to_radians() as f32,
            stops: self.backdrop_exposure as f32,
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
            frame: LightFrame::View,
            azimuth: 120.0,
            elevation: 15.0,
            strength: 5.5,
            temperature: 3200.0,
            environment: 1.25,
            bounce: 0.0,
            occlusion: 0.5,
            exposure: -1.5,
            shadows: false,
            backdrop_turn: -45.0,
            backdrop_exposure: 1.5,
            backdrop_colour: Some(Rgb::new(0x20, 0x40, 0x80)),
            floor: true,
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
        odd.set_number(BACKDROP_TURN_KEY, 900.0);
        odd.set_text(BACKDROP_COLOUR_KEY, "blue");
        odd.set_text(FLOOR_KEY, "yes");
        let read = Lighting::from_settings(&odd);
        assert_eq!(read.backdrop_turn, *BACKDROP_TURN.end());
        assert_eq!(read.frame, LightFrame::Model);
        assert_eq!(read.backdrop_colour, None);
        assert!(!read.floor);
        assert_eq!(read.elevation, *ELEVATION.end());
        assert_eq!(read.strength, Lighting::default().strength);
    }

    #[test]
    fn a_plain_backdrop_and_the_floor_reach_the_renderer_and_forgetting_them_clears_the_keys() {
        let lighting = Lighting {
            backdrop_colour: Some(Rgb::new(0x20, 0x40, 0x80)),
            floor: true,
            ..Lighting::default()
        };
        let mut settings = Settings::default();
        lighting.write(&mut settings);
        Lighting::default().write(&mut settings);

        let surroundings = lighting.surroundings();

        assert_eq!(
            surroundings.backdrop,
            Some(Color::from_rgb8(0x20, 0x40, 0x80))
        );
        assert!(surroundings.floor);
        assert_eq!(settings.text(BACKDROP_COLOUR_KEY), None);
        assert_eq!(Lighting::from_settings(&settings), Lighting::default());
        assert_eq!(
            Lighting::default().render().surroundings,
            Surroundings::default()
        );
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
            LightDirection::Model { .. }
        ));
    }

    #[test]
    fn the_picture_turns_by_degrees_in_radians_and_brightens_by_stops() {
        let environment = Arc::new(Environment::new(32, 16, vec![1.0; 32 * 16 * 3]).unwrap());
        let lighting = Lighting {
            backdrop_turn: 90.0,
            backdrop_exposure: -2.0,
            ..Lighting::default()
        };

        let backdrop = lighting.backdrop(Arc::clone(&environment));

        assert!((backdrop.turn - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(backdrop.stops, -2.0);
        assert!(Arc::ptr_eq(&backdrop.environment, &environment));
    }
}
