use std::path::PathBuf;

use caditor_cloud::{Design, ProjectId, Service};
use egui::{
    Align, Area, Id, Label, Layout, Order, Rect, RichText, ScrollArea, Sense, Sides, TextStyle, Ui,
    UiBuilder, vec2,
};
use jiff::Timestamp;

use crate::{
    appearance::{self, SPACE_L, SPACE_M, SPACE_S, SPACE_XL},
    cloud::{Cloud, CloudCommand, Link, Linked, Load, PROJECT_ROWS_ON_HOME, Refusal, SigningIn},
    cloud_parts::{self, Tile},
    commands::Command,
    editing::SketchEditing,
    files::Files,
    icons, logo,
    model::{Model, display_name},
    onboarding::{folder_name, sample_icon},
    samples::Sample,
    widgets::{self, Tone},
};

pub const TITLE: &str = "Home";
pub const CLOSE: &str = "Close Home (Esc)";
pub const RECENT_TITLE: &str = "Recent files";
pub const NO_RECENT: &str = "Models you open or save show up here.";
pub const SAMPLES_TITLE: &str = "Learn from a sample";
pub const FUSION_TITLE: &str = "Fusion 360";
pub const ADVERT_TITLE: &str = "Bring your Fusion 360 designs into caditor";
pub const ADVERT_TEXT: &str = "Sign in with your Autodesk account and your Fusion projects show \
                               up right here. One click has Autodesk convert a design to STEP, \
                               and caditor imports it as a body you can keep modelling on.";
pub const ADVERT_POINTS: [&str; 3] = [
    "You sign in on Autodesk's own page; caditor never sees your password.",
    "caditor only asks Autodesk to convert the design you click, and never changes your designs.",
    "Sign out at any time, or turn cloud features off and caditor makes no cloud request at all.",
];
pub const SIGN_IN: &str = "Sign in to Fusion 360";
pub const ONSHAPE_TITLE: &str = "Onshape";
pub const ONSHAPE_ADVERT_TITLE: &str = "Bring your Onshape documents into caditor";
pub const ONSHAPE_ADVERT_TEXT: &str = "Sign in with your Onshape account and your documents show \
                                       up right here. One click has Onshape export a Part Studio \
                                       or assembly to STEP, and caditor imports it as a body you \
                                       can keep modelling on.";
pub const ONSHAPE_ADVERT_POINTS: [&str; 3] = [
    "You sign in on Onshape's own page; caditor never sees your password.",
    "caditor only asks Onshape to export the design you click, and never changes your documents.",
    "Sign out at any time, or turn cloud features off and caditor makes no cloud request at all.",
];
pub const ONSHAPE_SIGN_IN: &str = "Sign in to Onshape";
pub const MANAGE: &str = "Cloud projects…";
pub const TURN_OFF: &str = "Turn off cloud features";
pub const TURN_OFF_HOVER: &str = "Hide cloud features everywhere. caditor makes no cloud request, \
                                  and Preferences › General › Cloud turns them back on";
pub const WAITING_FOR_BROWSER: &str =
    "Waiting for you to finish signing in on Autodesk's page in your browser…";
pub const ONSHAPE_WAITING_FOR_BROWSER: &str =
    "Waiting for you to finish signing in on Onshape's page in your browser…";
pub const COPY_LINK: &str = "Copy the sign-in link";
pub const OPEN_PREFERENCES: &str = "Open preferences";
pub const REFRESH: &str = "Refresh cloud projects";
pub const SIGN_OUT: &str = "Sign out";
pub const HIDE_AT_START: &str = "Don't show Home when caditor starts";
pub const LOADING_PROJECTS: &str = "Loading your projects…";
pub const LOADING_DESIGNS: &str = "Loading designs…";
const START_HOVER: &str = "Sketch on the plane or flat face you click next, then extrude it";
const BOX_HOVER: &str = "Add a box and set its sizes in its panel";
const IMPORT_HOVER: &str = "Bring in a STEP, mesh, DXF or SVG file";
const OPEN_HOVER: &str = "Choose a model file to open";
const MAX_WIDTH: f32 = 1040.0;
const PAGE_MARGIN: f32 = SPACE_XL;
const LOGO_SIDE: f32 = 40.0;
const TILE_WIDTH: f32 = 250.0;
const DESIGNS_PER_PROJECT: usize = 8;
const START_BUTTON_HEIGHT: f32 = 30.0;

struct Wording {
    title: &'static str,
    advert_title: &'static str,
    advert_text: &'static str,
    advert_points: [&'static str; 3],
    sign_in: &'static str,
    waiting: &'static str,
    checking: &'static str,
    no_projects: &'static str,
    no_designs: &'static str,
}

const FUSION_WORDING: Wording = Wording {
    title: FUSION_TITLE,
    advert_title: ADVERT_TITLE,
    advert_text: ADVERT_TEXT,
    advert_points: ADVERT_POINTS,
    sign_in: SIGN_IN,
    waiting: WAITING_FOR_BROWSER,
    checking: "Checking your Fusion 360 sign-in…",
    no_projects: "This hub has no projects yet.",
    no_designs: "No Fusion designs here yet.",
};

const ONSHAPE_WORDING: Wording = Wording {
    title: ONSHAPE_TITLE,
    advert_title: ONSHAPE_ADVERT_TITLE,
    advert_text: ONSHAPE_ADVERT_TEXT,
    advert_points: ONSHAPE_ADVERT_POINTS,
    sign_in: ONSHAPE_SIGN_IN,
    waiting: ONSHAPE_WAITING_FOR_BROWSER,
    checking: "Checking your Onshape sign-in…",
    no_projects: "You have no Onshape documents yet.",
    no_designs: "No Part Studios or assemblies in this document yet.",
};

fn wording(service: Service) -> &'static Wording {
    match service {
        Service::Fusion => &FUSION_WORDING,
        Service::Onshape => &ONSHAPE_WORDING,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Basis {
    session: u64,
    revision: u64,
}

impl Basis {
    fn of(model: &Model) -> Self {
        Self {
            session: model.session(),
            revision: model.revision(),
        }
    }
}

#[derive(Debug, Default)]
pub struct Home {
    basis: Option<Basis>,
    requested: bool,
}

impl Home {
    pub fn open(&mut self, model: &Model) {
        self.basis = Some(Basis::of(model));
    }

    pub fn request(&mut self) {
        self.requested = true;
    }

    pub fn close(&mut self) {
        self.basis = None;
        self.requested = false;
    }

    pub fn is_open(&self) -> bool {
        self.basis.is_some()
    }

    pub fn follow(&mut self, model: &Model, editing: &SketchEditing) {
        if std::mem::take(&mut self.requested) {
            self.basis = Some(Basis::of(model));
            return;
        }
        let moved_on = self.basis.is_some_and(|basis| basis != Basis::of(model));
        if moved_on || editing.active().is_some() || editing.is_choosing_plane() {
            self.basis = None;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Close,
    Start(Command),
    Open,
    OpenRecent(PathBuf),
    LoadRecentPicture(PathBuf),
    ClearRecent,
    Sample(Sample),
    Cloud(CloudCommand),
    ManageCloud(Option<(Service, ProjectId)>),
    Preferences,
    TurnOffCloud,
    HideAtStart,
}

pub struct HomeView<'a> {
    pub files: &'a Files,
    pub cloud: &'a Cloud,
    pub now: Timestamp,
}

pub fn show(ctx: &egui::Context, area: Rect, view: &HomeView<'_>) -> Vec<Choice> {
    let mut choices = Vec::new();
    let tokens = appearance::tokens_of(ctx);
    Area::new(Id::unique("home page"))
        .order(Order::Middle)
        .fixed_pos(area.min)
        .constrain(false)
        .show(ctx, |ui| {
            ui.set_clip_rect(area);
            ui.painter().rect_filled(area, 0.0, tokens.panel);
            ui.interact(
                area,
                Id::unique("home page backdrop"),
                Sense::click_and_drag(),
            );
            let mut page = ui.new_child(
                UiBuilder::new()
                    .max_rect(area)
                    .layout(Layout::top_down(Align::Min)),
            );
            ScrollArea::vertical()
                .id_salt("home scroll")
                .auto_shrink([false, false])
                .show(&mut page, |ui| {
                    let total = ui.available_width();
                    let width = (total - 2.0 * PAGE_MARGIN).clamp(0.0, MAX_WIDTH);
                    let side = ((total - width) / 2.0).max(0.0);
                    ui.horizontal_top(|ui| {
                        ui.add_space(side);
                        ui.vertical(|ui| {
                            ui.set_width(width);
                            sections(ui, view, &mut choices);
                        });
                    });
                });
        });
    choices
}

fn sections(ui: &mut Ui, view: &HomeView<'_>, choices: &mut Vec<Choice>) {
    ui.add_space(SPACE_XL);
    header(ui, choices);
    ui.add_space(SPACE_L);
    starts(ui, choices);
    ui.add_space(SPACE_XL);
    recent(ui, view.files, choices);
    if view.cloud.shows_on_home() {
        ui.add_space(SPACE_XL);
        cloud(ui, view, choices);
    }
    ui.add_space(SPACE_XL);
    samples(ui, choices);
    ui.add_space(SPACE_XL);
    footer(ui, choices);
    ui.add_space(SPACE_XL);
}

fn header(ui: &mut Ui, choices: &mut Vec<Choice>) {
    let closed = Sides::new()
        .show(
            ui,
            |ui| {
                logo::show(ui, LOGO_SIDE);
                ui.vertical(|ui| {
                    ui.label(RichText::new(TITLE).text_style(TextStyle::Heading));
                    ui.label(widgets::muted(
                        "Start a model, pick up where you left off or bring one in",
                        ui,
                    ));
                });
            },
            |ui| widgets::icon_button(ui, icons::CLOSE, CLOSE).clicked(),
        )
        .1;
    if closed {
        choices.push(Choice::Close);
    }
}

fn starts(ui: &mut Ui, choices: &mut Vec<Choice>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_M;
        let sketch = widgets::primary_icon_button(
            ui,
            icons::command(Command::NewSketch),
            crate::start_card::START_WITH_SKETCH,
        )
        .min_size(vec2(0.0, START_BUTTON_HEIGHT));
        if ui.add(sketch).on_hover_text(START_HOVER).clicked() {
            choices.push(Choice::Start(Command::NewSketch));
        }
        for (command, label, hover) in [
            (
                Command::NewBox,
                crate::start_card::START_WITH_BOX,
                BOX_HOVER,
            ),
            (
                Command::Import,
                crate::start_card::START_WITH_IMPORT,
                IMPORT_HOVER,
            ),
        ] {
            let button = widgets::small_button(ui, icons::command(command), label);
            if ui.add(button).on_hover_text(hover).clicked() {
                choices.push(Choice::Start(command));
            }
        }
        let open = widgets::small_button(ui, icons::command(Command::Open), &Command::Open.title());
        if ui.add(open).on_hover_text(OPEN_HOVER).clicked() {
            choices.push(Choice::Open);
        }
    });
}

fn recent(ui: &mut Ui, files: &Files, choices: &mut Vec<Choice>) {
    let recent = files.recent();
    Sides::new().show(
        ui,
        |ui| ui.label(widgets::section_title(RECENT_TITLE)),
        |ui| {
            if recent.is_empty() {
                return;
            }
            let clear = widgets::small_button(
                ui,
                icons::command(Command::ClearRecent),
                &Command::ClearRecent.title(),
            );
            if ui
                .add(clear)
                .on_hover_text("Forget these files; the files themselves stay where they are")
                .clicked()
            {
                choices.push(Choice::ClearRecent);
            }
        },
    );
    ui.add_space(SPACE_S);
    if recent.is_empty() {
        ui.label(widgets::muted(NO_RECENT, ui));
        return;
    }
    let mut wanted = Vec::new();
    let chosen = cloud_parts::tiles(ui, recent.len(), TILE_WIDTH, |ui, at, width| {
        let Some(path) = recent.get(at) else {
            return false;
        };
        let name = display_name(Some(path));
        let missing = files.is_missing(path);
        let folder = folder_name(path);
        let detail = if missing {
            format!("{folder} (not found)")
        } else {
            folder
        };
        let place = Rect::from_min_size(
            ui.next_widget_position(),
            vec2(width, cloud_parts::TILE_HEIGHT),
        );
        let picture = files
            .recent_picture(path)
            .map(|picture| picture.texture(ui.ctx(), &format!("picture of {}", path.display())));
        if files.wants_recent_picture(path)
            && files.has_room_for_recent_picture()
            && ui.is_rect_visible(place)
        {
            wanted.push(Choice::LoadRecentPicture(path.clone()));
        }
        cloud_parts::tile(
            ui,
            width,
            &Tile {
                glyph: icons::FILE,
                title: &name,
                detail: &detail,
                name: &format!("Open {name}"),
                hover: &path.display().to_string(),
                enabled: true,
                picture,
            },
        )
    });
    choices.append(&mut wanted);
    if let Some(path) = chosen.and_then(|at| recent.get(at)) {
        choices.push(Choice::OpenRecent(path.clone()));
    }
}

fn samples(ui: &mut Ui, choices: &mut Vec<Choice>) {
    ui.label(widgets::section_title(SAMPLES_TITLE));
    ui.add_space(SPACE_S);
    let chosen = cloud_parts::tiles(ui, Sample::ALL.len(), TILE_WIDTH, |ui, at, width| {
        let Some(sample) = Sample::ALL.get(at).copied() else {
            return false;
        };
        cloud_parts::tile(
            ui,
            width,
            &Tile {
                glyph: sample_icon(sample),
                title: sample.title(),
                detail: sample.description(),
                name: &Command::OpenSample(sample).title(),
                hover: sample.description(),
                enabled: true,
                picture: None,
            },
        )
    });
    if let Some(sample) = chosen.and_then(|at| Sample::ALL.get(at)) {
        choices.push(Choice::Sample(*sample));
    }
}

fn footer(ui: &mut Ui, choices: &mut Vec<Choice>) {
    ui.horizontal_wrapped(|ui| {
        ui.label(widgets::muted(
            "Home opens when caditor starts and from File › Home.",
            ui,
        ));
        let hide = widgets::small_button(ui, icons::HIDE, HIDE_AT_START);
        if ui
            .add(hide)
            .on_hover_text("Turn this off in Preferences › General › Starting and recent files")
            .clicked()
        {
            choices.push(Choice::HideAtStart);
        }
    });
}

fn cloud(ui: &mut Ui, view: &HomeView<'_>, choices: &mut Vec<Choice>) {
    let mut options_offered = false;
    let mut first = true;
    for service in Service::ALL {
        let Some(connection) = view.cloud.connection(service) else {
            continue;
        };
        if matches!(connection.link, Link::Off) {
            continue;
        }
        if !std::mem::take(&mut first) {
            ui.add_space(SPACE_XL);
        }
        let words = wording(service);
        match &connection.link {
            Link::Off => {}
            Link::Starting => {
                ui.label(widgets::section_title(words.title));
                ui.add_space(SPACE_S);
                widgets::progress(words.checking).show(ui);
            }
            Link::SignedOut => {
                let with_options = !std::mem::replace(&mut options_offered, true);
                advert(ui, view.cloud, service, with_options, choices);
            }
            Link::SigningIn(signing) => signing_in(ui, service, signing, choices),
            Link::Linked(linked) => linked_section(ui, view, service, linked, choices),
        }
    }
}

fn advert(
    ui: &mut Ui,
    cloud: &Cloud,
    service: Service,
    with_options: bool,
    choices: &mut Vec<Choice>,
) {
    let tokens = appearance::tokens(ui);
    let words = wording(service);
    cloud_parts::page_card(ui, |ui| {
        ui.horizontal(|ui| {
            widgets::icon_label(ui, icons::command(Command::Cloud), tokens.accent_text);
            ui.label(widgets::strong(words.advert_title));
        });
        ui.add_space(SPACE_S);
        ui.add(Label::new(widgets::muted(words.advert_text, ui)).wrap());
        ui.add_space(SPACE_M);
        for point in words.advert_points {
            ui.horizontal_top(|ui| {
                widgets::icon_label(ui, icons::UP_TO_DATE, tokens.success);
                ui.add(Label::new(point).wrap());
            });
        }
        ui.add_space(SPACE_L);
        if let Err(refusal @ Refusal::NoClientId(_)) = cloud.can_sign_in(service) {
            widgets::callout(ui, Tone::Warning, |ui| {
                ui.add(Label::new(refusal.to_string()).wrap());
                let open = widgets::small_button(ui, icons::GENERAL, OPEN_PREFERENCES);
                if ui.add(open).clicked() {
                    choices.push(Choice::Preferences);
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_M;
            let can_sign_in = cloud.can_sign_in(service).is_ok();
            let sign_in = widgets::primary_icon_button(
                ui,
                icons::command(sign_in_command(service)),
                words.sign_in,
            )
            .min_size(vec2(0.0, START_BUTTON_HEIGHT));
            let response = ui.add_enabled(can_sign_in, sign_in);
            if response
                .on_hover_text(format!(
                    "Opens {}'s sign-in page in your browser",
                    service.company()
                ))
                .on_disabled_hover_text(format!(
                    "This build of caditor has no {} registered",
                    service.app_name()
                ))
                .clicked()
            {
                choices.push(Choice::Cloud(CloudCommand::SignIn(service)));
            }
            if !with_options {
                return;
            }
            let manage = widgets::small_button(ui, icons::command(Command::Cloud), MANAGE);
            if ui.add(manage).clicked() {
                choices.push(Choice::ManageCloud(None));
            }
            let off = widgets::small_button(ui, icons::HIDE, TURN_OFF);
            if ui.add(off).on_hover_text(TURN_OFF_HOVER).clicked() {
                choices.push(Choice::TurnOffCloud);
            }
        });
    });
}

fn sign_in_command(service: Service) -> Command {
    match service {
        Service::Fusion => Command::CloudSignIn,
        Service::Onshape => Command::OnshapeSignIn,
    }
}

fn sign_out_command(service: Service) -> Command {
    match service {
        Service::Fusion => Command::CloudSignOut,
        Service::Onshape => Command::OnshapeSignOut,
    }
}

fn signing_in(ui: &mut Ui, service: Service, signing: &SigningIn, choices: &mut Vec<Choice>) {
    let words = wording(service);
    ui.label(widgets::section_title(words.title));
    ui.add_space(SPACE_S);
    cloud_parts::page_card(ui, |ui| {
        let cancelled = widgets::progress(words.waiting)
            .cancel("Stop waiting for the browser")
            .wrapping()
            .show(ui);
        if cancelled {
            choices.push(Choice::Cloud(CloudCommand::CancelSignIn(service)));
        }
        ui.add_space(SPACE_S);
        ui.add(Label::new(widgets::muted(
            "If no browser opened, copy the link into one yourself.",
            ui,
        )));
        if let Some(url) = &signing.url {
            let copy = widgets::small_button(ui, icons::COPY, COPY_LINK);
            if ui.add(copy).clicked() {
                ui.ctx().copy_text(url.clone());
            }
        }
    });
}

fn linked_section(
    ui: &mut Ui,
    view: &HomeView<'_>,
    service: Service,
    linked: &Linked,
    choices: &mut Vec<Choice>,
) {
    cloud_header(ui, service, linked, choices);
    ui.add_space(SPACE_S);
    if let Some(command) = cloud_parts::import_status(ui, service, linked) {
        choices.push(Choice::Cloud(command));
        ui.add_space(SPACE_S);
    }
    if let Some(failure) = linked.workspaces.failure()
        && cloud_parts::failure_callout(
            ui,
            Tone::Error,
            &format!("Could not list your {}s.", service.workspace_noun()),
            failure,
        )
    {
        choices.push(Choice::Cloud(CloudCommand::Refresh(service)));
        return;
    }
    match &linked.projects {
        Load::Idle | Load::Loading => {
            widgets::progress(LOADING_PROJECTS).show(ui);
        }
        Load::Failed(failure) => {
            if cloud_parts::failure_callout(
                ui,
                Tone::Error,
                "Could not list your projects.",
                failure,
            ) {
                choices.push(Choice::Cloud(CloudCommand::Refresh(service)));
            }
        }
        Load::Ready(projects) if projects.is_empty() => {
            ui.label(widgets::muted(wording(service).no_projects, ui));
        }
        Load::Ready(projects) => {
            for project in projects.iter().take(PROJECT_ROWS_ON_HOME) {
                project_line(ui, view, service, linked, project, choices);
                ui.add_space(SPACE_M);
            }
            if projects.len() > PROJECT_ROWS_ON_HOME {
                let more = format!("Show all {} {}s…", projects.len(), service.project_name());
                if widgets::link(ui, &more, None).clicked() {
                    choices.push(Choice::ManageCloud(None));
                }
            }
        }
    }
}

fn cloud_header(ui: &mut Ui, service: Service, linked: &Linked, choices: &mut Vec<Choice>) {
    let tokens = appearance::tokens(ui);
    Sides::new().show(
        ui,
        |ui| {
            widgets::icon_label(ui, icons::command(Command::Cloud), tokens.accent_text);
            ui.label(widgets::section_title(wording(service).title));
            let hub = linked
                .chosen_workspace()
                .filter(|_| service.has_workspaces())
                .map(|workspace| workspace.name.as_str());
            let account = linked.account.ready().map(|account| account.name.as_str());
            let detail = match (hub, account) {
                (Some(hub), Some(account)) => format!("{hub} · {account}"),
                (Some(text), None) | (None, Some(text)) => text.to_owned(),
                (None, None) => String::new(),
            };
            ui.label(widgets::muted(detail, ui));
        },
        |ui| {
            let out =
                widgets::small_button(ui, icons::command(sign_out_command(service)), SIGN_OUT);
            if ui.add(out).clicked() {
                choices.push(Choice::Cloud(CloudCommand::SignOut(service)));
            }
            let manage = widgets::small_button(ui, icons::command(Command::Cloud), MANAGE);
            if ui.add(manage).clicked() {
                choices.push(Choice::ManageCloud(None));
            }
            if widgets::icon_button(ui, icons::command(Command::CloudRefresh), REFRESH).clicked() {
                choices.push(Choice::Cloud(CloudCommand::Refresh(service)));
            }
        },
    );
}

fn project_line(
    ui: &mut Ui,
    view: &HomeView<'_>,
    service: Service,
    linked: &Linked,
    project: &caditor_cloud::Project,
    choices: &mut Vec<Choice>,
) {
    let root = linked.root(&project.id);
    if matches!(root, Load::Idle) {
        choices.push(Choice::Cloud(CloudCommand::LoadProject(
            service,
            project.id.clone(),
        )));
    }
    ui.horizontal(|ui| {
        ui.label(widgets::strong(&project.name));
        if let Some(folder) = root.ready() {
            let count = folder.designs.len();
            let plural = if count == 1 { "design" } else { "designs" };
            ui.label(widgets::muted(format!("{count} {plural}"), ui));
        }
    });
    match root {
        Load::Idle | Load::Loading => {
            widgets::progress(LOADING_DESIGNS).show(ui);
        }
        Load::Failed(failure) => {
            if cloud_parts::failure_callout(
                ui,
                Tone::Warning,
                "Could not open this project.",
                failure,
            ) {
                choices.push(Choice::Cloud(CloudCommand::ReloadProject(
                    service,
                    project.id.clone(),
                )));
            }
        }
        Load::Ready(folder) if folder.designs.is_empty() => {
            let folders = folder.folders.len();
            let text = if folders == 0 {
                wording(service).no_designs.to_owned()
            } else {
                format!("The designs are inside this project's {folders} folders.")
            };
            ui.horizontal(|ui| {
                ui.label(widgets::muted(text, ui));
                if folders > 0 && widgets::link(ui, "Browse them…", None).clicked() {
                    choices.push(Choice::ManageCloud(Some((service, project.id.clone()))));
                }
            });
        }
        Load::Ready(folder) => {
            design_tiles(ui, view, service, linked, project, &folder.designs, choices);
        }
    }
}

fn design_tiles(
    ui: &mut Ui,
    view: &HomeView<'_>,
    service: Service,
    linked: &Linked,
    project: &caditor_cloud::Project,
    designs: &[Design],
    choices: &mut Vec<Choice>,
) {
    let shown = designs.get(..DESIGNS_PER_PROJECT).unwrap_or(designs);
    let importing = linked.is_importing();
    let mut wanted = Vec::new();
    let chosen = cloud_parts::tiles(ui, shown.len(), TILE_WIDTH, |ui, at, width| {
        let Some(design) = shown.get(at) else {
            return false;
        };
        let detail = cloud_parts::modified_text(design.modified, view.now);
        let hover = if importing {
            "Another design is being imported; wait for it or cancel it first".to_owned()
        } else {
            format!(
                "Have {service} convert “{}” to STEP and add it to the model",
                design.name
            )
        };
        let place = Rect::from_min_size(
            ui.next_widget_position(),
            vec2(width, cloud_parts::TILE_HEIGHT),
        );
        let picture = linked.picture(&design.id).map(|picture| {
            picture.texture(
                ui.ctx(),
                &format!("{service} design {}", design.id.as_str()),
            )
        });
        if linked.wants_picture(&design.id) && ui.is_rect_visible(place) {
            wanted.push(Choice::Cloud(CloudCommand::LoadThumbnail(
                service,
                design.clone(),
            )));
        }
        cloud_parts::tile(
            ui,
            width,
            &Tile {
                glyph: icons::BODIES,
                title: &design.name,
                detail: &detail,
                name: &format!("Import {} from {service}", design.name),
                hover: &hover,
                enabled: !importing,
                picture,
            },
        )
    });
    choices.append(&mut wanted);
    if let Some(design) = chosen.and_then(|at| shown.get(at)) {
        choices.push(Choice::Cloud(CloudCommand::Import(service, design.clone())));
    }
    let hidden = designs.len().saturating_sub(shown.len());
    if hidden > 0 {
        let text = format!("{hidden} more in {}…", project.name);
        if widgets::link(ui, &text, None).clicked() {
            choices.push(Choice::ManageCloud(Some((service, project.id.clone()))));
        }
    }
}
