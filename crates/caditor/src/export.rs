use std::{
    collections::BTreeSet,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use caditor_document::{CancelToken, FeatureId, FeatureResult};
use caditor_file::{ExportBody, ExportError, ExportFormat, Exported, MeshResolution};
use egui::{Button, Id, KeyboardShortcut, Modal, Modifiers, RichText, Ui};
use parking_lot::Mutex;

use crate::{
    feature_tree::count,
    files::FileCommand,
    model::{Action, Model, Notice, RecomputeStatus, display_name},
    units::LengthUnit,
};

pub const EXPORT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, egui::Key::E);
const DIALOG_WIDTH: f32 = 420.0;
const NO_BODIES: &str =
    "There are no bodies to export yet. Extrude or revolve a sketch to make one.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportCommand {
    Show,
    Hide,
    SetFormat(ExportFormat),
    SetResolution(MeshResolution),
    Include { body: FeatureId, included: bool },
    Choose,
    Cancel,
}

pub type Finished = Box<dyn FnOnce(PathBuf, Result<Exported, ExportError>) + Send>;

struct Running {
    path: PathBuf,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct Exporter {
    open: bool,
    format: ExportFormat,
    resolution: MeshResolution,
    left_out: BTreeSet<FeatureId>,
    running: Option<Running>,
}

struct Body {
    id: FeatureId,
    name: String,
    result: Arc<FeatureResult>,
}

impl Exporter {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    pub fn format(&self) -> ExportFormat {
        self.format
    }

    pub fn perform(&mut self, command: ExportCommand) {
        match command {
            ExportCommand::Show => self.open = true,
            ExportCommand::Hide | ExportCommand::Choose => self.open = false,
            ExportCommand::SetFormat(format) => self.format = format,
            ExportCommand::SetResolution(resolution) => self.resolution = resolution,
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
        let path = with_format_extension(path, format);
        let bodies: Vec<(String, Arc<FeatureResult>)> = self
            .chosen(model)
            .map(|body| (body.name, body.result))
            .collect();
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancelled);
        let cancel = CancelToken::new(move || flag.load(Ordering::SeqCst));
        let resolution = self.resolution;
        let target = path.clone();
        let slot = Arc::new(Mutex::new(Some(finished)));
        let worker_slot = Arc::clone(&slot);
        let spawned = thread::Builder::new()
            .name("export".to_owned())
            .spawn(move || {
                let exported = export_results(&target, format, resolution, &bodies, &cancel);
                if let Some(finished) = worker_slot.lock().take() {
                    finished(target, exported);
                }
            });
        match spawned {
            Ok(_) => self.running = Some(Running { path, cancelled }),
            Err(error) => {
                log::error!("could not start the export: {error}");
                if let Some(finished) = slot.lock().take() {
                    finished(
                        path,
                        Err(ExportError::Writing(
                            "the background worker could not start".to_owned(),
                        )),
                    );
                }
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
        }
        let name = display_name(Some(path));
        match result {
            Ok(exported) => {
                let bodies = count(exported.bodies, "body", "bodies");
                Notice::info(match exported.triangles {
                    Some(triangles) => format!(
                        "Exported {bodies} to “{name}” ({}).",
                        count(triangles, "triangle", "triangles")
                    ),
                    None => format!("Exported {bodies} to “{name}”."),
                })
            }
            Err(ExportError::Cancelled) => Notice::info("The export was cancelled."),
            Err(error) => Notice::error(format!("Could not export “{name}”: {error}.")),
        }
    }

    fn bodies(model: &Model) -> impl Iterator<Item = Body> + '_ {
        let evaluation = model.evaluation();
        evaluation.bodies().filter_map(move |(id, _)| {
            let result = evaluation.body_result(id)?;
            result.solid()?;
            Some(Body {
                id,
                name: model
                    .document()
                    .feature(id)
                    .map_or_else(|| "a body".to_owned(), |feature| feature.name.clone()),
                result: Arc::clone(result),
            })
        })
    }

    fn chosen<'a>(&'a self, model: &'a Model) -> impl Iterator<Item = Body> + 'a {
        Self::bodies(model).filter(|body| !self.left_out.contains(&body.id))
    }
}

fn export_results(
    path: &Path,
    format: ExportFormat,
    resolution: MeshResolution,
    bodies: &[(String, Arc<FeatureResult>)],
    cancel: &CancelToken,
) -> Result<Exported, ExportError> {
    let bodies: Vec<ExportBody<'_>> = bodies
        .iter()
        .filter_map(|(name, result)| {
            Some(ExportBody {
                name,
                solid: &result.solid()?.solid,
            })
        })
        .collect();
    caditor_file::export_bodies(path, format, resolution, &bodies, cancel)
}

fn with_format_extension(path: PathBuf, format: ExportFormat) -> PathBuf {
    if format.matches(&path) {
        return path;
    }
    let mut named = OsString::from(path.as_os_str());
    named.push(".");
    named.push(format.extension());
    PathBuf::from(named)
}

pub fn menu_status(ui: &mut Ui, exporter: &Exporter, actions: &mut Vec<Action>) {
    let Some(running) = &exporter.running else {
        return;
    };
    ui.spinner();
    ui.label(format!(
        "Exporting “{}”…",
        display_name(Some(&running.path))
    ));
    if ui
        .small_button("Cancel")
        .on_hover_text("Stop the export without writing the file")
        .clicked()
    {
        actions.push(Action::File(FileCommand::Export(ExportCommand::Cancel)));
    }
}

pub fn dialog(ctx: &egui::Context, model: &Model, exporter: &Exporter) -> Option<ExportCommand> {
    let response = Modal::new(Id::new("export")).show(ctx, |ui| {
        ui.set_max_width(DIALOG_WIDTH);
        ui.heading("Export");
        let bodies: Vec<Body> = Exporter::bodies(model).collect();
        if bodies.is_empty() {
            ui.label(NO_BODIES);
            ui.add_space(8.0);
            return ui.button("Close").clicked().then_some(ExportCommand::Hide);
        }
        let mut command = None;
        format_choice(ui, exporter, &mut command);
        if exporter.format.is_mesh() {
            resolution_choice(ui, exporter, &bodies, model.length_unit(), &mut command);
        }
        body_choice(ui, exporter, &bodies, &mut command);
        let failed = model.evaluation().failed_count();
        if failed > 0 {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!(
                    "{} failed, so each body is exported as it was before them.",
                    count(failed, "feature", "features")
                ),
            );
        }
        ui.add_space(8.0);
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
        if let Some(blocker) = blocker {
            ui.weak(blocker);
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(blocker.is_none(), Button::new("Export…"))
                .clicked()
            {
                command = Some(ExportCommand::Choose);
            }
            if ui.button("Cancel").clicked() {
                command = Some(ExportCommand::Hide);
            }
        });
        command
    });
    let closed = response.should_close().then_some(ExportCommand::Hide);
    response.inner.or(closed)
}

fn format_choice(ui: &mut Ui, exporter: &Exporter, command: &mut Option<ExportCommand>) {
    ui.label(RichText::new("Format").strong());
    ui.horizontal(|ui| {
        for format in ExportFormat::ALL {
            if ui
                .radio(exporter.format == format, format.name())
                .on_hover_text(format_hint(format))
                .clicked()
            {
                *command = Some(ExportCommand::SetFormat(format));
            }
        }
    });
    ui.weak(format_hint(exporter.format));
}

fn format_hint(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Stl => "Triangles only, read by every slicer and mesh tool.",
        ExportFormat::ThreeMf => {
            "Keeps each body as a separate named object with its units, preferred by modern \
             slicers."
        }
        ExportFormat::Step => {
            "Exact faces and edges that other CAD programs can open and keep editing."
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
    ui.add_space(4.0);
    ui.label(RichText::new("Resolution").strong());
    ui.horizontal(|ui| {
        for resolution in MeshResolution::ALL {
            if ui
                .radio(exporter.resolution == resolution, resolution.name())
                .clicked()
            {
                *command = Some(ExportCommand::SetResolution(resolution));
            }
        }
    });
    let solids = bodies
        .iter()
        .filter(|body| !exporter.left_out.contains(&body.id))
        .filter_map(|body| body.result.solid().map(|result| &result.solid));
    let tolerance = exporter.resolution.tolerance(solids);
    ui.weak(format!(
        "Curved faces stay within {} of the model, with at most {}° between neighbouring \
         triangles.",
        unit.small_length_text(tolerance.chord()),
        tolerance.angle().to_degrees().round()
    ));
}

fn body_choice(
    ui: &mut Ui,
    exporter: &Exporter,
    bodies: &[Body],
    command: &mut Option<ExportCommand>,
) {
    ui.add_space(4.0);
    ui.label(RichText::new("Bodies").strong());
    for body in bodies {
        let mut included = !exporter.left_out.contains(&body.id);
        if ui.checkbox(&mut included, &body.name).changed() {
            *command = Some(ExportCommand::Include {
                body: body.id,
                included,
            });
        }
    }
}
