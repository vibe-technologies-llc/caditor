use caditor_file::MeshUnit;
use tempfile::TempDir;

use super::{Harness, cube_stl, volume_about};
use crate::{files::FileCommand, import_unit::ImportUnitCommand};

fn start_import(harness: &mut Harness, dir: &TempDir, name: &str, side: f64) {
    let path = dir.path().join(name);
    std::fs::write(&path, cube_stl(side)).unwrap();
    harness.answer_dialog(Some(path));
    harness.command(FileCommand::Import { into: None });
}

fn wait_for_unit_offer(harness: &mut Harness, file: &str) {
    let title = format!("Import “{file}”");
    harness.wait_until("the unit offer is shown", |harness| harness.shows(&title));
}

#[test]
fn a_mesh_far_too_small_for_millimetres_is_offered_metres_before_anything_is_added() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let before = harness.document().features().len();

    start_import(&mut harness, &dir, "tiny.stl", 0.01);
    wait_for_unit_offer(&mut harness, "tiny.stl");

    assert!(harness.shows_containing("STL files carry no units"));
    assert!(harness.shows_containing("so caditor guessed metres"));
    assert!(harness.shows_containing("Read as metres, the part is 10.000 mm by 10.000 mm"));
    assert_eq!(harness.document().features().len(), before);
    assert!(harness.files.is_blocking());

    harness.click("Import");
    harness.wait_until("the body is imported", |harness| {
        harness.document().features().len() == before + 1
    });
    harness.settle();
    let body = harness.document().features().last().unwrap().id();

    assert!(volume_about(&harness, body, 1000.0));
    assert!(harness.shows_containing("its sizes were read as metres"));
}

#[test]
fn choosing_another_unit_reads_the_mesh_again_in_it_and_the_report_says_it_was_chosen() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let before = harness.document().features().len();

    start_import(&mut harness, &dir, "tiny.stl", 0.5);
    wait_for_unit_offer(&mut harness, "tiny.stl");
    harness.command(FileCommand::ImportUnit(ImportUnitCommand::Choose(
        MeshUnit::Centimetres,
    )));
    harness.frame();

    assert!(harness.shows_containing("Read as centimetres, the part is 5.000 mm by 5.000 mm"));
    assert_eq!(harness.document().features().len(), before);

    harness.click("Import");
    harness.wait_until("the body is imported", |harness| {
        harness.document().features().len() == before + 1
    });
    harness.settle();
    let body = harness.document().features().last().unwrap().id();

    assert!(volume_about(&harness, body, 125.0));
    assert!(harness.shows_containing("read as centimetres, as you chose"));
}

#[test]
fn cancelling_the_unit_offer_adds_nothing() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let before = harness.document().features().len();

    start_import(&mut harness, &dir, "tiny.stl", 0.01);
    wait_for_unit_offer(&mut harness, "tiny.stl");
    harness.click("Cancel");
    harness.frame();

    assert!(!harness.files.is_blocking());
    assert!(!harness.files.is_importing());
    assert_eq!(harness.document().features().len(), before);
    assert!(!harness.shows("Import “tiny.stl”"));
}

#[test]
fn a_mesh_of_plausible_millimetre_size_imports_without_the_offer() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let before = harness.document().features().len();

    start_import(&mut harness, &dir, "cube.stl", 10.0);
    harness.wait_until("the body is imported", |harness| {
        harness.document().features().len() == before + 1
    });

    assert!(!harness.shows_containing("files carry no units"));
}
