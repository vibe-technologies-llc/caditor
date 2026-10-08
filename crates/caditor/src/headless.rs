use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

use anyhow::{Context, Result, anyhow, bail};
use caditor_document::{
    CancelToken, Document, Evaluation, FeatureState, ModelEvaluator, Recompute,
};
use caditor_file::{
    DXF_EXTENSION, ExportBody, ExportFormat, MeshOptions, MeshResolution, PNG_EXTENSION,
    PngExportError, RgbaImage, bodies_transaction, export_png,
};
use caditor_render::{Background, GraphicsSettings, ImageRequest, OffscreenRenderer, SurfaceSize};

use crate::{
    export::{self, ExportSource, THUMBNAIL_SIZE},
    image_export::RenderedRows,
    import,
    model::display_name,
    snapshot,
};

pub const PARTIAL_EXIT_STATUS: u8 = 2;
pub const DEFAULT_IMAGE_SIZE: SurfaceSize = SurfaceSize {
    width: 1920,
    height: 1080,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversion {
    pub model: PathBuf,
    pub output: PathBuf,
    pub resolution: MeshResolution,
    pub size: Option<SurfaceSize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    pub summary: String,
    pub warnings: Vec<String>,
    pub failed_features: usize,
}

struct Opened {
    document: Document,
    issues: Vec<String>,
}

enum Target {
    Mesh(ExportFormat),
    Image,
}

impl Target {
    fn of(output: &Path) -> Option<Self> {
        let is_image = output
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case(PNG_EXTENSION));
        if is_image {
            return Some(Self::Image);
        }
        ExportFormat::ALL
            .into_iter()
            .find(|format| format.matches(output))
            .map(Self::Mesh)
    }
}

pub fn convert(conversion: &Conversion) -> Result<Converted> {
    let Conversion {
        model,
        output,
        resolution,
        size,
    } = conversion;
    let target = Target::of(output).ok_or_else(|| {
        anyhow!(
            "“{}” must end in .stl, .3mf, .obj, .glb, .step, .stp or .png, the formats caditor writes",
            display_name(Some(output))
        )
    })?;
    if size.is_some() && !matches!(target, Target::Image) {
        bail!("--size only applies to a .png export");
    }
    let Opened {
        document,
        issues: mut warnings,
    } = open(model)?;
    let evaluation = match target {
        Target::Mesh(format) if format != ExportFormat::ThreeMf => Recompute::default()
            .run_without_display(&document, &ModelEvaluator, &CancelToken::never()),
        Target::Mesh(_) | Target::Image => Recompute::default().run(
            &document,
            &ModelEvaluator,
            &CancelToken::never(),
            &|_, _| {},
        ),
    };
    let failed = failures(&document, &evaluation);
    let failed_features = failed.len();
    warnings.extend(failed);

    let bodies = bodies(&document, &evaluation);
    if bodies.is_empty() {
        let failed = if warnings.is_empty() {
            String::new()
        } else {
            format!(": {}", warnings.join("; "))
        };
        bail!(
            "“{}” has no bodies to export{failed}",
            display_name(Some(model))
        );
    }
    let exported = match target {
        Target::Mesh(format) => {
            let export_bodies: Vec<ExportBody<'_>> =
                bodies.iter().filter_map(ExportSource::exported).collect();
            let thumbnail = match format {
                ExportFormat::ThreeMf => draw_thumbnail(&document, &evaluation),
                _ => None,
            };
            let exported = caditor_file::export_bodies(
                output,
                format,
                &MeshOptions {
                    resolution: *resolution,
                    thumbnail: thumbnail.as_ref().map(|(size, pixels)| RgbaImage {
                        width: size.width,
                        height: size.height,
                        pixels,
                    }),
                    ..MeshOptions::default()
                },
                &export_bodies,
                document.properties(),
                &CancelToken::never(),
            )
            .with_context(|| format!("could not export “{}”", display_name(Some(output))))?;
            warnings.extend(exported.left_out.iter().map(ToString::to_string));
            warnings.extend(exported.moved.map(export::moved_note));
            exported.bodies
        }
        Target::Image => {
            draw_image(
                &document,
                &evaluation,
                output,
                size.unwrap_or(DEFAULT_IMAGE_SIZE),
            )?;
            bodies.len()
        }
    };
    Ok(Converted {
        summary: format!(
            "Exported {} of “{}” to “{}”.",
            count(exported),
            display_name(Some(model)),
            display_name(Some(output))
        ),
        warnings,
        failed_features,
    })
}

fn draw_thumbnail(document: &Document, evaluation: &Evaluation) -> Option<(SurfaceSize, Vec<u8>)> {
    let drawn = snapshot::take(document, evaluation, THUMBNAIL_SIZE).and_then(|snapshot| {
        let mut renderer = OffscreenRenderer::new(GraphicsSettings::default())?;
        Ok(renderer.render(&ImageRequest {
            size: THUMBNAIL_SIZE,
            view: &snapshot.view,
            scene: &snapshot.scene,
            pixels_per_point: snapshot.pixels_per_point,
            background: Background::Transparent,
        })?)
    });
    match drawn {
        Ok(bands) => export::read_thumbnail(Box::new(RenderedRows(bands))),
        Err(error) => {
            log::info!("the 3MF goes without a thumbnail: {error}");
            None
        }
    }
}

fn draw_image(
    document: &Document,
    evaluation: &Evaluation,
    output: &Path,
    size: SurfaceSize,
) -> Result<()> {
    let name = display_name(Some(output));
    let snapshot = snapshot::take(document, evaluation, size)?;
    let mut renderer = OffscreenRenderer::new(GraphicsSettings::default())
        .map_err(|error| anyhow!("no graphics adapter could draw “{name}”: {error}"))?;
    let bands = renderer
        .render(&ImageRequest {
            size,
            view: &snapshot.view,
            scene: &snapshot.scene,
            pixels_per_point: snapshot.pixels_per_point,
            background: Background::Viewport,
        })
        .map_err(|error| anyhow!("could not draw “{name}”: {error}"))?;
    export_png(output, &mut RenderedRows(bands), &CancelToken::never()).map_err(|error| match error
    {
        PngExportError::Pixels(error) => anyhow!("could not draw “{name}”: {error}"),
        PngExportError::Export(error) => {
            anyhow!(error).context(format!("could not export “{name}”"))
        }
    })
}

fn open(model: &Path) -> Result<Opened> {
    let name = display_name(Some(model));
    let is_drawing = model
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(DXF_EXTENSION));
    if is_drawing {
        bail!("“{name}” is a drawing, which holds sketches and no bodies to export");
    }
    if import::is_model(model) {
        let imported = import::read_model(model, &CancelToken::never())
            .map_err(|error| anyhow!("could not import “{name}”: {error}"))?;
        let mut document = Document::default();
        let transaction = bodies_transaction(&document, &imported.bodies, format!("Import {name}"));
        document
            .apply(transaction)
            .map_err(|error| anyhow!("could not import “{name}”: {error}"))?;
        return Ok(Opened {
            document,
            issues: imported.notes,
        });
    }
    let loaded =
        caditor_file::load(model).map_err(|error| anyhow!("could not open “{name}”: {error}"))?;
    Ok(Opened {
        document: loaded.document,
        issues: loaded.issues,
    })
}

fn count(bodies: usize) -> String {
    match bodies {
        1 => "1 body".to_owned(),
        bodies => format!("{bodies} bodies"),
    }
}

fn bodies(document: &Document, evaluation: &Evaluation) -> Vec<ExportSource> {
    evaluation
        .bodies()
        .filter_map(|(body, _)| ExportSource::of(document, evaluation, body))
        .collect()
}

fn failures(document: &Document, evaluation: &Evaluation) -> Vec<String> {
    document
        .features()
        .filter_map(|feature| {
            let status = evaluation.feature(feature.id())?;
            match &status.state {
                FeatureState::Failed(error) => Some(format!(
                    "{} failed: {} {}",
                    feature.name, error.reason, error.remedy
                )),
                _ => None,
            }
        })
        .collect()
}

pub fn run(conversion: &Conversion) -> Result<ExitCode> {
    let converted = convert(conversion)?;
    for warning in &converted.warnings {
        eprintln!("warning: {warning}");
    }
    println!("{}", converted.summary);
    Ok(if converted.failed_features == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(PARTIAL_EXIT_STATUS)
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::TempDir;

    use super::*;
    use crate::samples::Sample;

    fn saved_sample(folder: &TempDir) -> PathBuf {
        let path = folder.path().join("plate.caditor");
        let document = Sample::Plate.document().unwrap();
        caditor_file::save(&document, &path, false).unwrap();
        path
    }

    fn conversion(model: &Path, output: &Path) -> Conversion {
        Conversion {
            model: model.to_owned(),
            output: output.to_owned(),
            resolution: MeshResolution::Coarse,
            size: None,
        }
    }

    #[test]
    fn a_saved_model_converts_to_every_format_without_a_window() {
        let folder = TempDir::new().unwrap();
        let model = saved_sample(&folder);

        for (name, start) in [
            ("plate.step", b"ISO-10303-21;".as_slice()),
            ("plate.stp", b"ISO-10303-21;".as_slice()),
            ("plate.3mf", b"PK".as_slice()),
        ] {
            let output = folder.path().join(name);

            let converted = convert(&conversion(&model, &output)).unwrap();

            assert!(converted.warnings.is_empty(), "{:?}", converted.warnings);
            assert!(
                converted
                    .summary
                    .starts_with("Exported 1 body of “plate.caditor”")
            );
            assert!(std::fs::read(&output).unwrap().starts_with(start), "{name}");
        }
        let stl = folder.path().join("plate.stl");
        convert(&conversion(&model, &stl)).unwrap();
        assert!(std::fs::metadata(&stl).unwrap().len() > 84);
    }

    #[test]
    fn a_model_converts_to_a_png_of_the_chosen_size_without_a_window() {
        let folder = TempDir::new().unwrap();
        let model = saved_sample(&folder);
        let output = folder.path().join("plate.png");
        let size = SurfaceSize {
            width: 320,
            height: 200,
        };

        let converted = convert(&Conversion {
            size: Some(size),
            ..conversion(&model, &output)
        });

        let Ok(converted) = converted else {
            assert!(
                std::env::var_os("CADITOR_REQUIRE_GPU").is_none(),
                "{converted:?}"
            );
            return;
        };
        let bytes = std::fs::read(&output).unwrap();
        let decoded = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let info = decoded.info();

        assert!(converted.warnings.is_empty(), "{:?}", converted.warnings);
        assert!(
            converted
                .summary
                .starts_with("Exported 1 body of “plate.caditor” to “plate.png”")
        );
        assert_eq!((info.width, info.height), (320, 200));
    }

    #[test]
    fn a_3mf_carries_a_thumbnail_when_a_graphics_adapter_can_draw_one() {
        let folder = TempDir::new().unwrap();
        let model = saved_sample(&folder);
        let output = folder.path().join("plate.3mf");

        let converted = convert(&conversion(&model, &output)).unwrap();
        let package = std::fs::read(&output).unwrap();
        let thumbnail = b"Metadata/thumbnail.png";
        let has_thumbnail = package
            .windows(thumbnail.len())
            .any(|name| name == thumbnail);

        assert!(converted.warnings.is_empty(), "{:?}", converted.warnings);
        assert!(has_thumbnail || std::env::var_os("CADITOR_REQUIRE_GPU").is_none());
    }

    #[test]
    fn a_size_is_refused_for_anything_but_an_image() {
        let folder = TempDir::new().unwrap();
        let model = saved_sample(&folder);

        let refused = convert(&Conversion {
            size: Some(DEFAULT_IMAGE_SIZE),
            ..conversion(&model, &folder.path().join("plate.stl"))
        })
        .unwrap_err()
        .to_string();

        assert_eq!(refused, "--size only applies to a .png export");
    }

    #[test]
    fn a_step_file_converts_like_a_model() {
        let folder = TempDir::new().unwrap();
        let model = saved_sample(&folder);
        let step = folder.path().join("plate.step");
        convert(&conversion(&model, &step)).unwrap();

        let stl = folder.path().join("again.stl");
        let converted = convert(&conversion(&step, &stl)).unwrap();

        assert_eq!(converted.failed_features, 0);
        assert!(
            converted
                .summary
                .starts_with("Exported 1 body of “plate.step”")
        );
        assert!(std::fs::metadata(&stl).unwrap().len() > 84);
    }

    #[test]
    fn a_drawing_is_refused_as_holding_no_bodies() {
        let folder = TempDir::new().unwrap();
        let drawing = folder.path().join("outline.dxf");
        std::fs::write(&drawing, "0\nEOF\n").unwrap();

        let refused = convert(&conversion(&drawing, &folder.path().join("out.step")))
            .unwrap_err()
            .to_string();

        assert_eq!(
            refused,
            "“outline.dxf” is a drawing, which holds sketches and no bodies to export"
        );
    }

    #[test]
    fn what_cannot_be_converted_is_refused_in_words() {
        let folder = TempDir::new().unwrap();
        let model = saved_sample(&folder);
        let refused = |model: &Path, output: &str| {
            convert(&conversion(model, &folder.path().join(output)))
                .unwrap_err()
                .to_string()
        };

        assert_eq!(
            refused(&model, "plate.fbx"),
            "“plate.fbx” must end in .stl, .3mf, .obj, .glb, .step, .stp or .png, the formats caditor writes"
        );
        assert!(
            refused(&folder.path().join("missing.caditor"), "out.step")
                .starts_with("could not open “missing.caditor”")
        );

        let empty = folder.path().join("empty.caditor");
        caditor_file::save(&Document::default(), &empty, false).unwrap();
        assert_eq!(
            refused(&empty, "out.step"),
            "“empty.caditor” has no bodies to export"
        );
        assert!(!folder.path().join("out.step").exists());
    }
}
