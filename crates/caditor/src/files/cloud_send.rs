use std::{
    borrow::Cow,
    fs, io,
    path::{Path, PathBuf},
};

use caditor_file::ExportFormat;

use super::{Event, Files};
use crate::{
    cloud::{self, Destination, Shipment},
    commands::Reason,
    export::{self, ExportCommand},
    model::{Model, Notice, NoticeKind, RecomputeStatus, display_name},
};

pub const HINT: &str = "Export the bodies chosen for export as STEP, without asking for a file, \
                        and put them in the cloud folder you choose";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CloudSendRefusal {
    #[error("There are no bodies to send yet. Extrude or revolve a sketch to make one")]
    NoBodies,
    #[error("Every body is left out in the Export dialog. Include one there to send it")]
    AllLeftOut,
    #[error("An export is already running")]
    ExportRunning,
    #[error("Waiting for the model to finish recomputing")]
    WaitingForRecompute,
    #[error("The model is already being made ready to send")]
    Preparing,
    #[error("caditor has no folder to keep the file in while it is sent")]
    NoFolder,
}

impl Reason for CloudSendRefusal {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

enum Stage {
    Preparing(Destination),
    Exporting {
        path: PathBuf,
        destination: Destination,
    },
}

#[derive(Default)]
pub struct CloudSending {
    stage: Option<Stage>,
    ready: Option<Shipment>,
}

fn clear_outgoing(dir: &Path) -> Result<(), io::ErrorKind> {
    match fs::remove_dir_all(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.kind()),
    }
    fs::create_dir_all(dir).map_err(|error| error.kind())
}

impl Files {
    pub fn cloud_send_availability(&self, model: &Model) -> Result<(), CloudSendRefusal> {
        if self.config.state_dir.is_none() {
            return Err(CloudSendRefusal::NoFolder);
        }
        if self.cloud_sending.stage.is_some() {
            return Err(CloudSendRefusal::Preparing);
        }
        if self.exporter.is_running() {
            return Err(CloudSendRefusal::ExportRunning);
        }
        if matches!(model.status(), RecomputeStatus::Running { .. }) {
            return Err(CloudSendRefusal::WaitingForRecompute);
        }
        if self.exporter.has_chosen_body(model) {
            Ok(())
        } else if export::Exporter::default().has_chosen_body(model) {
            Err(CloudSendRefusal::AllLeftOut)
        } else {
            Err(CloudSendRefusal::NoBodies)
        }
    }

    pub fn cloud_send_name(&self, model: &Model) -> String {
        export::named_for(model, ExportFormat::Step)
    }

    pub fn preparing_cloud_send(&self) -> Option<&Destination> {
        match self.cloud_sending.stage.as_ref()? {
            Stage::Preparing(destination) | Stage::Exporting { destination, .. } => {
                Some(destination)
            }
        }
    }

    pub(super) fn send_to_cloud(&mut self, destination: Destination, model: &mut Model) {
        if let Err(refusal) = self.cloud_send_availability(model) {
            model.set_notice(Notice::warning(format!("{refusal}.")));
            return;
        }
        let Some(dir) = self.config.state_dir.as_deref().map(cloud::outgoing_dir) else {
            return;
        };
        let session = model.session();
        self.cloud_sending.stage = Some(Stage::Preparing(destination.clone()));
        let failed = destination.clone();
        self.spawn(
            move || Event::CloudSendPrepared {
                session,
                destination,
                folder: clear_outgoing(&dir).map(|()| dir),
            },
            move || Event::CloudSendPrepared {
                session,
                destination: failed,
                folder: Err(io::ErrorKind::Other),
            },
        );
    }

    pub(super) fn cloud_send_prepared(
        &mut self,
        session: u64,
        destination: Destination,
        folder: Result<PathBuf, io::ErrorKind>,
        model: &mut Model,
    ) {
        self.cloud_sending.stage = None;
        let service = destination.service;
        let folder = match folder {
            Ok(folder) => folder,
            Err(kind) => {
                model.set_notice(Notice::failure(format!(
                    "Could not prepare the model to send to {service}: {kind}. Check the free \
                     space on the disk and try again."
                )));
                return;
            }
        };
        if session != model.session() {
            model.set_notice(Notice::info(format!(
                "Nothing was sent to {service}, since another model is open now."
            )));
            return;
        }
        if let Err(refusal) = self.cloud_send_availability(model) {
            model.set_notice(Notice::warning(format!("{refusal}.")));
            return;
        }
        let path = folder.join(export::named_for(model, ExportFormat::Step));
        self.cloud_sending.stage = Some(Stage::Exporting {
            path: path.clone(),
            destination,
        });
        let finished = self.export_finished();
        self.exporter
            .start(path, ExportFormat::Step, Vec::new(), model, finished);
        self.exporter.perform(ExportCommand::Hide, model);
    }

    pub(super) fn sent_to_cloud(&mut self, path: &Path) -> Option<Destination> {
        match self.cloud_sending.stage.take() {
            Some(Stage::Exporting {
                path: sent,
                destination,
            }) if sent == path => Some(destination),
            other => {
                self.cloud_sending.stage = other;
                None
            }
        }
    }

    pub(super) fn exported_for_cloud(
        &mut self,
        destination: Destination,
        path: &Path,
        wrote: bool,
        exported: Notice,
    ) -> Notice {
        let service = destination.service;
        if !wrote {
            return exported;
        }
        if exported.kind != NoticeKind::Success {
            return Notice::failure(format!("{} Nothing was sent to {service}.", exported.text));
        }
        let name = display_name(Some(path));
        self.cloud_sending.ready = Some(Shipment {
            destination,
            name: name.clone(),
            file: path.to_path_buf(),
        });
        Notice::info(format!("Sending “{name}” to {service}…"))
    }

    pub fn take_cloud_shipment(&mut self) -> Option<Shipment> {
        self.cloud_sending.ready.take()
    }
}
