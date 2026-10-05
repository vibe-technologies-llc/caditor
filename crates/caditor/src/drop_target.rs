use std::path::PathBuf;

use egui::{
    Align2, Area, Context, CornerRadius, HoveredFile, Id, Label, LayerId, Order, Rect, RichText,
    Stroke, StrokeKind,
};

use crate::{
    appearance::{self, CARD_RADIUS, SPACE_M},
    files::{is_drawing_file, is_importable_file, is_model_file},
    icons,
    model::display_name,
    widgets::{self, Tone},
};

const CARD_WIDTH: f32 = 380.0;
const FRAME_INSET: f32 = 8.0;
const FRAME_WIDTH: f32 = 2.0;
const BLOCKED: &str = "Finish with the open dialog before dropping files on caditor.";
const IMPORT_RUNNING: &str =
    "An import is already running. Drop the files again once it has finished.";
const MIXED: &str = "Drop a single model to open it, or drawings and STEP files to import them.";
const ANYTHING: &str = "A model opens; DXF drawings and STEP files are imported.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Situation {
    pub blocked: bool,
    pub importing: bool,
    pub into_sketch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub tone: Tone,
    pub title: String,
    pub detail: String,
}

impl Verdict {
    fn info(title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            tone: Tone::Info,
            title: title.into(),
            detail: detail.into(),
        }
    }

    fn glyph(&self) -> &'static str {
        match self.tone {
            Tone::Info => icons::DROP_FILES,
            tone => tone.icon(),
        }
    }

    fn warning(title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            tone: Tone::Warning,
            title: title.into(),
            detail: detail.into(),
        }
    }
}

pub fn verdict(paths: &[PathBuf], situation: Situation) -> Verdict {
    if paths.is_empty() {
        return Verdict::info("Drop to open or import", ANYTHING);
    }
    if situation.blocked {
        return Verdict::warning("Not now", BLOCKED);
    }
    let models = paths.iter().filter(|path| is_model_file(path)).count();
    match (models, paths) {
        (0, _) if situation.importing => Verdict::warning("Not now", IMPORT_RUNNING),
        (0, _) => importing(paths, situation),
        (1, [path]) => Verdict::info(
            format!("Drop to open {}", display_name(Some(path))),
            "It replaces the model open now; unsaved changes are asked about first.",
        ),
        _ => Verdict::warning("Not one model", MIXED),
    }
}

fn importing(paths: &[PathBuf], situation: Situation) -> Verdict {
    if let Some(unreadable) = paths.iter().find(|path| !is_importable_file(path)) {
        return Verdict::warning(
            format!(
                "{} is not a drawing or a STEP file",
                display_name(Some(unreadable))
            ),
            "Dropping it says why it cannot be imported.",
        );
    }
    let [path] = paths else {
        return Verdict::info(
            format!("Drop to import {} files", paths.len()),
            "They are imported one after another.",
        );
    };
    let name = display_name(Some(path));
    let detail = match (is_drawing_file(path), situation.into_sketch) {
        (true, true) => "Its curves are added to the sketch being edited.",
        (true, false) => "It becomes a new sketch.",
        (false, _) => "It is added as a body.",
    };
    Verdict::info(format!("Drop to import {name}"), detail)
}

pub fn show(ctx: &Context, area: Rect, hovered: &[HoveredFile], situation: Situation) {
    if hovered.is_empty() {
        return;
    }
    let paths: Vec<PathBuf> = hovered
        .iter()
        .filter_map(|file| file.path.clone())
        .collect();
    let paths = if paths.len() == hovered.len() {
        paths
    } else {
        Vec::new()
    };
    let verdict = verdict(&paths, situation);
    let tokens = appearance::tokens_for(&ctx.global_style().visuals);
    let color = verdict.tone.color(tokens);
    ctx.layer_painter(LayerId::new(
        Order::Foreground,
        Id::new("drop-target-frame"),
    ))
    .rect_stroke(
        area.shrink(FRAME_INSET),
        CornerRadius::same(CARD_RADIUS),
        Stroke::new(FRAME_WIDTH, color),
        StrokeKind::Inside,
    );
    Area::new(Id::new("drop-target"))
        .order(Order::Foreground)
        .interactable(false)
        .pivot(Align2::CENTER_CENTER)
        .fixed_pos(area.center())
        .show(ctx, |ui| {
            widgets::dialog_frame(ctx).show(ui, |ui| {
                ui.set_width(widgets::fitting_width(ctx, CARD_WIDTH));
                ui.horizontal(|ui| {
                    widgets::icon_label(ui, verdict.glyph(), color);
                    ui.add(Label::new(widgets::section_title(&verdict.title)).wrap());
                });
                ui.add_space(SPACE_M);
                ui.add(Label::new(RichText::new(&verdict.detail).color(tokens.text_muted)).wrap());
            });
        });
}
