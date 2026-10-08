use std::{
    collections::BTreeSet,
    ffi::OsString,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use caditor_document::{
    BodyAppearance, CancelToken, Document, Evaluation, FeatureId, FeatureResult, ModelProperties,
    Rgb,
};
use caditor_file::{
    ExportBody, ExportError, ExportFormat, Exported, Look, MeshOptions, MeshResolution, RgbaImage,
    StlEncoding,
};
use caditor_geometry::Vector3;
use caditor_render::{ImageError, SurfaceSize};
use egui::{ScrollArea, Sides, Ui};
use parking_lot::Mutex;

use crate::{
    appearance::SPACE_M,
    commands::{Command, CommandFrame},
    feature_tree::count,
    files::FileCommand,
    icons,
    image_export::ReadPixels,
    model::{Action, Model, Notice, RecomputeStatus, display_name},
    preferences,
    units::LengthUnit,
    widgets::{self, DialogWidth, Tone},
};

const BODY_LIST_HEIGHT: f32 = 160.0;
const OUTDATED: &str = "Recomputing was stopped, so some features still show their earlier results. \
                        Recompute the model first to include the latest changes.";
const FAILED_OUTCOME: &str = "each body is exported as it was before them";
const NOT_EXPORTING: &str = "No export is running";
const NO_BODIES: &str =
    "There are no bodies to export yet. Extrude or revolve a sketch to make one.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportCommand {
    Show,
    Hide,
    SetFormat(ExportFormat),
    SetResolution(MeshResolution),
    SetStlEncoding(StlEncoding),
    Include { body: FeatureId, included: bool },
    IncludeAll(bool),
    Choose,
    Cancel,
}

pub type Finished = Box<dyn FnOnce(PathBuf, Result<Exported, ExportError>) + Send>;

pub const THUMBNAIL_SIZE: SurfaceSize = SurfaceSize {
    width: 256,
    height: 256,
};

struct Running {
    path: PathBuf,
    cancelled: Arc<AtomicBool>,
}

struct Job {
    path: PathBuf,
    format: ExportFormat,
    resolution: MeshResolution,
    stl: StlEncoding,
    bodies: Vec<ExportSource>,
    chosen: Vec<FeatureId>,
    properties: ModelProperties,
    cancel: CancelToken,
    finished: Finished,
}

struct Waiting {
    job: Job,
    rendering: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailJob {
    pub size: SurfaceSize,
    pub bodies: Vec<FeatureId>,
}

#[derive(Default)]
pub struct Exporter {
    open: bool,
    format: ExportFormat,
    resolution: MeshResolution,
    stl: StlEncoding,
    left_out: BTreeSet<FeatureId>,
    running: Option<Running>,
    waiting: Option<Waiting>,
    session: u64,
}

struct Body {
    id: FeatureId,
    source: ExportSource,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OwnedLook {
    pub colour: Rgb,
    pub opacity: Option<u8>,
    pub material: Option<String>,
}

impl OwnedLook {
    pub fn of(appearance: &BodyAppearance) -> Option<Self> {
        (appearance.colour.is_some()
            || appearance.material.is_some()
            || appearance.opacity.is_some())
        .then(|| Self {
            colour: appearance
                .colour
                .unwrap_or(crate::body_appearance::DEFAULT_COLOUR),
            opacity: appearance.opacity,
            material: appearance.material.clone(),
        })
    }

    pub fn borrowed(&self) -> Look<'_> {
        Look {
            colour: self.colour,
            opacity: self.opacity,
            material: self.material.as_deref(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExportSource {
    pub name: String,
    pub result: Arc<FeatureResult>,
    pub look: Option<OwnedLook>,
    pub group: Option<String>,
}

impl ExportSource {
    pub fn of(document: &Document, evaluation: &Evaluation, body: FeatureId) -> Option<Self> {
        let result = evaluation.body_result(body)?;
        result.solid()?;
        let feature = document.feature(body);
        Some(Self {
            name: document
                .body_name(body)
                .map_or_else(|| "a body".to_owned(), str::to_owned),
            result: Arc::clone(result),
            look: feature.and_then(|feature| OwnedLook::of(&feature.appearance)),
            group: feature.and_then(|feature| feature.group.clone()),
        })
    }

    pub fn exported(&self) -> Option<ExportBody<'_>> {
        Some(ExportBody {
            name: &self.name,
            solid: &self.result.solid()?.solid,
            look: self.look.as_ref().map(OwnedLook::borrowed),
            group: self.group.as_deref(),
        })
    }
}

impl Exporter {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    #[cfg(test)]
    pub fn includes(&self, body: FeatureId) -> bool {
        !self.left_out.contains(&body)
    }

    pub fn sync(&mut self, model: &Model) {
        if self.session != model.session() {
            self.session = model.session();
            self.left_out.clear();
        }
    }

    pub fn format(&self) -> ExportFormat {
        self.format
    }

    pub fn perform(&mut self, command: ExportCommand, model: &Model) {
        match command {
            ExportCommand::IncludeAll(true) => self.left_out.clear(),
            ExportCommand::IncludeAll(false) => {
                self.left_out = Self::bodies(model).map(|body| body.id).collect();
            }
            ExportCommand::Show => self.open = true,
            ExportCommand::Hide => self.open = false,
            ExportCommand::Choose => {}
            ExportCommand::SetFormat(format) => self.format = format,
            ExportCommand::SetResolution(resolution) => self.resolution = resolution,
            ExportCommand::SetStlEncoding(stl) => self.stl = stl,
            ExportCommand::Include { body, included } => {
                if included {
                    self.left_out.remove(&body);
                } else {
                    self.left_out.insert(body);
                }
            }
            ExportCommand::Cancel => {
                if let Some(running) = &self.running {
                    running.cancelled.store(true, Ordering::SeqCst);
                }
            }
        }
    }

    pub fn file_name(&self, model: &Model) -> String {
        let name = display_name(model.path());
        let stem = Path::new(&name)
            .file_stem()
            .map_or(name.clone(), |stem| stem.to_string_lossy().into_owned());
        format!("{stem}.{}", self.format.extension())
    }

    pub fn start(
        &mut self,
        path: PathBuf,
        format: ExportFormat,
        model: &Model,
        finished: Finished,
    ) {
        let (chosen, bodies): (Vec<FeatureId>, Vec<ExportSource>) = self
            .chosen(model)
            .map(|body| (body.id, body.source))
            .unzip();
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancelled);
        let job = Job {
            path: path.clone(),
            format,
            resolution: self.resolution,
            stl: self.stl,
            bodies,
            chosen,
            properties: model.document().properties().clone(),
            cancel: CancelToken::new(move || flag.load(Ordering::SeqCst)),
            finished,
        };
        self.running = Some(Running { path, cancelled });
        if format == ExportFormat::ThreeMf {
            self.waiting = Some(Waiting {
                job,
                rendering: false,
            });
        } else {
            self.spawn(job, None);
        }
    }

    pub fn thumbnail_job(&mut self) -> Option<ThumbnailJob> {
        let waiting = self.waiting.as_mut().filter(|waiting| !waiting.rendering)?;
        waiting.rendering = true;
        Some(ThumbnailJob {
            size: THUMBNAIL_SIZE,
            bodies: waiting.job.chosen.clone(),
        })
    }

    pub fn thumbnail_rendered(&mut self, pixels: Option<ReadPixels>) {
        if let Some(waiting) = self.waiting.take() {
            self.spawn(waiting.job, pixels);
        }
    }

    fn spawn(&mut self, job: Job, pixels: Option<ReadPixels>) {
        let Job {
            path,
            format,
            resolution,
            stl,
            bodies,
            properties,
            cancel,
            finished,
            ..
        } = job;
        let target = path.clone();
        let slot = Arc::new(Mutex::new(Some(finished)));
        let worker_slot = Arc::clone(&slot);
        let spawned = thread::Builder::new()
            .name("export".to_owned())
            .spawn(move || {
                let thumbnail = pixels.and_then(read_thumbnail);
                let options = MeshOptions {
                    resolution,
                    stl,
                    thumbnail: thumbnail.as_ref().map(|(size, pixels)| RgbaImage {
                        width: size.width,
                        height: size.height,
                        pixels,
                    }),
                };
                let exported =
                    export_results(&target, format, &options, &bodies, &properties, &cancel);
                if let Some(finished) = worker_slot.lock().take() {
                    finished(target, exported);
                }
            });
        if let Err(error) = spawned {
            log::error!("could not start the export: {error}");
            if let Some(finished) = slot.lock().take() {
                finished(path, Err(ExportError::WorkerUnavailable));
            }
        }
    }

    pub fn finished(&mut self, path: &Path, result: Result<Exported, ExportError>) -> Notice {
        if self
            .running
            .as_ref()
            .is_some_and(|running| running.path == path)
        {
            self.running = None;
            self.waiting = None;
        }
        let name = display_name(Some(path));
        match result {
            Ok(exported) => {
                let bodies = count(exported.bodies, "body", "bodies");
                let summary = match exported.triangles {
                    Some(triangles) => format!(
                        "Exported {bodies} to “{name}” ({}).",
                        count(triangles, "triangle", "triangles")
                    ),
                    None => format!("Exported {bodies} to “{name}”."),
                };
                let summary = match exported.moved {
                    Some(moved) => format!("{summary} {}", moved_note(moved)),
                    None => summary,
                };
                if exported.left_out.is_empty() {
                    return Notice {
                        outlasts_edits: exported.moved.is_some(),
                        ..Notice::info(summary)
                    };
                }
                let reasons: Vec<String> =
                    exported.left_out.iter().map(ToString::to_string).collect();
                Notice::failure(format!(
                    "{summary} {} left out: {}.",
                    count(reasons.len(), "body was", "bodies were"),
                    reasons.join("; ")
                ))
            }
            Err(ExportError::Cancelled) => Notice::info("The export was cancelled."),
            Err(error) => Notice::failure(format!("Could not export “{name}”: {error}.")),
        }
    }

    fn bodies(model: &Model) -> impl Iterator<Item = Body> + '_ {
        let evaluation = model.evaluation();
        evaluation.bodies().filter_map(move |(id, _)| {
            Some(Body {
                id,
                source: ExportSource::of(model.document(), evaluation, id)?,
            })
        })
    }

    fn chosen<'a>(&'a self, model: &'a Model) -> impl Iterator<Item = Body> + 'a {
        Self::bodies(model).filter(|body| !self.left_out.contains(&body.id))
    }
}

pub fn moved_note(moved: Vector3) -> String {
    let back = Vector3::ZERO - moved;
    format!(
        "Everything was moved by ({}, {}, {}) mm, so that binary STL keeps a micrometre; move it by \
         ({}, {}, {}) mm to put it back.",
        moved.x, moved.y, moved.z, back.x, back.y, back.z
    )
}

pub fn read_thumbnail(mut rows: ReadPixels) -> Option<(SurfaceSize, Vec<u8>)> {
    let size = SurfaceSize {
        width: rows.width(),
        height: rows.height(),
    };
    let read = panic::catch_unwind(AssertUnwindSafe(move || {
        let mut pixels = Vec::new();
        while let Some(band) = rows.next_rows() {
            pixels.extend_from_slice(band?);
        }
        Ok::<_, ImageError>(pixels)
    }));
    match read {
        Ok(Ok(pixels)) => Some((size, pixels)),
        Ok(Err(error)) => {
            log::warn!("the 3MF thumbnail was left out: {error}");
            None
        }
        Err(_) => {
            log::error!("reading the 3MF thumbnail panicked");
            None
        }
    }
}

fn export_results(
    path: &Path,
    format: ExportFormat,
    options: &MeshOptions,
    bodies: &[ExportSource],
    properties: &ModelProperties,
    cancel: &CancelToken,
) -> Result<Exported, ExportError> {
    let bodies: Vec<ExportBody<'_>> = bodies.iter().filter_map(ExportSource::exported).collect();
    caditor_file::export_bodies(path, format, options, &bodies, properties, cancel)
}

pub fn with_format_extension(path: PathBuf, format: ExportFormat) -> PathBuf {
    if format.matches(&path) {
        return path;
    }
    let mut named = OsString::from(path.as_os_str());
    named.push(".");
    named.push(format.extension());
    PathBuf::from(named)
}

pub fn activity(
    ui: &mut Ui,
    exporter: &Exporter,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let running = exporter.running.as_ref().ok_or(NOT_EXPORTING);
    let mut cancel = commands.invoke(Command::CancelExport, &running);
    if let Ok(running) = running {
        ui.spinner();
        ui.label(format!(
            "Exporting “{}”…",
            display_name(Some(&running.path))
        ));
        cancel |= ui
            .add(widgets::button("Cancel"))
            .on_hover_text(commands.with_keys(
                Command::CancelExport,
                "Stop the export without writing the file",
            ))
            .clicked();
    }
    if cancel {
        actions.push(Action::File(FileCommand::Export(ExportCommand::Cancel)));
    }
}

pub fn dialog(ctx: &egui::Context, model: &Model, exporter: &Exporter) -> Option<ExportCommand> {
    let response = widgets::dialog(ctx, "export", "Export", DialogWidth::Medium, |ui| {
        let bodies: Vec<Body> = Exporter::bodies(model).collect();
        if bodies.is_empty() {
            widgets::empty_state(ui, icons::command(Command::Export), NO_BODIES, |_| {});
            return None;
        }
        let mut command = None;
        format_choice(ui, exporter, &mut command);
        if exporter.format == ExportFormat::Stl {
            stl_choice(ui, exporter, &mut command);
        }
        if exporter.format.is_mesh() {
            resolution_choice(ui, exporter, &bodies, model.length_unit(), &mut command);
        }
        body_choice(ui, exporter, &bodies, &mut command);
        let blocker = if exporter.is_running() {
            Some("An export is already running.")
        } else if matches!(model.status(), RecomputeStatus::Running { .. }) {
            Some("Waiting for the model to finish recomputing…")
        } else if bodies
            .iter()
            .all(|body| exporter.left_out.contains(&body.id))
        {
            Some("Choose at least one body to export.")
        } else {
            None
        };
        warnings(ui, model, FAILED_OUTCOME, blocker);
        widgets::footer(ui, |ui| {
            let export = widgets::primary_button(ui, "Export…");
            let response = ui.add_enabled(blocker.is_none(), export);
            let response = match blocker {
                Some(blocker) => response.on_disabled_hover_text(blocker),
                None => response,
            };
            if response.clicked() {
                command = Some(ExportCommand::Choose);
            }
            if ui.add(widgets::button("Cancel")).clicked() {
                command = Some(ExportCommand::Hide);
            }
        });
        command
    });
    let closed = response.should_close().then_some(ExportCommand::Hide);
    response.inner.or(closed)
}

pub fn heading(ui: &mut Ui, title: &str) {
    ui.add_space(SPACE_M);
    ui.label(widgets::section_title(title));
}

pub fn warnings(ui: &mut Ui, model: &Model, outcome: &str, blocker: Option<&str>) {
    let failed = model.evaluation().failed_count();
    if failed > 0 {
        ui.add_space(SPACE_M);
        widgets::callout(ui, Tone::Warning, |ui| {
            ui.label(format!(
                "{} failed, so {outcome}.",
                count(failed, "feature", "features")
            ));
        });
    }
    if let Some(stale) = outdated(model.status()) {
        ui.add_space(SPACE_M);
        widgets::callout(ui, Tone::Warning, |ui| {
            ui.label(stale);
        });
    }
    if let Some(blocker) = blocker {
        ui.add_space(SPACE_M);
        widgets::callout(ui, Tone::Warning, |ui| {
            ui.label(blocker);
        });
    }
}

fn outdated(status: RecomputeStatus) -> Option<&'static str> {
    match status {
        RecomputeStatus::Cancelled | RecomputeStatus::Stopped => Some(OUTDATED),
        RecomputeStatus::UpToDate | RecomputeStatus::Running { .. } => None,
    }
}

fn format_choice(ui: &mut Ui, exporter: &Exporter, command: &mut Option<ExportCommand>) {
    heading(ui, "Format");
    let options = ExportFormat::ALL.map(|format| (format, format.name(), format_hint(format)));
    if let Some(format) = preferences::choice(ui, &options, exporter.format) {
        *command = Some(ExportCommand::SetFormat(format));
    }
    ui.label(widgets::muted(format_hint(exporter.format), ui));
}

fn format_hint(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Stl => "Triangles only, read by every slicer and mesh tool.",
        ExportFormat::ThreeMf => {
            "Keeps each body as a separate named object with its units, preferred by modern \
             slicers."
        }
        ExportFormat::Obj => {
            "Triangles with each body as a named object, read by renderers and most 3D tools; \
             no units, so millimetres."
        }
        ExportFormat::Gltf => {
            "A single binary file in metres with Y up, the format game engines and web viewers \
             load."
        }
        ExportFormat::Step => {
            "Exact faces and edges that other CAD programs can open and keep editing."
        }
    }
}

fn stl_choice(ui: &mut Ui, exporter: &Exporter, command: &mut Option<ExportCommand>) {
    heading(ui, "Encoding");
    let options = StlEncoding::ALL.map(|stl| (stl, stl.name(), stl_hint(stl)));
    if let Some(stl) = preferences::choice(ui, &options, exporter.stl) {
        *command = Some(ExportCommand::SetStlEncoding(stl));
    }
    ui.label(widgets::muted(stl_hint(exporter.stl), ui));
}

fn stl_hint(stl: StlEncoding) -> &'static str {
    match stl {
        StlEncoding::Binary => {
            "Compact, with every body in one surface; a model far from the origin is moved near \
             it so its single-precision numbers keep a micrometre."
        }
        StlEncoding::Text => {
            "Each body a named solid with exact coordinates, about five times larger; some \
             programs read the first solid only."
        }
    }
}

fn resolution_choice(
    ui: &mut Ui,
    exporter: &Exporter,
    bodies: &[Body],
    unit: LengthUnit,
    command: &mut Option<ExportCommand>,
) {
    heading(ui, "Resolution");
    let bounds: Vec<_> = bodies
        .iter()
        .filter(|body| !exporter.left_out.contains(&body.id))
        .filter_map(|body| body.source.result.solid()?.bounding_box())
        .collect();
    let describe = |resolution: MeshResolution| {
        let tolerance = resolution.tolerance_within(bounds.iter().copied());
        format!(
            "Curved faces stay within {} of the model, with at most {}° between neighbouring \
             triangles.",
            unit.small_length_text(tolerance.chord()),
            tolerance.angle().to_degrees().round()
        )
    };
    let hovers = MeshResolution::ALL.map(describe);
    let options: Vec<(MeshResolution, &str, &str)> = MeshResolution::ALL
        .iter()
        .zip(&hovers)
        .map(|(resolution, hover)| (*resolution, resolution.name(), hover.as_str()))
        .collect();
    if let Some(resolution) = preferences::choice(ui, &options, exporter.resolution) {
        *command = Some(ExportCommand::SetResolution(resolution));
    }
    ui.label(widgets::muted(describe(exporter.resolution), ui));
}

fn body_choice(
    ui: &mut Ui,
    exporter: &Exporter,
    bodies: &[Body],
    command: &mut Option<ExportCommand>,
) {
    ui.add_space(SPACE_M);
    let all = bodies
        .iter()
        .all(|body| !exporter.left_out.contains(&body.id));
    let none = bodies
        .iter()
        .all(|body| exporter.left_out.contains(&body.id));
    let picked = Sides::new()
        .show(
            ui,
            |ui| ui.label(widgets::section_title("Bodies")),
            |ui| {
                let none_chosen = ui
                    .add_enabled(!none, widgets::button("Select none"))
                    .clicked()
                    .then_some(false);
                let all_chosen = ui
                    .add_enabled(!all, widgets::button("Select all"))
                    .clicked()
                    .then_some(true);
                none_chosen.or(all_chosen)
            },
        )
        .1;
    if let Some(included) = picked {
        *command = Some(ExportCommand::IncludeAll(included));
    }
    widgets::card(ui, |ui| {
        ScrollArea::vertical()
            .max_height(widgets::list_height(ui.ctx(), BODY_LIST_HEIGHT))
            .show(ui, |ui| {
                for body in bodies {
                    let mut included = !exporter.left_out.contains(&body.id);
                    if ui.checkbox(&mut included, &body.source.name).changed() {
                        *command = Some(ExportCommand::Include {
                            body: body.id,
                            included,
                        });
                    }
                }
            });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_exports_a_look_only_when_it_has_a_colour_a_material_or_an_opacity() {
        let plain = BodyAppearance::default();
        let steel = BodyAppearance {
            material: Some("Steel".to_owned()),
            ..BodyAppearance::default()
        };
        let red = BodyAppearance {
            colour: Some(Rgb::new(200, 64, 52)),
            ..BodyAppearance::default()
        };

        let faded = BodyAppearance {
            opacity: Some(50),
            ..BodyAppearance::default()
        };

        assert_eq!(OwnedLook::of(&plain), None);
        assert_eq!(
            OwnedLook::of(&faded).map(|look| (look.colour, look.opacity)),
            Some((crate::body_appearance::DEFAULT_COLOUR, Some(50)))
        );
        assert_eq!(
            OwnedLook::of(&steel),
            Some(OwnedLook {
                colour: crate::body_appearance::DEFAULT_COLOUR,
                opacity: None,
                material: Some("Steel".to_owned()),
            })
        );
        assert_eq!(
            OwnedLook::of(&red).map(|look| look.colour),
            Some(Rgb::new(200, 64, 52))
        );
    }

    #[test]
    fn a_partial_export_names_every_body_left_out_and_stays_until_dismissed() {
        let mut exporter = Exporter::default();
        let exported = Exported {
            bodies: 2,
            triangles: Some(24),
            left_out: vec![ExportError::Meshing("Bracket".to_owned())],
            moved: None,
        };

        let notice = exporter.finished(Path::new("parts.stl"), Ok(exported));

        assert!(notice.outlasts_edits);
        assert_eq!(
            notice.text,
            "Exported 2 bodies to “parts.stl” (24 triangles). 1 body was left out: the body of \
             “Bracket” could not be turned into triangles at this resolution; try another \
             resolution."
        );
    }

    #[test]
    fn a_moved_stl_says_how_to_put_it_back() {
        let mut exporter = Exporter::default();
        let exported = Exported {
            bodies: 1,
            triangles: Some(12),
            left_out: Vec::new(),
            moved: Some(Vector3::new(-1_000_020.0, 0.0, -5.0)),
        };

        let notice = exporter.finished(Path::new("far.stl"), Ok(exported));

        assert!(notice.outlasts_edits);
        assert_eq!(
            notice.text,
            "Exported 1 body to “far.stl” (12 triangles). Everything was moved by (-1000020, 0, \
             -5) mm, so that binary STL keeps a micrometre; move it by (1000020, 0, 5) mm to put \
             it back."
        );
    }

    #[test]
    fn only_a_stopped_recompute_warns_that_results_may_be_outdated() {
        assert!(outdated(RecomputeStatus::Cancelled).is_some());
        assert!(outdated(RecomputeStatus::Stopped).is_some());
        assert!(outdated(RecomputeStatus::UpToDate).is_none());
        assert!(
            outdated(RecomputeStatus::Running {
                since: std::time::Instant::now()
            })
            .is_none()
        );
    }
}
