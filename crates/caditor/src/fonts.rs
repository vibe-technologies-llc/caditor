use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily, FontTweak, epaint::text::VariationCoords};

use crate::{font_fallbacks::FallbackFont, icon_font};

pub const INTER: &[u8] = include_bytes!("../assets/fonts/InterVariable.ttf");
const ICONS: &str = "phosphor";
const WEIGHT_AXIS: &[u8; 4] = b"wght";

pub const MEDIUM: &str = "medium";
pub const SEMIBOLD: &str = "semibold";
pub const ICON_FAMILY: &str = "icons";

struct Weight {
    font: &'static str,
    family: Option<&'static str>,
    value: f32,
}

const WEIGHTS: [Weight; 3] = [
    Weight {
        font: "inter",
        family: None,
        value: 400.0,
    },
    Weight {
        font: "inter-medium",
        family: Some(MEDIUM),
        value: 500.0,
    },
    Weight {
        font: "inter-semibold",
        family: Some(SEMIBOLD),
        value: 600.0,
    },
];

pub fn medium() -> FontFamily {
    FontFamily::Name(MEDIUM.into())
}

pub fn semibold() -> FontFamily {
    FontFamily::Name(SEMIBOLD.into())
}

pub fn icons() -> FontFamily {
    FontFamily::Name(ICON_FAMILY.into())
}

pub fn installed(ctx: &egui::Context) -> bool {
    ctx.fonts(|fonts| fonts.definitions().families.contains_key(&semibold()))
}

pub fn definitions_with(scripts: &[FallbackFont]) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    for script in scripts {
        fonts
            .font_data
            .insert(script.key.clone(), Arc::clone(&script.data));
    }
    let script_keys = scripts.iter().map(|script| script.key.clone());
    let fallbacks: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .into_iter()
        .flatten()
        .filter(|key| *key != ICONS)
        .cloned()
        .chain(script_keys.clone())
        .collect();
    if let Some(monospace) = fonts.families.get_mut(&FontFamily::Monospace) {
        monospace.extend(script_keys);
    }
    for weight in WEIGHTS {
        let tweak = FontTweak {
            coords: VariationCoords::new([(WEIGHT_AXIS, weight.value)]),
            ..FontTweak::default()
        };
        fonts.font_data.insert(
            weight.font.to_owned(),
            Arc::new(FontData::from_static(INTER).tweak(tweak)),
        );
        let family = weight.family.map_or(FontFamily::Proportional, |name| {
            FontFamily::Name(name.into())
        });
        let keys = std::iter::once(weight.font.to_owned())
            .chain(fallbacks.iter().cloned())
            .collect();
        fonts.families.insert(family, keys);
    }
    fonts.font_data.insert(
        icon_font::FONT_NAME.to_owned(),
        Arc::new(FontData::from_owned(icon_font::font())),
    );
    let icon_keys = [
        ICONS.to_owned(),
        icon_font::FONT_NAME.to_owned(),
        WEIGHTS[0].font.to_owned(),
    ]
    .into_iter()
    .chain(fallbacks.iter().cloned())
    .collect();
    fonts.families.insert(icons(), icon_keys);
    fonts.families.remove(&FontFamily::Name(ICONS.into()));
    fonts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_starts_with_inter_and_icons_with_phosphor() {
        let fonts = definitions_with(&[]);
        for (family, font) in [
            (FontFamily::Proportional, "inter"),
            (medium(), "inter-medium"),
            (semibold(), "inter-semibold"),
            (icons(), ICONS),
        ] {
            let keys = fonts.families.get(&family).unwrap();
            assert_eq!(keys.first().map(String::as_str), Some(font));
        }
        let proportional = fonts.families.get(&FontFamily::Proportional).unwrap();
        assert!(!proportional.iter().any(|key| key == ICONS));
    }

    #[test]
    fn inter_and_icons_render() {
        let context = egui::Context::default();
        context.set_fonts(definitions_with(&[]));
        let mut output = context.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        context.fonts_mut(|fonts| {
            for family in [FontFamily::Proportional, medium(), semibold()] {
                let font = egui::FontId::new(14.0, family);
                assert!(fonts.has_glyphs(&font, "Ag0°›…"));
            }
            let icons = egui::FontId::new(14.0, icons());
            assert!(fonts.has_glyphs(&icons, egui_phosphor::regular::ARROW_U_UP_LEFT));
            for glyph in [
                icon_font::FILLET,
                icon_font::CHAMFER,
                icon_font::SHELL,
                icon_font::EXTRUDE,
                icon_font::REVOLVE,
                icon_font::LINEAR_PATTERN,
                icon_font::CIRCULAR_PATTERN,
            ] {
                assert!(fonts.has_glyphs(&icons, glyph));
            }
        });
    }
}
