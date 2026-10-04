use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result, anyhow, bail};
use caditor_document::{
    CancelToken, Document, Evaluation, FeatureResult, FeatureState, ModelEvaluator, Recompute,
};
use caditor_file::{ExportBody, ExportFormat, MeshResolution};

use crate::model::display_name;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversion {
    pub model: PathBuf,
    pub output: PathBuf,
    pub resolution: MeshResolution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    pub summary: String,
    pub warnings: Vec<String>,
}

pub fn convert(conversion: &Conversion) -> Result<Converted> {
    let Conversion {
        model,
        output,
        resolution,
    } = conversion;
    let format = ExportFormat::ALL
        .into_iter()
        .find(|format| format.matches(output))
        .ok_or_else(|| {
            anyhow!(
                "“{}” must end in .stl, .3mf, .step or .stp, the formats caditor writes",
                display_name(Some(output))
            )
        })?;
    let loaded = caditor_file::load(model)
        .map_err(|error| anyhow!("could not open “{}”: {error}", display_name(Some(model))))?;
    let mut warnings: Vec<String> = loaded.issues.clone();
    let evaluation = Recompute::default().run_without_display(
        &loaded.document,
        &ModelEvaluator,
        &CancelToken::never(),
    );
    warnings.extend(failures(&loaded.document, &evaluation));

    let bodies = bodies(&loaded.document, &evaluation);
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
    let export_bodies: Vec<ExportBody<'_>> = bodies
        .iter()
        .filter_map(|(name, result)| {
            Some(ExportBody {
                name,
                solid: &result.solid()?.solid,
            })
        })
        .collect();
    let exported = caditor_file::export_bodies(
        output,
        format,
        *resolution,
        &export_bodies,
        &CancelToken::never(),
    )
    .with_context(|| format!("could not export “{}”", display_name(Some(output))))?;
    warnings.extend(exported.left_out.iter().map(ToString::to_string));
    Ok(Converted {
        summary: format!(
            "Exported {} of “{}” to “{}”.",
            count(exported.bodies),
            display_name(Some(model)),
            display_name(Some(output))
        ),
        warnings,
    })
}

fn count(bodies: usize) -> String {
    match bodies {
        1 => "1 body".to_owned(),
        bodies => format!("{bodies} bodies"),
    }
}

fn bodies(document: &Document, evaluation: &Evaluation) -> Vec<(String, Arc<FeatureResult>)> {
    evaluation
        .bodies()
        .filter_map(|(body, _)| {
            let result = evaluation.body_result(body)?;
            result.solid()?;
            let name = document
                .feature(body)
                .map_or_else(|| "a body".to_owned(), |feature| feature.name.clone());
            Some((name, Arc::clone(result)))
        })
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

pub fn run(conversion: &Conversion) -> Result<()> {
    let converted = convert(conversion)?;
    for warning in &converted.warnings {
        eprintln!("warning: {warning}");
    }
    println!("{}", converted.summary);
    Ok(())
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
    fn what_cannot_be_converted_is_refused_in_words() {
        let folder = TempDir::new().unwrap();
        let model = saved_sample(&folder);
        let refused = |model: &Path, output: &str| {
            convert(&conversion(model, &folder.path().join(output)))
                .unwrap_err()
                .to_string()
        };

        assert_eq!(
            refused(&model, "plate.obj"),
            "“plate.obj” must end in .stl, .3mf, .step or .stp, the formats caditor writes"
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
