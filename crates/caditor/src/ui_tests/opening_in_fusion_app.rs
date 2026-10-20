use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
};

use parking_lot::Mutex;
use tempfile::TempDir;

use super::{Harness, add_peg, extruded_plate, offer, run_from_palette, sample_document};
use crate::{
    app::Workspace,
    commands::Command,
    export::{self, ExportCommand},
    files::FileCommand,
    fusion_app::LaunchError,
    model::NoticeKind,
};

type Launched = Arc<Mutex<Vec<PathBuf>>>;

fn with_fusion_app(dir: &Path, launch: Result<(), LaunchError>) -> (Harness, Launched) {
    let launched = Launched::default();
    let record = Arc::clone(&launched);
    let harness = Harness::starting_with(
        Some(dir),
        sample_document().unwrap(),
        Workspace::new(),
        move |files| {
            files.launch_fusion_app_with(Arc::new(move |file: &Path| {
                record.lock().push(file.to_path_buf());
                launch.clone()
            }));
        },
    );
    (harness, launched)
}

fn notice_text(harness: &Harness) -> Option<(String, NoticeKind)> {
    harness
        .model
        .notice()
        .map(|notice| (notice.text.clone(), notice.kind))
}

#[test]
fn open_in_fusion_writes_a_step_of_its_own_and_hands_it_to_the_app() {
    let dir = TempDir::new().unwrap();
    let (mut harness, launched) = with_fusion_app(dir.path(), Ok(()));
    let expected = dir
        .path()
        .join("state")
        .join("fusion")
        .join("Untitled.step");

    assert_eq!(
        offer(&harness, Command::OpenInFusionApp).availability,
        Err(
            "There are no bodies to open in Fusion 360 yet. Extrude or revolve a sketch to make one"
                .into()
        )
    );

    extruded_plate(&mut harness);
    harness.frame();

    assert_eq!(
        offer(&harness, Command::OpenInFusionApp).availability,
        Ok(())
    );

    run_from_palette(&mut harness, "open in fusion 360");
    harness.wait_until("Fusion 360 is handed the file", |_| {
        !launched.lock().is_empty()
    });
    harness.frame();

    assert_eq!(*launched.lock(), std::slice::from_ref(&expected));
    assert!(fs::read(&expected).unwrap().starts_with(b"ISO-10303-21;"));
    assert!(
        notice_text(&harness).is_some_and(|(text, kind)| kind == NoticeKind::Success
            && text.ends_with(" Fusion 360 is opening it."))
    );
    assert!(offer(&harness, Command::ExportAgain).availability.is_err());
}

#[test]
fn the_export_dialog_opens_the_chosen_bodies_in_the_app_and_closes() {
    let dir = TempDir::new().unwrap();
    let (mut harness, launched) = with_fusion_app(dir.path(), Ok(()));
    let (extrude, _) = extruded_plate(&mut harness);

    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.command(FileCommand::Export(ExportCommand::Include {
        body: extrude,
        included: false,
    }));
    harness.frame();

    assert!(
        offer(&harness, Command::OpenInFusionApp)
            .availability
            .is_err_and(|reason| reason.starts_with("Every body is left out"))
    );

    harness.command(FileCommand::Export(ExportCommand::Include {
        body: extrude,
        included: true,
    }));
    harness.click(export::FUSION_APP_LABEL);
    harness.wait_until("Fusion 360 is handed the file", |_| {
        !launched.lock().is_empty()
    });
    harness.frame();

    assert!(!harness.files.is_blocking());
    assert!(!harness.shows("Export…"));
}

#[test]
fn a_refused_hand_over_is_said_after_the_export() {
    let dir = TempDir::new().unwrap();
    let refused = LaunchError::Unavailable(ErrorKind::NotFound);
    let (mut harness, _) = with_fusion_app(dir.path(), Err(refused.clone()));
    extruded_plate(&mut harness);

    harness.command(FileCommand::OpenInFusionApp(Vec::new()));
    harness.wait_until("the refusal is said", |harness| {
        notice_text(harness).is_some_and(|(_, kind)| kind == NoticeKind::Error)
    });

    assert!(notice_text(&harness).is_some_and(
        |(text, _)| text.ends_with(&format!(" {refused}.")) && text.contains("Untitled.step")
    ));
}

#[test]
fn bodies_named_by_the_command_are_the_only_ones_handed_over() {
    let dir = TempDir::new().unwrap();
    let (mut harness, launched) = with_fusion_app(dir.path(), Ok(()));
    let (plate, _) = extruded_plate(&mut harness);
    add_peg(&mut harness);
    harness.settle();

    harness.command(FileCommand::OpenInFusionApp(vec![plate]));
    harness.wait_until("Fusion 360 is handed the file", |_| {
        !launched.lock().is_empty()
    });
    harness.frame();

    assert!(notice_text(&harness).is_some_and(|(text, _)| text.starts_with("Exported 1 body to")));

    harness.command(FileCommand::OpenInFusionApp(Vec::new()));
    harness.wait_until("Fusion 360 is handed the file again", |_| {
        launched.lock().len() == 2
    });
    harness.frame();

    assert!(
        notice_text(&harness).is_some_and(|(text, _)| text.starts_with("Exported 2 bodies to"))
    );
}

#[test]
fn the_palette_hands_over_only_the_selected_body() {
    let dir = TempDir::new().unwrap();
    let (mut harness, launched) = with_fusion_app(dir.path(), Ok(()));
    let (_, top) = extruded_plate(&mut harness);
    add_peg(&mut harness);
    harness.settle();

    harness.select([top]);
    run_from_palette(&mut harness, "open in fusion 360");
    harness.wait_until("Fusion 360 is handed the file", |_| {
        !launched.lock().is_empty()
    });
    harness.frame();

    assert!(notice_text(&harness).is_some_and(|(text, _)| text.starts_with("Exported 1 body to")));
}
