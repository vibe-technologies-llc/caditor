use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
};

use caditor_document::{CancelToken, DrawingViewError, PlacedView, SheetPlan, ViewLines};
use caditor_file::{ExportError, SheetExported, SheetFormat, SheetView};

use crate::{
    feature_tree::count,
    model::{Notice, display_name},
};

pub const KIND: &str = "PDF, SVG or DXF drawing sheet";

#[derive(Debug, Clone, PartialEq)]
pub struct SheetJob {
    pub name: String,
    pub plan: SheetPlan,
    pub views: Vec<(PlacedView, Arc<Result<ViewLines, DrawingViewError>>)>,
    pub hidden_lines: bool,
}

impl SheetJob {
    pub fn file_name(&self) -> String {
        let stem: String = self
            .name
            .chars()
            .map(|character| match character {
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                other => other,
            })
            .collect();
        format!("{}.{}", stem.trim(), SheetFormat::default().extension())
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

pub fn with_format_extension(path: PathBuf) -> PathBuf {
    if SheetFormat::of(&path).is_some() {
        return path;
    }
    let mut named = OsString::from(path.as_os_str());
    named.push(".");
    named.push(SheetFormat::default().extension());
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
            Notice::success(format!(
                "Exported the drawing “{drawing}” with {} to “{name}”.{approximated}",
                count(exported.views, "view", "views")
            ))
        }
        Err(ExportError::Cancelled) => Notice::info("The export was cancelled."),
        Err(error) => Notice::failure(format!("Could not export “{name}”: {error}.")),
    }
}
