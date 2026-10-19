use std::collections::{BTreeMap, BTreeSet};

use caditor_cloud::{
    Design, DesignId, Folder, FolderRef, Project, ProjectId, SendAs, Service, Upload,
};
use egui::{Label, ScrollArea, Sides, Ui};
use jiff::Timestamp;

use crate::{
    appearance::{self, SPACE_L, SPACE_M, SPACE_S},
    cloud::{Cloud, CloudCommand, Destination, Downloads, Link, Linked, Load, Refusal},
    cloud_parts::{self, SendChoice},
    commands::Command,
    export::ExportCommand,
    files::{FileCommand, cloud_send::CloudSendRefusal},
    icons,
    preferences::{CloudPreferences, PreferenceChange},
    widgets::{self, DialogWidth, Tone},
};

pub const TITLE: &str = "Cloud projects";
pub const SIGN_IN: &str = "Sign in";
pub const SIGN_OUT: &str = "Sign out";
pub const CANCEL_SIGN_IN: &str = "Cancel signing in";
pub const REFRESH: &str = "Refresh";
pub const IMPORT: &str = "Import";
pub const VERSIONS: &str = "Versions";
pub const NEWEST_VERSION: &str = "newest";
pub const CLEAR_DOWNLOADS: &str = "Clear downloaded designs";
pub const TURN_OFF: &str = "Turn off cloud features";
pub const TURN_ON: &str = "Turn on cloud services";
pub const OFF_NOTE: &str =
    "Cloud services are turned off, so caditor makes no cloud request and shows no cloud project.";
pub const SHOW_ON_HOME: &str = "Show cloud projects on Home";
pub const SEND_HERE: &str = "Send the model here";
pub const SEND_AS_PART_STUDIO: &str = "Send as a Part Studio";
pub const CHOOSE_A_FOLDER_TO_SEND: &str =
    "Open a folder of a project, then press Send the model here.";
pub const CHOOSE_A_PROJECT: &str = "Choose a project to see its designs.";
pub const OPEN_PREFERENCES: &str = "Open preferences";
const LIST_HEIGHT: f32 = 300.0;
const PROJECT_LIST_WIDTH: f32 = 180.0;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Crumb {
    reference: FolderRef,
    name: String,
}

#[derive(Debug, Default)]
struct Browser {
    project: Option<ProjectId>,
    trail: Vec<Crumb>,
    versions_shown: BTreeSet<DesignId>,
}

impl Browser {
    fn choose(&mut self, project: ProjectId) {
        if self.project.as_ref() != Some(&project) {
            self.project = Some(project);
            self.trail.clear();
        }
    }
}

#[derive(Debug, Default)]
pub struct CloudDialog {
    browsers: BTreeMap<Service, Browser>,
    choosing: bool,
}

impl CloudDialog {
    pub fn choosing_where_to_send() -> Self {
        Self {
            choosing: true,
            ..Self::default()
        }
    }

    pub fn at(project: Option<(Service, ProjectId)>) -> Self {
        let mut dialog = Self::default();
        if let Some((service, project)) = project {
            dialog.browsers.entry(service).or_default().choose(project);
        }
        dialog
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Close,
    Cloud(CloudCommand),
    File(FileCommand),
    Change(PreferenceChange),
    Preferences,
}

pub struct SendView<'a> {
    pub name: String,
    pub availability: Result<(), CloudSendRefusal>,
    pub preparing: Option<&'a Destination>,
}

pub struct DialogView<'a> {
    pub cloud: &'a Cloud,
    pub preferences: &'a CloudPreferences,
    pub now: Timestamp,
    pub sending: SendView<'a>,
}

pub fn dialog(ctx: &egui::Context, view: &DialogView<'_>, state: &mut CloudDialog) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    let response = widgets::dialog(ctx, "cloud", TITLE, DialogWidth::Wide, |ui| {
        widgets::dialog_body(ui, |ui| {
            if view.cloud.is_enabled() {
                if state.choosing {
                    widgets::callout(ui, Tone::Info, |ui| {
                        ui.add(Label::new(CHOOSE_A_FOLDER_TO_SEND).wrap());
                    });
                    ui.add_space(SPACE_L);
                }
                accounts(ui, view, &mut outcomes);
                for service in Service::ALL {
                    if let Some(linked) = view.cloud.linked(service) {
                        ui.add_space(SPACE_L);
                        let browser = state.browsers.entry(service).or_default();
                        projects(
                            ui,
                            view,
                            service,
                            linked,
                            (browser, state.choosing),
                            &mut outcomes,
                        );
                    }
                }
                ui.add_space(SPACE_L);
                downloads(ui, &view.cloud.downloads, &mut outcomes);
                ui.add_space(SPACE_L);
                settings(ui, view.preferences, &mut outcomes);
            } else {
                widgets::callout(ui, Tone::Info, |ui| {
                    ui.add(Label::new(OFF_NOTE).wrap());
                });
            }
        });
        widgets::footer_split(
            ui,
            |ui| {
                if view.cloud.is_enabled() {
                    let off = widgets::button(TURN_OFF);
                    ui.add(off)
                        .on_hover_text(
                            "Sign out, hide cloud features everywhere and make no cloud request",
                        )
                        .clicked()
                        .then_some(Outcome::Change(PreferenceChange::CloudEnabled(false)))
                } else {
                    ui.add(widgets::primary_button(ui, TURN_ON))
                        .clicked()
                        .then_some(Outcome::Change(PreferenceChange::CloudEnabled(true)))
                }
            },
            |ui| {
                ui.add(widgets::primary_button(ui, "Close"))
                    .clicked()
                    .then_some(Outcome::Close)
            },
        )
    });
    let closing = response.should_close();
    outcomes.extend(response.inner);
    if outcomes
        .iter()
        .any(|outcome| matches!(outcome, Outcome::File(FileCommand::SendToCloud(_))))
    {
        state.choosing = false;
    }
    if closing {
        outcomes.push(Outcome::Close);
    }
    outcomes
}

fn accounts(ui: &mut Ui, view: &DialogView<'_>, outcomes: &mut Vec<Outcome>) {
    ui.label(widgets::section_title("Accounts"));
    ui.add_space(SPACE_S);
    for connection in view.cloud.connections() {
        let service = connection.service;
        widgets::card(ui, |ui| {
            Sides::new().show(
                ui,
                |ui| {
                    let tokens = appearance::tokens(ui);
                    widgets::icon_label(ui, icons::command(Command::Cloud), tokens.accent_text);
                    ui.label(widgets::strong(service.name()));
                    cloud_parts::link_pill(ui, &connection.link);
                },
                |ui| account_buttons(ui, view.cloud, service, &connection.link, outcomes),
            );
            account_details(ui, view.cloud, service, connection, outcomes);
        });
    }
}

fn account_buttons(
    ui: &mut Ui,
    cloud: &Cloud,
    service: Service,
    link: &Link,
    outcomes: &mut Vec<Outcome>,
) {
    match link {
        Link::SignedOut => {
            let allowed = cloud.can_sign_in(service);
            let button = widgets::primary_button(ui, SIGN_IN);
            let response = ui
                .add_enabled(allowed.is_ok(), button)
                .on_hover_text(format!(
                    "Opens {}'s sign-in page in your browser",
                    service.company()
                ));
            let reason = allowed.err().map(|refusal| refusal.to_string());
            if response
                .on_disabled_hover_text(reason.unwrap_or_default())
                .clicked()
            {
                outcomes.push(Outcome::Cloud(CloudCommand::SignIn(service)));
            }
        }
        Link::SigningIn(_) => {
            if ui.add(widgets::button(CANCEL_SIGN_IN)).clicked() {
                outcomes.push(Outcome::Cloud(CloudCommand::CancelSignIn(service)));
            }
        }
        Link::Linked(_) => {
            if ui.add(widgets::button(SIGN_OUT)).clicked() {
                outcomes.push(Outcome::Cloud(CloudCommand::SignOut(service)));
            }
            let refresh = widgets::small_button(ui, icons::command(Command::CloudRefresh), REFRESH);
            if ui.add(refresh).clicked() {
                outcomes.push(Outcome::Cloud(CloudCommand::Refresh(service)));
            }
        }
        Link::Off | Link::Starting => {}
    }
}

fn account_details(
    ui: &mut Ui,
    cloud: &Cloud,
    service: Service,
    connection: &crate::cloud::Connection,
    outcomes: &mut Vec<Outcome>,
) {
    ui.add_space(SPACE_S);
    match &connection.link {
        Link::Linked(linked) => match &linked.account {
            Load::Ready(account) => {
                ui.label(widgets::muted(format!("Signed in as {}", account.name), ui));
            }
            Load::Failed(failure) => {
                ui.label(widgets::muted(failure.to_string(), ui));
            }
            Load::Idle | Load::Loading => {
                widgets::progress("Reading your account…").show(ui);
            }
        },
        Link::SigningIn(signing) => {
            ui.add(Label::new(widgets::muted(
                format!(
                    "Finish signing in on {}'s page in your browser.",
                    service.company()
                ),
                ui,
            )));
            if let Some(url) = &signing.url {
                let copy = widgets::small_button(ui, icons::COPY, "Copy the sign-in link");
                if ui.add(copy).clicked() {
                    ui.ctx().copy_text(url.clone());
                }
            }
        }
        Link::SignedOut => {
            if let Some(failure) = &connection.last_failure {
                ui.add(Label::new(widgets::muted(
                    format!("The last sign-in did not finish. {failure}"),
                    ui,
                )));
            } else {
                ui.add(Label::new(widgets::muted(
                    format!(
                        "Sign in to list your {} {}s and import a design with one click.",
                        service.name(),
                        service.project_name()
                    ),
                    ui,
                )));
            }
            if let Err(refusal @ Refusal::NoClientId(_)) = cloud.can_sign_in(service) {
                widgets::callout(ui, Tone::Warning, |ui| {
                    ui.add(Label::new(refusal.to_string()).wrap());
                    let open = widgets::small_button(ui, icons::GENERAL, OPEN_PREFERENCES);
                    if ui.add(open).clicked() {
                        outcomes.push(Outcome::Preferences);
                    }
                });
            }
        }
        Link::Starting => {
            widgets::progress("Checking the saved sign-in…").show(ui);
        }
        Link::Off => {}
    }
}

fn projects(
    ui: &mut Ui,
    view: &DialogView<'_>,
    service: Service,
    linked: &Linked,
    (state, choosing): (&mut Browser, bool),
    outcomes: &mut Vec<Outcome>,
) {
    ui.label(widgets::section_title(&format!(
        "{} {}s",
        service.name(),
        service.project_name()
    )));
    ui.add_space(SPACE_S);
    if let Some(command) = cloud_parts::import_status(ui, service, linked) {
        outcomes.push(Outcome::Cloud(command));
        ui.add_space(SPACE_S);
    }
    if let Some(destination) = view
        .sending
        .preparing
        .filter(|destination| destination.service == service)
    {
        let words = format!(
            "Preparing “{}” to send to {service}, in “{}”…",
            view.sending.name, destination.folder_name
        );
        let stop = widgets::progress(words)
            .cancel(cloud_parts::CANCEL_PREPARING_HOVER)
            .wrapping()
            .show(ui);
        if stop {
            outcomes.push(Outcome::File(FileCommand::Export(ExportCommand::Cancel)));
        }
        ui.add_space(SPACE_S);
    }
    match cloud_parts::send_status(ui, service, linked) {
        Some(SendChoice::Cloud(command)) => outcomes.push(Outcome::Cloud(command)),
        Some(SendChoice::Again(destination)) => {
            outcomes.push(Outcome::File(FileCommand::SendToCloud(destination)));
        }
        None => {}
    }
    if linked.sending.is_some() {
        ui.add_space(SPACE_S);
    }
    hub_row(ui, service, linked, outcomes);
    ui.add_space(SPACE_S);
    match &linked.projects {
        Load::Idle | Load::Loading => {
            widgets::progress("Loading your projects…").show(ui);
        }
        Load::Failed(failure) => {
            if cloud_parts::failure_callout(
                ui,
                Tone::Error,
                "Could not list your projects.",
                failure,
            ) {
                outcomes.push(Outcome::Cloud(CloudCommand::Refresh(service)));
            }
        }
        Load::Ready(projects) if projects.is_empty() => {
            ui.label(widgets::muted(
                format!("No {}s to list yet.", service.project_name()),
                ui,
            ));
        }
        Load::Ready(projects) => {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(PROJECT_LIST_WIDTH);
                    project_list(ui, projects, state);
                });
                ui.vertical(|ui| {
                    contents(
                        ui,
                        view,
                        service,
                        linked,
                        projects,
                        (state, choosing),
                        outcomes,
                    );
                });
            });
        }
    }
}

fn hub_row(ui: &mut Ui, service: Service, linked: &Linked, outcomes: &mut Vec<Outcome>) {
    let Some(workspaces) = linked
        .workspaces
        .ready()
        .filter(|_| service.has_workspaces())
    else {
        return;
    };
    let noun = service.workspace_noun();
    let current = linked.chosen_workspace().map_or_else(
        || format!("Choose a {noun}"),
        |workspace| workspace.name.clone(),
    );
    ui.horizontal(|ui| {
        ui.label(capitalised(noun));
        widgets::combo(("cloud-hub", service.id()), current)
            .hover(format!(
                "The {noun} whose {}s are listed",
                service.project_name()
            ))
            .show(ui, |ui| {
                for workspace in workspaces {
                    let chosen = linked.workspace.as_ref() == Some(&workspace.id);
                    if widgets::menu_option(ui, chosen, &workspace.name).clicked() && !chosen {
                        outcomes.push(Outcome::Cloud(CloudCommand::ChooseWorkspace(
                            service,
                            workspace.id.clone(),
                        )));
                    }
                }
            });
    });
}

fn capitalised(word: &str) -> String {
    let mut letters = word.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
}

fn project_list(ui: &mut Ui, projects: &[Project], state: &mut Browser) {
    ScrollArea::vertical()
        .id_salt("cloud projects")
        .max_height(LIST_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for project in projects {
                let chosen = state.project.as_ref() == Some(&project.id);
                if widgets::menu_option(ui, chosen, &project.name).clicked() {
                    state.choose(project.id.clone());
                }
            }
        });
}

fn contents(
    ui: &mut Ui,
    view: &DialogView<'_>,
    service: Service,
    linked: &Linked,
    projects: &[Project],
    (state, choosing): (&mut Browser, bool),
    outcomes: &mut Vec<Outcome>,
) {
    let Some(project) = state
        .project
        .as_ref()
        .and_then(|id| projects.iter().find(|project| project.id == *id))
    else {
        ui.label(widgets::muted(CHOOSE_A_PROJECT, ui));
        return;
    };
    breadcrumbs(ui, project, state);
    let (load, reload, open) = match state.trail.last() {
        Some(crumb) => (
            linked.folder(&crumb.reference),
            CloudCommand::ReloadFolder(service, crumb.reference.clone()),
            CloudCommand::OpenFolder(service, crumb.reference.clone()),
        ),
        None => (
            linked.root(&project.id),
            CloudCommand::ReloadProject(service, project.id.clone()),
            CloudCommand::LoadProject(service, project.id.clone()),
        ),
    };
    if matches!(load, Load::Idle) {
        outcomes.push(Outcome::Cloud(open));
    }
    match load {
        Load::Idle | Load::Loading => {
            widgets::progress("Loading designs…").show(ui);
        }
        Load::Failed(failure) => {
            if cloud_parts::failure_callout(
                ui,
                Tone::Warning,
                "Could not open this folder.",
                failure,
            ) {
                outcomes.push(Outcome::Cloud(reload));
            }
        }
        Load::Ready(folder) => {
            let folder_name = state
                .trail
                .last()
                .map_or_else(|| project.name.clone(), |crumb| crumb.name.clone());
            send_row(
                ui,
                view,
                service,
                linked,
                (folder, &folder_name, choosing),
                outcomes,
            );
            ScrollArea::vertical()
                .id_salt("cloud designs")
                .max_height(LIST_HEIGHT)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if folder.folders.is_empty()
                        && folder.designs.is_empty()
                        && folder.uploads.is_empty()
                    {
                        ui.label(widgets::muted("Nothing to import in this folder.", ui));
                    }
                    for entry in &folder.folders {
                        let tokens = appearance::tokens(ui);
                        ui.horizontal(|ui| {
                            widgets::icon_label(ui, icons::FEATURE_GROUP, tokens.text_muted);
                            if widgets::link(ui, &entry.name, None).clicked() {
                                state.trail.push(Crumb {
                                    reference: entry.reference.clone(),
                                    name: entry.name.clone(),
                                });
                            }
                        });
                    }
                    for design in &folder.designs {
                        design_row(
                            ui,
                            view,
                            service,
                            linked,
                            design,
                            &mut state.versions_shown,
                            outcomes,
                        );
                    }
                    for upload in &folder.uploads {
                        upload_row(ui, view, service, linked, upload, outcomes);
                    }
                });
        }
    }
}

fn send_row(
    ui: &mut Ui,
    view: &DialogView<'_>,
    service: Service,
    linked: &Linked,
    (folder, folder_name, choosing): (&Folder, &str, bool),
    outcomes: &mut Vec<Outcome>,
) {
    let busy = linked.is_sending() || view.sending.preparing.is_some();
    let reason = match (&folder.place, busy, &view.sending.availability) {
        (None, _, _) => Some(match service {
            Service::Fusion => "Open one of the project's folders first; files go inside a \
                                folder, not beside the folders at the top"
                .to_owned(),
            Service::Onshape => "A saved version cannot be changed; open the document's main \
                                 workspace or one of its branches"
                .to_owned(),
        }),
        (_, true, _) => {
            Some("Another model is being sent; wait for it or cancel it first".to_owned())
        }
        (_, _, Err(refusal)) => Some(format!("{refusal}.")),
        _ => None,
    };
    let hover = format!(
        "Export the bodies chosen for export as STEP and add “{}” to this folder of {service}, \
         or as a new version if a file of that name is already there",
        view.sending.name
    );
    let enabled = reason.is_none();
    let disabled = reason.unwrap_or_default();
    let mut how = None;
    ui.horizontal_wrapped(|ui| {
        let response = if choosing {
            ui.add_enabled(enabled, widgets::primary_button(ui, SEND_HERE))
        } else {
            ui.add_enabled(enabled, widgets::button(SEND_HERE))
        };
        if response
            .on_hover_text(hover)
            .on_disabled_hover_text(&disabled)
            .clicked()
        {
            how = Some(SendAs::File);
        }
        if service.sends_part_studios()
            && ui
                .add_enabled(enabled, widgets::button(SEND_AS_PART_STUDIO))
                .on_hover_text(format!(
                    "Export the bodies chosen for export as STEP and ask {service} to import \
                     “{}” as a new Part Studio in this workspace",
                    view.sending.name
                ))
                .on_disabled_hover_text(&disabled)
                .clicked()
        {
            how = Some(SendAs::PartStudio);
        }
    });
    if let Some(how) = how
        && let Some(place) = &folder.place
    {
        outcomes.push(Outcome::File(FileCommand::SendToCloud(Destination {
            service,
            folder: place.clone(),
            folder_name: folder_name.to_owned(),
            how,
        })));
    }
    ui.add_space(SPACE_S);
}

fn breadcrumbs(ui: &mut Ui, project: &Project, state: &mut Browser) {
    ui.horizontal_wrapped(|ui| {
        let at_root = state.trail.is_empty();
        if at_root {
            ui.label(widgets::strong(&project.name));
        } else if widgets::link(ui, &project.name, None).clicked() {
            state.trail.clear();
        }
        let last = state.trail.len();
        let mut jump = None;
        for (index, crumb) in state.trail.iter().enumerate() {
            ui.label(widgets::muted(icons::BREADCRUMB, ui));
            if index + 1 == last {
                ui.label(widgets::strong(&crumb.name));
            } else if widgets::link(ui, &crumb.name, None).clicked() {
                jump = Some(index + 1);
            }
        }
        if let Some(keep) = jump {
            state.trail.truncate(keep);
        }
    });
    ui.add_space(SPACE_S);
}

fn upload_row(
    ui: &mut Ui,
    view: &DialogView<'_>,
    service: Service,
    linked: &Linked,
    upload: &Upload,
    outcomes: &mut Vec<Outcome>,
) {
    let tokens = appearance::tokens(ui);
    Sides::new().shrink_left().show(
        ui,
        |ui| {
            widgets::icon_label(
                ui,
                cloud_parts::upload_icon(upload.format),
                tokens.accent_text,
            );
            ui.add(Label::new(&upload.name).truncate());
            ui.label(widgets::muted(
                cloud_parts::modified_text(upload.modified, view.now),
                ui,
            ));
        },
        |ui| {
            let button = widgets::small_button(ui, icons::command(Command::Import), IMPORT);
            let importing = linked.is_importing();
            let response = ui
                .add_enabled(!importing, button)
                .on_hover_text(format!(
                    "Download “{}” from {service} and add it to the model",
                    upload.name
                ))
                .on_disabled_hover_text(
                    "Another import is running; wait for it or cancel it first",
                );
            if response.clicked() {
                outcomes.push(Outcome::Cloud(CloudCommand::ImportUpload(
                    service,
                    upload.clone(),
                )));
            }
        },
    );
}

fn design_row(
    ui: &mut Ui,
    view: &DialogView<'_>,
    service: Service,
    linked: &Linked,
    design: &Design,
    versions_shown: &mut BTreeSet<DesignId>,
    outcomes: &mut Vec<Outcome>,
) {
    let tokens = appearance::tokens(ui);
    Sides::new().shrink_left().show(
        ui,
        |ui| {
            widgets::icon_label(ui, icons::BODIES, tokens.accent_text);
            ui.add(Label::new(&design.name).truncate());
            ui.label(widgets::muted(
                cloud_parts::modified_text(design.modified, view.now),
                ui,
            ));
        },
        |ui| {
            let button = widgets::small_button(ui, icons::command(Command::Import), IMPORT);
            let importing = linked.is_importing();
            let response = ui
                .add_enabled(!importing, button)
                .on_hover_text(format!(
                    "Have {service} convert “{}” to STEP and add it to the model",
                    design.name
                ))
                .on_disabled_hover_text(
                    "Another design is being imported; wait for it or cancel it first",
                );
            if response.clicked() {
                outcomes.push(Outcome::Cloud(CloudCommand::Import(
                    service,
                    design.clone(),
                )));
            }
            if service.has_design_versions() {
                let shown = versions_shown.contains(&design.id);
                let toggle = widgets::small_button(ui, icons::DESIGN_VERSIONS, VERSIONS);
                let toggle = ui.add(toggle).on_hover_text(format!(
                    "Choose which saved version of “{}” to import",
                    design.name
                ));
                if toggle.clicked() {
                    if shown {
                        versions_shown.remove(&design.id);
                    } else {
                        versions_shown.insert(design.id.clone());
                        outcomes.push(Outcome::Cloud(CloudCommand::LoadVersions(
                            service,
                            design.clone(),
                        )));
                    }
                }
            }
        },
    );
    if versions_shown.contains(&design.id) {
        ui.indent(("versions", design.id.as_str()), |ui| {
            version_rows(ui, view, service, linked, design, outcomes);
        });
    }
}

fn version_rows(
    ui: &mut Ui,
    view: &DialogView<'_>,
    service: Service,
    linked: &Linked,
    design: &Design,
    outcomes: &mut Vec<Outcome>,
) {
    match linked.versions(&design.id) {
        Load::Idle | Load::Loading => {
            widgets::progress("Loading versions…").show(ui);
        }
        Load::Failed(failure) => {
            if cloud_parts::failure_callout(
                ui,
                Tone::Warning,
                "Could not list the versions.",
                failure,
            ) {
                outcomes.push(Outcome::Cloud(CloudCommand::ReloadVersions(
                    service,
                    design.clone(),
                )));
            }
        }
        Load::Ready(versions) if versions.is_empty() => {
            ui.label(widgets::muted("This design has no saved versions.", ui));
        }
        Load::Ready(versions) => {
            for (index, version) in versions.iter().enumerate() {
                Sides::new().shrink_left().show(
                    ui,
                    |ui| {
                        ui.add(Label::new(format!("Version {}", version.number)).truncate());
                        if index == 0 {
                            ui.label(widgets::muted(NEWEST_VERSION, ui));
                        }
                        ui.label(widgets::muted(
                            cloud_parts::modified_text(version.modified, view.now),
                            ui,
                        ));
                    },
                    |ui| {
                        let button =
                            widgets::small_button(ui, icons::command(Command::Import), IMPORT);
                        let response = ui
                            .add_enabled(!linked.is_importing(), button)
                            .on_hover_text(format!(
                                "Have {service} convert version {} of “{}” to STEP and add it to \
                                 the model",
                                version.number, design.name
                            ))
                            .on_disabled_hover_text(
                                "Another design is being imported; wait for it or cancel it first",
                            );
                        if response.clicked() {
                            outcomes.push(Outcome::Cloud(CloudCommand::ImportVersion(
                                service,
                                design.clone(),
                                version.clone(),
                            )));
                        }
                    },
                );
            }
        }
    }
}

fn downloads(ui: &mut Ui, downloads: &Downloads, outcomes: &mut Vec<Outcome>) {
    ui.label(widgets::section_title("Downloaded designs"));
    ui.add_space(SPACE_S);
    let Some(dir) = &downloads.dir else {
        ui.label(widgets::muted(
            "caditor has no folder to keep downloaded designs in, so imports from the cloud are unavailable.",
            ui,
        ));
        return;
    };
    if downloads.usage.is_none() && !downloads.measuring {
        outcomes.push(Outcome::Cloud(CloudCommand::MeasureDownloads));
    }
    let summary = match downloads.usage {
        Some(usage) if usage.designs == 0 => "No design is kept yet.".to_owned(),
        Some(usage) => {
            let plural = if usage.designs == 1 {
                "design"
            } else {
                "designs"
            };
            format!(
                "{} {plural} kept, {}.",
                usage.designs,
                cloud_parts::size_text(usage.bytes)
            )
        }
        None => "Counting…".to_owned(),
    };
    ui.add(
        Label::new(widgets::muted(
            format!(
                "{summary} They sit in {}, so an import can be reloaded from its file.",
                dir.display()
            ),
            ui,
        ))
        .wrap(),
    );
    ui.add_space(SPACE_S);
    let empty = downloads.usage.is_none_or(|usage| usage.designs == 0);
    let clear = widgets::small_button(ui, icons::DELETE, CLEAR_DOWNLOADS);
    let response = ui
        .add_enabled(!empty && !downloads.measuring, clear)
        .on_hover_text("Delete the kept STEP files; models already imported keep their bodies")
        .on_disabled_hover_text("No design is kept");
    if response.clicked() {
        outcomes.push(Outcome::Cloud(CloudCommand::ClearDownloads));
    }
}

fn settings(ui: &mut Ui, preferences: &CloudPreferences, outcomes: &mut Vec<Outcome>) {
    ui.label(widgets::section_title("Cloud features"));
    ui.add_space(SPACE_M - SPACE_S);
    let mut on_home = preferences.on_home;
    if widgets::checkbox(ui, &mut on_home, SHOW_ON_HOME)
        .on_hover_text("Off keeps cloud projects to this window and the palette")
        .changed()
    {
        outcomes.push(Outcome::Change(PreferenceChange::CloudOnHome(on_home)));
    }
}
