use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use caditor_document::FeatureId;
use caditor_file::{ExportError, FaceExported, SketchExported, SketchFormat};

use crate::{
    bodies,
    commands::{Command, CommandFrame},
    feature_tree::count,
    files::FileCommand,
    model::{Action, Model, Notice, display_name},
    selection::Selection,
    sketch_placement::{self, FaceChoice},
};

pub const SKETCH_HINT: &str = "Save the curves of a sketch as a DXF or SVG drawing in millimetres";
pub const NOT_A_SKETCH: &str = "Choose a sketch in the feature tree, or edit one, to export it";
pub const NOT_SOLVED: &str = "The sketch has not been solved, so there is nothing to export yet";
pub const FACE_HINT: &str = "Save the outlines and holes of the selected flat faces as a DXF or SVG \
                             drawing in millimetres, side by side, for laser or CNC cutting";
pub const NOT_A_FACE: &str = "Select one or more flat faces of bodies to export their outlines";
const CURVED: &str = "A selected face is curved; only flat faces export as drawings";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrawingSource {
    Sketch(FeatureId),
    Faces(Vec<FaceChoice>),
}

pub fn face_name(model: &Model, choice: FaceChoice) -> Option<String> {
    let body = bodies::shown(model.evaluation(), choice.body)?;
    bodies::find_face(body, choice.face)?;
    Some(bodies::describe_face(model.document(), body, choice.face))
}

pub fn body_name(model: &Model, choice: FaceChoice) -> String {
    model
        .document()
        .feature(choice.body)
        .map_or_else(|| model.display_name(), |feature| feature.name.clone())
}

pub fn exportable_faces(
    model: &Model,
    selection: &Selection,
) -> Result<Vec<FaceChoice>, &'static str> {
    let choices: Vec<FaceChoice> = selection.iter().filter_map(FaceChoice::of).collect();
    if choices.is_empty() {
        return Err(NOT_A_FACE);
    }
    if choices
        .iter()
        .all(|choice| sketch_placement::is_flat(model, *choice))
    {
        Ok(choices)
    } else {
        Err(CURVED)
    }
}

pub fn faces_name(model: &Model, choices: &[FaceChoice]) -> Option<String> {
    match choices {
        [only] => face_name(model, *only),
        several => Some(count(several.len(), "face", "faces")),
    }
}

pub fn face_commands(
    model: &Model,
    selection: &Selection,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let exportable = exportable_faces(model, selection);
    let detail = exportable
        .as_ref()
        .ok()
        .and_then(|choices| faces_name(model, choices));
    let available = exportable.as_ref().map(|_| ()).map_err(|reason| *reason);
    if commands.invoke_detailed(Command::ExportFace, detail, &available)
        && let Ok(choices) = exportable
    {
        actions.push(Action::File(FileCommand::ExportDrawing(
            DrawingSource::Faces(choices),
        )));
    }
}

pub fn file_name(name: &str) -> String {
    format!("{name}.{}", SketchFormat::default().extension())
}

pub fn face_file_name(body: &str) -> String {
    file_name(&format!("{body} face"))
}

pub fn with_format_extension(path: PathBuf) -> PathBuf {
    if SketchFormat::of(&path).is_some() {
        return path;
    }
    let mut named = OsString::from(path.as_os_str());
    named.push(".");
    named.push(SketchFormat::default().extension());
    PathBuf::from(named)
}

pub fn finished(path: &Path, sketch: &str, result: Result<SketchExported, ExportError>) -> Notice {
    let name = display_name(Some(path));
    match result {
        Ok(exported) => {
            let drawn = exported.curves + exported.points + exported.construction;
            let summary = format!(
                "Exported {} of “{sketch}” to “{name}”.",
                count(drawn, "object", "objects")
            );
            match (exported.construction_left_out, exported.construction) {
                (0, 0) => Notice::info(summary),
                (0, kept) => Notice::info(format!(
                    "{summary} {} on the Construction layer.",
                    count(kept, "construction curve is", "construction curves are")
                )),
                (left_out, _) => Notice::info(format!(
                    "{summary} {} left out; File › Keep construction geometry in drawings keeps \
                     them.",
                    count(
                        left_out,
                        "construction curve was",
                        "construction curves were"
                    )
                )),
            }
        }
        Err(ExportError::Cancelled) => Notice::info("The export was cancelled."),
        Err(ExportError::NoCurves) => Notice::failure(format!(
            "“{sketch}” has no curves or points to export. Construction geometry is left out \
             unless File › Keep construction geometry in drawings is on."
        )),
        Err(error) => Notice::failure(format!("Could not export “{name}”: {error}.")),
    }
}

pub fn face_finished(path: &Path, face: &str, result: Result<FaceExported, ExportError>) -> Notice {
    let name = display_name(Some(path));
    match result {
        Ok(exported) => {
            let side_by_side = if exported.faces > 1 {
                ", side by side"
            } else {
                ""
            };
            let summary = format!(
                "Exported {} of {face} to “{name}”, in {}{side_by_side}.",
                count(exported.curves, "curve", "curves"),
                count(exported.loops, "loop", "loops")
            );
            match exported.approximated {
                0 => Notice::info(summary),
                approximated => Notice::info(format!(
                    "{summary} {} with no exact form in a drawing {} fitted within a micrometre.",
                    count(approximated, "curve", "curves"),
                    if approximated == 1 { "was" } else { "were" }
                )),
            }
        }
        Err(ExportError::Cancelled) => Notice::info("The export was cancelled."),
        Err(error) => Notice::failure(format!("Could not export “{name}”: {error}.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_without_a_sketch_extension_gets_dxf_appended() {
        assert_eq!(
            with_format_extension(PathBuf::from("/tmp/outline")),
            PathBuf::from("/tmp/outline.dxf")
        );
        assert_eq!(
            with_format_extension(PathBuf::from("/tmp/outline.DXF")),
            PathBuf::from("/tmp/outline.DXF")
        );
        assert_eq!(
            with_format_extension(PathBuf::from("/tmp/outline.svg")),
            PathBuf::from("/tmp/outline.svg")
        );
        assert_eq!(
            with_format_extension(PathBuf::from("/tmp/part.caditor")),
            PathBuf::from("/tmp/part.caditor.dxf")
        );
    }

    #[test]
    fn the_notice_counts_what_was_written_and_what_was_left_out() {
        let exported = SketchExported {
            curves: 3,
            points: 1,
            construction: 0,
            construction_left_out: 2,
        };
        let kept = SketchExported {
            construction: 2,
            construction_left_out: 0,
            ..exported
        };

        let notice = finished(Path::new("/tmp/a.dxf"), "Sketch 1", Ok(exported));
        let on_layer = finished(Path::new("/tmp/a.dxf"), "Sketch 1", Ok(kept));

        assert_eq!(
            notice.text,
            "Exported 4 objects of “Sketch 1” to “a.dxf”. 2 construction curves were left out; \
             File › Keep construction geometry in drawings keeps them."
        );
        assert_eq!(
            on_layer.text,
            "Exported 6 objects of “Sketch 1” to “a.dxf”. 2 construction curves are on the \
             Construction layer."
        );
        assert_eq!(
            finished(
                Path::new("/tmp/a.dxf"),
                "Sketch 1",
                Err(ExportError::NoCurves)
            )
            .text,
            "“Sketch 1” has no curves or points to export. Construction geometry is left out \
             unless File › Keep construction geometry in drawings is on."
        );
    }

    #[test]
    fn the_face_notice_counts_curves_and_loops_and_says_what_was_approximated() {
        let path = Path::new("/tmp/plate.dxf");
        let exact = FaceExported {
            faces: 1,
            loops: 2,
            curves: 5,
            approximated: 0,
        };
        let approximated = FaceExported {
            faces: 2,
            loops: 1,
            curves: 3,
            approximated: 1,
        };

        assert_eq!(
            face_finished(path, "“Top”", Ok(exact)).text,
            "Exported 5 curves of “Top” to “plate.dxf”, in 2 loops."
        );
        assert_eq!(
            face_finished(path, "2 faces", Ok(approximated)).text,
            "Exported 3 curves of 2 faces to “plate.dxf”, in 1 loop, side by side. 1 curve with \
             no exact form in a drawing was fitted within a micrometre."
        );
        assert_eq!(
            face_finished(path, "“Top”", Err(ExportError::FaceNotFlat)).text,
            "Could not export “plate.dxf”: the face is curved; only flat faces export as drawings."
        );
    }
}
