use egui::Color32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deficiency {
    Protanopia,
    Deuteranopia,
    Tritanopia,
}

impl Deficiency {
    pub const ALL: [Self; 3] = [Self::Protanopia, Self::Deuteranopia, Self::Tritanopia];

    fn matrix(self) -> [[f32; 3]; 3] {
        match self {
            Self::Protanopia => [
                [0.152_286, 1.052_583, -0.204_868],
                [0.114_503, 0.786_281, 0.099_216],
                [-0.003_882, -0.048_116, 1.051_998],
            ],
            Self::Deuteranopia => [
                [0.367_322, 0.860_646, -0.227_968],
                [0.280_085, 0.672_501, 0.047_413],
                [-0.011_820, 0.042_940, 0.968_881],
            ],
            Self::Tritanopia => [
                [1.255_528, -0.076_749, -0.178_779],
                [-0.078_411, 0.930_809, 0.147_602],
                [0.004_733, 0.691_367, 0.303_900],
            ],
        }
    }
}

fn to_linear(value: u8) -> f32 {
    let value = f32::from(value) / 255.0;
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_rgb(color: Color32) -> [f32; 3] {
    [
        to_linear(color.r()),
        to_linear(color.g()),
        to_linear(color.b()),
    ]
}

fn through(matrix: [[f32; 3]; 3], rgb: [f32; 3]) -> [f32; 3] {
    matrix.map(|row| {
        row.iter()
            .zip(rgb)
            .map(|(weight, channel)| weight * channel)
            .sum::<f32>()
            .clamp(0.0, 1.0)
    })
}

fn xyz(rgb: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = rgb;
    [
        0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b,
        0.212_672_9 * r + 0.715_152_2 * g + 0.072_175 * b,
        0.019_333_9 * r + 0.119_192 * g + 0.950_304_1 * b,
    ]
}

fn lab(xyz: [f32; 3]) -> [f32; 3] {
    const WHITE: [f32; 3] = [0.950_47, 1.0, 1.088_83];
    let curve = |ratio: f32| {
        if ratio > 0.008_856 {
            ratio.cbrt()
        } else {
            7.787 * ratio + 16.0 / 116.0
        }
    };
    let [x, y, z] = [
        curve(xyz[0] / WHITE[0]),
        curve(xyz[1] / WHITE[1]),
        curve(xyz[2] / WHITE[2]),
    ];
    [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
}

pub fn seen_by(deficiency: Deficiency, color: Color32) -> [f32; 3] {
    lab(xyz(through(deficiency.matrix(), linear_rgb(color))))
}

pub fn normal_vision(color: Color32) -> [f32; 3] {
    lab(xyz(linear_rgb(color)))
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(first, second)| (first - second).powi(2))
        .sum::<f32>()
        .sqrt()
}

pub fn separation(deficiency: Deficiency, a: Color32, b: Color32) -> f32 {
    distance(seen_by(deficiency, a), seen_by(deficiency, b))
}

pub fn separation_normally(a: Color32, b: Color32) -> f32 {
    distance(normal_vision(a), normal_vision(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        appearance::{DARK, GRAPHITE, LIGHT, MIDNIGHT, PAPER, SLATE, STUDIO, Tokens},
        widgets::Tone,
    };

    #[test]
    fn grey_is_unchanged_by_every_deficiency() {
        let grey = Color32::from_rgb(128, 128, 128);

        for deficiency in Deficiency::ALL {
            let seen = seen_by(deficiency, grey);
            let normal = normal_vision(grey);

            assert!(distance(seen, normal) < 2.0, "{deficiency:?}: {seen:?}");
        }
    }

    #[test]
    fn the_error_and_success_colours_collapse_for_red_green_deficiencies_but_not_normally() {
        let (error, success) = (DARK.error, DARK.success);

        assert!(separation_normally(error, success) > 90.0);
        assert!(separation(Deficiency::Deuteranopia, error, success) < 25.0);
        assert!(separation(Deficiency::Protanopia, error, success) < 25.0);
        assert!(separation(Deficiency::Tritanopia, error, success) > 90.0);
    }

    #[test]
    fn white_and_black_stay_far_apart() {
        for deficiency in Deficiency::ALL {
            assert!(separation(deficiency, Color32::WHITE, Color32::BLACK) > 90.0);
        }
    }

    const TONES: [Tone; 4] = [Tone::Info, Tone::Success, Tone::Warning, Tone::Error];
    const HARD_TO_TELL_APART: f32 = 30.0;

    fn shipped() -> Vec<(&'static str, Tokens)> {
        vec![
            ("dark", DARK),
            ("light", LIGHT),
            ("midnight", MIDNIGHT),
            ("graphite", GRAPHITE),
            ("paper", PAPER),
            ("slate", SLATE),
            ("studio", STUDIO),
            ("dark high contrast", *Tokens::of(true, true)),
            ("light high contrast", *Tokens::of(false, true)),
        ]
    }

    #[test]
    fn status_tones_that_look_alike_to_a_colour_blind_eye_have_icons_that_differ() {
        let mut alike = 0;
        for (theme, tokens) in shipped() {
            for (index, first) in TONES.iter().enumerate() {
                for second in TONES.iter().skip(index + 1) {
                    let (a, b) = (first.color(&tokens), second.color(&tokens));
                    for deficiency in Deficiency::ALL {
                        if separation(deficiency, a, b) < HARD_TO_TELL_APART {
                            alike += 1;
                            assert_ne!(
                                first.icon(),
                                second.icon(),
                                "{theme}: {first:?} and {second:?} look alike under {deficiency:?} and share an icon"
                            );
                        }
                    }
                }
            }
        }
        assert!(
            alike > 0,
            "the guard found no look-alike pair, so it guards nothing"
        );
    }

    #[test]
    fn every_status_tone_but_the_neutral_has_an_icon_of_its_own() {
        let icons: Vec<&str> = TONES.iter().map(|tone| tone.icon()).collect();

        for (index, icon) in icons.iter().enumerate() {
            assert!(
                !icons.iter().skip(index + 1).any(|other| other == icon),
                "{icon:?} serves two tones"
            );
        }
    }
}
