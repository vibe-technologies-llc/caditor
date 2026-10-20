use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::Arc,
};

use caditor_file::ExportFormat;

use super::{Event, Files};
use crate::{
    commands::Reason,
    export::{self, ExportCommand, Exporter},
    fusion_app::{self, Launcher, LinkHandler, PrepareError},
    model::{Model, Notice, RecomputeStatus},
};

pub const HINT: &str = "Export the bodies chosen for export as STEP, without asking for a file, \
                        and open them in the Fusion 360 app on this computer";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FusionAppRefusal {
    #[error(
        "There are no bodies to open in Fusion 360 yet. Extrude or revolve a sketch to make one"
    )]
    NoBodies,
    #[error(
        "Every body is left out in the Export dialog. Include one there to open it in Fusion 360"
    )]
    AllLeftOut,
    #[error("An export is already running")]
    ExportRunning,
    #[error("Waiting for the model to finish recomputing")]
    WaitingForRecompute,
    #[error("The model is already being made ready for Fusion 360")]
    Sending,
}

impl Reason for FusionAppRefusal {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

enum Sending {
    Preparing,
    Exporting { path: PathBuf },
}

pub struct FusionApp {
    launcher: Launcher,
    handler: LinkHandler,
    sending: Option<Sending>,
}

impl Default for FusionApp {
    fn default() -> Self {
        Self {
            launcher: Arc::new(fusion_app::launch),
            handler: Arc::new(fusion_app::opens_links),
            sending: None,
        }
    }
}

impl Files {
    #[cfg(test)]
    pub fn launch_fusion_app_with(&mut self, launcher: Launcher) {
        self.fusion_app.launcher = launcher;
        self.fusion_app.handler = Arc::new(|| true);
    }

    pub fn fusion_app_availability(&self, model: &Model) -> Result<(), FusionAppRefusal> {
        if self.fusion_app.sending.is_some() {
            return Err(FusionAppRefusal::Sending);
        }
        if self.exporter.is_running() {
            return Err(FusionAppRefusal::ExportRunning);
        }
        if matches!(model.status(), RecomputeStatus::Running { .. }) {
            return Err(FusionAppRefusal::WaitingForRecompute);
        }
        if self.exporter.has_chosen_body(model) {
            Ok(())
        } else if Exporter::default().has_chosen_body(model) {
            Err(FusionAppRefusal::AllLeftOut)
        } else {
            Err(FusionAppRefusal::NoBodies)
        }
    }

    pub(super) fn open_in_fusion_app(&mut self, model: &mut Model) {
        if let Err(refusal) = self.fusion_app_availability(model) {
            model.set_notice(Notice::warning(format!("{refusal}.")));
            return;
        }
        let folder = self
            .config
            .state_dir
            .as_ref()
            .map(|dir| dir.join(fusion_app::FOLDER));
        let handler = Arc::clone(&self.fusion_app.handler);
        let session = model.session();
        self.fusion_app.sending = Some(Sending::Preparing);
        self.spawn(
            move || Event::FusionAppReady {
                session,
                result: fusion_app::prepare(folder.as_deref(), handler.as_ref()),
            },
            move || Event::FusionAppReady {
                session,
                result: Err(PrepareError::Crashed),
            },
        );
    }

    pub(super) fn fusion_app_ready(
        &mut self,
        session: u64,
        result: Result<PathBuf, PrepareError>,
        model: &mut Model,
    ) {
        self.fusion_app.sending = None;
        let folder = match result {
            Ok(folder) => folder,
            Err(error) => {
                model.set_notice(Notice::failure(format!("{error}.")));
                return;
            }
        };
        if session != model.session() {
            model.set_notice(Notice::info(
                "Nothing was opened in Fusion 360, since another model is open now.",
            ));
            return;
        }
        if let Err(refusal) = self.fusion_app_availability(model) {
            model.set_notice(Notice::warning(format!("{refusal}.")));
            return;
        }
        let path = folder.join(export::named_for(model, ExportFormat::Step));
        self.fusion_app.sending = Some(Sending::Exporting { path: path.clone() });
        let finished = self.export_finished();
        self.exporter
            .start(path, ExportFormat::Step, Vec::new(), model, finished);
        self.exporter.perform(ExportCommand::Hide, model);
    }

    pub(super) fn sent_to_fusion_app(&mut self, path: &Path) -> bool {
        match self.fusion_app.sending.take() {
            Some(Sending::Exporting { path: sent }) if sent == path => true,
            other => {
                self.fusion_app.sending = other;
                false
            }
        }
    }

    pub(super) fn open_exported_in_fusion_app(&self, path: &Path, exported: Notice) -> Notice {
        match (self.fusion_app.launcher)(path) {
            Ok(()) => Notice {
                text: format!("{} Fusion 360 is opening it.", exported.text),
                ..exported
            },
            Err(error) => Notice::failure(format!("{} {error}.", exported.text)),
        }
    }
}
