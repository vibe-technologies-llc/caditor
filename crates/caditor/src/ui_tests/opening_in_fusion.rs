use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use caditor_cloud::{
    Error, Service, Subject,
    fake::{FakeProvider, Step},
};
use egui::{Event, Key, Modifiers};
use parking_lot::Mutex;
use tempfile::TempDir;

use super::{
    Harness, extruded_plate,
    home::{
        browser, open_home, outgoing_files, sendable_provider, sign_in, top_folder, with_opener,
    },
    offer, run_from_palette,
};
use crate::{
    browser::OpenError,
    cloud::{CloudCommand, Opener},
    cloud_dialog, cloud_parts,
    commands::Command,
    export::{self, ExportCommand},
    files::{FileCommand, cloud_send},
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand},
};

const COMMAND: &str = "Export and open in Fusion 360";

type Opened = Arc<Mutex<Vec<String>>>;

fn recording(opened: &Opened, outcome: fn() -> Result<(), OpenError>) -> Opener {
    let signing_in = browser();
    let opened = Arc::clone(opened);
    Arc::new(move |address: &str| {
        if address.contains("redirect_uri=") {
            return signing_in(address);
        }
        opened.lock().push(address.to_owned());
        outcome()
    })
}

fn opens() -> Result<(), OpenError> {
    Ok(())
}

fn no_browser() -> Result<(), OpenError> {
    Err(OpenError::Unavailable(io::ErrorKind::NotFound))
}

fn destination_file(dir: &Path) -> PathBuf {
    dir.join("state")
        .join("cloud")
        .join("fusion-destination.json")
}

fn signed_in(
    dir: &Path,
    provider: &Arc<FakeProvider>,
    outcome: fn() -> Result<(), OpenError>,
) -> (Harness, Opened) {
    let opened = Opened::default();
    let mut harness = with_opener(dir, provider, recording(&opened, outcome));
    open_home(&mut harness);
    sign_in(&mut harness);
    (harness, opened)
}

fn choose_the_folder_and_send_and_open(harness: &mut Harness) {
    run_from_palette(harness, COMMAND);
    harness.frame();
    harness.frame();

    assert!(harness.workspace.cloud_dialog.is_some());
    assert!(harness.shows(cloud_dialog::CHOOSE_A_FOLDER_TO_OPEN));

    harness.wait_until("the projects are listed", |harness| harness.shows("Parts"));
    harness.click("Parts");
    harness.wait_until("the folder is listed", |harness| {
        harness.shows(cloud_dialog::SEND_AND_OPEN)
    });
    harness.click(cloud_dialog::SEND_AND_OPEN);
    confirm_the_name(harness);
}

fn confirm_the_name(harness: &mut Harness) {
    harness.wait_until("the name is asked", |harness| {
        harness.shows(cloud_send::NAME_SEND)
    });
    harness.click(cloud_send::NAME_SEND);
}

fn choose_the_folder_without_naming(harness: &mut Harness) {
    run_from_palette(harness, COMMAND);
    harness.frame();
    harness.frame();
    harness.wait_until("the projects are listed", |harness| harness.shows("Parts"));
    harness.click("Parts");
    harness.wait_until("the folder is listed", |harness| {
        harness.shows(cloud_dialog::SEND_AND_OPEN)
    });
    harness.click(cloud_dialog::SEND_AND_OPEN);
}

fn name_the_file(harness: &mut Harness, name: &str) {
    harness.wait_until("the name is asked", |harness| {
        harness.shows(cloud_send::NAME_SEND)
    });
    harness.type_into_field(cloud_send::name_field_id(), name);
}

fn type_the_name(harness: &mut Harness, name: &str) {
    let field = cloud_send::name_field_id();
    harness
        .context
        .memory_mut(|memory| memory.request_focus(field));
    harness.frame();
    harness.key(Key::A, Modifiers::COMMAND);
    harness.events.push(Event::Text(name.to_owned()));
    harness.frame();
    harness.frame();
}

fn typed_name(harness: &Harness) -> Option<&str> {
    harness
        .files
        .cloud_naming()
        .map(cloud_send::CloudNaming::text)
}

fn save_as(harness: &mut Harness, path: PathBuf) {
    harness.answer_dialog(Some(path));
    harness.command(FileCommand::SaveAs);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
}

fn wait_for_the_send_to_end(harness: &mut Harness) {
    harness.wait_until("the sending ends", |harness| {
        harness
            .workspace
            .cloud
            .linked(Service::Fusion)
            .is_some_and(|linked| linked.sending.is_none())
    });
}

fn notice_text(harness: &Harness) -> String {
    harness
        .model
        .notice()
        .map(|notice| notice.text.clone())
        .unwrap_or_default()
}

fn close_the_manager(harness: &mut Harness) {
    harness.perform(Action::Preferences(PreferencesCommand::CloseCloud));
    harness.frame();
    harness.frame();
}

#[test]
fn the_first_run_chooses_a_folder_remembers_it_and_opens_the_file_on_the_web() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let (mut harness, opened) = signed_in(dir.path(), &provider, opens);
    extruded_plate(&mut harness);

    choose_the_folder_and_send_and_open(&mut harness);
    harness.wait_until("the file is opened", |_| opened.lock().len() == 1);
    harness.wait_until("the folder is remembered", |_| {
        destination_file(dir.path()).exists()
    });

    let first = provider.received().remove(0);
    assert_eq!(first.folder, top_folder());
    assert!(first.name.ends_with(".step"));
    assert_eq!(
        opened.lock().first().cloned(),
        Some(format!(
            "https://fake.invalid/p1/top/{}?version=1",
            first.name
        ))
    );
    assert!(notice_text(&harness).contains("Opening it in your browser"));
    assert!(
        fs::read_to_string(destination_file(dir.path()))
            .unwrap()
            .contains("\"folder\":\"top\"")
    );
    assert!(outgoing_files(dir.path()).is_empty());
}

#[test]
fn the_second_run_sends_to_the_remembered_folder_as_version_two_and_opens_it_without_asking() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let (mut harness, opened) = signed_in(dir.path(), &provider, opens);
    extruded_plate(&mut harness);
    choose_the_folder_and_send_and_open(&mut harness);
    harness.wait_until("the file is opened", |_| opened.lock().len() == 1);
    wait_for_the_send_to_end(&mut harness);
    close_the_manager(&mut harness);

    run_from_palette(&mut harness, COMMAND);
    confirm_the_name(&mut harness);
    harness.wait_until("the second file is opened", |_| opened.lock().len() == 2);
    wait_for_the_send_to_end(&mut harness);

    let names: Vec<_> = provider
        .received()
        .into_iter()
        .map(|received| received.name)
        .collect();
    assert_eq!(names.len(), 2);
    assert_eq!(names.first(), names.get(1));
    assert!(harness.workspace.cloud_dialog.is_none());
    assert!(
        opened
            .lock()
            .get(1)
            .is_some_and(|link| link.ends_with("?version=2"))
    );
    assert!(notice_text(&harness).contains("as version 2"));
    assert!(outgoing_files(dir.path()).is_empty());
}

#[test]
fn a_plain_send_never_opens_anything() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let (mut harness, opened) = signed_in(dir.path(), &provider, opens);
    extruded_plate(&mut harness);
    run_from_palette(&mut harness, "Cloud projects");
    harness.wait_until("the projects are listed", |harness| harness.shows("Parts"));
    harness.click("Parts");
    harness.wait_until("the folder is listed", |harness| {
        harness.shows(cloud_dialog::SEND_HERE)
    });

    harness.click(cloud_dialog::SEND_HERE);
    harness.wait_until("the model is sent", |_| provider.received().len() == 1);
    wait_for_the_send_to_end(&mut harness);

    assert!(opened.lock().is_empty());
    assert!(!destination_file(dir.path()).exists());
}

#[test]
fn cancelling_the_send_opens_nothing_and_sends_nothing() {
    let dir = TempDir::new().unwrap();
    let released = Arc::new(AtomicBool::new(false));
    let provider = Arc::new(
        Arc::into_inner(sendable_provider(Some(top_folder()), Vec::new()))
            .unwrap()
            .holding_sends_until(Arc::clone(&released)),
    );
    let (mut harness, opened) = signed_in(dir.path(), &provider, opens);
    extruded_plate(&mut harness);
    choose_the_folder_and_send_and_open(&mut harness);
    harness.wait_until("the upload is under way", |_| {
        provider
            .calls()
            .iter()
            .any(|call| call.starts_with("send top"))
    });

    harness.perform(Action::Cloud(CloudCommand::CancelSend(Service::Fusion)));
    harness.wait_until("the job has ended", |_| {
        outgoing_files(dir.path()).is_empty()
    });
    released.store(true, Ordering::Relaxed);
    harness.frame();
    harness.frame();

    assert!(provider.received().is_empty());
    assert!(opened.lock().is_empty());
    assert!(
        harness
            .workspace
            .cloud
            .linked(Service::Fusion)
            .is_some_and(|linked| linked.sending.is_none())
    );
}

#[test]
fn a_browser_that_will_not_start_leaves_the_upload_and_offers_the_link() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let (mut harness, opened) = signed_in(dir.path(), &provider, no_browser);
    extruded_plate(&mut harness);

    choose_the_folder_and_send_and_open(&mut harness);
    harness.wait_until("the link is kept for copying", |harness| {
        harness
            .workspace
            .cloud
            .linked(Service::Fusion)
            .is_some_and(|linked| linked.unopened.is_some())
    });

    assert_eq!(provider.received().len(), 1);
    assert_eq!(opened.lock().len(), 1);
    assert!(notice_text(&harness).contains("could not open it"));
    assert!(harness.shows(cloud_parts::COPY_LINK));

    harness.click(cloud_parts::DISMISS);
    harness.frame();

    assert!(!harness.shows(cloud_parts::COPY_LINK));
    assert_eq!(provider.received().len(), 1);
}

#[test]
fn an_answer_without_a_web_address_says_so_and_opens_nothing() {
    let dir = TempDir::new().unwrap();
    let provider = Arc::new(
        Arc::into_inner(sendable_provider(Some(top_folder()), Vec::new()))
            .unwrap()
            .without_web_links(),
    );
    let (mut harness, opened) = signed_in(dir.path(), &provider, opens);
    extruded_plate(&mut harness);

    choose_the_folder_and_send_and_open(&mut harness);
    harness.wait_until("the model is sent", |_| provider.received().len() == 1);
    wait_for_the_send_to_end(&mut harness);

    assert!(notice_text(&harness).contains("gave no web address"));
    assert!(opened.lock().is_empty());
    harness.wait_until("the folder is remembered", |_| {
        destination_file(dir.path()).exists()
    });
}

#[test]
fn a_remembered_folder_that_is_gone_is_forgotten_and_the_manager_opens_to_choose_another() {
    let dir = TempDir::new().unwrap();
    let provider = Arc::new(
        Arc::into_inner(sendable_provider(Some(top_folder()), Vec::new()))
            .unwrap()
            .failing(Step::Send, || Error::NotFound {
                subject: Subject::Folder,
            }),
    );
    let (mut harness, opened) = signed_in(dir.path(), &provider, opens);
    extruded_plate(&mut harness);
    choose_the_folder_and_send_and_open(&mut harness);
    harness.wait_until("the failure is told", |harness| {
        notice_text(harness).contains("is gone or does not let caditor add files")
    });
    harness.wait_until("the destination is forgotten", |_| {
        !destination_file(dir.path()).exists()
    });
    close_the_manager(&mut harness);

    run_from_palette(&mut harness, COMMAND);
    harness.frame();
    harness.frame();

    assert!(harness.workspace.cloud_dialog.is_some());
    assert!(harness.shows(cloud_dialog::CHOOSE_A_FOLDER_TO_OPEN));
    assert!(opened.lock().is_empty());
    assert!(provider.received().is_empty());
}

#[test]
fn signing_out_forgets_the_remembered_folder() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let (mut harness, opened) = signed_in(dir.path(), &provider, opens);
    extruded_plate(&mut harness);
    choose_the_folder_and_send_and_open(&mut harness);
    harness.wait_until("the file is opened", |_| opened.lock().len() == 1);
    harness.wait_until("the folder is remembered", |_| {
        destination_file(dir.path()).exists()
    });

    harness.perform(Action::Cloud(CloudCommand::SignOut(Service::Fusion)));

    harness.wait_until("the folder is forgotten", |_| {
        !destination_file(dir.path()).exists()
    });
}

#[test]
fn the_command_is_refused_with_its_reason_before_anything_is_exported() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let opened = Opened::default();
    let mut harness = with_opener(dir.path(), &provider, recording(&opened, opens));

    run_from_palette(&mut harness, COMMAND);
    harness.frame();

    assert!(notice_text(&harness).contains("Sign in to Fusion 360 first"));
    assert!(harness.workspace.cloud_dialog.is_none());

    open_home(&mut harness);
    sign_in(&mut harness);
    harness.frame();

    let refusal = offer(&harness, Command::CloudOpenInFusion).availability;
    assert!(
        refusal
            .as_ref()
            .is_err_and(|reason| reason.contains("no bodies to send"))
    );

    run_from_palette(&mut harness, COMMAND);
    harness.frame();

    assert!(notice_text(&harness).contains("no bodies to send"));
    assert!(harness.workspace.cloud_dialog.is_none());
    assert!(provider.received().is_empty());
    assert!(outgoing_files(dir.path()).is_empty());
    assert!(opened.lock().is_empty());
}

#[test]
fn with_cloud_off_the_command_is_refused_and_no_request_is_made() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let opened = Opened::default();
    let mut harness = with_opener(dir.path(), &provider, recording(&opened, opens));
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::CloudEnabled(false),
    )));
    harness.frame();

    run_from_palette(&mut harness, COMMAND);
    harness.frame();

    assert!(notice_text(&harness).contains("Cloud services are turned off"));
    assert!(harness.workspace.cloud_dialog.is_none());
    assert!(provider.calls().is_empty());
    assert!(opened.lock().is_empty());
}

#[test]
fn an_untitled_model_asks_for_a_name_first_and_is_sent_under_it() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let (mut harness, opened) = signed_in(dir.path(), &provider, opens);
    extruded_plate(&mut harness);
    run_from_palette(&mut harness, COMMAND);
    harness.wait_until("the projects are listed", |harness| harness.shows("Parts"));
    harness.click("Parts");
    harness.wait_until("the folder is listed", |harness| {
        harness.shows(cloud_dialog::SEND_AND_OPEN)
    });

    harness.click(cloud_dialog::SEND_AND_OPEN);
    harness.wait_until("the name is asked", |harness| {
        harness.shows(cloud_send::NAME_SEND)
    });

    assert!(harness.shows("Name the file for Fusion 360"));
    assert_eq!(typed_name(&harness), Some("Untitled"));
    assert!(harness.files.is_blocking());
    assert!(provider.received().is_empty());
    assert!(outgoing_files(dir.path()).is_empty());

    name_the_file(&mut harness, "Bracket");
    harness.wait_until("the file is opened", |_| opened.lock().len() == 1);
    wait_for_the_send_to_end(&mut harness);

    let sent = provider.received().remove(0);
    assert_eq!(sent.name, "Bracket.step");
    assert!(!harness.shows(cloud_send::NAME_SEND));
    assert!(notice_text(&harness).contains("Bracket.step"));
}

#[test]
fn the_name_dialog_starts_from_untitled_and_the_second_run_from_the_name_sent_before() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let (mut harness, opened) = signed_in(dir.path(), &provider, opens);
    extruded_plate(&mut harness);
    choose_the_folder_and_send_and_open(&mut harness);
    harness.wait_until("the first file is opened", |_| opened.lock().len() == 1);
    wait_for_the_send_to_end(&mut harness);
    close_the_manager(&mut harness);

    assert_eq!(
        provider.received().first().map(|sent| sent.name.as_str()),
        Some("Untitled.step")
    );

    run_from_palette(&mut harness, COMMAND);
    harness.wait_until("the name is asked", |harness| {
        harness.shows(cloud_send::NAME_SEND)
    });
    name_the_file(&mut harness, "Plate.STEP");
    harness.wait_until("the second file is opened", |_| opened.lock().len() == 2);
    wait_for_the_send_to_end(&mut harness);

    let names: Vec<_> = provider
        .received()
        .into_iter()
        .map(|received| received.name)
        .collect();
    assert_eq!(names, ["Untitled.step", "Plate.step"]);

    run_from_palette(&mut harness, COMMAND);
    harness.wait_until("the name is asked", |harness| {
        harness.shows(cloud_send::NAME_SEND)
    });

    assert_eq!(typed_name(&harness), Some("Plate.STEP"));
}

#[test]
fn a_name_that_cannot_be_a_file_name_is_told_and_cannot_be_sent() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let (mut harness, opened) = signed_in(dir.path(), &provider, opens);
    extruded_plate(&mut harness);
    choose_the_folder_without_naming(&mut harness);
    harness.wait_until("the name is asked", |harness| {
        harness.shows(cloud_send::NAME_SEND)
    });

    type_the_name(&mut harness, "a/b");

    assert!(harness.shows_containing("cannot contain"));

    harness.click(cloud_send::NAME_SEND);
    harness.frame();

    assert!(harness.shows(cloud_send::NAME_SEND));
    assert!(provider.received().is_empty());

    type_the_name(&mut harness, "  ");

    assert!(harness.shows_containing("Type a name"));
    assert!(opened.lock().is_empty());
}

#[test]
fn cancelling_the_name_sends_nothing_and_exports_nothing() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let (mut harness, opened) = signed_in(dir.path(), &provider, opens);
    extruded_plate(&mut harness);
    choose_the_folder_without_naming(&mut harness);
    harness.wait_until("the name is asked", |harness| {
        harness.shows(cloud_send::NAME_SEND)
    });

    harness.click(cloud_send::NAME_CANCEL);
    harness.frame();
    harness.frame();

    assert!(!harness.shows(cloud_send::NAME_SEND));
    assert!(!harness.files.is_blocking());
    assert!(provider.received().is_empty());
    assert!(outgoing_files(dir.path()).is_empty());
    assert!(opened.lock().is_empty());
}

#[test]
fn a_saved_model_is_sent_under_its_own_name_without_asking() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let (mut harness, opened) = signed_in(dir.path(), &provider, opens);
    extruded_plate(&mut harness);
    save_as(&mut harness, dir.path().join("plate.caditor"));

    choose_the_folder_without_naming(&mut harness);
    harness.wait_until("the file is opened", |_| opened.lock().len() == 1);

    assert!(!harness.shows(cloud_send::NAME_SEND));
    assert_eq!(
        provider.received().first().map(|sent| sent.name.as_str()),
        Some("plate.step")
    );
}

#[test]
fn a_plain_send_of_an_untitled_model_does_not_ask_for_a_name() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let (mut harness, _) = signed_in(dir.path(), &provider, opens);
    extruded_plate(&mut harness);
    run_from_palette(&mut harness, "Cloud projects");
    harness.wait_until("the projects are listed", |harness| harness.shows("Parts"));
    harness.click("Parts");
    harness.wait_until("the folder is listed", |harness| {
        harness.shows(cloud_dialog::SEND_HERE)
    });

    harness.click(cloud_dialog::SEND_HERE);
    harness.wait_until("the model is sent", |_| provider.received().len() == 1);

    assert!(!harness.shows(cloud_send::NAME_SEND));
    assert_eq!(
        provider.received().first().map(|sent| sent.name.as_str()),
        Some("Untitled.step")
    );
}

#[test]
fn the_export_dialog_opens_the_model_in_fusion_through_the_same_command() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let (mut harness, opened) = signed_in(dir.path(), &provider, opens);
    extruded_plate(&mut harness);
    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.frame();

    assert!(harness.shows(export::FUSION_LABEL));

    harness.click(export::FUSION_LABEL);
    harness.frame();
    harness.frame();

    assert!(!harness.shows("Export…"));
    assert!(harness.workspace.cloud_dialog.is_some());
    assert!(harness.shows(cloud_dialog::CHOOSE_A_FOLDER_TO_OPEN));

    harness.wait_until("the projects are listed", |harness| harness.shows("Parts"));
    harness.click("Parts");
    harness.wait_until("the folder is listed", |harness| {
        harness.shows(cloud_dialog::SEND_AND_OPEN)
    });
    harness.click(cloud_dialog::SEND_AND_OPEN);
    confirm_the_name(&mut harness);
    harness.wait_until("the file is opened", |_| opened.lock().len() == 1);
    close_the_manager(&mut harness);

    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.frame();
    harness.click(export::FUSION_LABEL);
    confirm_the_name(&mut harness);
    harness.wait_until("the second file is opened", |_| opened.lock().len() == 2);

    assert!(harness.workspace.cloud_dialog.is_none());
    assert_eq!(provider.received().len(), 2);
}

#[test]
fn the_export_dialog_button_is_refused_with_its_reason_until_signed_in_and_hidden_with_cloud_off() {
    let dir = TempDir::new().unwrap();
    let provider = sendable_provider(Some(top_folder()), Vec::new());
    let opened = Opened::default();
    let mut harness = with_opener(dir.path(), &provider, recording(&opened, opens));
    extruded_plate(&mut harness);
    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.frame();

    assert!(harness.shows(export::FUSION_LABEL));

    harness.click(export::FUSION_LABEL);
    harness.frame();

    assert!(harness.shows("Export…"));
    assert!(harness.workspace.cloud_dialog.is_none());
    assert!(provider.calls().is_empty());

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::CloudEnabled(false),
    )));
    harness.frame();
    harness.frame();

    assert!(harness.shows("Export…"));
    assert!(!harness.shows(export::FUSION_LABEL));
    assert!(opened.lock().is_empty());
}
