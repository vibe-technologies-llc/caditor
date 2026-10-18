use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use parking_lot::Mutex;
use tempfile::TempDir;

use super::{Harness, extruded_plate, offer, run_from_palette, sample_document};
use crate::{
    app::Workspace,
    commands::Command,
    export::ExportCommand,
    files::FileCommand,
    model::{Action, NoticeKind},
    preferences::{PreferenceChange, PreferencesCommand},
    slicer::{FindError, Installation, Launch, LaunchError, Slicer},
};

type Launched = Arc<Mutex<Vec<PathBuf>>>;
type Asked = Arc<Mutex<Vec<Option<PathBuf>>>>;

fn installed(program: &Path) -> Installation {
    Installation {
        slicer: Slicer::OrcaSlicer,
        launch: Launch::Program(program.to_path_buf()),
    }
}

fn with_slicer(
    dir: &Path,
    found: Result<Installation, FindError>,
    launch: Result<(), LaunchError>,
) -> (Harness, Launched, Asked) {
    let launched = Launched::default();
    let asked = Asked::default();
    let (record, ask) = (Arc::clone(&launched), Arc::clone(&asked));
    let harness = Harness::starting_with(
        Some(dir),
        sample_document().unwrap(),
        Workspace::new(),
        move |files| {
            files.find_slicers_with(Arc::new(move |_, chosen: Option<&Path>| {
                ask.lock().push(chosen.map(Path::to_path_buf));
                found.clone()
            }));
            files.launch_slicers_with(Arc::new(move |_, file: &Path| {
                record.lock().push(file.to_path_buf());
                launch.clone()
            }));
        },
    );
    (harness, launched, asked)
}

fn wait_until_found(harness: &mut Harness) {
    harness.wait_until("the slicer is looked for", |harness| {
        harness.files.found_slicer().is_some()
    });
    harness.frame();
}

fn notice_text(harness: &Harness) -> Option<(String, NoticeKind)> {
    harness
        .model
        .notice()
        .map(|notice| (notice.text.clone(), notice.kind))
}

#[test]
fn open_in_slicer_writes_a_3mf_of_its_own_and_starts_the_slicer_with_it() {
    let dir = TempDir::new().unwrap();
    let program = dir.path().join("orca-slicer");
    let (mut harness, launched, _) = with_slicer(dir.path(), Ok(installed(&program)), Ok(()));
    let expected = dir.path().join("state").join("slicer").join("Untitled.3mf");

    wait_until_found(&mut harness);

    assert_eq!(
        offer(&harness, Command::OpenInSlicer).availability,
        Err(
            "There are no bodies to open in OrcaSlicer yet. Extrude or revolve a sketch to make one"
                .into()
        )
    );

    extruded_plate(&mut harness);
    harness.frame();

    assert_eq!(offer(&harness, Command::OpenInSlicer).availability, Ok(()));

    run_from_palette(&mut harness, "open in orcaslicer");
    harness.wait_until("the slicer is started", |_| !launched.lock().is_empty());
    harness.frame();

    assert_eq!(*launched.lock(), std::slice::from_ref(&expected));
    assert!(fs::read(&expected).unwrap().starts_with(b"PK\x03\x04"));
    assert_eq!(
        notice_text(&harness),
        Some((
            "Exported 1 body to “Untitled.3mf” (12 triangles). OrcaSlicer is opening it."
                .to_owned(),
            NoticeKind::Success
        ))
    );
    assert!(offer(&harness, Command::ExportAgain).availability.is_err());
}

#[test]
fn the_export_dialog_opens_the_chosen_bodies_in_the_slicer_and_closes() {
    let dir = TempDir::new().unwrap();
    let program = dir.path().join("orca-slicer");
    let (mut harness, launched, _) = with_slicer(dir.path(), Ok(installed(&program)), Ok(()));
    let (extrude, _) = extruded_plate(&mut harness);
    wait_until_found(&mut harness);

    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.command(FileCommand::Export(ExportCommand::Include {
        body: extrude,
        included: false,
    }));
    harness.frame();

    assert!(
        offer(&harness, Command::OpenInSlicer)
            .availability
            .is_err_and(|reason| reason.starts_with("Every body is left out"))
    );

    harness.command(FileCommand::Export(ExportCommand::Include {
        body: extrude,
        included: true,
    }));
    harness.click("Open in OrcaSlicer");
    harness.wait_until("the slicer is started", |_| !launched.lock().is_empty());
    harness.frame();

    assert!(!harness.files.is_blocking());
    assert!(!harness.shows("Export…"));
}

#[test]
fn a_missing_slicer_is_named_and_a_failed_start_says_which_program() {
    let dir = TempDir::new().unwrap();
    let missing = FindError::NotFound {
        slicer: Slicer::OrcaSlicer,
    };
    let (mut harness, launched, _) = with_slicer(dir.path(), Err(missing.clone()), Ok(()));
    extruded_plate(&mut harness);
    wait_until_found(&mut harness);

    assert_eq!(
        offer(&harness, Command::OpenInSlicer).availability,
        Err(missing.to_string().into())
    );

    harness.command(FileCommand::OpenInSlicer);
    harness.wait_until("the missing slicer is said", |harness| {
        notice_text(harness).is_some_and(|(_, kind)| kind == NoticeKind::Error)
    });

    assert_eq!(
        notice_text(&harness).map(|(text, _)| text),
        Some(format!("{missing}."))
    );
    assert!(launched.lock().is_empty());

    let dir = TempDir::new().unwrap();
    let program = dir.path().join("orca-slicer");
    let refused = LaunchError::Unavailable {
        slicer: Slicer::OrcaSlicer,
        program: program.clone(),
        kind: std::io::ErrorKind::NotFound,
    };
    let (mut harness, _, _) =
        with_slicer(dir.path(), Ok(installed(&program)), Err(refused.clone()));
    extruded_plate(&mut harness);
    wait_until_found(&mut harness);

    harness.command(FileCommand::OpenInSlicer);
    harness.wait_until("the failed start is said", |harness| {
        notice_text(harness).is_some_and(|(_, kind)| kind == NoticeKind::Error)
    });

    assert_eq!(
        notice_text(&harness).map(|(text, _)| text),
        Some(format!(
            "Exported 1 body to “Untitled.3mf” (12 triangles). {refused}."
        ))
    );
}

#[test]
fn naming_the_slicer_program_in_preferences_looks_for_it_there() {
    let dir = TempDir::new().unwrap();
    let program = dir.path().join("orca-slicer");
    let (mut harness, _, asked) = with_slicer(dir.path(), Ok(installed(&program)), Ok(()));
    wait_until_found(&mut harness);

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::SlicerProgram(" /opt/Orca.AppImage ".to_owned()),
    )));
    harness.wait_until("the named program is looked for", |_| {
        asked
            .lock()
            .contains(&Some(PathBuf::from("/opt/Orca.AppImage")))
    });

    assert_eq!(
        harness.workspace.preferences.slicer_program,
        Some(PathBuf::from("/opt/Orca.AppImage"))
    );
    assert_eq!(asked.lock().first(), Some(&None));
}
