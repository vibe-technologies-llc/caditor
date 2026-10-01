use std::{
    ffi::OsString,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use caditor_document::CancelToken;
use caditor_file::{ImageExportError, PNG_EXTENSION, RgbaImage, export_png};
use caditor_render::{Background, Image, ImageError, MAX_IMAGE_SIDE, SurfaceSize};
use egui::{Id, Ui};
use parking_lot::Mutex;

use crate::{
    appearance::{SPACE_M, SPACE_S},
    commands::{Command, CommandFrame},
    export, field,
    files::FileCommand,
    model::{Action, Model, Notice, display_name},
    preferences,
    widgets::{self, DialogWidth},
};

const TITLE: &str = "Export image";
const FAILED_OUTCOME: &str = "the image shows each body as it was before them";
const NOT_EXPORTING: &str = "No image export is running";
pub const IMAGE_HINT: &str = "Save the 3D view as a PNG image";
const DEFAULT_CUSTOM: SurfaceSize = SurfaceSize {
    width: 1920,
    height: 1080,
};
const LEFT_OUT: &str = "The image shows the model as the view does now, without highlights, the \
                        grid, labels or the view cube.";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SizeChoice {
    #[default]
    View,
    Custom,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ImageScale {
    #[default]
    One,
    Two,
    Four,
}

impl ImageScale {
    pub const ALL: [Self; 3] = [Self::One, Self::Two, Self::Four];

    pub fn factor(self) -> u32 {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Four => 4,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::One => "1×",
            Self::Two => "2×",
            Self::Four => "4×",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageCommand {
    Show,
    Hide,
    Size(SizeChoice),
    Width(u32),
    Height(u32),
    Scale(ImageScale),
    Background(Background),
    Choose(SurfaceSize),
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderJob {
    pub size: SurfaceSize,
    pub background: Background,
}

#[derive(Debug)]
pub enum ImageFailure {
    Cancelled,
    Render(ImageError),
    Write(ImageExportError),
    WorkerUnavailable,
    Crashed,
}

pub type ReadPixels = Box<dyn FnOnce() -> Result<Image, ImageError> + Send>;
pub type Finished = Box<dyn FnOnce(PathBuf, Result<SurfaceSize, ImageFailure>) + Send>;

struct Queued {
    path: PathBuf,
    job: RenderJob,
    cancelled: bool,
}

enum Stage {
    Idle,
    Queued(Queued),
    Rendering(Queued),
    Writing {
        path: PathBuf,
        cancelled: Arc<AtomicBool>,
    },
}

pub struct ImageExporter {
    open: bool,
    size: SizeChoice,
    custom: SurfaceSize,
    scale: ImageScale,
    background: Background,
    folder: Option<PathBuf>,
    chosen: Option<SurfaceSize>,
    stage: Stage,
}

impl Default for ImageExporter {
    fn default() -> Self {
        Self {
            open: false,
            size: SizeChoice::default(),
            custom: DEFAULT_CUSTOM,
            scale: ImageScale::default(),
            background: Background::default(),
            folder: None,
            chosen: None,
            stage: Stage::Idle,
        }
    }
}

impl ImageExporter {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_running(&self) -> bool {
        !matches!(self.stage, Stage::Idle)
    }

    pub fn folder(&self) -> Option<PathBuf> {
        self.folder.clone()
    }

    pub fn perform(&mut self, command: ImageCommand) {
        match command {
            ImageCommand::Show => self.open = true,
            ImageCommand::Hide => self.open = false,
            ImageCommand::Size(size) => self.size = size,
            ImageCommand::Width(width) => self.custom.width = width,
            ImageCommand::Height(height) => self.custom.height = height,
            ImageCommand::Scale(scale) => self.scale = scale,
            ImageCommand::Background(background) => self.background = background,
            ImageCommand::Choose(size) => self.chosen = Some(size),
            ImageCommand::Cancel => match &mut self.stage {
                Stage::Queued(queued) | Stage::Rendering(queued) => queued.cancelled = true,
                Stage::Writing { cancelled, .. } => cancelled.store(true, Ordering::SeqCst),
                Stage::Idle => {}
            },
        }
    }

    pub fn file_name(model: &Model) -> String {
        let name = display_name(model.path());
        let stem = Path::new(&name)
            .file_stem()
            .map_or(name.clone(), |stem| stem.to_string_lossy().into_owned());
        format!("{stem}.{PNG_EXTENSION}")
    }

    pub fn pick_cancelled(&mut self) {
        self.chosen = None;
    }

    pub fn picked(&mut self, path: PathBuf) {
        let Some(size) = self.chosen.take() else {
            return;
        };
        self.open = false;
        self.folder = path.parent().map(Path::to_path_buf);
        self.stage = Stage::Queued(Queued {
            path,
            job: RenderJob {
                size,
                background: self.background,
            },
            cancelled: false,
        });
    }

    pub fn render_job(&mut self) -> Option<RenderJob> {
        match std::mem::replace(&mut self.stage, Stage::Idle) {
            Stage::Queued(queued) => {
                let job = queued.job;
                self.stage = Stage::Rendering(queued);
                Some(job)
            }
            other => {
                self.stage = other;
                None
            }
        }
    }

    pub fn rendered(
        &mut self,
        pixels: Result<ReadPixels, ImageError>,
        finished: Finished,
    ) -> Option<Notice> {
        let Stage::Rendering(queued) = std::mem::replace(&mut self.stage, Stage::Idle) else {
            return None;
        };
        let name = display_name(Some(&queued.path));
        if queued.cancelled {
            return Some(outcome(&name, Err(ImageFailure::Cancelled)));
        }
        let read = match pixels {
            Ok(read) => read,
            Err(error) => return Some(outcome(&name, Err(ImageFailure::Render(error)))),
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancelled);
        let cancel = CancelToken::new(move || flag.load(Ordering::SeqCst));
        let target = queued.path.clone();
        let slot = Arc::new(Mutex::new(Some(finished)));
        let worker_slot = Arc::clone(&slot);
        let spawned = thread::Builder::new()
            .name("image-export".to_owned())
            .spawn(move || {
                let written =
                    panic::catch_unwind(AssertUnwindSafe(|| write_image(&target, read, &cancel)))
                        .unwrap_or_else(|_| {
                            log::error!("writing an exported image panicked");
                            Err(ImageFailure::Crashed)
                        });
                if let Some(finished) = worker_slot.lock().take() {
                    finished(target, written);
                }
            });
        match spawned {
            Ok(_) => {
                self.stage = Stage::Writing {
                    path: queued.path,
                    cancelled,
                };
                None
            }
            Err(error) => {
                log::error!("could not start writing the image: {error}");
                slot.lock().take();
                Some(outcome(&name, Err(ImageFailure::WorkerUnavailable)))
            }
        }
    }

    pub fn finished(&mut self, path: &Path, result: Result<SurfaceSize, ImageFailure>) -> Notice {
        if matches!(&self.stage, Stage::Writing { path: writing, .. } if writing == path) {
            self.stage = Stage::Idle;
        }
        outcome(&display_name(Some(path)), result)
    }

    fn exporting(&self) -> Option<&Path> {
        match &self.stage {
            Stage::Idle => None,
            Stage::Queued(queued) | Stage::Rendering(queued) => Some(&queued.path),
            Stage::Writing { path, .. } => Some(path),
        }
    }

    fn base_size(&self, view: Option<SurfaceSize>) -> Option<SurfaceSize> {
        match self.size {
            SizeChoice::View => view,
            SizeChoice::Custom => Some(self.custom),
        }
    }
}

fn write_image(
    path: &Path,
    read: ReadPixels,
    cancel: &CancelToken,
) -> Result<SurfaceSize, ImageFailure> {
    let image = read().map_err(ImageFailure::Render)?;
    let size = SurfaceSize {
        width: image.width,
        height: image.height,
    };
    export_png(
        path,
        &RgbaImage {
            width: image.width,
            height: image.height,
            pixels: &image.pixels,
        },
        cancel,
    )
    .map_err(ImageFailure::Write)?;
    Ok(size)
}

pub fn with_png_extension(path: PathBuf) -> PathBuf {
    let is_png = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(PNG_EXTENSION));
    if is_png {
        return path;
    }
    let mut named = OsString::from(path.as_os_str());
    named.push(".");
    named.push(PNG_EXTENSION);
    PathBuf::from(named)
}

pub fn scaled(size: SurfaceSize, scale: ImageScale) -> Result<SurfaceSize, String> {
    let side = |length: u32| {
        length
            .checked_mul(scale.factor())
            .filter(|side| (1..=MAX_IMAGE_SIDE).contains(side))
    };
    match (side(size.width), side(size.height)) {
        (Some(width), Some(height)) => Ok(SurfaceSize { width, height }),
        _ => Err(format!(
            "At {} the image would be {} × {} pixels, more than the {MAX_IMAGE_SIDE} pixels a side \
             caditor draws. Choose a smaller size or scale.",
            scale.name(),
            u64::from(size.width) * u64::from(scale.factor()),
            u64::from(size.height) * u64::from(scale.factor()),
        )),
    }
}

fn outcome(name: &str, result: Result<SurfaceSize, ImageFailure>) -> Notice {
    let failure = match result {
        Ok(size) => {
            return Notice::info(format!(
                "Exported a {} × {} image to “{name}”.",
                size.width, size.height
            ));
        }
        Err(ImageFailure::Cancelled | ImageFailure::Write(ImageExportError::Cancelled)) => {
            return Notice::info("The image export was cancelled.");
        }
        Err(failure) => failure,
    };
    let reason = match failure {
        ImageFailure::Render(ImageError::Busy) => {
            "another image was still being drawn. Try again in a moment".to_owned()
        }
        ImageFailure::Render(ImageError::OutOfRange {
            width,
            height,
            largest,
        }) => format!(
            "an image of {width} × {height} pixels is larger than the {largest} pixels a side \
             caditor draws. Choose a smaller size or scale"
        ),
        ImageFailure::Render(ImageError::DeviceLost) => {
            "the graphics device stopped working while drawing it. caditor has recovered the \
             view; export the image again"
                .to_owned()
        }
        ImageFailure::Render(ImageError::OutOfMemory) => {
            "the graphics card does not have enough memory for an image this large. Choose a \
             smaller size or scale, or lower the anti-aliasing in Preferences › Graphics"
                .to_owned()
        }
        ImageFailure::Render(ImageError::Refused) => {
            "the graphics device refused to draw it. Try a smaller size, or update the graphics \
             driver"
                .to_owned()
        }
        ImageFailure::Render(ImageError::Readback) => {
            "the image could not be read back from the graphics card. Try again".to_owned()
        }
        ImageFailure::Write(error @ ImageExportError::Writing(_)) => {
            format!("{error}. Choose another folder or name")
        }
        ImageFailure::Write(
            ImageExportError::PixelCount { .. }
            | ImageExportError::Encoding(_)
            | ImageExportError::Cancelled,
        )
        | ImageFailure::Crashed => {
            "caditor ran into an internal error while writing it. Try again".to_owned()
        }
        ImageFailure::WorkerUnavailable => {
            "the background worker could not start. Try again".to_owned()
        }
        ImageFailure::Cancelled => "the export was cancelled".to_owned(),
    };
    Notice::failure(format!("Could not export “{name}”: {reason}."))
}

pub fn activity(
    ui: &mut Ui,
    exporter: &ImageExporter,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let running = exporter.exporting().ok_or(NOT_EXPORTING);
    let mut cancel = commands.invoke(Command::CancelImageExport, &running);
    if let Ok(path) = running {
        ui.spinner();
        ui.label(format!("Exporting “{}”…", display_name(Some(path))));
        cancel |= ui
            .add(widgets::button("Cancel"))
            .on_hover_text(commands.with_keys(
                Command::CancelImageExport,
                "Stop the image export without writing the file",
            ))
            .clicked();
    }
    if cancel {
        actions.push(Action::File(FileCommand::ExportImage(ImageCommand::Cancel)));
    }
}

pub fn dialog(
    ctx: &egui::Context,
    model: &Model,
    exporter: &ImageExporter,
    view: Option<SurfaceSize>,
) -> Option<ImageCommand> {
    let response = widgets::dialog(ctx, "export-image", TITLE, DialogWidth::Medium, |ui| {
        let mut command = None;
        size_choice(ui, exporter, view, &mut command);
        scale_choice(ui, exporter, &mut command);
        background_choice(ui, exporter, &mut command);
        ui.add_space(SPACE_M);
        ui.label(widgets::muted(LEFT_OUT, ui));
        let size = exporter
            .base_size(view)
            .ok_or_else(|| "Show the 3D view to export it at its size.".to_owned())
            .and_then(|size| scaled(size, exporter.scale));
        let blocker = if exporter.is_running() {
            Err("An image export is already running.".to_owned())
        } else {
            size
        };
        if let Ok(size) = &blocker {
            ui.add_space(SPACE_M);
            ui.label(format!(
                "The image will be {} × {} pixels.",
                size.width, size.height
            ));
        }
        export::warnings(
            ui,
            model,
            FAILED_OUTCOME,
            blocker.as_ref().err().map(String::as_str),
        );
        widgets::footer(ui, |ui| {
            let export = widgets::primary_button(ui, "Export…");
            let response = ui.add_enabled(blocker.is_ok(), export);
            let response = match &blocker {
                Err(blocker) => response.on_disabled_hover_text(blocker),
                Ok(_) => response,
            };
            if response.clicked()
                && let Ok(size) = blocker
            {
                command = Some(ImageCommand::Choose(size));
            }
            if ui.add(widgets::button("Cancel")).clicked() {
                command = Some(ImageCommand::Hide);
            }
        });
        command
    });
    let closed = response.should_close().then_some(ImageCommand::Hide);
    response.inner.or(closed)
}

fn size_choice(
    ui: &mut Ui,
    exporter: &ImageExporter,
    view: Option<SurfaceSize>,
    command: &mut Option<ImageCommand>,
) {
    export::heading(ui, "Size");
    let view_label = match view {
        Some(view) => format!("View size ({} × {})", view.width, view.height),
        None => "View size".to_owned(),
    };
    let options = [
        (
            SizeChoice::View,
            view_label.as_str(),
            "The size of the 3D view on screen",
        ),
        (
            SizeChoice::Custom,
            "Custom",
            "A width and height of your own, in pixels",
        ),
    ];
    if let Some(choice) = preferences::choice(ui, &options, exporter.size) {
        *command = Some(ImageCommand::Size(choice));
    }
    if exporter.size == SizeChoice::Custom {
        ui.add_space(SPACE_S);
        let mut errors = Vec::new();
        widgets::properties(ui, "image-size", |ui| {
            let sides = [
                (
                    "Width",
                    exporter.custom.width,
                    ImageCommand::Width as fn(u32) -> ImageCommand,
                ),
                ("Height", exporter.custom.height, ImageCommand::Height),
            ];
            for (caption, stored, change) in sides {
                widgets::property(ui, caption, |ui| {
                    ui.horizontal(|ui| {
                        let field = field::commit_field(
                            ui,
                            Id::new(("image-side", caption)),
                            &stored.to_string(),
                            widgets::FIELD_WIDTH,
                            false,
                            parse_side,
                        );
                        ui.label(widgets::muted("px", ui));
                        if let Some(side) = field.committed {
                            *command = Some(change(side));
                        }
                        if let Some(error) = field.error {
                            errors.push(error);
                        }
                    });
                });
            }
            for error in &errors {
                widgets::error_row(ui, error);
            }
        });
    }
}

pub fn parse_side(text: &str) -> Result<u32, String> {
    text.parse::<u32>()
        .ok()
        .filter(|side| (1..=MAX_IMAGE_SIDE).contains(side))
        .ok_or_else(|| format!("Enter a whole number of pixels from 1 to {MAX_IMAGE_SIDE}"))
}

fn scale_choice(ui: &mut Ui, exporter: &ImageExporter, command: &mut Option<ImageCommand>) {
    export::heading(ui, "Scale");
    let hovers = ImageScale::ALL.map(|scale| {
        format!(
            "{} times as many pixels each way, with lines and points as much thicker",
            scale.factor()
        )
    });
    let options: Vec<(ImageScale, &str, &str)> = ImageScale::ALL
        .iter()
        .zip(&hovers)
        .map(|(scale, hover)| (*scale, scale.name(), hover.as_str()))
        .collect();
    if let Some(scale) = preferences::choice(ui, &options, exporter.scale) {
        *command = Some(ImageCommand::Scale(scale));
    }
}

fn background_choice(ui: &mut Ui, exporter: &ImageExporter, command: &mut Option<ImageCommand>) {
    export::heading(ui, "Background");
    let options = [Background::Viewport, Background::Transparent].map(|background| {
        (
            background,
            background_name(background),
            background_hint(background),
        )
    });
    if let Some(background) = preferences::choice(ui, &options, exporter.background) {
        *command = Some(ImageCommand::Background(background));
    }
    ui.label(widgets::muted(background_hint(exporter.background), ui));
}

fn background_name(background: Background) -> &'static str {
    match background {
        Background::Viewport => "3D view",
        Background::Transparent => "Transparent",
    }
}

fn background_hint(background: Background) -> &'static str {
    match background {
        Background::Viewport => "The dark background of the 3D view.",
        Background::Transparent => "Only the model, for placing over a page or slide.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scaled_size_stays_within_the_largest_side() {
        let view = SurfaceSize {
            width: 1200,
            height: 800,
        };

        assert_eq!(
            scaled(view, ImageScale::Four),
            Ok(SurfaceSize {
                width: 4800,
                height: 3200,
            })
        );
        let refused = scaled(
            SurfaceSize {
                width: 2500,
                height: 10,
            },
            ImageScale::Four,
        )
        .unwrap_err();
        assert!(refused.contains("10000 × 40 pixels"), "{refused}");
        assert!(
            scaled(
                SurfaceSize {
                    width: u32::MAX,
                    height: 1,
                },
                ImageScale::Two
            )
            .is_err()
        );
    }

    #[test]
    fn sides_are_whole_pixels_up_to_the_largest() {
        assert_eq!(parse_side("640"), Ok(640));
        assert_eq!(parse_side("8192"), Ok(8192));
        assert!(parse_side("0").is_err());
        assert!(parse_side("8193").is_err());
        assert!(parse_side("12.5").is_err());
        assert!(parse_side("wide").is_err());
    }

    #[test]
    fn a_path_without_the_png_extension_gets_it() {
        assert_eq!(
            with_png_extension(PathBuf::from("/tmp/plate")),
            PathBuf::from("/tmp/plate.png")
        );
        assert_eq!(
            with_png_extension(PathBuf::from("/tmp/plate.caditor")),
            PathBuf::from("/tmp/plate.caditor.png")
        );
        assert_eq!(
            with_png_extension(PathBuf::from("/tmp/plate.PNG")),
            PathBuf::from("/tmp/plate.PNG")
        );
    }

    #[test]
    fn failures_say_what_to_do_next_without_internals() {
        let out_of_memory = outcome(
            "plate.png",
            Err(ImageFailure::Render(ImageError::OutOfMemory)),
        );
        let lost = outcome(
            "plate.png",
            Err(ImageFailure::Render(ImageError::DeviceLost)),
        );
        let unwritable = outcome(
            "plate.png",
            Err(ImageFailure::Write(ImageExportError::Writing(
                std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            ))),
        );

        assert_eq!(
            out_of_memory.text,
            "Could not export “plate.png”: the graphics card does not have enough memory for an \
             image this large. Choose a smaller size or scale, or lower the anti-aliasing in \
             Preferences › Graphics."
        );
        assert!(
            lost.text.contains("export the image again"),
            "{}",
            lost.text
        );
        assert_eq!(
            unwritable.text,
            "Could not export “plate.png”: you do not have permission to write to its folder. \
             Choose another folder or name."
        );
        assert_eq!(
            outcome("plate.png", Err(ImageFailure::Cancelled)).text,
            "The image export was cancelled."
        );
    }
}
