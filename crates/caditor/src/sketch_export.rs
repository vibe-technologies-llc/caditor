use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use caditor_file::{DXF_EXTENSION, ExportError, SketchExported};

use crate::{
    feature_tree::count,
    model::{Notice, display_name},
};

pub const SKETCH_HINT: &str = "Save the curves of a sketch as a DXF drawing in millimetres";
pub const NOT_A_SKETCH: &str = "Choose a sketch in the feature tree, or edit one, to export it";
pub const NOT_SOLVED: &str = "The sketch has not been solved, so there is nothing to export yet";

pub fn file_name(sketch: &str) -> String {
    format!("{sketch}.{DXF_EXTENSION}")
}

pub fn with_dxf_extension(path: PathBuf) -> PathBuf {
    let is_dxf = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(DXF_EXTENSION));
    if is_dxf {
        return path;
    }
    let mut named = OsString::from(path.as_os_str());
    named.push(".");
    named.push(DXF_EXTENSION);
    PathBuf::from(named)
}

pub fn finished(path: &Path, sketch: &str, result: Result<SketchExported, ExportError>) -> Notice {
    let name = display_name(Some(path));
    match result {
        Ok(exported) => {
            let drawn = exported.curves + exported.points;
            let summary = format!(
                "Exported {} of “{sketch}” to “{name}”.",
                count(drawn, "object", "objects")
            );
            match exported.construction_left_out {
                0 => Notice::info(summary),
                left_out => Notice::info(format!(
                    "{summary} {} left out.",
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
            "“{sketch}” has no curves or points to export. Construction geometry is not exported."
        )),
        Err(error) => Notice::failure(format!("Could not export “{name}”: {error}.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_without_the_dxf_extension_gets_it_appended() {
        assert_eq!(
            with_dxf_extension(PathBuf::from("/tmp/outline")),
            PathBuf::from("/tmp/outline.dxf")
        );
        assert_eq!(
            with_dxf_extension(PathBuf::from("/tmp/outline.DXF")),
            PathBuf::from("/tmp/outline.DXF")
        );
        assert_eq!(
            with_dxf_extension(PathBuf::from("/tmp/part.caditor")),
            PathBuf::from("/tmp/part.caditor.dxf")
        );
    }

    #[test]
    fn the_notice_counts_what_was_written_and_what_was_left_out() {
        let exported = SketchExported {
            curves: 3,
            points: 1,
            construction_left_out: 2,
        };

        let notice = finished(Path::new("/tmp/a.dxf"), "Sketch 1", Ok(exported));

        assert_eq!(
            notice.text,
            "Exported 4 objects of “Sketch 1” to “a.dxf”. 2 construction curves were left out."
        );
        assert_eq!(
            finished(
                Path::new("/tmp/a.dxf"),
                "Sketch 1",
                Err(ExportError::NoCurves)
            )
            .text,
            "“Sketch 1” has no curves or points to export. Construction geometry is not exported."
        );
    }
}
