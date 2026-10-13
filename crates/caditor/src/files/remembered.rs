use std::path::PathBuf;

use caditor_file::{
    Construction, ExportFormat, MeshResolution, Settings, SketchFormat, StlEncoding,
};
use caditor_render::{Background, SurfaceSize};

use crate::{
    drawing_export::{DrawingChoices, Layout},
    export::{ExportChoices, ExportThreads},
    image_export::{ImageChoices, ImageScale, SizeChoice},
};

pub const REMEMBER_KEY: &str = "export.remember";
const MODEL_FORMAT_KEY: &str = "export.model.format";
const RESOLUTION_KEY: &str = "export.model.resolution";
const STL_KEY: &str = "export.model.stl";
const THREADS_KEY: &str = "export.model.threads";
const DRAWING_FORMAT_KEY: &str = "export.drawing.format";
const LAYOUT_KEY: &str = "export.drawing.layout";
const SHEET_WIDTH_KEY: &str = "export.drawing.sheet_width";
const SPACING_KEY: &str = "export.drawing.spacing";
const TURNS_KEY: &str = "export.drawing.turns";
const ANNOTATIONS_KEY: &str = "export.drawing.annotations";
const CONSTRUCTION_KEY: &str = "export.drawing.construction";
const IMAGE_SIZE_KEY: &str = "export.image.size";
const IMAGE_WIDTH_KEY: &str = "export.image.width";
const IMAGE_HEIGHT_KEY: &str = "export.image.height";
const IMAGE_SCALE_KEY: &str = "export.image.scale";
const BACKGROUND_KEY: &str = "export.image.background";
const MODEL_FOLDER_KEY: &str = "export.folder.model";
const DRAWING_FOLDER_KEY: &str = "export.folder.drawing";
const IMAGE_FOLDER_KEY: &str = "export.folder.image";
const LONGEST_REMEMBERED_SIDE: f64 = 65_536.0;
const WIDEST_REMEMBERED_SHEET: f64 = 100_000.0;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RememberedExports {
    pub model: ExportChoices,
    pub drawing: DrawingChoices,
    pub construction: Construction,
    pub image: ImageChoices,
    pub folders: ExportFolders,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExportFolders {
    pub model: Option<PathBuf>,
    pub drawing: Option<PathBuf>,
    pub image: Option<PathBuf>,
}

impl RememberedExports {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn from_settings(raw: &Settings) -> Self {
        let defaults = Self::default();
        let text = |key: &str| raw.text(key);
        let folder = |key: &str| text(key).filter(|path| !path.is_empty()).map(PathBuf::from);
        let length = |key: &str, default: f64, most: f64| {
            raw.number(key)
                .filter(|value| value.is_finite() && *value > 0.0 && *value <= most)
                .unwrap_or(default)
        };
        let side = |key: &str, default: u32| {
            raw.number(key)
                .filter(|value| value.fract() == 0.0 && *value >= 1.0)
                .filter(|value| *value <= LONGEST_REMEMBERED_SIDE)
                .map_or(default, |value| value as u32)
        };
        Self {
            model: ExportChoices {
                format: ExportFormat::ALL
                    .into_iter()
                    .find(|format| text(MODEL_FORMAT_KEY) == Some(format.extension()))
                    .unwrap_or_default(),
                resolution: MeshResolution::ALL
                    .into_iter()
                    .find(|resolution| text(RESOLUTION_KEY) == Some(resolution_key(*resolution)))
                    .unwrap_or_default(),
                stl: StlEncoding::ALL
                    .into_iter()
                    .find(|stl| text(STL_KEY) == Some(stl_key(*stl)))
                    .unwrap_or_default(),
                every_configuration: false,
                threads: ExportThreads::ALL
                    .into_iter()
                    .find(|threads| text(THREADS_KEY) == Some(threads_key(*threads)))
                    .unwrap_or_default(),
            },
            drawing: DrawingChoices {
                format: SketchFormat::ALL
                    .into_iter()
                    .find(|format| text(DRAWING_FORMAT_KEY) == Some(format.extension()))
                    .unwrap_or_default(),
                layout: match text(LAYOUT_KEY) {
                    Some("nested") => Layout::Nested,
                    Some(_) | None => Layout::SideBySide,
                },
                sheet_width: length(
                    SHEET_WIDTH_KEY,
                    defaults.drawing.sheet_width,
                    WIDEST_REMEMBERED_SHEET,
                ),
                spacing: length(
                    SPACING_KEY,
                    defaults.drawing.spacing,
                    WIDEST_REMEMBERED_SHEET,
                ),
                turns: raw.flag(TURNS_KEY).unwrap_or(defaults.drawing.turns),
                annotations: raw
                    .flag(ANNOTATIONS_KEY)
                    .unwrap_or(defaults.drawing.annotations),
            },
            construction: match raw.flag(CONSTRUCTION_KEY) {
                Some(true) => Construction::OnLayer,
                Some(false) | None => Construction::LeftOut,
            },
            image: ImageChoices {
                size: match text(IMAGE_SIZE_KEY) {
                    Some("custom") => SizeChoice::Custom,
                    Some(_) | None => SizeChoice::View,
                },
                custom: SurfaceSize {
                    width: side(IMAGE_WIDTH_KEY, defaults.image.custom.width),
                    height: side(IMAGE_HEIGHT_KEY, defaults.image.custom.height),
                },
                scale: ImageScale::ALL
                    .into_iter()
                    .find(|scale| text(IMAGE_SCALE_KEY) == Some(scale_key(*scale)))
                    .unwrap_or_default(),
                background: match text(BACKGROUND_KEY) {
                    Some("transparent") => Background::Transparent,
                    Some(_) | None => Background::Viewport,
                },
            },
            folders: ExportFolders {
                model: folder(MODEL_FOLDER_KEY),
                drawing: folder(DRAWING_FOLDER_KEY),
                image: folder(IMAGE_FOLDER_KEY),
            },
        }
    }

    pub fn write(&self, settings: &mut Settings) {
        let defaults = Self::default();
        let mut text = |key: &str, value: &str, default: &str| {
            if value == default {
                settings.remove(key);
            } else {
                settings.set_text(key, value);
            }
        };
        let model = self.model;
        text(
            MODEL_FORMAT_KEY,
            model.format.extension(),
            defaults.model.format.extension(),
        );
        text(
            RESOLUTION_KEY,
            resolution_key(model.resolution),
            resolution_key(defaults.model.resolution),
        );
        text(STL_KEY, stl_key(model.stl), stl_key(defaults.model.stl));
        text(
            THREADS_KEY,
            threads_key(model.threads),
            threads_key(defaults.model.threads),
        );
        let drawing = self.drawing;
        text(
            DRAWING_FORMAT_KEY,
            drawing.format.extension(),
            defaults.drawing.format.extension(),
        );
        text(
            LAYOUT_KEY,
            layout_key(drawing.layout),
            layout_key(defaults.drawing.layout),
        );
        let image = self.image;
        text(
            IMAGE_SIZE_KEY,
            size_key(image.size),
            size_key(defaults.image.size),
        );
        text(
            IMAGE_SCALE_KEY,
            scale_key(image.scale),
            scale_key(defaults.image.scale),
        );
        text(
            BACKGROUND_KEY,
            background_key(image.background),
            background_key(defaults.image.background),
        );
        for (key, folder) in [
            (MODEL_FOLDER_KEY, &self.folders.model),
            (DRAWING_FOLDER_KEY, &self.folders.drawing),
            (IMAGE_FOLDER_KEY, &self.folders.image),
        ] {
            match folder.as_deref().and_then(|folder| folder.to_str()) {
                Some(path) => text(key, path, ""),
                None => text(key, "", ""),
            }
        }
        let mut number = |key: &str, value: f64, default: f64| {
            if value == default {
                settings.remove(key);
            } else {
                settings.set_number(key, value);
            }
        };
        number(
            SHEET_WIDTH_KEY,
            drawing.sheet_width,
            defaults.drawing.sheet_width,
        );
        number(SPACING_KEY, drawing.spacing, defaults.drawing.spacing);
        number(
            IMAGE_WIDTH_KEY,
            f64::from(image.custom.width),
            f64::from(defaults.image.custom.width),
        );
        number(
            IMAGE_HEIGHT_KEY,
            f64::from(image.custom.height),
            f64::from(defaults.image.custom.height),
        );
        let mut flag = |key: &str, value: bool, default: bool| {
            if value == default {
                settings.remove(key);
            } else {
                settings.set_flag(key, value);
            }
        };
        flag(TURNS_KEY, drawing.turns, defaults.drawing.turns);
        flag(
            ANNOTATIONS_KEY,
            drawing.annotations,
            defaults.drawing.annotations,
        );
        flag(
            CONSTRUCTION_KEY,
            self.construction == Construction::OnLayer,
            false,
        );
    }
}

fn resolution_key(resolution: MeshResolution) -> &'static str {
    match resolution {
        MeshResolution::Coarse => "coarse",
        MeshResolution::Standard => "standard",
        MeshResolution::Fine => "fine",
    }
}

fn stl_key(stl: StlEncoding) -> &'static str {
    match stl {
        StlEncoding::Binary => "binary",
        StlEncoding::Text => "text",
    }
}

fn threads_key(threads: ExportThreads) -> &'static str {
    match threads {
        ExportThreads::Modelled => "modelled",
        ExportThreads::AsShown => "as_shown",
    }
}

fn layout_key(layout: Layout) -> &'static str {
    match layout {
        Layout::SideBySide => "side_by_side",
        Layout::Nested => "nested",
    }
}

fn size_key(size: SizeChoice) -> &'static str {
    match size {
        SizeChoice::View => "view",
        SizeChoice::Custom => "custom",
    }
}

fn scale_key(scale: ImageScale) -> &'static str {
    match scale {
        ImageScale::One => "1",
        ImageScale::Two => "2",
        ImageScale::Four => "4",
    }
}

fn background_key(background: Background) -> &'static str {
    match background {
        Background::Viewport => "view",
        Background::Transparent => "transparent",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembered_exports_round_trip_and_odd_or_missing_keys_read_as_the_defaults() {
        let remembered = RememberedExports {
            model: ExportChoices {
                format: ExportFormat::ThreeMf,
                resolution: MeshResolution::Fine,
                stl: StlEncoding::Text,
                every_configuration: false,
                threads: ExportThreads::AsShown,
            },
            drawing: DrawingChoices {
                format: SketchFormat::Svg,
                layout: Layout::Nested,
                sheet_width: 420.0,
                spacing: 2.5,
                turns: false,
                annotations: true,
            },
            construction: Construction::OnLayer,
            image: ImageChoices {
                size: SizeChoice::Custom,
                custom: SurfaceSize {
                    width: 800,
                    height: 600,
                },
                scale: ImageScale::Two,
                background: Background::Transparent,
            },
            folders: ExportFolders {
                model: Some(PathBuf::from("/models/out")),
                drawing: Some(PathBuf::from("/drawings")),
                image: None,
            },
        };
        let mut settings = Settings::default();
        remembered.write(&mut settings);

        assert_eq!(RememberedExports::from_settings(&settings), remembered);
        assert_eq!(settings.text(MODEL_FORMAT_KEY), Some("3mf"));
        assert_eq!(settings.text(DRAWING_FORMAT_KEY), Some("svg"));
        assert_eq!(settings.text(IMAGE_FOLDER_KEY), None);

        RememberedExports::default().write(&mut settings);
        assert!(settings.keys_under("export.").is_empty());

        let mut odd = Settings::default();
        odd.set_text(MODEL_FORMAT_KEY, "from a newer caditor");
        odd.set_number(SHEET_WIDTH_KEY, -3.0);
        odd.set_number(IMAGE_WIDTH_KEY, 1.5);
        odd.set_text(IMAGE_SCALE_KEY, "16");
        assert!(RememberedExports::from_settings(&odd).is_empty());
        assert!(RememberedExports::from_settings(&Settings::default()).is_empty());
    }
}
