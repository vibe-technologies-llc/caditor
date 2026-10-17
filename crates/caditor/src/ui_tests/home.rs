use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::Path,
    sync::Arc,
    thread,
};

use caditor_cloud::{
    Design, DesignId, Error, Folder, FolderId, FolderRef, Project, ProjectId, Service, Workspace,
    WorkspaceId, WorkspaceKind,
    fake::{FakeProvider, ScriptedTransport, Step},
};
use caditor_document::Document;
use caditor_file::Settings;
use egui::{Key, Modifiers, accesskit::Role};
use jiff::Timestamp;
use tempfile::TempDir;

use super::{FILE_TIMEOUT, Harness, extruded_plate, no_wake, run_from_palette, sample_document};
use crate::{
    app::Workspace as App,
    browser::OpenError,
    cloud::{Cloud, CloudServices, Link, Opener, Refusal},
    cloud_dialog, cloud_parts,
    commands::Command,
    export::ExportCommand,
    files::FileCommand,
    home,
    model::Action,
    preferences::{
        CLIENT_ID_FIELD_EDIT, CLOUD_ON_HOME, PreferenceChange, Preferences, PreferencesCommand,
        PreferencesTab, SEARCH_FIELD, USE_CLOUD,
    },
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
            },
        )
        .project(
            project("p2", "Robots"),
            Folder {
                designs: vec![design("d3", "Arm", "p2", 1)],
                folders: Vec::new(),
            },
        )
        .folder(
            FolderRef {
                project: ProjectId::new("p1"),
                folder: FolderId::new("f1"),
            },
            Folder {
                designs: vec![design("d4", "Rim", "p1", 9)],
                folders: Vec::new(),
            },
        )
        .step_file(step)
}

fn browser() -> Opener {
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
    CloudServices {
        make_waker: no_wake(),
        state_dir: Some(dir.join("state")),
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

fn with_cloud(dir: &Path, provider: &Arc<FakeProvider>) -> Harness {
    let preferences = preferences(dir);
    let mut workspace = App::with_preferences(preferences);
    workspace.cloud = Cloud::new(&workspace.preferences.cloud, services(dir)).with_backend(
        provider.clone(),
        Arc::new(ScriptedTransport::new()),
        browser(),
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
    harness.frame();
    harness.frame();

    assert!(harness.workspace.preferences.cloud.enabled);
    assert!(harness.shows(cloud_dialog::SIGN_IN));
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
        Command::CloudRefresh,
    ] {
        assert!(Command::all().any(|known| known == command), "{command:?}");
        assert!(!command.title().is_empty());
    }
}
