use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::Arc,
};

use caditor_document::FeatureId;
use caditor_file::{ExportFormat, Settings};

use super::{Event, Files};
use crate::{
    commands::Reason,
    export::{self, ExportCommand, Exporter},
    model::{Model, Notice, RecomputeStatus},
    slicer::{self, FindError, Finder, Installation, Launcher, PrepareError, Prepared, Slicer},
};

pub const HINT: &str = "Export the selected bodies, else the ones chosen in the Export dialog, as 3MF, \
                        without asking for a file, and open them in OrcaSlicer";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SlicerRefusal {
    #[error(transparent)]
    Missing(#[from] FindError),
    #[error("There are no bodies to open in {0} yet. Extrude or revolve a sketch to make one")]
    NoBodies(Slicer),
    #[error("Every body is left out in the Export dialog. Include one there to open it in {0}")]
    AllLeftOut(Slicer),
    #[error("An export is already running")]
    ExportRunning,
    #[error("Waiting for the model to finish recomputing")]
    WaitingForRecompute,
    #[error("The model is already being made ready for {0}")]
    Sending(Slicer),
}

impl Reason for SlicerRefusal {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

enum Sending {
    Preparing,
    Exporting {
        path: PathBuf,
        installation: Installation,
    },
}

pub struct Slicing {
    chosen: Option<PathBuf>,
    found: Option<Result<Installation, FindError>>,
    finder: Finder,
    launcher: Launcher,
    sending: Option<Sending>,
    only: Vec<FeatureId>,
}

impl Default for Slicing {
    fn default() -> Self {
        Self {
            chosen: None,
            found: None,
            finder: Arc::new(slicer::find),
            launcher: Arc::new(slicer::launch),
            sending: None,
            only: Vec::new(),
        }
    }
}

impl Files {
    #[cfg(test)]
    pub fn find_slicers_with(&mut self, finder: Finder) {
        self.slicing.finder = finder;
    }

    #[cfg(test)]
    pub fn launch_slicers_with(&mut self, launcher: Launcher) {
        self.slicing.launcher = launcher;
    }

    pub fn found_slicer(&self) -> Option<&Result<Installation, FindError>> {
        self.slicing.found.as_ref()
    }

    pub fn slicer_missing(&self) -> Option<&FindError> {
        self.slicing.found.as_ref()?.as_ref().err()
    }

    pub fn slicer_availability(
        &self,
        model: &Model,
        only: &[FeatureId],
    ) -> Result<(), SlicerRefusal> {
        let slicer = Slicer::SUPPORTED;
        if let Some(missing) = self.slicer_missing() {
            return Err(missing.clone().into());
        }
        if self.slicing.sending.is_some() {
            return Err(SlicerRefusal::Sending(slicer));
        }
        if self.exporter.is_running() {
            return Err(SlicerRefusal::ExportRunning);
        }
        if matches!(model.status(), RecomputeStatus::Running { .. }) {
            return Err(SlicerRefusal::WaitingForRecompute);
        }
        if self.exporter.has_chosen_body(model, only) {
            Ok(())
        } else if Exporter::default().has_chosen_body(model, &[]) {
            Err(SlicerRefusal::AllLeftOut(slicer))
        } else {
            Err(SlicerRefusal::NoBodies(slicer))
        }
    }

    pub(super) fn read_slicer_program(&mut self, settings: &Settings) {
        let chosen = slicer::chosen_program(settings);
        if chosen != self.slicing.chosen {
            self.slicing.chosen = chosen;
            self.find_slicer();
        }
    }

    pub(super) fn find_slicer(&mut self) {
        let finder = Arc::clone(&self.slicing.finder);
        let chosen = self.slicing.chosen.clone();
        let asked = chosen.clone();
        self.spawn(
            move || Event::SlicerFound {
                result: finder(Slicer::SUPPORTED, chosen.as_deref()),
                chosen: asked,
            },
            || Event::SlicerSearchFailed,
        );
    }

    pub(super) fn slicer_found(
        &mut self,
        chosen: Option<PathBuf>,
        result: Result<Installation, FindError>,
    ) {
        if chosen == self.slicing.chosen {
            self.slicing.found = Some(result);
        }
    }

    pub(super) fn open_in_slicer(&mut self, model: &mut Model, only: Vec<FeatureId>) {
        match self.slicer_availability(model, &only) {
            Ok(()) | Err(SlicerRefusal::Missing(_)) => {}
            Err(refusal) => {
                model.set_notice(Notice::warning(format!("{refusal}.")));
                return;
            }
        }
        let finder = Arc::clone(&self.slicing.finder);
        let chosen = self.slicing.chosen.clone();
        let folder = self
            .config
            .state_dir
            .as_ref()
            .map(|dir| dir.join(slicer::FOLDER));
        let session = model.session();
        self.slicing.sending = Some(Sending::Preparing);
        self.slicing.only = only;
        self.spawn(
            move || {
                let found = finder(Slicer::SUPPORTED, chosen.as_deref());
                Event::SlicerReady {
                    session,
                    chosen,
                    result: slicer::prepare(folder.as_deref(), found),
                }
            },
            move || Event::SlicerReady {
                session,
                chosen: None,
                result: Err(PrepareError::Crashed {
                    slicer: Slicer::SUPPORTED,
                }),
            },
        );
    }

    pub(super) fn slicer_ready(
        &mut self,
        session: u64,
        chosen: Option<PathBuf>,
        result: Result<Prepared, PrepareError>,
        model: &mut Model,
    ) {
        self.slicing.sending = None;
        let slicer = Slicer::SUPPORTED;
        let prepared = match result {
            Ok(prepared) => prepared,
            Err(PrepareError::Find(missing)) => {
                self.slicer_found(chosen, Err(missing.clone()));
                model.set_notice(Notice::failure(format!("{missing}.")));
                return;
            }
            Err(error) => {
                model.set_notice(Notice::failure(format!("{error}.")));
                return;
            }
        };
        self.slicer_found(chosen, Ok(prepared.installation.clone()));
        if session != model.session() {
            model.set_notice(Notice::info(format!(
                "Nothing was opened in {slicer}, since another model is open now."
            )));
            return;
        }
        let only = std::mem::take(&mut self.slicing.only);
        if let Err(refusal) = self.slicer_availability(model, &only) {
            model.set_notice(Notice::warning(format!("{refusal}.")));
            return;
        }
        let path = prepared
            .folder
            .join(export::named_for(model, ExportFormat::ThreeMf));
        self.slicing.sending = Some(Sending::Exporting {
            path: path.clone(),
            installation: prepared.installation,
        });
        let finished = self.export_finished();
        self.exporter.start_among(
            path,
            ExportFormat::ThreeMf,
            Vec::new(),
            model,
            &only,
            finished,
        );
        self.exporter.perform(ExportCommand::Hide, model);
    }

    pub(super) fn sent_to_slicer(&mut self, path: &Path) -> Option<Installation> {
        match self.slicing.sending.take() {
            Some(Sending::Exporting {
                path: sent,
                installation,
            }) if sent == path => Some(installation),
            other => {
                self.slicing.sending = other;
                None
            }
        }
    }

    pub(super) fn open_exported(
        &self,
        installation: &Installation,
        path: &Path,
        exported: Notice,
    ) -> Notice {
        let slicer = installation.slicer;
        match (self.slicing.launcher)(installation, path) {
            Ok(()) => Notice {
                text: format!("{} {slicer} is opening it.", exported.text),
                ..exported
            },
            Err(error) => Notice::failure(format!("{} {error}.", exported.text)),
        }
    }
}
