use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

use anyhow::{Context, Result, anyhow, bail};
use caditor_document::{
    CancelToken, ConfigurationId, Document, EditError, Evaluation, FeatureState, ModelEvaluator,
    Recompute,
};
use caditor_file::{
    ExportBody, ExportFormat, MeshOptions, MeshResolution, PNG_EXTENSION, PngExportError,
    ReadingProgress, RgbaImage, bodies_transaction, export_png,
};
use caditor_render::{Background, GraphicsSettings, ImageRequest, OffscreenRenderer, SurfaceSize};

use crate::{
    configuration_export,
    export::{self, ExportSource, THUMBNAIL_SIZE},
    files,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ConfigurationChoice {
    #[default]
    Active,
    Named(String),
    Every,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversion {
    pub model: PathBuf,
    pub output: PathBuf,
    pub resolution: MeshResolution,
    pub size: Option<SurfaceSize>,
    pub configurations: ConfigurationChoice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    pub summary: String,
    pub warnings: Vec<String>,
    pub failed_features: usize,
    pub configurations_left_out: usize,
}

impl Converted {
    fn is_partial(&self) -> bool {
        self.failed_features > 0 || self.configurations_left_out > 0
    }
}

struct Opened {
    document: Document,
    issues: Vec<String>,
}

#[derive(Clone, Copy)]
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

struct Settings {
    target: Target,
    resolution: MeshResolution,
    size: Option<SurfaceSize>,
}

struct Written {
    bodies: usize,
    warnings: Vec<String>,
    failed_features: usize,
}

struct Variant {
    name: String,
    output: PathBuf,
    document: Result<Document, EditError>,
}

pub fn convert(conversion: &Conversion) -> Result<Converted> {
    let Conversion {
        model,
        output,
        resolution,
        size,
        configurations,
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
    let settings = Settings {
        target,
        resolution: *resolution,
        size: *size,
    };
    let Opened {
        document,
        issues: mut warnings,
    } = open(model)?;
    let model_name = display_name(Some(model));
    match configurations {
        ConfigurationChoice::Active => {
            let written = write(&document, &settings, model, output)?;
            warnings.extend(written.warnings);
            Ok(Converted {
                summary: format!(
                    "Exported {} of “{model_name}” to “{}”.",
                    count(written.bodies),
                    display_name(Some(output))
                ),
                warnings,
                failed_features: written.failed_features,
                configurations_left_out: 0,
            })
        }
        ConfigurationChoice::Named(name) => {
            let switched = switched_to(&document, name, model)?;
            let written = write(&switched, &settings, model, output)?;
            warnings.extend(written.warnings);
            Ok(Converted {
                summary: format!(
                    "Exported {} of “{model_name}” in its configuration {} to “{}”.",
                    count(written.bodies),
                    active_name(&switched),
                    display_name(Some(output))
                ),
                warnings,
                failed_features: written.failed_features,
                configurations_left_out: 0,
            })
        }
        ConfigurationChoice::Every => {
            let mut converted = Converted {
                summary: String::new(),
                warnings,
                failed_features: 0,
                configurations_left_out: 0,
            };
            let mut files = Vec::new();
            for variant in variants(&document, model, output)? {
                let written = variant
                    .document
                    .map_err(|error| anyhow!("the model could not be switched to it ({error})"))
                    .and_then(|document| write(&document, &settings, model, &variant.output));
                match written {
                    Ok(written) => {
                        converted.warnings.extend(
                            written
                                .warnings
                                .into_iter()
                                .map(|warning| format!("in “{}”: {warning}", variant.name)),
                        );
                        converted.failed_features += written.failed_features;
                        files.push(format!("“{}”", display_name(Some(&variant.output))));
                    }
                    Err(error) => {
                        converted.configurations_left_out += 1;
                        converted
                            .warnings
                            .push(format!("“{}” was not exported: {error:#}", variant.name));
                    }
                }
            }
            if files.is_empty() {
                bail!(
                    "no configuration of “{model_name}” was exported: {}",
                    converted.warnings.join("; ")
                );
            }
            converted.summary = format!(
                "Exported {} of “{model_name}” to {}.",
                configurations_count(files.len()),
                configuration_export::listed(&files)
            );
            Ok(converted)
        }
    }
}

fn active_name(document: &Document) -> String {
    document
        .active_configuration()
        .map(|row| row.name.clone())
        .unwrap_or_default()
}

fn names_of(document: &Document) -> String {
    let names: Vec<String> = document
        .configurations()
        .rows
        .iter()
        .map(|row| row.name.clone())
        .collect();
    configuration_export::listed(&names)
}

fn switched_to(document: &Document, name: &str, model: &Path) -> Result<Document> {
    let model_name = display_name(Some(model));
    let configurations = document.configurations();
    if configurations.rows.is_empty() {
        bail!("“{model_name}” has no configurations, so there is no configuration “{name}”");
    }
    let row = configurations
        .position(name.trim())
        .and_then(|index| configurations.rows.get(index))
        .ok_or_else(|| {
            anyhow!(
                "“{model_name}” has no configuration named “{name}”; its configurations are {}",
                names_of(document)
            )
        })?;
    let mut switched = document.clone();
    document
        .activating(row.id)
        .and_then(|transaction| switched.apply(transaction))
        .map_err(|error| {
            anyhow!(
                "“{model_name}” could not be switched to its configuration {}: {error}",
                row.name
            )
        })?;
    Ok(switched)
}

fn variants(document: &Document, model: &Path, output: &Path) -> Result<Vec<Variant>> {
    let rows: Vec<(ConfigurationId, String)> = document
        .configurations()
        .rows
        .iter()
        .map(|row| (row.id, row.name.clone()))
        .collect();
    if rows.is_empty() {
        bail!(
            "“{}” has no configurations to export one by one",
            display_name(Some(model))
        );
    }
    Ok(configuration_export::files_beside(output, rows)
        .into_iter()
        .map(|file| {
            let mut switched = document.clone();
            let applied = document
                .activating(file.id)
                .and_then(|transaction| switched.apply(transaction))
                .map(|_| switched);
            Variant {
                name: file.name,
                output: file.path,
                document: applied,
            }
        })
        .collect())
}

fn write(document: &Document, settings: &Settings, model: &Path, output: &Path) -> Result<Written> {
    let evaluation = match settings.target {
        Target::Mesh(format) if format != ExportFormat::ThreeMf => Recompute::default()
            .run_without_display(document, &ModelEvaluator, &CancelToken::never()),
        Target::Mesh(_) | Target::Image => {
            Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
        }
    };
    let mut warnings = failures(document, &evaluation);
    let failed_features = warnings.len();

    let bodies = bodies(document, &evaluation);
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
    let exported = match settings.target {
        Target::Mesh(format) => {
            let export_bodies: Vec<ExportBody<'_>> =
                bodies.iter().filter_map(ExportSource::exported).collect();
            let thumbnail = match format {
                ExportFormat::ThreeMf => draw_thumbnail(document, &evaluation),
                _ => None,
            };
            let exported = caditor_file::export_bodies(
                output,
                format,
                &MeshOptions {
                    resolution: settings.resolution,
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
                document,
                &evaluation,
                output,
                settings.size.unwrap_or(DEFAULT_IMAGE_SIZE),
            )?;
            bodies.len()
        }
    };
    Ok(Written {
        bodies: exported,
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
    if files::is_drawing_file(model) {
        bail!("“{name}” is a drawing, which holds sketches and no bodies to export");
    }
    if import::is_model(model) {
        let imported =
            import::read_model(model, &CancelToken::never(), &ReadingProgress::default())
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

fn configurations_count(configurations: usize) -> String {
    match configurations {
        1 => "1 configuration".to_owned(),
        configurations => format!("{configurations} configurations"),
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
    Ok(if converted.is_partial() {
        ExitCode::from(PARTIAL_EXIT_STATUS)
    } else {
        ExitCode::SUCCESS
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use caditor_document::{ConfiguredValue, Setting};
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
            configurations: ConfigurationChoice::Active,
        }
    }

    fn saved_configured_sample(folder: &TempDir) -> PathBuf {
        let path = folder.path().join("plate.caditor");
        let mut document = Sample::Plate.document().unwrap();
        let thickness = document.parameter_named("thickness").unwrap().id();
        let (added, _) = document.adding_configuration(Some("Thin"));
        document.apply(added).unwrap();
        let configure = document
            .configuring(ConfiguredValue::Parameter(thickness))
            .unwrap();
        document.apply(configure).unwrap();
        let (added, thick) = document.adding_configuration(Some("Thick/12"));
        document.apply(added).unwrap();
        let twelve = document.transaction("Parse").parse("12 mm").unwrap();
        let set = document
            .setting_configuration(
                thick,
                ConfiguredValue::Parameter(thickness),
                Setting::Expression(twelve),
            )
            .unwrap();
        document.apply(set).unwrap();
        caditor_file::save(&document, &path, false).unwrap();
        path
    }

    #[test]
    fn a_named_configuration_is_exported_instead_of_the_active_one() {
        let folder = TempDir::new().unwrap();
        let model = saved_configured_sample(&folder);
        let active = folder.path().join("active.stl");
        let thick = folder.path().join("thick.stl");

        convert(&conversion(&model, &active)).unwrap();
        let converted = convert(&Conversion {
            configurations: ConfigurationChoice::Named("thick/12".to_owned()),
            ..conversion(&model, &thick)
        })
        .unwrap();

        assert_eq!(
            converted.summary,
            "Exported 1 body of “plate.caditor” in its configuration Thick/12 to “thick.stl”."
        );
        assert_ne!(
            std::fs::read(&active).unwrap(),
            std::fs::read(&thick).unwrap()
        );
        assert_eq!(
            caditor_file::load(&model)
                .unwrap()
                .document
                .active_configuration()
                .unwrap()
                .name,
            "Thin"
        );
    }

    #[test]
    fn every_configuration_is_exported_beside_the_output_under_its_name() {
        let folder = TempDir::new().unwrap();
        let model = saved_configured_sample(&folder);
        let output = folder.path().join("plate.stp");

        let converted = convert(&Conversion {
            configurations: ConfigurationChoice::Every,
            ..conversion(&model, &output)
        })
        .unwrap();

        assert_eq!(
            converted.summary,
            "Exported 2 configurations of “plate.caditor” to “plate Thin.stp” and “plate \
             Thick-12.stp”."
        );
        assert!(!converted.is_partial());
        assert!(!output.exists());
        for name in ["plate Thin.stp", "plate Thick-12.stp"] {
            assert!(
                std::fs::read(folder.path().join(name))
                    .unwrap()
                    .starts_with(b"ISO-10303-21;")
            );
        }
    }

    #[test]
    fn configurations_that_are_not_there_are_refused_in_words() {
        let folder = TempDir::new().unwrap();
        let configured = saved_configured_sample(&folder);
        let plain = folder.path().join("plain.caditor");
        caditor_file::save(&Sample::Plate.document().unwrap(), &plain, false).unwrap();
        let refused = |model: &Path, configurations: ConfigurationChoice| {
            convert(&Conversion {
                configurations,
                ..conversion(model, &folder.path().join("out.stl"))
            })
            .unwrap_err()
            .to_string()
        };

        assert_eq!(
            refused(&configured, ConfigurationChoice::Named("M8".to_owned())),
            "“plate.caditor” has no configuration named “M8”; its configurations are Thin and \
             Thick/12"
        );
        assert_eq!(
            refused(&plain, ConfigurationChoice::Named("M8".to_owned())),
            "“plain.caditor” has no configurations, so there is no configuration “M8”"
        );
        assert_eq!(
            refused(&plain, ConfigurationChoice::Every),
            "“plain.caditor” has no configurations to export one by one"
        );
        assert!(!folder.path().join("out.stl").exists());
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
