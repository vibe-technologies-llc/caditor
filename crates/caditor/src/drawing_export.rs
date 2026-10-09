use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use caditor_document::{Feature, FeatureId};
use caditor_file::{
    Annotations, Construction, DrawingExported, DrawingSheet, ExportError, FaceExported, Nesting,
    SheetLayout, SketchExported, SketchFormat,
};
use egui::Id;

use crate::{
    appearance::{SPACE_M, SPACE_S},
    bodies,
    commands::{Command, CommandFrame},
    export,
    feature_tree::count,
    field,
    files::FileCommand,
    model::{Action, Model, Notice, display_name},
    modifying, preferences,
    selection::Selection,
    sketch_placement::{self, FaceChoice},
    widgets::{self, DialogWidth},
};

pub const SKETCH_HINT: &str = "Save the curves of one or more sketches as a DXF or SVG drawing in \
                               millimetres, side by side or nested on a sheet";
pub const NOT_A_SKETCH: &str = "Choose a sketch in the feature tree, or edit one, to export it";
pub const NOT_SOLVED: &str = "The sketch has not been solved, so there is nothing to export yet";
pub const FACE_HINT: &str = "Save the outlines and holes of the selected flat faces, with any \
                             sketches chosen in the feature tree, as a DXF or SVG drawing in \
                             millimetres, side by side or nested on a sheet, for laser or CNC \
                             cutting";
pub const NOT_A_FACE: &str = "Select one or more flat faces of bodies to export their outlines";
const CURVED: &str = "A selected face is curved; only flat faces export as drawings";
const DEFAULT_SHEET_WIDTH: f64 = 600.0;
const DEFAULT_SPACING: f64 = 5.0;
const NOT_A_WIDTH: &str = "Enter a sheet width greater than zero";
const NOT_A_SPACING: &str = "Enter a spacing of zero or more";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrawingSource {
    Sketches(Vec<FeatureId>),
    Faces(Vec<FaceChoice>),
    Both {
        sketches: Vec<FeatureId>,
        faces: Vec<FaceChoice>,
    },
}

impl DrawingSource {
    pub fn with_faces(sketches: Vec<FeatureId>, faces: Vec<FaceChoice>) -> Self {
        if faces.is_empty() {
            Self::Sketches(sketches)
        } else {
            Self::Both { sketches, faces }
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::Sketches(sketches) if sketches.len() == 1 => "Export sketch",
            Self::Sketches(_) => "Export sketches",
            Self::Faces(faces) if faces.len() == 1 => "Export face",
            Self::Faces(_) => "Export faces",
            Self::Both { .. } => "Export sketches and faces",
        }
    }

    fn sketches(&self) -> &[FeatureId] {
        match self {
            Self::Sketches(sketches) | Self::Both { sketches, .. } => sketches,
            Self::Faces(_) => &[],
        }
    }

    fn faces(&self) -> &[FaceChoice] {
        match self {
            Self::Faces(faces) | Self::Both { faces, .. } => faces,
            Self::Sketches(_) => &[],
        }
    }

    fn without_faces(self) -> Self {
        match self {
            Self::Both { sketches, .. } => Self::Sketches(sketches),
            source => source,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    #[default]
    SideBySide,
    Nested,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrawingCommand {
    Hide,
    Layout(Layout),
    SheetWidth(f64),
    Spacing(f64),
    Turns(bool),
    Annotations(bool),
    Faces(bool),
    Choose,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrawingExporter {
    open: bool,
    source: Option<DrawingSource>,
    layout: Layout,
    sheet_width: f64,
    spacing: f64,
    turns: bool,
    annotations: bool,
    faces: bool,
}

impl Default for DrawingExporter {
    fn default() -> Self {
        Self {
            open: false,
            source: None,
            layout: Layout::default(),
            sheet_width: DEFAULT_SHEET_WIDTH,
            spacing: DEFAULT_SPACING,
            turns: true,
            annotations: false,
            faces: true,
        }
    }
}

impl DrawingExporter {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn source(&self) -> Option<&DrawingSource> {
        self.source.as_ref()
    }

    pub fn show(&mut self, source: DrawingSource) {
        self.source = Some(source);
        self.faces = true;
        self.open = true;
    }

    pub fn hide(&mut self) {
        self.open = false;
        self.source = None;
    }

    pub fn perform(&mut self, command: DrawingCommand) {
        match command {
            DrawingCommand::Hide => self.hide(),
            DrawingCommand::Layout(layout) => self.layout = layout,
            DrawingCommand::SheetWidth(width) => self.sheet_width = width,
            DrawingCommand::Spacing(spacing) => self.spacing = spacing,
            DrawingCommand::Turns(turns) => self.turns = turns,
            DrawingCommand::Annotations(annotations) => self.annotations = annotations,
            DrawingCommand::Faces(faces) => self.faces = faces,
            DrawingCommand::Choose => {}
        }
    }

    pub fn start(&mut self) -> Option<DrawingSource> {
        self.open = false;
        let source = self.source.take()?;
        Some(if self.faces {
            source
        } else {
            source.without_faces()
        })
    }

    fn included(&self) -> (usize, usize) {
        let Some(source) = &self.source else {
            return (0, 0);
        };
        let faces = if self.faces { source.faces().len() } else { 0 };
        (source.sketches().len(), faces)
    }

    pub fn sheet(&self, construction: Construction) -> DrawingSheet {
        let layout = match self.layout {
            Layout::SideBySide => None,
            Layout::Nested => Nesting::new(self.sheet_width, self.spacing, self.turns),
        };
        DrawingSheet {
            layout: layout.map_or(SheetLayout::SideBySide, SheetLayout::Nested),
            annotations: if self.annotations {
                Annotations::Included
            } else {
                Annotations::LeftOut
            },
            construction,
        }
    }
}

pub fn exportable_sketches(
    model: &Model,
    targets: &[&Feature],
) -> Result<Vec<FeatureId>, &'static str> {
    let sketches: Vec<&Feature> = targets
        .iter()
        .copied()
        .filter(|feature| feature.kind.sketch().is_some())
        .collect();
    if sketches.is_empty() {
        return Err(NOT_A_SKETCH);
    }
    if sketches
        .iter()
        .any(|feature| model.displayed_sketch(feature).is_none())
    {
        return Err(NOT_SOLVED);
    }
    Ok(sketches.iter().map(|feature| feature.id()).collect())
}

pub fn sketches_name(model: &Model, sketches: &[FeatureId]) -> String {
    match sketches {
        [only] => model
            .document()
            .feature(*only)
            .map_or_else(|| model.display_name(), |feature| feature.name.clone()),
        several => count(several.len(), "sketch", "sketches"),
    }
}

pub fn quoted_sketches(model: &Model, sketches: &[FeatureId]) -> String {
    match sketches {
        [_] => format!("“{}”", sketches_name(model, sketches)),
        several => sketches_name(model, several),
    }
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

fn chosen_sketches(model: &Model, chosen: &[FeatureId]) -> Result<Vec<FeatureId>, &'static str> {
    let sketches: Vec<&Feature> = model
        .document()
        .features()
        .filter(|feature| chosen.contains(&feature.id()) && feature.kind.sketch().is_some())
        .collect();
    if sketches.is_empty() {
        return Ok(Vec::new());
    }
    exportable_sketches(model, &sketches)
}

fn face_source(
    model: &Model,
    selection: &Selection,
    chosen: &[FeatureId],
) -> Result<DrawingSource, &'static str> {
    let faces = exportable_faces(model, selection)?;
    let sketches = chosen_sketches(model, chosen)?;
    Ok(if sketches.is_empty() {
        DrawingSource::Faces(faces)
    } else {
        DrawingSource::Both { sketches, faces }
    })
}

fn source_detail(model: &Model, source: &DrawingSource) -> Option<String> {
    let faces = faces_name(model, source.faces())?;
    Some(match source.sketches() {
        [] => faces,
        sketches => format!("{faces} and {}", sketches_name(model, sketches)),
    })
}

pub fn face_commands(
    model: &Model,
    selection: &Selection,
    chosen: &[FeatureId],
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let exportable = face_source(model, selection, chosen);
    let detail = exportable
        .as_ref()
        .ok()
        .and_then(|source| source_detail(model, source));
    let available = exportable.as_ref().map(|_| ()).map_err(|reason| *reason);
    if commands.invoke_detailed(Command::ExportFace, detail, &available)
        && let Ok(source) = exportable
    {
        actions.push(Action::File(FileCommand::ExportDrawing(source)));
    }
}

pub fn source_file_name(model: &Model, source: Option<&DrawingSource>) -> String {
    match source {
        Some(DrawingSource::Both { .. }) => file_name(&model.display_name()),
        Some(DrawingSource::Faces(choices)) => choices.first().map_or_else(
            || file_name(&model.display_name()),
            |choice| face_file_name(&body_name(model, *choice)),
        ),
        Some(DrawingSource::Sketches(sketches)) => match sketches.as_slice() {
            [_] => file_name(&sketches_name(model, sketches)),
            _ => file_name(&model.display_name()),
        },
        None => file_name(&model.display_name()),
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
            let dimensions = match exported.dimensions {
                0 => String::new(),
                dimensions => format!(" with {}", count(dimensions, "dimension", "dimensions")),
            };
            let summary = format!(
                "Exported {} of {sketch}{dimensions} to “{name}”.{}",
                count(drawn, "object", "objects"),
                too_wide(exported.too_wide)
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
            "There are no curves or points to export in {sketch}. Construction geometry is left \
             out unless File › Keep construction geometry in drawings is on."
        )),
        Err(error) => Notice::failure(format!("Could not export “{name}”: {error}.")),
    }
}

fn too_wide(parts: usize) -> String {
    match parts {
        0 => String::new(),
        parts => format!(
            " {} wider than the sheet, so {} placed above the others; widen the sheet to nest {}.",
            count(parts, "part is", "parts are"),
            if parts == 1 { "it was" } else { "they were" },
            if parts == 1 { "it" } else { "them" },
        ),
    }
}

pub fn face_finished(
    path: &Path,
    face: &str,
    nested: bool,
    result: Result<FaceExported, ExportError>,
) -> Notice {
    let name = display_name(Some(path));
    match result {
        Ok(exported) => {
            let arranged = match (nested, exported.faces) {
                (true, _) => ", nested on a sheet",
                (false, 2..) => ", side by side",
                (false, _) => "",
            };
            let summary = format!(
                "Exported {} of {face} to “{name}”, in {}{arranged}.{}",
                count(exported.curves, "curve", "curves"),
                count(exported.loops, "loop", "loops"),
                too_wide(exported.too_wide)
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

pub fn sources_name(model: &Model, sketches: &[FeatureId], faces: &[FaceChoice]) -> String {
    let sketches = quoted_sketches(model, sketches);
    let faces = match faces {
        [only] => face_name(model, *only)
            .map_or_else(|| count(1, "face", "faces"), |name| format!("“{name}”")),
        several => count(several.len(), "face", "faces"),
    };
    format!("{sketches} and {faces}")
}

pub fn both_finished(
    path: &Path,
    name: &str,
    nested: bool,
    result: Result<DrawingExported, ExportError>,
) -> Notice {
    let file = display_name(Some(path));
    let Ok(exported) = result else {
        return finished(path, name, result.map(|exported| exported.sketches));
    };
    let (sketches, faces) = (exported.sketches, exported.faces);
    let dimensions = match sketches.dimensions {
        0 => String::new(),
        dimensions => format!(" with {}", count(dimensions, "dimension", "dimensions")),
    };
    let arranged = if nested {
        ", nested on a sheet"
    } else {
        ", side by side"
    };
    let mut text = format!(
        "Exported {name} to “{file}”{arranged}: {} from the sketches{dimensions} and {} in {} \
         from the faces.{}",
        count(
            sketches.curves + sketches.points + sketches.construction,
            "object",
            "objects"
        ),
        count(faces.curves, "curve", "curves"),
        count(faces.loops, "loop", "loops"),
        too_wide(exported.too_wide)
    );
    if sketches.construction_left_out > 0 {
        text.push_str(&format!(
            " {} left out; File › Keep construction geometry in drawings keeps them.",
            count(
                sketches.construction_left_out,
                "construction curve was",
                "construction curves were"
            )
        ));
    }
    if faces.approximated > 0 {
        text.push_str(&format!(
            " {} with no exact form in a drawing {} fitted within a micrometre.",
            count(faces.approximated, "curve", "curves"),
            if faces.approximated == 1 {
                "was"
            } else {
                "were"
            }
        ));
    }
    Notice::info(text)
}

pub fn dialog(
    ctx: &egui::Context,
    model: &Model,
    exporter: &DrawingExporter,
    keeps_construction: bool,
) -> Option<FileCommand> {
    let title = exporter
        .source
        .as_ref()
        .map_or("Export drawing", DrawingSource::title);
    let (sketches, faces) = exporter.included();
    let offered_faces = exporter
        .source
        .as_ref()
        .filter(|source| !source.sketches().is_empty())
        .map_or(0, |source| source.faces().len());
    let sketches_shown = exporter
        .source
        .as_ref()
        .is_some_and(|source| !source.sketches().is_empty());
    let response = widgets::dialog(ctx, "export-drawing", title, DialogWidth::Medium, |ui| {
        let mut command = None;
        export::heading(ui, "Layout");
        let layouts = [
            (
                Layout::SideBySide,
                "Side by side",
                "Each part in a row, bottoms level; a single sketch keeps its own coordinates",
            ),
            (
                Layout::Nested,
                "Nested on a sheet",
                "Parts packed close together on a sheet of the width you set, to save material",
            ),
        ];
        if let Some(layout) = preferences::choice(ui, &layouts, exporter.layout) {
            command = Some(FileCommand::DrawingExport(DrawingCommand::Layout(layout)));
        }
        if exporter.layout == Layout::Nested {
            ui.add_space(SPACE_S);
            sheet_fields(ui, model, exporter, &mut command);
        }
        export::heading(ui, "Contents");
        if offered_faces > 0 {
            let mut included = exporter.faces;
            if ui
                .checkbox(
                    &mut included,
                    format!(
                        "The {} selected in the view",
                        count(offered_faces, "flat face", "flat faces")
                    ),
                )
                .on_hover_text(
                    "Write the outlines and holes of the faces selected in the view into the same \
                     drawing as the sketches",
                )
                .changed()
            {
                command = Some(FileCommand::DrawingExport(DrawingCommand::Faces(included)));
            }
        }
        let mut annotations = exporter.annotations;
        let hint = if sketches_shown {
            "Write each sketch's dimensions, in millimetres and degrees, on a Dimensions layer, and \
             each sketch's name below it on a Labels layer"
        } else {
            "Write each face's name below it on a Labels layer"
        };
        let caption = if sketches_shown {
            "Dimensions and names"
        } else {
            "Names"
        };
        if ui
            .checkbox(&mut annotations, caption)
            .on_hover_text(hint)
            .changed()
        {
            command = Some(FileCommand::DrawingExport(DrawingCommand::Annotations(
                annotations,
            )));
        }
        if sketches_shown {
            let mut keep = keeps_construction;
            if ui
                .checkbox(&mut keep, "Construction geometry")
                .on_hover_text(
                    "Write construction curves on a dashed Construction layer instead of leaving \
                     them out",
                )
                .changed()
            {
                command = Some(FileCommand::KeepDrawingConstruction(keep));
            }
        }
        ui.add_space(SPACE_M);
        ui.label(widgets::muted(
            summary(exporter.layout, sketches, faces),
            ui,
        ));
        widgets::footer(ui, |ui| {
            if ui.add(widgets::primary_button(ui, "Export…")).clicked() {
                command = Some(FileCommand::DrawingExport(DrawingCommand::Choose));
            }
            if ui.add(widgets::button("Cancel")).clicked() {
                command = Some(FileCommand::DrawingExport(DrawingCommand::Hide));
            }
        });
        command
    });
    let closed = response
        .should_close()
        .then_some(FileCommand::DrawingExport(DrawingCommand::Hide));
    response.inner.or(closed)
}

fn summary(layout: Layout, sketches: usize, faces: usize) -> String {
    let noun = match (sketches, faces) {
        (_, 0) => count(sketches, "sketch", "sketches"),
        (0, _) => count(faces, "face", "faces"),
        _ => format!(
            "{} and {}",
            count(sketches, "sketch", "sketches"),
            count(faces, "face", "faces")
        ),
    };
    match (layout, sketches + faces) {
        (Layout::SideBySide, 1) => format!("The drawing holds {noun}, in millimetres."),
        (Layout::SideBySide, _) => {
            format!("The drawing holds {noun} side by side, in millimetres.")
        }
        (Layout::Nested, _) => format!(
            "The drawing holds {noun} nested from the sheet's lower left corner, in millimetres."
        ),
    }
}

fn sheet_fields(
    ui: &mut egui::Ui,
    model: &Model,
    exporter: &DrawingExporter,
    command: &mut Option<FileCommand>,
) {
    let mut errors = Vec::new();
    widgets::properties(ui, "drawing-sheet", |ui| {
        let lengths = [
            (
                "Sheet width",
                exporter.sheet_width,
                DrawingCommand::SheetWidth as fn(f64) -> DrawingCommand,
                parse_width as fn(&Model, &str) -> Result<f64, String>,
            ),
            (
                "Spacing",
                exporter.spacing,
                DrawingCommand::Spacing,
                parse_spacing,
            ),
        ];
        for (caption, stored, change, parse) in lengths {
            widgets::property(ui, caption, |ui| {
                let field = field::commit_field(
                    ui,
                    Id::new(("drawing-sheet", caption)),
                    &modifying::length_text(model.length_unit(), stored),
                    widgets::FIELD_WIDTH,
                    false,
                    |text| parse(model, text),
                );
                if let Some(length) = field.committed {
                    *command = Some(FileCommand::DrawingExport(change(length)));
                }
                if let Some(error) = field.error {
                    errors.push(error);
                }
            });
        }
        for error in &errors {
            widgets::error_row(ui, error);
        }
    });
    let mut turns = exporter.turns;
    if ui
        .checkbox(&mut turns, "Turn parts where that packs them closer")
        .on_hover_text(
            "Try each part in quarter turns, and at the angle in 15° steps that boxes it smallest, \
             and keep whichever fits lowest on the sheet",
        )
        .changed()
    {
        *command = Some(FileCommand::DrawingExport(DrawingCommand::Turns(turns)));
    }
}

pub fn parse_width(model: &Model, text: &str) -> Result<f64, String> {
    let length = modifying::Value::typed(model, text)?.millimetres;
    (length.is_finite() && length > 0.0)
        .then_some(length)
        .ok_or_else(|| NOT_A_WIDTH.to_owned())
}

pub fn parse_spacing(model: &Model, text: &str) -> Result<f64, String> {
    let length = modifying::Value::typed(model, text)?.millimetres;
    (length.is_finite() && length >= 0.0)
        .then_some(length)
        .ok_or_else(|| NOT_A_SPACING.to_owned())
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
            sketches: 1,
            curves: 3,
            points: 1,
            construction_left_out: 2,
            ..SketchExported::default()
        };
        let several = SketchExported {
            sketches: 3,
            curves: 12,
            dimensions: 4,
            too_wide: 1,
            ..SketchExported::default()
        };
        let kept = SketchExported {
            construction: 2,
            construction_left_out: 0,
            ..exported
        };

        let notice = finished(Path::new("/tmp/a.dxf"), "“Sketch 1”", Ok(exported));
        let on_layer = finished(Path::new("/tmp/a.dxf"), "“Sketch 1”", Ok(kept));
        let nested = finished(Path::new("/tmp/a.dxf"), "3 sketches", Ok(several));

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
                "“Sketch 1”",
                Err(ExportError::NoCurves)
            )
            .text,
            "There are no curves or points to export in “Sketch 1”. Construction geometry is left \
             out unless File › Keep construction geometry in drawings is on."
        );
        assert_eq!(
            nested.text,
            "Exported 12 objects of 3 sketches with 4 dimensions to “a.dxf”. 1 part is wider than \
             the sheet, so it was placed above the others; widen the sheet to nest it."
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
            too_wide: 0,
        };
        let approximated = FaceExported {
            faces: 2,
            loops: 1,
            curves: 3,
            approximated: 1,
            too_wide: 0,
        };

        assert_eq!(
            face_finished(path, "“Top”", false, Ok(exact)).text,
            "Exported 5 curves of “Top” to “plate.dxf”, in 2 loops."
        );
        assert_eq!(
            face_finished(path, "2 faces", false, Ok(approximated)).text,
            "Exported 3 curves of 2 faces to “plate.dxf”, in 1 loop, side by side. 1 curve with \
             no exact form in a drawing was fitted within a micrometre."
        );
        assert_eq!(
            face_finished(path, "2 faces", true, Ok(approximated)).text,
            "Exported 3 curves of 2 faces to “plate.dxf”, in 1 loop, nested on a sheet. 1 curve \
             with no exact form in a drawing was fitted within a micrometre."
        );
        assert_eq!(
            face_finished(path, "“Top”", false, Err(ExportError::FaceNotFlat)).text,
            "Could not export “plate.dxf”: the face is curved; only flat faces export as drawings."
        );
    }
}
