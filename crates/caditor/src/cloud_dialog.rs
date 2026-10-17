use caditor_cloud::{FolderRef, Project, ProjectId, Service};
use egui::{Label, ScrollArea, Sides, Ui};
use jiff::Timestamp;

use crate::{
    appearance::{self, SPACE_L, SPACE_M, SPACE_S},
    cloud::{Cloud, CloudCommand, Downloads, Link, Linked, Load, Refusal},
    cloud_parts,
    commands::Command,
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
pub const CLEAR_DOWNLOADS: &str = "Clear downloaded designs";
pub const TURN_OFF: &str = "Turn off cloud features";
pub const TURN_ON: &str = "Turn on cloud services";
pub const OFF_NOTE: &str =
    "Cloud services are turned off, so caditor makes no cloud request and shows no cloud project.";
pub const SHOW_ON_HOME: &str = "Show cloud projects on Home";
pub const CHOOSE_A_PROJECT: &str = "Choose a project to see its designs.";
pub const HUB: &str = "Hub";
pub const OPEN_PREFERENCES: &str = "Open preferences";
const LIST_HEIGHT: f32 = 300.0;
const PROJECT_LIST_WIDTH: f32 = 180.0;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Crumb {
    reference: FolderRef,
    name: String,
}

#[derive(Debug, Default)]
pub struct CloudDialog {
    project: Option<ProjectId>,
    trail: Vec<Crumb>,
}

impl CloudDialog {
    pub fn at(project: Option<ProjectId>) -> Self {
        Self {
            project,
            trail: Vec::new(),
        }
    }

    fn choose(&mut self, project: ProjectId) {
        if self.project.as_ref() != Some(&project) {
            self.project = Some(project);
            self.trail.clear();
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Close,
    Cloud(CloudCommand),
    Change(PreferenceChange),
    Preferences,
}

pub struct DialogView<'a> {
    pub cloud: &'a Cloud,
    pub preferences: &'a CloudPreferences,
    pub now: Timestamp,
}

pub fn dialog(ctx: &egui::Context, view: &DialogView<'_>, state: &mut CloudDialog) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    let response = widgets::dialog(ctx, "cloud", TITLE, DialogWidth::Wide, |ui| {
        widgets::dialog_body(ui, |ui| {
            if view.cloud.is_enabled() {
                accounts(ui, view, &mut outcomes);
                if let Some(linked) = view.cloud.linked(Service::Fusion) {
                    ui.add_space(SPACE_L);
                    projects(ui, view, Service::Fusion, linked, state, &mut outcomes);
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
                .on_hover_text("Opens Autodesk's sign-in page in your browser");
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
                "Finish signing in on Autodesk's page in your browser.",
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
                    "Sign in to list your Fusion projects and import a design with one click.",
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
    state: &mut CloudDialog,
    outcomes: &mut Vec<Outcome>,
) {
    ui.label(widgets::section_title("Projects"));
    ui.add_space(SPACE_S);
    if let Some(command) = cloud_parts::import_status(ui, service, linked) {
        outcomes.push(Outcome::Cloud(command));
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
            ui.label(widgets::muted("This hub has no projects yet.", ui));
        }
        Load::Ready(projects) => {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(PROJECT_LIST_WIDTH);
                    project_list(ui, projects, state);
                });
                ui.vertical(|ui| {
                    contents(ui, view, service, linked, projects, state, outcomes);
                });
            });
        }
    }
}

fn hub_row(ui: &mut Ui, service: Service, linked: &Linked, outcomes: &mut Vec<Outcome>) {
    let Some(workspaces) = linked.workspaces.ready() else {
        return;
    };
    let current = linked
        .chosen_workspace()
        .map_or("Choose a hub", |workspace| workspace.name.as_str());
    ui.horizontal(|ui| {
        ui.label(HUB);
        widgets::combo(("cloud-hub", service.id()), current)
            .hover("The Fusion team or personal hub whose projects are listed")
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

fn project_list(ui: &mut Ui, projects: &[Project], state: &mut CloudDialog) {
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
    state: &mut CloudDialog,
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
            ScrollArea::vertical()
                .id_salt("cloud designs")
                .max_height(LIST_HEIGHT)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if folder.folders.is_empty() && folder.designs.is_empty() {
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
                        design_row(ui, view, service, linked, design, outcomes);
                    }
                });
        }
    }
}

fn breadcrumbs(ui: &mut Ui, project: &Project, state: &mut CloudDialog) {
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

fn design_row(
    ui: &mut Ui,
    view: &DialogView<'_>,
    service: Service,
    linked: &Linked,
    design: &caditor_cloud::Design,
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
        },
    );
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
