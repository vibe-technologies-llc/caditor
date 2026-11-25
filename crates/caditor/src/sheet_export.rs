use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
};

use caditor_document::{CancelToken, DrawingViewError, PlacedView, SheetPlan, ViewLines};
use caditor_file::{ExportError, SheetExported, SheetFormat, SheetView};

use crate::{
    feature_tree::count,
    model::{Notice, display_name, file_name_stem},
};

pub const KIND: &str = "PDF, SVG or DXF drawing sheet";
const UNNAMED: &str = "Drawing";

#[derive(Clone, Debug, PartialEq)]
pub struct SheetJob {
    pub name: String,
    pub plan: SheetPlan,
    pub views: Vec<(PlacedView, Arc<Result<ViewLines, DrawingViewError>>)>,
    pub hidden_lines: bool,
}

impl SheetJob {
    pub fn file_name(&self, format: SheetFormat) -> String {
        let stem = file_name_stem(&self.name);
        format!("{stem}.{}", format.extension())
    }

    pub fn write(&self, path: &Path, cancel: &CancelToken) -> Result<SheetExported, ExportError> {
        let format = SheetFormat::of(path).unwrap_or_default();
        let views: Vec<SheetView<'_>> = self
            .views
            .iter()
            .filter_map(|(placed, outcome)| {
                outcome.as_ref().as_ref().ok().map(|lines| SheetView {
                    placed: *placed,
                    lines,
                })
            })
            .collect();
        caditor_file::export_sheet(path, &self.plan, &views, self.hidden_lines, format, cancel)
    }
}

pub fn unnamed_file_name(format: SheetFormat) -> String {
    format!("{UNNAMED}.{}", format.extension())
}

pub fn with_format_extension(path: PathBuf, format: SheetFormat) -> PathBuf {
    if SheetFormat::of(&path).is_some() {
        return path;
    }

    let mut named = OsString::from(path.as_os_str());

    named.push(".");
    named.push(format.extension());
    PathBuf::from(named)
}

pub fn finished(path: &Path, drawing: &str, result: Result<SheetExported, ExportError>) -> Notice {
    let name = display_name(Some(path));
    match result {
        Ok(exported) => {
            let approximated = match exported.approximated {
                0 => String::new(),
                some => format!(
                    " {} drawn as short straight pieces.",
                    count(some, "curve was", "curves were")
                ),
            };
            let annotated = match (exported.dimensions, exported.notes) {
                (0, 0) => String::new(),
                (dimensions, 0) => format!(
                    " It holds {}.",
                    count(dimensions, "dimension", "dimensions")
                ),
                (0, notes) => format!(" It holds {}.", count(notes, "note", "notes")),
                (dimensions, notes) => format!(
                    " It holds {} and {}.",
                    count(dimensions, "dimension", "dimensions"),
                    count(notes, "note", "notes")
                ),
            };
            let left_out = match exported.left_out {
                0 => String::new(),
                some => format!(
                    " Left out because the edges they refer to are gone: {} (a note keeps its \
                     text without the leader); re-attach or delete them in the Drawings panel.",
                    count(some, "dimension or leader", "dimensions or leaders")
                ),
            };
            Notice::success(format!(
                "Exported the drawing “{drawing}” with {} to “{name}”.{approximated}{annotated}\
                 {left_out}",
                count(exported.views, "view", "views")
            ))
        }
        Err(ExportError::Cancelled) => Notice::info("The export was cancelled."),
        Err(error) => Notice::failure(format!("Could not export “{name}”: {error}.")),
    }
}
