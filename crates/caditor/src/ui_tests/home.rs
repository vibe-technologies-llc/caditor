use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
};

use caditor_cloud::{
    Design, DesignId, Error, Folder, FolderId, FolderRef, Project, ProjectId, SendAs, Service,
    Upload, UploadFormat, UploadId, Version, VersionId, Workspace, WorkspaceId, WorkspaceKind,
    fake::{FakeProvider, ScriptedTransport, Step},
};
use caditor_document::Document;
use caditor_file::{
    Preview, PreviewSlot, RecentFiles, SaveOptions, Settings, read_preview, save_with,
};
use egui::{Key, Modifiers, accesskit::Role};
use jiff::Timestamp;
use tempfile::TempDir;

use super::{FILE_TIMEOUT, Harness, extruded_plate, no_wake, run_from_palette, sample_document};
use crate::{
    app::Workspace as App,
    browser::OpenError,
    cloud::{
        Cloud, CloudCommand, CloudServices, FakeKeyring, Link, Opener, Refusal, Thumbnail,
        sample_png,
    },
    cloud_dialog, cloud_parts,
    commands::Command,
    export::ExportCommand,
    files::FileCommand,
    home, icons,
    model::Action,
    preferences::{
        CLIENT_ID_FIELD_EDIT, CLOUD_ON_HOME, PreferenceChange, Preferences, PreferencesCommand,
        PreferencesTab, SEARCH_FIELD, USE_CLOUD,
    },
    samples::Sample,
    save_preview::PREVIEW_SIZE,
    start_card,
};

fn plate_step() -> Vec<u8> {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    extruded_plate(&mut harness);
    let exported = dir.path().join("plate.step");
    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("STEP");
    harness.answer_dialog(Some(exported.clone()));
    harness.click("Export…");
    harness.wait_until("the STEP file is written", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.starts_with("Exported 1 body"))
    });
    let bytes = fs::read(&exported).unwrap();
    let closing = harness.model.close().unwrap();
    assert!(closing.wait(FILE_TIMEOUT));
    bytes
}

fn design(id: &str, name: &str, project: &str, days_ago: i64) -> Design {
    let at = Timestamp::now().as_second() - days_ago * 86_400;
    Design {
        id: DesignId::new(id),
        project: ProjectId::new(project),
        name: name.to_owned(),
        modified: Timestamp::from_second(at).ok(),
    }
}

pub(super) fn fusion_provider(step: &[u8]) -> Arc<FakeProvider> {
    Arc::new(catalogue(step))
}

pub(super) fn fusion_provider_with_pictures() -> Arc<FakeProvider> {
    Arc::new(
        ["d1", "d3", "d4"]
            .into_iter()
            .fold(catalogue(b""), |provider, design| {
                provider.thumbnail(DesignId::new(design), &sample_png(64))
            }),
    )
}

fn catalogue(step: &[u8]) -> FakeProvider {
    let hub = WorkspaceId::new("hub");
    let project = |id: &str, name: &str| Project {
        id: ProjectId::new(id),
        workspace: hub.clone(),
        name: name.to_owned(),
    };
    FakeProvider::new()
        .account("Ada Lovelace")
        .workspace(Workspace {
            id: hub.clone(),
            name: "Studio".to_owned(),
            kind: WorkspaceKind::Team,
        })
        .project(
            project("p1", "Bikes"),
            Folder {
                designs: vec![
                    design("d1", "Frame", "p1", 3),
                    design("d2", "Fork", "p1", 40),
                ],
                folders: vec![caditor_cloud::FolderEntry {
                    reference: FolderRef {
                        project: ProjectId::new("p1"),
                        folder: FolderId::new("f1"),
                    },
                    name: "Wheels".to_owned(),
                }],
                ..Folder::default()
            },
        )
        .project(
            project("p2", "Robots"),
            Folder {
                designs: vec![design("d3", "Arm", "p2", 1)],
                ..Folder::default()
            },
        )
        .folder(
            FolderRef {
                project: ProjectId::new("p1"),
                folder: FolderId::new("f1"),
            },
            Folder {
                designs: vec![design("d4", "Rim", "p1", 9)],
                ..Folder::default()
            },
        )
        .versions(
            DesignId::new("d1"),
            vec![
                Version {
                    id: VersionId::new("v2"),
                    number: 2,
                    modified: None,
                },
                Version {
                    id: VersionId::new("v1"),
                    number: 1,
                    modified: None,
                },
            ],
        )
        .step_file(step)
}

pub(super) fn browser() -> Opener {
    Arc::new(|address: &str| -> Result<(), OpenError> {
        let redirect = address
            .split("redirect_uri=")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .map(str::to_owned)
            .unwrap();
        thread::spawn(move || {
            let port: u16 = redirect
                .rsplit(':')
                .next()
                .and_then(|rest| rest.split('/').next())
                .and_then(|port| port.parse().ok())
                .unwrap();
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(
                stream,
                "GET /callback?code=the-code&state=fake-state HTTP/1.1\r\nHost: localhost\r\n\r\n"
            )
            .unwrap();
            let mut reply = String::new();
            stream.read_to_string(&mut reply).unwrap();
        });
        Ok(())
    })
}

fn services(dir: &Path) -> CloudServices {
    keyring_services(dir, FakeKeyring::unreachable())
}

fn keyring_services(dir: &Path, keyring: Arc<FakeKeyring>) -> CloudServices {
    CloudServices {
        make_waker: no_wake(),
        state_dir: Some(dir.join("state")),
        keyring,
    }
}

fn started(dir: &Path, workspace: App) -> Harness {
    let mut harness = Harness::starting(Some(dir), Document::default(), workspace);
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();
    harness
}

fn preferences(dir: &Path) -> Preferences {
    let mut preferences = Preferences::from_settings(Settings::load(&dir.join("config")));
    preferences.onboarding.welcomed = true;
    preferences.onboarding.hints = false;
    preferences
}

pub(super) fn attach(harness: &mut Harness, dir: &Path, provider: &Arc<FakeProvider>) {
    let cloud = harness.workspace.preferences.cloud.clone();
    harness.workspace.cloud = Cloud::new(&cloud, services(dir)).with_backend(
        provider.clone(),
        Arc::new(ScriptedTransport::new()),
        browser(),
    );
    harness.frame();
    harness.frame();
}

pub(super) fn with_cloud(dir: &Path, provider: &Arc<FakeProvider>) -> Harness {
    with_opener(dir, provider, browser())
}

pub(super) fn with_opener(dir: &Path, provider: &Arc<FakeProvider>, opener: Opener) -> Harness {
    with_backend(dir, provider, services(dir), opener)
}

fn with_services(dir: &Path, provider: &Arc<FakeProvider>, services: CloudServices) -> Harness {
    with_backend(dir, provider, services, browser())
}

fn with_backend(
    dir: &Path,
    provider: &Arc<FakeProvider>,
    services: CloudServices,
    opener: Opener,
) -> Harness {
    let preferences = preferences(dir);
    let mut workspace = App::with_preferences(preferences);
    workspace.cloud = Cloud::new(&workspace.preferences.cloud, services).with_backend(
        provider.clone(),
        Arc::new(ScriptedTransport::new()),
        opener,
    );
    started(dir, workspace)
}

fn without_app(dir: &Path) -> Harness {
    let mut workspace = App::with_preferences(preferences(dir));
    workspace.cloud =
        Cloud::new(&workspace.preferences.cloud, services(dir)).without_built_in_client_id();
    started(dir, workspace)
}

pub(super) fn open_home(harness: &mut Harness) {
    harness.perform(Action::Preferences(PreferencesCommand::ShowHome));
    harness.frame();
    harness.frame();
}

pub(super) fn sign_in(harness: &mut Harness) {
    harness.wait_until("the sign-in button is shown", |harness| {
        harness.shows(home::SIGN_IN)
    });
    harness.click(home::SIGN_IN);
    harness.wait_until("signed in", |harness| {
        harness.workspace.cloud.linked(Service::Fusion).is_some()
    });
}

fn is_linked(harness: &Harness) -> bool {
    matches!(
        harness
            .workspace
            .cloud
            .connection(Service::Fusion)
            .map(|connection| &connection.link),
        Some(Link::Linked(_))
    )
}

fn top_of(harness: &Harness, text: &str) -> f32 {
    harness
        .texts
        .iter()
        .find(|(shown, _)| shown == text)
        .map(|(_, rect)| rect.top())
        .unwrap_or_else(|| panic!("{text:?} is not shown"))
}

fn seed_recent(dir: &Path) {
    let path = dir.join("kept.caditor");
    caditor_file::save(&sample_document().unwrap(), &path, false).unwrap();
    let mut earlier = Harness::with_directories(Some(dir));
    earlier.command(FileCommand::OpenPath(path));
    earlier.wait_until("the file is open", |harness| harness.model.path().is_some());
    assert!(earlier.files.wait_for_jobs(FILE_TIMEOUT));
    let closing = earlier.model.close().unwrap();
    assert!(closing.wait(FILE_TIMEOUT));
}

fn downloaded_files(dir: &Path) -> Vec<String> {
    let root = dir.join("state").join("cloud").join("downloads");
    let mut names = Vec::new();
    for folder in fs::read_dir(root).into_iter().flatten().flatten() {
        for file in fs::read_dir(folder.path()).into_iter().flatten().flatten() {
            names.push(file.file_name().to_string_lossy().into_owned());
        }
    }
    names
}

#[test]
fn home_is_a_page_over_the_view_with_starts_recent_files_and_samples() {
    let dir = TempDir::new().unwrap();
    seed_recent(dir.path());
    let provider = fusion_provider(b"");
    let mut harness = with_cloud(dir.path(), &provider);
    harness.wait_until("the recent files are read", |harness| {
        !harness.files.recent().is_empty()
    });
    assert!(!harness.shows(home::SAMPLES_TITLE));
    assert!(harness.shows(start_card::TITLE));

    open_home(&mut harness);

    assert!(harness.shows(home::SAMPLES_TITLE));
    assert!(harness.shows(home::RECENT_TITLE));
    assert!(harness.shows("kept.caditor"));
    assert!(harness.shows("Flanged spool"));
    assert!(harness.accessible_named(Role::Button, home::CLOSE));
    assert!(!harness.shows(start_card::TITLE));
    assert!(!harness.shows(start_card::DONT_SHOW_AGAIN));
}

#[test]
fn home_opens_from_the_palette_and_closes_with_escape_or_its_close_button() {
    let dir = TempDir::new().unwrap();
    let mut harness = with_cloud(dir.path(), &fusion_provider(b""));

    run_from_palette(&mut harness, "Home");
    harness.frame();
    harness.frame();

    assert!(harness.workspace.home.is_open());

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(!harness.workspace.home.is_open());
    assert!(harness.shows(start_card::TITLE));

    open_home(&mut harness);
    harness.click_button(home::CLOSE);
    harness.frame();

    assert!(!harness.workspace.home.is_open());
}

#[test]
fn a_start_on_home_runs_and_takes_the_page_away() {
    let dir = TempDir::new().unwrap();
    let mut harness = with_cloud(dir.path(), &fusion_provider(b""));
    open_home(&mut harness);

    harness.click(start_card::START_WITH_SKETCH);
    harness.frame();
    harness.frame();

    assert!(harness.workspace.editing.is_choosing_plane());
    assert!(!harness.workspace.home.is_open());
}

#[test]
fn home_goes_when_the_model_underneath_changes() {
    let dir = TempDir::new().unwrap();
    let mut harness = with_cloud(dir.path(), &fusion_provider(b""));
    open_home(&mut harness);

    harness.command(FileCommand::New);
    harness.settle();

    assert!(!harness.workspace.home.is_open());
}

#[test]
fn signed_out_home_advertises_fusion_and_asks_nothing_of_the_network() {
    let dir = TempDir::new().unwrap();
    let provider = fusion_provider(b"");
    let mut harness = with_cloud(dir.path(), &provider);

    open_home(&mut harness);
    harness.frame();

    assert!(harness.shows(home::ADVERT_TITLE));
    assert!(harness.shows_containing("One click has Autodesk convert a design to STEP"));
    for point in home::ADVERT_POINTS {
        assert!(harness.shows_containing(point), "{point}");
    }
    assert!(harness.shows(home::SIGN_IN));
    assert!(harness.shows(home::TURN_OFF));
    assert!(harness.shows(home::MANAGE));
    assert!(provider.calls().is_empty());
}

#[test]
fn signing_in_shows_history_first_then_each_fusion_project_as_a_line_of_designs() {
    let dir = TempDir::new().unwrap();
    seed_recent(dir.path());
    let provider = fusion_provider(b"");
    let mut harness = with_cloud(dir.path(), &provider);
    harness.wait_until("the recent files are read", |harness| {
        !harness.files.recent().is_empty()
    });
    open_home(&mut harness);

    sign_in(&mut harness);
    harness.wait_until("the designs are listed", |harness| {
        harness.shows("Frame") && harness.shows("Arm")
    });

    assert!(!harness.shows(home::ADVERT_TITLE));
    assert!(harness.shows_containing("Studio · Ada Lovelace"));
    assert!(harness.shows("Bikes") && harness.shows("Robots"));
    assert!(harness.shows("Fork"));
    assert!(harness.shows("2 designs") && harness.shows("1 design"));
    assert!(harness.shows_containing("Edited 3 days ago"));
    assert!(harness.shows_containing("Edited 1 month ago"));
    assert!(top_of(&harness, home::RECENT_TITLE) < top_of(&harness, "kept.caditor") + 1.0);
    assert!(top_of(&harness, "kept.caditor") < top_of(&harness, "Bikes"));
    assert!(top_of(&harness, "Bikes") < top_of(&harness, "Robots"));
    assert!(top_of(&harness, "Robots") < top_of(&harness, home::SAMPLES_TITLE));
    assert!(harness.accessible_named(Role::Button, "Import Frame from Fusion 360"));
    let calls = provider.calls();
    assert!(calls.contains(&"account".to_owned()));
    assert!(calls.contains(&"projects hub".to_owned()));
    assert!(calls.contains(&"root p1".to_owned()));
    assert!(calls.contains(&"root p2".to_owned()));
}

#[test]
fn clicking_a_design_converts_downloads_and_imports_it_in_one_go() {
    let dir = TempDir::new().unwrap();
    let step = plate_step();
    let provider = Arc::new(catalogue(&step).working_polls(0));
    let mut harness = with_cloud(dir.path(), &provider);
    open_home(&mut harness);
    sign_in(&mut harness);
    harness.wait_until("the designs are listed", |harness| harness.shows("Frame"));
    let before = harness.document().features().len();

    harness.click("Frame");
    harness.wait_until("the design is imported", |harness| {
        harness.document().features().len() > before
    });
    harness.settle();

    let imported = harness.document().features().last().unwrap();
    assert!(imported.kind.import().is_some());
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(!harness.workspace.home.is_open());
    assert_eq!(downloaded_files(dir.path()), vec!["Frame.step".to_owned()]);
    let calls = provider.calls();
    assert!(calls.contains(&"convert d1".to_owned()));
    assert!(calls.contains(&"fetch".to_owned()));
    let linked = harness.workspace.cloud.linked(Service::Fusion).unwrap();
    assert!(linked.import.is_none());
}

#[test]
fn a_failed_conversion_is_said_in_words_and_try_again_imports_once_it_works() {
    let dir = TempDir::new().unwrap();
    let step = plate_step();
    let provider =
        Arc::new(
            catalogue(&step).failing(Step::Conversion, || Error::ConversionFailed {
                service: Service::Fusion,
            }),
        );
    let mut harness = with_cloud(dir.path(), &provider);
    open_home(&mut harness);
    sign_in(&mut harness);
    harness.wait_until("the designs are listed", |harness| harness.shows("Frame"));
    let before = harness.document().features().len();

    harness.click("Frame");
    harness.wait_until("the failure is shown", |harness| {
        harness.shows_containing("Could not import “Frame” from Fusion 360.")
    });

    assert!(harness.shows(cloud_parts::TRY_AGAIN));
    assert!(harness.shows(cloud_parts::DISMISS));
    assert_eq!(harness.document().features().len(), before);
    assert!(
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.contains("Could not import “Frame”"))
    );

    provider.recover(Step::Conversion);
    harness.click(cloud_parts::TRY_AGAIN);
    harness.wait_until("the design is imported", |harness| {
        harness.document().features().len() > before
    });
}

#[test]
fn dismissing_a_failed_import_clears_it() {
    let dir = TempDir::new().unwrap();
    let provider = Arc::new(
        catalogue(b"").failing(Step::Conversion, || Error::ConversionFailed {
            service: Service::Fusion,
        }),
    );
    let mut harness = with_cloud(dir.path(), &provider);
    open_home(&mut harness);
    sign_in(&mut harness);
    harness.wait_until("the designs are listed", |harness| harness.shows("Frame"));
    harness.click("Frame");
    harness.wait_until("the failure is shown", |harness| {
        harness.shows(cloud_parts::DISMISS)
    });

    harness.click(cloud_parts::DISMISS);
    harness.frame();

    assert!(!harness.shows(cloud_parts::DISMISS));
    assert!(
        harness
            .workspace
            .cloud
            .linked(Service::Fusion)
            .unwrap()
            .import
            .is_none()
    );
}

#[test]
fn the_sign_in_survives_a_restart_and_signing_out_forgets_it() {
    let dir = TempDir::new().unwrap();
    let provider = fusion_provider(b"");
    let mut first = with_cloud(dir.path(), &provider);
    open_home(&mut first);
    sign_in(&mut first);
    first.wait_until("the designs are listed", |harness| harness.shows("Frame"));
    let closing = first.model.close().unwrap();
    assert!(closing.wait(FILE_TIMEOUT));
    drop(first);

    let again = fusion_provider(b"");
    let mut restarted = with_cloud(dir.path(), &again);
    restarted.wait_until("the saved sign-in is used", is_linked);
    open_home(&mut restarted);
    restarted.wait_until("the designs are listed", |harness| harness.shows("Frame"));

    assert!(!restarted.shows(home::ADVERT_TITLE));
    assert!(
        dir.path()
            .join("state")
            .join("cloud")
            .join("fusion-session.json")
            .exists()
    );

    restarted.click(home::SIGN_OUT);
    restarted.wait_until("the advert is back", |harness| {
        harness.shows(home::ADVERT_TITLE)
    });
    restarted.wait_until("the sign-in is revoked and forgotten", |_| {
        again.calls().contains(&"revoke".to_owned())
            && !dir
                .path()
                .join("state")
                .join("cloud")
                .join("fusion-session.json")
                .exists()
    });
}

fn two_hubs() -> FakeProvider {
    let personal = WorkspaceId::new("home");
    catalogue(b"")
        .workspace(Workspace {
            id: personal.clone(),
            name: "Mine".to_owned(),
            kind: WorkspaceKind::Personal,
        })
        .project(
            Project {
                id: ProjectId::new("p9"),
                workspace: personal,
                name: "Garden".to_owned(),
            },
            Folder {
                designs: vec![design("d9", "Shed", "p9", 2)],
                ..Folder::default()
            },
        )
}

fn chosen_hub(harness: &Harness) -> Option<WorkspaceId> {
    harness
        .workspace
        .cloud
        .linked(Service::Fusion)
        .and_then(|linked| linked.workspace.clone())
}

#[test]
fn the_chosen_hub_comes_back_after_a_restart_and_a_missing_one_falls_back_to_the_first() {
    let dir = TempDir::new().unwrap();
    let mut first = with_cloud(dir.path(), &Arc::new(two_hubs()));
    open_home(&mut first);
    sign_in(&mut first);
    first.wait_until("the team hub is listed", |harness| harness.shows("Frame"));
    assert_eq!(chosen_hub(&first), Some(WorkspaceId::new("hub")));

    first.perform(Action::Cloud(CloudCommand::ChooseWorkspace(
        Service::Fusion,
        WorkspaceId::new("home"),
    )));
    first.wait_until("the other hub is listed", |harness| harness.shows("Shed"));
    let remembered = dir
        .path()
        .join("state")
        .join("cloud")
        .join("fusion-hub.json");
    first.wait_until("the choice is kept", |_| remembered.exists());
    let closing = first.model.close().unwrap();
    assert!(closing.wait(FILE_TIMEOUT));
    drop(first);

    let mut restarted = with_cloud(dir.path(), &Arc::new(two_hubs()));
    restarted.wait_until("the saved sign-in is used", is_linked);
    open_home(&mut restarted);
    restarted.wait_until("the remembered hub is listed", |harness| {
        harness.shows("Shed")
    });
    assert_eq!(chosen_hub(&restarted), Some(WorkspaceId::new("home")));
    let closing = restarted.model.close().unwrap();
    assert!(closing.wait(FILE_TIMEOUT));
    drop(restarted);

    let mut gone = with_cloud(dir.path(), &Arc::new(catalogue(b"")));
    gone.wait_until("the saved sign-in is used", is_linked);
    open_home(&mut gone);
    gone.wait_until("the first hub is listed", |harness| harness.shows("Frame"));
    assert_eq!(chosen_hub(&gone), Some(WorkspaceId::new("hub")));
}

#[test]
fn signing_out_forgets_the_remembered_hub() {
    let dir = TempDir::new().unwrap();
    let mut harness = with_cloud(dir.path(), &Arc::new(two_hubs()));
    open_home(&mut harness);
    sign_in(&mut harness);
    harness.wait_until("the team hub is listed", |harness| harness.shows("Frame"));
    harness.perform(Action::Cloud(CloudCommand::ChooseWorkspace(
        Service::Fusion,
        WorkspaceId::new("home"),
    )));
    let remembered = dir
        .path()
        .join("state")
        .join("cloud")
        .join("fusion-hub.json");
    harness.wait_until("the choice is kept", |_| remembered.exists());

    harness.click(home::SIGN_OUT);

    harness.wait_until("the choice is forgotten", |_| !remembered.exists());
}

fn thumbnail_dir(dir: &Path) -> std::path::PathBuf {
    dir.join("state")
        .join("cloud")
        .join("thumbnails")
        .join("fusion")
}

fn picture_of(harness: &Harness, design: &str) -> bool {
    harness
        .workspace
        .cloud
        .linked(Service::Fusion)
        .is_some_and(|linked| linked.picture(&DesignId::new(design)).is_some())
}

#[test]
fn a_design_with_a_thumbnail_shows_it_on_its_tile_and_the_picture_is_kept_for_next_time() {
    let dir = TempDir::new().unwrap();
    let provider = Arc::new(catalogue(b"").thumbnail(DesignId::new("d1"), &sample_png(8)));
    let mut harness = with_cloud(dir.path(), &provider);
    open_home(&mut harness);

    sign_in(&mut harness);
    harness.wait_until("the designs are listed", |harness| harness.shows("Frame"));
    harness.wait_until("the picture of Frame arrives", |harness| {
        picture_of(harness, "d1")
    });

    assert!(!picture_of(&harness, "d2"));
    assert!(harness.shows("Fork"));
    assert!(
        fs::read_dir(thumbnail_dir(dir.path()))
            .unwrap()
            .flatten()
            .any(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
    );
    let closing = harness.model.close().unwrap();
    assert!(closing.wait(FILE_TIMEOUT));
    drop(harness);

    let mut restarted = with_cloud(dir.path(), &provider);
    restarted.wait_until("the saved sign-in is used", is_linked);
    open_home(&mut restarted);
    restarted.wait_until("the kept picture is shown", |harness| {
        picture_of(harness, "d1")
    });

    let asked = provider
        .calls()
        .iter()
        .filter(|call| *call == "thumbnail d1")
        .count();
    assert_eq!(asked, 1);
}

#[test]
fn a_thumbnail_that_cannot_be_had_leaves_the_tile_as_it_was_and_still_imports() {
    let dir = TempDir::new().unwrap();
    let provider = Arc::new(
        catalogue(&plate_step()).failing(Step::Thumbnail, || Error::Refused {
            service: Service::Fusion,
            status: 500,
        }),
    );
    let mut harness = with_cloud(dir.path(), &provider);
    open_home(&mut harness);
    sign_in(&mut harness);
    harness.wait_until("the designs are listed", |harness| harness.shows("Frame"));
    harness.wait_until("the pictures were tried", |_| {
        provider.calls().iter().any(|call| call == "thumbnail d1")
    });

    assert!(!picture_of(&harness, "d1"));
    assert!(harness.shows("Frame"));
    assert!(harness.model.notice().is_none());

    let before = harness.document().features().len();

    harness.click("Frame");

    harness.wait_until("the design is imported", |harness| {
        harness.document().features().len() > before
    });
}

#[test]
fn signing_out_removes_the_kept_pictures() {
    let dir = TempDir::new().unwrap();
    let provider = Arc::new(catalogue(b"").thumbnail(DesignId::new("d1"), &sample_png(8)));
    let mut harness = with_cloud(dir.path(), &provider);
    open_home(&mut harness);
    sign_in(&mut harness);
    harness.wait_until("the picture of Frame arrives", |harness| {
        picture_of(harness, "d1")
    });

    harness.click(home::SIGN_OUT);

    harness.wait_until("the pictures are removed", |_| {
        !thumbnail_dir(dir.path()).exists()
    });
}

#[test]
fn a_sign_in_kept_in_the_keyring_survives_a_restart_without_a_file_and_signing_out_forgets_it() {
    let dir = TempDir::new().unwrap();
    let keyring = FakeKeyring::reachable();
    let file = dir
        .path()
        .join("state")
        .join("cloud")
        .join("fusion-session.json");
    let provider = fusion_provider(b"");
    let mut first = with_services(
        dir.path(),
        &provider,
        keyring_services(dir.path(), keyring.clone()),
    );
    open_home(&mut first);
    sign_in(&mut first);
    first.wait_until("the designs are listed", |harness| harness.shows("Frame"));
    let closing = first.model.close().unwrap();
    assert!(closing.wait(FILE_TIMEOUT));
    drop(first);

    assert!(keyring.holds(Service::Fusion).is_some());
    assert!(!file.exists());

    let again = fusion_provider(b"");
    let mut restarted = with_services(
        dir.path(),
        &again,
        keyring_services(dir.path(), keyring.clone()),
    );
    restarted.wait_until("the saved sign-in is used", is_linked);
    open_home(&mut restarted);

    restarted.click(home::SIGN_OUT);
    restarted.wait_until("the sign-in is revoked and forgotten", |_| {
        again.calls().contains(&"revoke".to_owned()) && keyring.holds(Service::Fusion).is_none()
    });
}

#[test]
fn a_sign_in_that_has_ended_asks_for_signing_in_again_in_words() {
    let dir = TempDir::new().unwrap();
    let provider = Arc::new(catalogue(b"").failing(Step::Workspaces, || Error::SignedOut));
    let mut harness = with_cloud(dir.path(), &provider);
    open_home(&mut harness);

    harness.click(home::SIGN_IN);
    harness.wait_until("the sign-in has ended", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.contains("sign-in has ended"))
    });

    assert!(harness.shows(home::ADVERT_TITLE));
    assert!(harness.shows(home::SIGN_IN));
}

#[test]
fn cancelling_a_sign_in_waiting_for_the_browser_goes_back_to_the_advert() {
    let dir = TempDir::new().unwrap();
    let provider = fusion_provider(b"");
    let mut workspace = App::with_preferences(preferences(dir.path()));
    workspace.cloud = Cloud::new(&workspace.preferences.cloud, services(dir.path())).with_backend(
        provider.clone(),
        Arc::new(ScriptedTransport::new()),
        Arc::new(|_: &str| -> Result<(), OpenError> { Ok(()) }),
    );
    let mut harness = started(dir.path(), workspace);
    open_home(&mut harness);

    harness.click(home::SIGN_IN);
    harness.wait_until("the browser is awaited", |harness| {
        harness.shows(home::WAITING_FOR_BROWSER) && harness.shows(home::COPY_LINK)
    });

    assert!(!harness.shows(home::SIGN_IN));

    harness.click("Cancel");
    harness.wait_until("the advert is back", |harness| {
        harness.shows(home::ADVERT_TITLE)
    });

    assert!(provider.calls().iter().all(|call| call != "finish sign in"));
}

#[test]
fn turning_cloud_off_from_the_advert_hides_every_trace_and_makes_no_request() {
    let dir = TempDir::new().unwrap();
    let provider = fusion_provider(b"");
    let mut harness = with_cloud(dir.path(), &provider);
    open_home(&mut harness);

    harness.click(home::TURN_OFF);
    harness.frame();
    harness.frame();

    assert!(!harness.workspace.preferences.cloud.enabled);
    assert!(!harness.shows(home::ADVERT_TITLE));
    assert!(!harness.shows(home::SIGN_IN));
    assert!(!harness.shows(home::FUSION_TITLE));
    assert!(harness.shows(home::SAMPLES_TITLE));
    assert!(matches!(
        harness
            .workspace
            .cloud
            .connection(Service::Fusion)
            .map(|connection| &connection.link),
        Some(Link::Off)
    ));
    assert!(
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.contains("Cloud features are off"))
    );
    let config = dir.path().join("config");
    harness.wait_until("the choice is saved", |_| {
        Settings::load(&config).flag("cloud.enabled") == Some(false)
    });
    assert!(provider.calls().is_empty());

    assert_eq!(
        harness.workspace.cloud.can_sign_in(Service::Fusion),
        Err(Refusal::Off)
    );
}

#[test]
fn off_stays_off_after_a_restart_with_no_request_and_the_commands_say_how_to_turn_it_on() {
    let dir = TempDir::new().unwrap();
    let provider = fusion_provider(b"");
    let mut harness = with_cloud(dir.path(), &provider);
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::CloudEnabled(false),
    )));
    let config = dir.path().join("config");
    harness.wait_until("the choice is saved", |_| {
        Settings::load(&config).flag("cloud.enabled") == Some(false)
    });
    drop(harness);

    let again = fusion_provider(b"");
    let mut restarted = with_cloud(dir.path(), &again);
    open_home(&mut restarted);
    restarted.frame();

    assert!(!restarted.workspace.preferences.cloud.enabled);
    assert!(!restarted.shows(home::ADVERT_TITLE));
    assert!(again.calls().is_empty());

    run_from_palette(&mut restarted, "Cloud projects");
    restarted.frame();

    assert!(restarted.workspace.cloud_dialog.is_none());
    assert!(
        restarted
            .model
            .notice()
            .is_some_and(|notice| notice.text.contains("Cloud services are turned off"))
    );
    assert!(again.calls().is_empty());
}

#[test]
fn turning_cloud_off_while_signed_in_signs_out_and_forgets_the_saved_sign_in() {
    let dir = TempDir::new().unwrap();
    let provider = fusion_provider(b"");
    let mut harness = with_cloud(dir.path(), &provider);
    open_home(&mut harness);
    sign_in(&mut harness);
    let session = dir
        .path()
        .join("state")
        .join("cloud")
        .join("fusion-session.json");
    assert!(session.exists());

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::CloudEnabled(false),
    )));
    harness.wait_until("the sign-in is revoked and forgotten", |_| {
        provider.calls().contains(&"revoke".to_owned()) && !session.exists()
    });

    assert!(harness.workspace.cloud.linked(Service::Fusion).is_none());
    assert!(!harness.shows(home::FUSION_TITLE));
}

#[test]
fn cloud_can_be_kept_off_home_while_staying_on_in_the_manager() {
    let dir = TempDir::new().unwrap();
    let mut harness = with_cloud(dir.path(), &fusion_provider(b""));
    open_home(&mut harness);
    assert!(harness.shows(home::ADVERT_TITLE));

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::CloudOnHome(false),
    )));
    harness.frame();

    assert!(!harness.shows(home::ADVERT_TITLE));
    assert!(harness.workspace.preferences.cloud.enabled);

    harness.perform(Action::Preferences(PreferencesCommand::ShowCloud(None)));
    harness.frame();
    harness.frame();

    assert!(harness.shows(cloud_dialog::TITLE));
    assert!(harness.shows(cloud_dialog::SIGN_IN));
}

#[test]
fn a_build_without_an_autodesk_app_says_so_and_points_to_preferences() {
    let dir = TempDir::new().unwrap();
    let mut harness = without_app(dir.path());
    open_home(&mut harness);
    harness.frame();

    assert!(harness.shows_containing("has no Fusion 360 app registered"));
    assert!(harness.shows(home::OPEN_PREFERENCES));
    assert!(matches!(
        harness.workspace.cloud.can_sign_in(Service::Fusion),
        Err(Refusal::NoClientId(Service::Fusion))
    ));

    harness.click(home::OPEN_PREFERENCES);
    harness.show_new_windows();

    assert!(harness.workspace.preferences_open);
}

#[test]
fn onshape_is_advertised_under_fusion_and_names_its_own_missing_app() {
    let dir = TempDir::new().unwrap();
    let mut harness = without_app(dir.path());
    open_home(&mut harness);
    harness.frame();

    assert!(harness.shows(home::ADVERT_TITLE));
    assert!(harness.shows(home::ONSHAPE_ADVERT_TITLE));
    assert!(harness.shows_containing("One click has Onshape export a Part Studio"));
    for point in home::ONSHAPE_ADVERT_POINTS {
        assert!(harness.shows_containing(point), "{point}");
    }
    assert!(harness.shows(home::ONSHAPE_SIGN_IN));
    assert!(harness.shows_containing("has no Onshape app registered"));
    assert!(top_of(&harness, home::ADVERT_TITLE) < top_of(&harness, home::ONSHAPE_ADVERT_TITLE));
    assert!(top_of(&harness, home::ONSHAPE_ADVERT_TITLE) < top_of(&harness, home::SAMPLES_TITLE));
    let turn_offs = harness
        .texts
        .iter()
        .filter(|(shown, _)| shown == home::TURN_OFF)
        .count();
    assert_eq!(turn_offs, 1);
    assert!(matches!(
        harness.workspace.cloud.can_sign_in(Service::Onshape),
        Err(Refusal::NoClientId(Service::Onshape))
    ));
}

#[test]
fn an_onshape_client_id_enables_its_sign_in_and_leaves_fusion_alone() {
    let dir = TempDir::new().unwrap();
    let mut harness = without_app(dir.path());
    let mut cloud = harness.workspace.preferences.cloud.clone();
    cloud.onshape_client_id = " onshape-id ".to_owned();

    harness.workspace.cloud.configure(&cloud);

    assert!(
        harness
            .workspace
            .cloud
            .can_sign_in(Service::Onshape)
            .is_ok()
    );
    assert!(matches!(
        harness.workspace.cloud.can_sign_in(Service::Fusion),
        Err(Refusal::NoClientId(Service::Fusion))
    ));
    assert_eq!(
        harness
            .workspace
            .cloud
            .client_id(Service::Onshape)
            .as_deref(),
        Some("onshape-id")
    );
}

#[test]
fn the_manager_signs_in_browses_folders_and_imports() {
    let dir = TempDir::new().unwrap();
    let step = plate_step();
    let provider = fusion_provider(&step);
    let mut harness = with_cloud(dir.path(), &provider);

    run_from_palette(&mut harness, "Cloud projects");
    harness.frame();
    harness.frame();

    assert!(harness.shows(cloud_dialog::TITLE));
    assert!(harness.shows("Accounts"));
    assert!(harness.shows("Signed out"));
    assert!(harness.shows(cloud_dialog::SIGN_IN));
    assert!(harness.shows(cloud_dialog::TURN_OFF));

    harness.click(cloud_dialog::SIGN_IN);
    harness.wait_until("the projects are listed", |harness| {
        harness.shows("Bikes") && harness.shows("Robots")
    });

    assert!(harness.shows("Signed in as Ada Lovelace"));
    assert!(harness.shows(cloud_dialog::CHOOSE_A_PROJECT));

    harness.click("Bikes");
    harness.wait_until("the designs are listed", |harness| harness.shows("Frame"));

    assert!(harness.shows("Wheels"));
    assert!(harness.shows("Fork"));

    harness.click("Wheels");
    harness.wait_until("the folder is listed", |harness| harness.shows("Rim"));

    assert!(!harness.shows("Frame"));

    let before = harness.document().features().len();
    harness.click(cloud_dialog::IMPORT);
    harness.wait_until("the design is imported", |harness| {
        harness.document().features().len() > before
    });
    harness.settle();

    assert_eq!(downloaded_files(dir.path()), vec!["Rim.step".to_owned()]);
    harness.wait_until("the kept designs are counted", |harness| {
        harness
            .workspace
            .cloud
            .downloads
            .usage
            .is_some_and(|usage| usage.designs == 1)
    });
    assert!(harness.shows_containing("1 design kept"));

    harness.click(cloud_dialog::CLEAR_DOWNLOADS);
    harness.wait_until("the kept designs are gone", |harness| {
        harness
            .workspace
            .cloud
            .downloads
            .usage
            .is_some_and(|usage| usage.designs == 0)
    });
    assert!(downloaded_files(dir.path()).is_empty());
    assert!(harness.shows_containing("No design is kept yet."));
}

fn upload(id: &str, name: &str, format: UploadFormat) -> Upload {
    Upload {
        id: UploadId::new(id),
        project: ProjectId::new("p1"),
        name: name.to_owned(),
        format,
        modified: None,
    }
}

fn uploads_provider(step: &[u8], uploads: Vec<Upload>) -> FakeProvider {
    let hub = WorkspaceId::new("hub");
    FakeProvider::new()
        .workspace(Workspace {
            id: hub.clone(),
            name: "Studio".to_owned(),
            kind: WorkspaceKind::Team,
        })
        .project(
            Project {
                id: ProjectId::new("p1"),
                workspace: hub,
                name: "Parts".to_owned(),
            },
            Folder {
                uploads,
                ..Folder::default()
            },
        )
        .stored_file(UploadId::new("u1"), step)
}

fn plate_upload() -> Vec<Upload> {
    vec![upload("u1", "Plate.STP", UploadFormat::Step)]
}

fn open_parts_in_manager(harness: &mut Harness) {
    run_from_palette(harness, "Cloud projects");
    harness.frame();
    harness.frame();
    harness.click(cloud_dialog::SIGN_IN);
    harness.wait_until("the projects are listed", |harness| harness.shows("Parts"));
    harness.click("Parts");
    harness.wait_until("the folder is listed", |harness| harness.shows("Plate.STP"));
}

#[test]
fn the_manager_lists_uploaded_files_with_an_icon_for_their_kind() {
    let dir = TempDir::new().unwrap();
    let every_kind = vec![
        upload("u1", "Plate.STP", UploadFormat::Step),
        upload("u2", "Bracket.stl", UploadFormat::Stl),
        upload("u3", "Plan.dxf", UploadFormat::Dxf),
    ];
    let provider = Arc::new(uploads_provider(&plate_step(), every_kind));
    let mut harness = with_cloud(dir.path(), &provider);
    open_parts_in_manager(&mut harness);

    assert!(harness.shows("Plate.STP"));
    assert!(harness.shows("Bracket.stl"));
    assert!(harness.shows("Plan.dxf"));
    assert!(harness.shows(icons::BODIES));
    assert!(harness.shows(icons::UPLOADED_MESH));
    assert!(harness.shows(icons::UPLOADED_DRAWING));
}

#[test]
fn an_uploaded_step_file_comes_in_like_a_local_one_with_no_conversion() {
    let dir = TempDir::new().unwrap();
    let provider = Arc::new(uploads_provider(&plate_step(), plate_upload()));
    let mut harness = with_cloud(dir.path(), &provider);
    open_parts_in_manager(&mut harness);

    let before = harness.document().features().len();
    harness.click(cloud_dialog::IMPORT);
    harness.wait_until("the file is imported", |harness| {
        harness.document().features().len() > before
    });
    harness.settle();

    assert_eq!(downloaded_files(dir.path()), vec!["Plate.step".to_owned()]);
    let calls = provider.calls();
    assert!(calls.contains(&"fetch upload u1".to_owned()));
    assert!(!calls.iter().any(|call| call.starts_with("convert")));
}

#[test]
fn an_uploaded_file_that_cannot_be_downloaded_says_so_and_offers_to_try_again() {
    let dir = TempDir::new().unwrap();
    let provider = Arc::new(uploads_provider(&plate_step(), plate_upload()).failing(
        Step::Fetch,
        || Error::NoStoredFile {
            service: Service::Fusion,
        },
    ));
    let mut harness = with_cloud(dir.path(), &provider);
    open_parts_in_manager(&mut harness);

    harness.click(cloud_dialog::IMPORT);
    harness.wait_until("the failure is shown", |harness| {
        harness.shows_containing("Could not import “Plate.STP” from Fusion 360.")
    });

    assert!(harness.shows(cloud_parts::TRY_AGAIN));
    assert!(downloaded_files(dir.path()).is_empty());
}

#[test]
fn a_designs_versions_are_listed_on_request_and_a_chosen_one_is_converted_and_imported() {
    let dir = TempDir::new().unwrap();
    let step = plate_step();
    let provider = fusion_provider(&step);
    let mut harness = with_cloud(dir.path(), &provider);
    run_from_palette(&mut harness, "Cloud projects");
    harness.frame();
    harness.frame();
    harness.click(cloud_dialog::SIGN_IN);
    harness.wait_until("the projects are listed", |harness| harness.shows("Bikes"));
    harness.click("Bikes");
    harness.wait_until("the designs are listed", |harness| harness.shows("Frame"));

    assert!(!harness.shows("Version 1"));

    harness.click(cloud_dialog::VERSIONS);
    harness.wait_until("the versions are listed", |harness| {
        harness.shows("Version 1") && harness.shows("Version 2")
    });

    assert!(harness.shows(cloud_dialog::NEWEST_VERSION));

    let before = harness.document().features().len();
    harness.click_beside(cloud_dialog::IMPORT, "Version 1");
    harness.wait_until("the version is imported", |harness| {
        harness.document().features().len() > before
    });
    harness.settle();

    assert_eq!(
        downloaded_files(dir.path()),
        vec!["Frame v1.step".to_owned()]
    );
    assert!(provider.calls().contains(&"convert d1 v1".to_owned()));
    harness.click(cloud_dialog::VERSIONS);
    assert!(!harness.shows("Version 1"));
}

#[test]
fn listing_versions_that_fails_says_so_and_try_again_lists_them() {
    let dir = TempDir::new().unwrap();
    let provider = Arc::new(catalogue(b"").failing(Step::Versions, || Error::NotFound {
        subject: caditor_cloud::Subject::Design,
    }));
    let mut harness = with_cloud(dir.path(), &provider);
    run_from_palette(&mut harness, "Cloud projects");
    harness.frame();
    harness.frame();
    harness.click(cloud_dialog::SIGN_IN);
    harness.wait_until("the projects are listed", |harness| harness.shows("Bikes"));
    harness.click("Bikes");
    harness.wait_until("the designs are listed", |harness| harness.shows("Frame"));

    harness.click(cloud_dialog::VERSIONS);
    harness.wait_until("the failure is shown", |harness| {
        harness.shows_containing("Could not list the versions.")
    });

    provider.recover(Step::Versions);
    harness.click(cloud_parts::TRY_AGAIN);
    harness.wait_until("the versions are listed", |harness| {
        harness.shows("Version 2")
    });
}

#[test]
fn the_manager_turns_cloud_off_and_back_on() {
    let dir = TempDir::new().unwrap();
    let provider = fusion_provider(b"");
    let mut harness = with_cloud(dir.path(), &provider);
    harness.perform(Action::Preferences(PreferencesCommand::ShowCloud(None)));
    harness.frame();
    harness.frame();

    harness.click(cloud_dialog::TURN_OFF);
    harness.frame();
    harness.frame();

    assert!(!harness.workspace.preferences.cloud.enabled);
    assert!(harness.shows_containing(cloud_dialog::OFF_NOTE));
    assert!(!harness.shows(cloud_dialog::SIGN_IN));
    assert!(harness.shows(cloud_dialog::TURN_ON));

    harness.click(cloud_dialog::TURN_ON);
    harness.wait_until("the sign-in check finishes", |harness| {
        harness.shows(cloud_dialog::SIGN_IN)
    });

    assert!(harness.workspace.preferences.cloud.enabled);
}

#[test]
fn preferences_hold_the_cloud_choices_and_restoring_defaults_never_turns_cloud_back_on() {
    let dir = TempDir::new().unwrap();
    let mut harness = with_cloud(dir.path(), &fusion_provider(b""));
    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    harness.type_into_field(egui::Id::unique(SEARCH_FIELD), "cloud");

    assert!(harness.shows(USE_CLOUD));

    harness.click(USE_CLOUD);
    harness.frame();

    assert!(!harness.workspace.preferences.cloud.enabled);

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Defaults(PreferencesTab::General),
    )));

    assert!(!harness.workspace.preferences.cloud.enabled);

    harness.click(USE_CLOUD);
    harness.frame();

    assert!(harness.workspace.preferences.cloud.enabled);
    assert!(harness.shows(CLOUD_ON_HOME));
}

#[test]
fn the_autodesk_app_can_be_typed_in_preferences() {
    let dir = TempDir::new().unwrap();
    let mut harness = without_app(dir.path());
    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    harness.type_into_field(egui::Id::unique(SEARCH_FIELD), "client id");

    harness.type_into_field(egui::Id::unique(CLIENT_ID_FIELD_EDIT), "my-client-id");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();

    assert_eq!(
        harness.workspace.preferences.cloud.fusion_client_id,
        "my-client-id"
    );
    assert!(harness.workspace.cloud.can_sign_in(Service::Fusion).is_ok());
    let config = dir.path().join("config");
    harness.wait_until("the client id is saved", |_| {
        Settings::load(&config).text("cloud.fusion.client_id") == Some("my-client-id")
    });
}

#[test]
fn closing_preferences_keeps_a_field_still_being_typed_in() {
    let dir = TempDir::new().unwrap();
    let mut harness = without_app(dir.path());
    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    harness.type_into_field(egui::Id::unique(SEARCH_FIELD), "client id");

    harness.draft_into_field(egui::Id::unique(CLIENT_ID_FIELD_EDIT), "my-client-id");
    harness.click("Close");
    harness.frame();

    assert!(!harness.workspace.preferences_open);
    assert_eq!(
        harness.workspace.preferences.cloud.fusion_client_id,
        "my-client-id"
    );
}

#[test]
fn home_at_startup_is_a_preference_and_the_footer_turns_it_off() {
    let dir = TempDir::new().unwrap();
    let mut harness = with_cloud(dir.path(), &fusion_provider(b""));
    open_home(&mut harness);

    harness.click(home::HIDE_AT_START);
    harness.frame();

    assert!(!harness.workspace.preferences.onboarding.home);
    assert!(!harness.workspace.home.is_open());
    let config = dir.path().join("config");
    harness.wait_until("the choice is saved", |_| {
        Settings::load(&config).flag("onboarding.home") == Some(false)
    });
}

#[test]
fn every_cloud_command_is_in_the_palette_and_refused_with_a_reason_when_off() {
    for command in [
        Command::Home,
        Command::Cloud,
        Command::CloudSignIn,
        Command::CloudSignOut,
        Command::OnshapeSignIn,
        Command::OnshapeSignOut,
        Command::CloudRefresh,
        Command::CloudSend,
    ] {
        assert!(Command::all().any(|known| known == command), "{command:?}");
        assert!(!command.title().is_empty());
    }
}

fn save_with_picture(path: &Path, side: u32) {
    let (slot, drawing) = PreviewSlot::new();
    drawing.fill(Preview::from_png(sample_png(side)).unwrap());
    let options = SaveOptions {
        preview: Some(&slot),
        ..SaveOptions::default()
    };
    save_with(&sample_document().unwrap(), path, &options).unwrap();
}

fn remember_models(dir: &Path, paths: &[&Path]) {
    let mut earlier = Harness::with_directories(Some(dir));
    for path in paths {
        earlier.command(FileCommand::OpenPath(path.to_path_buf()));
        earlier.wait_until("the file is open", |harness| {
            harness
                .model
                .path()
                .is_some_and(|open| open.file_name() == path.file_name())
        });
        assert!(earlier.files.wait_for_jobs(FILE_TIMEOUT));
    }
    let closing = earlier.model.close().unwrap();
    assert!(closing.wait(FILE_TIMEOUT));
}

fn recent_path(harness: &Harness, name: &str) -> PathBuf {
    harness
        .files
        .recent()
        .iter()
        .find(|path| path.file_name().is_some_and(|file| file == name))
        .unwrap_or_else(|| panic!("{name} is not among the recent files"))
        .clone()
}

fn picture_shown(harness: &Harness, name: &str) -> bool {
    harness
        .files
        .recent_picture(&recent_path(harness, name))
        .is_some_and(|picture| picture.is_uploaded())
}

#[test]
fn a_recent_model_saved_with_a_picture_shows_it_on_its_tile_and_one_without_keeps_the_icon() {
    let dir = TempDir::new().unwrap();
    let pictured = dir.path().join("pictured.caditor");
    let plain = dir.path().join("plain.caditor");
    save_with_picture(&pictured, 16);
    caditor_file::save(&sample_document().unwrap(), &plain, false).unwrap();
    remember_models(dir.path(), &[&plain, &pictured]);
    let provider = fusion_provider(b"");
    let mut harness = with_cloud(dir.path(), &provider);
    harness.wait_until("the recent files are read", |harness| {
        harness.files.recent().len() == 2
    });
    assert!(
        harness
            .files
            .recent_picture(&recent_path(&harness, "pictured.caditor"))
            .is_none()
    );

    open_home(&mut harness);

    harness.wait_until("the picture arrives and is drawn", |harness| {
        picture_shown(harness, "pictured.caditor")
    });
    let plain = recent_path(&harness, "plain.caditor");
    harness.wait_until("the other file was looked at", |harness| {
        !harness.files.wants_recent_picture(&plain)
    });
    assert!(harness.files.recent_picture(&plain).is_none());
    assert!(harness.shows("pictured.caditor"));
    assert!(harness.shows("plain.caditor"));

    harness.click("pictured.caditor");

    harness.wait_until("the model opens", |harness| {
        harness.model.path().is_some_and(|open| {
            open.file_name()
                .is_some_and(|name| name == "pictured.caditor")
        })
    });
}

#[test]
fn a_picture_that_cannot_be_read_leaves_the_tile_as_it_was() {
    let dir = TempDir::new().unwrap();
    let broken = dir.path().join("broken.caditor");
    save_with_picture(&broken, 16);
    remember_models(dir.path(), &[&broken]);
    let mut bytes = fs::read(&broken).unwrap();
    let at = bytes.len().min(80);
    bytes[at - 1] ^= 0x55;
    fs::write(&broken, bytes).unwrap();
    let provider = fusion_provider(b"");
    let mut harness = with_cloud(dir.path(), &provider);
    harness.wait_until("the recent files are read", |harness| {
        !harness.files.recent().is_empty()
    });
    let path = recent_path(&harness, "broken.caditor");

    open_home(&mut harness);

    harness.wait_until("the file was looked at", |harness| {
        !harness.files.wants_recent_picture(&path)
    });
    assert!(harness.files.recent_picture(&path).is_none());
    assert!(harness.shows("broken.caditor"));
    assert!(harness.model.notice().is_none());
}

#[test]
fn saving_stores_a_small_picture_of_the_bodies_and_home_shows_it_next_time() {
    let dir = TempDir::new().unwrap();
    let plate = Sample::Plate.document().unwrap();
    let mut harness = Harness::starting(Some(dir.path()), plate, App::new());
    harness.model.draw_previews_on_save();
    harness.settle();
    let path = dir.path().join("plate.caditor");
    harness.answer_dialog(Some(path.clone()));

    harness.command(FileCommand::SaveAs);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });

    let picture = read_preview(&path).unwrap().unwrap();
    let decoded = Thumbnail::from_bytes(picture.png()).unwrap();
    assert_eq!(
        decoded.size(),
        [PREVIEW_SIZE.width as usize, PREVIEW_SIZE.height as usize]
    );
    assert!(harness.model.notice().is_none());
    let closing = harness.model.close().unwrap();
    assert!(closing.wait(FILE_TIMEOUT));
    let state = dir.path().join("state");
    harness.wait_until("the saved model is remembered", |_| {
        !RecentFiles::load(&state).paths().is_empty()
    });
    drop(harness);

    let mut restarted = with_cloud(dir.path(), &fusion_provider(b""));
    restarted.wait_until("the saved model is listed", |harness| {
        !harness.files.recent().is_empty()
    });
    open_home(&mut restarted);
    restarted.wait_until("its picture is drawn", |harness| {
        picture_shown(harness, "plate.caditor")
    });
}

#[test]
fn a_save_whose_picture_never_gets_drawn_still_saves_without_one() {
    let dir = TempDir::new().unwrap();
    let plate = Sample::Plate.document().unwrap();
    let mut harness = Harness::starting(Some(dir.path()), plate, App::new());
    harness.model.draw_previews_on_save();
    harness.settle();
    harness.renders_held = true;
    let path = dir.path().join("plate.caditor");
    harness.answer_dialog(Some(path.clone()));

    harness.command(FileCommand::SaveAs);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });

    assert_eq!(read_preview(&path).unwrap(), None);
    assert_eq!(
        caditor_file::load(&path).unwrap().issues,
        Vec::<String>::new()
    );
    assert!(harness.model.notice().is_none());
}

pub(super) fn top_folder() -> FolderRef {
    FolderRef {
        project: ProjectId::new("p1"),
        folder: FolderId::new("top"),
    }
}

pub(super) fn sendable_provider(
    place: Option<FolderRef>,
    uploads: Vec<Upload>,
) -> Arc<FakeProvider> {
    let hub = WorkspaceId::new("hub");
    Arc::new(
        FakeProvider::new()
            .workspace(Workspace {
                id: hub.clone(),
                name: "Studio".to_owned(),
                kind: WorkspaceKind::Team,
            })
            .project(
                Project {
                    id: ProjectId::new("p1"),
                    workspace: hub,
                    name: "Parts".to_owned(),
                },
                Folder {
                    place,
                    uploads,
                    ..Folder::default()
                },
            ),
    )
}

fn open_parts_to_send(harness: &mut Harness) {
    run_from_palette(harness, "Cloud projects");
    harness.frame();
    harness.frame();
    harness.click(cloud_dialog::SIGN_IN);
    harness.wait_until("the projects are listed", |harness| harness.shows("Parts"));
    harness.click("Parts");
    harness.wait_until("the folder is listed", |harness| {
        harness.shows(cloud_dialog::SEND_HERE)
    });
}

pub(super) fn outgoing_files(dir: &Path) -> Vec<String> {
    let root = dir.join("state").join("cloud").join("outgoing");
    fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn the_manager_sends_the_model_as_step_into_a_folder_then_as_a_new_version() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let mut harness = with_cloud(dir.path(), &provider);
    extruded_plate(&mut harness);
    open_parts_to_send(&mut harness);

    harness.click(cloud_dialog::SEND_HERE);
    harness.wait_until("the model is sent", |_| provider.received().len() == 1);
    harness.wait_until("the sending ends", |harness| {
        harness
            .workspace
            .cloud
            .linked(Service::Fusion)
            .is_some_and(|linked| linked.sending.is_none())
    });

    let first = provider.received().remove(0);
    assert_eq!(first.folder, top_folder());
    assert!(first.name.ends_with(".step"));
    assert!(first.bytes.starts_with(b"ISO-10303-21"));
    assert!(harness.model.notice().is_some_and(
        |notice| notice.text.starts_with("Sent “") && notice.text.ends_with("as a new file.")
    ));
    assert!(outgoing_files(dir.path()).is_empty());

    harness.wait_until("the folder is listed again", |harness| {
        harness.shows(cloud_dialog::SEND_HERE)
    });
    harness.click(cloud_dialog::SEND_HERE);
    harness.wait_until("the model is sent again", |_| {
        provider.received().len() == 2
    });
    harness.wait_until("the second notice is shown", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.contains("as version 2"))
    });

    assert!(outgoing_files(dir.path()).is_empty());
    assert!(
        provider
            .calls()
            .iter()
            .any(|call| call == &format!("send top {}", first.name))
    );
}

#[test]
fn the_manager_sends_the_model_into_an_onshape_workspace() {
    let dir = TempDir::new().unwrap();
    let workspace = FolderRef {
        project: ProjectId::new("p1"),
        folder: FolderId::new("w/main"),
    };
    let provider = Arc::new(
        Arc::into_inner(sendable_provider(Some(workspace.clone()), Vec::new()))
            .unwrap()
            .serving(Service::Onshape),
    );
    let mut harness = with_cloud(dir.path(), &provider);
    extruded_plate(&mut harness);
    harness.perform(Action::Cloud(CloudCommand::SignIn(Service::Onshape)));
    harness.wait_until("signed in to Onshape", |harness| {
        harness.workspace.cloud.linked(Service::Onshape).is_some()
    });
    run_from_palette(&mut harness, "Cloud projects");
    harness.frame();
    harness.frame();
    harness.click("Parts");
    harness.wait_until("the folder is listed", |harness| {
        harness.shows(cloud_dialog::SEND_HERE)
    });

    harness.click(cloud_dialog::SEND_HERE);
    harness.wait_until("the model is sent", |_| provider.received().len() == 1);
    harness.wait_until("the sending ends", |harness| {
        harness
            .workspace
            .cloud
            .linked(Service::Onshape)
            .is_some_and(|linked| linked.sending.is_none())
    });

    let sent = provider.received().remove(0);
    assert_eq!(sent.folder, workspace);
    assert!(sent.name.ends_with(".step"));
    assert!(sent.bytes.starts_with(b"ISO-10303-21"));
    assert!(outgoing_files(dir.path()).is_empty());
}

fn onshape_workspace() -> FolderRef {
    FolderRef {
        project: ProjectId::new("p1"),
        folder: FolderId::new("w/main"),
    }
}

fn open_onshape_workspace(harness: &mut Harness) {
    harness.perform(Action::Cloud(CloudCommand::SignIn(Service::Onshape)));
    harness.wait_until("signed in to Onshape", |harness| {
        harness.workspace.cloud.linked(Service::Onshape).is_some()
    });
    run_from_palette(harness, "Cloud projects");
    harness.frame();
    harness.frame();
    harness.click("Parts");
    harness.wait_until("the folder is listed", |harness| {
        harness.shows(cloud_dialog::SEND_HERE)
    });
}

fn onshape_provider(working_polls: usize) -> Arc<FakeProvider> {
    Arc::new(
        Arc::into_inner(sendable_provider(Some(onshape_workspace()), Vec::new()))
            .unwrap()
            .serving(Service::Onshape)
            .working_polls(working_polls),
    )
}

#[test]
fn the_manager_sends_the_model_to_onshape_as_a_part_studio_once_the_translation_is_done() {
    let dir = TempDir::new().unwrap();
    let provider = onshape_provider(0);
    let mut harness = with_cloud(dir.path(), &provider);
    extruded_plate(&mut harness);
    open_onshape_workspace(&mut harness);

    harness.click(cloud_dialog::SEND_AS_PART_STUDIO);
    harness.wait_until("the model is sent", |_| provider.received().len() == 1);
    harness.wait_until("the notice says a Part Studio was made", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.ends_with("as a new Part Studio."))
    });

    let sent = provider.received().remove(0);
    assert_eq!(sent.how, SendAs::PartStudio);
    assert_eq!(sent.folder, onshape_workspace());
    assert!(sent.bytes.starts_with(b"ISO-10303-21"));
    assert!(
        provider
            .calls()
            .iter()
            .any(|call| call == "poll part studio")
    );
    assert!(
        harness
            .workspace
            .cloud
            .linked(Service::Onshape)
            .is_some_and(|linked| linked.sending.is_none())
    );
    assert!(outgoing_files(dir.path()).is_empty());
}

#[test]
fn a_translation_in_progress_is_shown_and_cancel_stops_waiting() {
    let dir = TempDir::new().unwrap();
    let provider = onshape_provider(usize::MAX);
    let mut harness = with_cloud(dir.path(), &provider);
    extruded_plate(&mut harness);
    open_onshape_workspace(&mut harness);

    harness.click(cloud_dialog::SEND_AS_PART_STUDIO);
    harness.wait_until("the translation is shown", |harness| {
        harness.shows_containing("is turning")
    });
    harness.click("Cancel");
    harness.wait_until("the sending is gone", |harness| {
        harness
            .workspace
            .cloud
            .linked(Service::Onshape)
            .is_some_and(|linked| linked.sending.is_none())
    });

    assert!(!harness.shows_containing("is turning"));
    harness.wait_until("the exported file is removed", |_| {
        outgoing_files(dir.path()).is_empty()
    });
}

#[test]
fn a_translation_that_fails_says_why_and_try_again_sends_as_a_part_studio_again() {
    let dir = TempDir::new().unwrap();
    let provider = Arc::new(Arc::into_inner(onshape_provider(0)).unwrap().failing(
        Step::Placement,
        || Error::PartStudioFailed {
            service: Service::Onshape,
        },
    ));
    let mut harness = with_cloud(dir.path(), &provider);
    extruded_plate(&mut harness);
    open_onshape_workspace(&mut harness);

    harness.click(cloud_dialog::SEND_AS_PART_STUDIO);
    harness.wait_until("the failure is shown", |harness| {
        harness.shows_containing("could not turn the file into a Part Studio")
    });

    provider.recover(Step::Placement);
    harness.click(cloud_parts::TRY_AGAIN);
    harness.wait_until("the second send is made", |_| {
        provider.received().len() == 2
    });

    assert!(
        provider
            .received()
            .iter()
            .all(|sent| sent.how == SendAs::PartStudio)
    );
}

#[test]
fn only_onshape_offers_the_part_studio_send() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let mut harness = with_cloud(dir.path(), &provider);
    extruded_plate(&mut harness);
    open_parts_to_send(&mut harness);

    assert!(!harness.shows(cloud_dialog::SEND_AS_PART_STUDIO));

    let other = TempDir::new().unwrap();
    let onshape = onshape_provider(0);
    let mut harness = with_cloud(other.path(), &onshape);
    extruded_plate(&mut harness);
    open_onshape_workspace(&mut harness);

    assert!(harness.shows(cloud_dialog::SEND_AS_PART_STUDIO));
}

#[test]
fn a_failed_send_says_why_and_try_again_sends_once_it_works() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let provider =
        Arc::new(
            Arc::into_inner(provider)
                .unwrap()
                .failing(Step::Send, || Error::SendForbidden {
                    service: Service::Fusion,
                }),
        );
    let mut harness = with_cloud(dir.path(), &provider);
    extruded_plate(&mut harness);
    open_parts_to_send(&mut harness);

    harness.click(cloud_dialog::SEND_HERE);
    harness.wait_until("the failure is shown", |harness| {
        harness.shows_containing("does not let caditor add files to this folder")
    });

    assert!(harness.shows(cloud_parts::TRY_AGAIN));
    assert!(provider.received().is_empty());
    assert!(outgoing_files(dir.path()).is_empty());

    provider.recover(Step::Send);
    harness.click(cloud_parts::TRY_AGAIN);
    harness.wait_until("the model is sent", |_| provider.received().len() == 1);
    harness.wait_until("the failure is gone", |harness| {
        !harness.shows(cloud_parts::TRY_AGAIN)
    });
}

#[test]
fn nothing_is_sent_without_a_body_or_from_a_project_top_that_holds_several_folders() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let mut harness = with_cloud(dir.path(), &provider);
    open_parts_to_send(&mut harness);

    harness.click(cloud_dialog::SEND_HERE);
    harness.frame();
    harness.frame();

    assert!(provider.received().is_empty());
    assert!(!provider.calls().iter().any(|call| call.starts_with("send")));

    let several = sendable_provider(None, Vec::new());
    let other = TempDir::new().unwrap();
    let mut harness = with_cloud(other.path(), &several);
    extruded_plate(&mut harness);
    open_parts_to_send(&mut harness);

    harness.click(cloud_dialog::SEND_HERE);
    harness.frame();
    harness.frame();

    assert!(several.received().is_empty());
}

#[test]
fn sending_from_the_palette_opens_the_manager_to_choose_a_folder_and_needs_a_sign_in() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let mut harness = with_cloud(dir.path(), &provider);

    run_from_palette(&mut harness, "Send model to the cloud");
    harness.frame();

    assert!(harness.workspace.cloud_dialog.is_none());
    assert!(harness.model.notice().is_some_and(|notice| {
        notice
            .text
            .contains("Sign in to Fusion 360 or Onshape first")
    }));

    open_home(&mut harness);
    sign_in(&mut harness);
    run_from_palette(&mut harness, "Send model to the cloud");
    harness.frame();
    harness.frame();

    assert!(harness.workspace.cloud_dialog.is_some());
    assert!(harness.shows(cloud_dialog::CHOOSE_A_FOLDER_TO_SEND));
}

#[test]
fn with_cloud_off_sending_is_refused_and_no_request_is_made() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let mut harness = with_cloud(dir.path(), &provider);
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::CloudEnabled(false),
    )));
    harness.frame();

    run_from_palette(&mut harness, "Send model to the cloud");
    harness.frame();

    assert!(harness.workspace.cloud_dialog.is_none());
    assert!(
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.contains("Cloud services are turned off"))
    );
    assert!(provider.calls().is_empty());
}
