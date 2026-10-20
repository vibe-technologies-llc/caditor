use std::{
    borrow::Cow,
    fs, io,
    path::{Path, PathBuf},
};

use caditor_file::ExportFormat;
use egui::{Id, Key, TextEdit};

use super::{Event, Files};
use crate::{
    appearance::SPACE_S,
    cloud::{self, Destination, Shipment},
    commands::Reason,
    export::{self, ExportCommand},
    model::{Model, Notice, NoticeKind, RecomputeStatus, UNTITLED, display_name},
    widgets::{self, DialogWidth, Tone},
};

pub const HINT: &str = "Export the bodies chosen for export as STEP, without asking for a file, \
                        and put them in the cloud folder you choose";

pub const OPEN_IN_FUSION_HINT: &str = "Export the bodies chosen for export as STEP, put them in \
                                       your remembered Fusion 360 folder and open the file \
                                       in your browser";

pub const NAME_LABEL: &str = "File name";
pub const NAME_SEND: &str = "Send and open";
pub const NAME_CANCEL: &str = "Cancel";
const NAME_DIALOG_ID: &str = "cloud-send-name";
const UNSAFE_NAME_CHARACTERS: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameRefusal {
    #[error("Type a name for the file.")]
    Empty,
    #[error("A file name cannot contain {0:?}.")]
    UnsafeCharacter(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudNameCommand {
    Edit(String),
    Send,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudNaming {
    destination: Destination,
    session: u64,
    text: String,
}

impl CloudNaming {
    #[cfg(test)]
    pub fn text(&self) -> &str {
        &self.text
    }

    fn file_name(&self) -> Result<String, NameRefusal> {
        let typed = self.text.trim();
        let stem = typed
            .len()
            .checked_sub(ExportFormat::Step.extension().len() + 1)
            .and_then(|split| typed.get(..split).zip(typed.get(split..)))
            .filter(|(_, tail)| {
                tail.strip_prefix('.')
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("step"))
            })
            .map_or(typed, |(stem, _)| stem)
            .trim();
        if stem.is_empty() {
            return Err(NameRefusal::Empty);
        }
        if let Some(unsafe_character) = stem
            .chars()
            .find(|character| character.is_control() || UNSAFE_NAME_CHARACTERS.contains(character))
        {
            return Err(NameRefusal::UnsafeCharacter(unsafe_character));
        }
        Ok(format!("{stem}.{}", ExportFormat::Step.extension()))
    }
}

pub fn name_field_id() -> Id {
    Id::unique((NAME_DIALOG_ID, "name"))
}

pub fn name_dialog(ctx: &egui::Context, naming: &CloudNaming) -> Option<CloudNameCommand> {
    let service = naming.destination.service;
    let title = format!("Name the file for {service}");
    let refusal = naming.file_name().err();
    let response = widgets::dialog(ctx, NAME_DIALOG_ID, &title, DialogWidth::Medium, |ui| {
        let explanation = format!(
            "This model has not been saved, so it has no name yet. Choose the name its STEP file \
             gets in “{}” on {service}. Sending again under the same name makes a new version of \
             the file.",
            naming.destination.folder_name
        );
        ui.label(widgets::muted(&explanation, ui));
        ui.add_space(SPACE_S);
        let mut text = naming.text.clone();
        let mut command = None;
        widgets::dialog_body(ui, |ui| {
            widgets::properties(ui, NAME_DIALOG_ID, |ui| {
                widgets::property(ui, NAME_LABEL, |ui| {
                    let field = widgets::text_field(ui, |ui| {
                        ui.add(
                            TextEdit::singleline(&mut text)
                                .id(name_field_id())
                                .desired_width(f32::INFINITY),
                        )
                    });
                    widgets::tie_to_caption(ui, &field);
                    if field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                        command = Some(CloudNameCommand::Send);
                    } else if ui.memory(|memory| memory.focused().is_none()) {
                        field.request_focus();
                    }
                });
            });
            if let Some(refusal) = &refusal {
                ui.add_space(SPACE_S);
                widgets::callout(ui, Tone::Warning, |ui| {
                    ui.label(refusal.to_string());
                });
            }
        });
        widgets::footer(ui, |ui| {
            let send = ui.add_enabled(refusal.is_none(), widgets::primary_button(ui, NAME_SEND));
            if send.clicked() {
                command = Some(CloudNameCommand::Send);
            }
            if ui.add(widgets::button(NAME_CANCEL)).clicked() {
                command = Some(CloudNameCommand::Cancel);
            }
        });
        if command.is_none() && text != naming.text {
            command = Some(CloudNameCommand::Edit(text));
        }
        command
    });
    let closed = response.should_close().then_some(CloudNameCommand::Cancel);
    response.inner.or(closed)
}

enum Stage {
    Preparing {
        destination: Destination,
        name: Option<String>,
    },
    Exporting {
        path: PathBuf,
        destination: Destination,
    },
}

#[derive(Default)]
pub struct CloudSending {
    stage: Option<Stage>,
    ready: Option<Shipment>,
    naming: Option<CloudNaming>,
    named: Option<(u64, String)>,
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
        match &self.cloud_sending.stage {
            Some(Stage::Preparing {
                name: Some(name), ..
            }) => name.clone(),
            _ => export::named_for(model, ExportFormat::Step),
        }
    }

    pub fn preparing_cloud_send(&self) -> Option<&Destination> {
        match self.cloud_sending.stage.as_ref()? {
            Stage::Preparing { destination, .. } | Stage::Exporting { destination, .. } => {
                Some(destination)
            }
        }
    }

    pub fn cloud_naming(&self) -> Option<&CloudNaming> {
        self.cloud_sending.naming.as_ref()
    }

    pub(super) fn close_cloud_naming(&mut self) {
        self.cloud_sending.naming = None;
    }

    pub(super) fn send_to_cloud(&mut self, destination: Destination, model: &mut Model) {
        if destination.opening() && model.path().is_none() {
            match self.cloud_send_availability(model) {
                Ok(()) => self.cloud_sending.naming = Some(self.naming_for(destination, model)),
                Err(refusal) => model.set_notice(Notice::warning(format!("{refusal}."))),
            }
            return;
        }
        self.begin_cloud_send(destination, None, model);
    }

    fn naming_for(&self, destination: Destination, model: &Model) -> CloudNaming {
        let session = model.session();
        let text = match &self.cloud_sending.named {
            Some((named, text)) if *named == session => text.clone(),
            _ => UNTITLED.to_owned(),
        };
        CloudNaming {
            destination,
            session,
            text,
        }
    }

    pub(super) fn name_cloud_send(&mut self, command: CloudNameCommand, model: &mut Model) {
        match command {
            CloudNameCommand::Edit(text) => {
                if let Some(naming) = &mut self.cloud_sending.naming {
                    naming.text = text;
                }
            }
            CloudNameCommand::Cancel => self.cloud_sending.naming = None,
            CloudNameCommand::Send => {
                let Some(Ok(name)) = self
                    .cloud_sending
                    .naming
                    .as_ref()
                    .map(CloudNaming::file_name)
                else {
                    return;
                };
                let Some(naming) = self.cloud_sending.naming.take() else {
                    return;
                };
                let service = naming.destination.service;
                if naming.session != model.session() {
                    model.set_notice(Notice::info(format!(
                        "Nothing was sent to {service}, since another model is open now."
                    )));
                    return;
                }
                self.cloud_sending.named = Some((naming.session, naming.text.trim().to_owned()));
                self.begin_cloud_send(naming.destination, Some(name), model);
            }
        }
    }

    fn begin_cloud_send(
        &mut self,
        destination: Destination,
        name: Option<String>,
        model: &mut Model,
    ) {
        if let Err(refusal) = self.cloud_send_availability(model) {
            model.set_notice(Notice::warning(format!("{refusal}.")));
            return;
        }
        let Some(dir) = self.config.state_dir.as_deref().map(cloud::outgoing_dir) else {
            return;
        };
        let session = model.session();
        self.cloud_sending.stage = Some(Stage::Preparing {
            destination: destination.clone(),
            name: name.clone(),
        });
        let failed = destination.clone();
        let failed_name = name.clone();
        self.spawn(
            move || Event::CloudSendPrepared {
                session,
                destination,
                name,
                folder: clear_outgoing(&dir).map(|()| dir),
            },
            move || Event::CloudSendPrepared {
                session,
                destination: failed,
                name: failed_name,
                folder: Err(io::ErrorKind::Other),
            },
        );
    }

    pub(super) fn cloud_send_prepared(
        &mut self,
        session: u64,
        destination: Destination,
        name: Option<String>,
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
        let file_name = name.unwrap_or_else(|| export::named_for(model, ExportFormat::Step));
        let path = folder.join(file_name);
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
