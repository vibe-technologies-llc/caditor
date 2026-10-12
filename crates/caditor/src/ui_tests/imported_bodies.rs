use egui::Id;
use tempfile::TempDir;

use super::{Harness, cube_stl, extruded_plate, plate_bounds, volume_about, write_drawing};
use crate::{export::ExportCommand, files::FileCommand, model::Action};

#[test]
fn a_step_export_imports_back_as_a_body_that_later_features_can_use() {
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
    harness.command(FileCommand::New);
    harness.frame();
    if harness.shows("Continue without saving") {
        harness.click("Continue without saving");
    }
    harness.settle();
    let before = harness.document().features().len();
    harness.answer_dialog(Some(exported));
    harness.command(FileCommand::Import { into: None });
    harness.wait_until("the body is imported", |harness| {
        harness.document().features().len() == before + 1
    });
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Imported 1 body from “plate.step”."
    );
    let body = harness.document().features().last().unwrap().id();
    assert!(
        harness
            .document()
            .feature(body)
            .unwrap()
            .kind
            .import()
            .is_some()
    );
    harness.settle();
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    let solid = harness.model.evaluation().body(body).unwrap();
    assert_eq!(solid.faces().count(), 6);
    harness.perform(Action::Undo);
    assert_eq!(harness.document().features().len(), before);
}

#[test]
fn an_imported_body_reloads_from_the_file_it_came_from_without_asking() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let path = dir.path().join("cube.stl");
    std::fs::write(&path, cube_stl(10.0)).unwrap();
    let before = harness.document().features().len();
    harness.answer_dialog(Some(path.clone()));
    harness.command(FileCommand::Import { into: None });
    harness.wait_until("the cube is imported", |harness| {
        harness.document().features().len() == before + 1
    });
    harness.settle();
    let body = harness.document().features().last().unwrap().id();
    let kept = harness
        .document()
        .feature(body)
        .and_then(|feature| feature.kind.import())
        .and_then(|import| import.path.clone());
    assert_eq!(kept, Some(path.clone()));

    std::fs::write(&path, cube_stl(20.0)).unwrap();
    harness.command(FileCommand::ReloadImport(body));
    harness.wait_until("the cube is reloaded", |harness| {
        harness
            .model
            .undo_label()
            .is_some_and(|label| label.starts_with("Replace"))
    });
    harness.settle();
    assert!(volume_about(&harness, body, 8000.0));

    std::fs::remove_file(&path).unwrap();
    harness.command(FileCommand::ReloadImport(body));
    harness.wait_until("the missing file is reported", |harness| {
        harness.shows_containing("is no longer there")
    });
}

#[test]
fn reloading_an_import_whose_file_is_gone_says_so_in_the_same_words() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let path = dir.path().join("cube.stl");
    std::fs::write(&path, cube_stl(10.0)).unwrap();
    let before = harness.document().features().len();
    harness.answer_dialog(Some(path.clone()));
    harness.command(FileCommand::Import { into: None });
    harness.wait_until("the cube is imported", |harness| {
        harness.document().features().len() == before + 1
    });
    harness.settle();
    let body = harness.document().features().last().unwrap().id();
    let name = harness.document().feature(body).unwrap().name.clone();
    std::fs::remove_file(&path).unwrap();

    harness.command(FileCommand::ReloadImport(body));
    harness.wait_until("the missing file is reported", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.contains("is no longer there"))
    });

    assert_eq!(
        harness.model.notice().unwrap().text,
        format!(
            "{name} cannot be reloaded: “{}” is no longer there. Use Replace from file to \
             choose where it is now.",
            path.display()
        )
    );
}

#[test]
fn an_imported_body_is_placed_from_its_details_and_keeps_its_place_when_reloaded() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let path = dir.path().join("cube.stl");
    std::fs::write(&path, cube_stl(10.0)).unwrap();
    let before = harness.document().features().len();
    harness.answer_dialog(Some(path.clone()));
    harness.command(FileCommand::Import { into: None });
    harness.wait_until("the cube is imported", |harness| {
        harness.document().features().len() == before + 1
    });
    harness.settle();
    let body = harness.document().features().last().unwrap().id();
    let name = harness.document().feature(body).unwrap().name.clone();

    harness.click("Close");
    harness.click_button(&format!("Show details of {name}"));
    harness.let_animations_finish();
    let described = harness.shows(crate::import_panel::DESCRIPTION);
    let captioned = harness.shows("Move along X") && harness.shows("Turn about Z");
    harness.type_into_field(Id::new(("import-field", ("offset", 0usize), body)), "15 mm");
    harness.settle();
    let (low, _) = plate_bounds(&harness, body);

    assert!(described);
    assert!(captioned);
    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Place {name}").as_str())
    );
    assert!((low.x - 15.0).abs() < 1e-6, "{low:?}");

    harness.type_into_field(Id::new(("import-field", ("turn", 2usize), body)), "90 deg");
    harness.settle();
    let (low, high) = plate_bounds(&harness, body);

    assert!((low.x - 5.0).abs() < 1e-6, "{low:?}");
    assert!(
        low.y.abs() < 1e-6 && (high.y - 10.0).abs() < 1e-6,
        "{high:?}"
    );

    harness.type_into_field(Id::new(("import-field", ("offset", 1usize), body)), "5 deg");

    assert!(harness.shows_containing("length"));

    std::fs::write(&path, cube_stl(20.0)).unwrap();
    harness.command(FileCommand::ReloadImport(body));
    harness.wait_until("the cube is reloaded", |harness| {
        harness
            .model
            .undo_label()
            .is_some_and(|label| label.starts_with("Replace"))
    });
    harness.settle();
    let (low, high) = plate_bounds(&harness, body);

    assert!(volume_about(&harness, body, 8000.0));
    assert!((low.x + 5.0).abs() < 1e-6, "{low:?}");
    assert!(
        low.y.abs() < 1e-6 && (high.y - 20.0).abs() < 1e-6,
        "{high:?}"
    );

    for _ in 0..3 {
        harness.perform(Action::Undo);
    }
    harness.settle();
    let (low, _) = plate_bounds(&harness, body);

    assert!(volume_about(&harness, body, 1000.0));
    assert!(low.length() < 1e-6, "{low:?}");
}

#[test]
fn an_imported_body_is_replaced_from_a_file_in_place_and_undone_as_one_step() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let small = dir.path().join("cube.stl");
    let large = dir.path().join("cube-v2.stl");
    std::fs::write(&small, cube_stl(10.0)).unwrap();
    std::fs::write(&large, cube_stl(20.0)).unwrap();
    let before = harness.document().features().len();
    harness.answer_dialog(Some(small));
    harness.command(FileCommand::Import { into: None });
    harness.wait_until("the cube is imported", |harness| {
        harness.document().features().len() == before + 1
    });
    harness.settle();
    let body = harness.document().features().last().unwrap().id();
    let name = harness.document().feature(body).unwrap().name.clone();

    assert!(volume_about(&harness, body, 1000.0));

    harness.answer_dialog(Some(large));
    harness.command(FileCommand::ReplaceImport(body));
    harness.wait_until("the cube is replaced", |harness| {
        harness
            .model
            .undo_label()
            .is_some_and(|label| label.starts_with("Replace"))
    });
    harness.settle();

    assert_eq!(harness.document().features().len(), before + 1);
    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Replace {name} from cube-v2.stl").as_str())
    );
    assert!(volume_about(&harness, body, 8000.0));
    assert_eq!(
        harness
            .document()
            .feature(body)
            .unwrap()
            .kind
            .import()
            .unwrap()
            .source,
        "cube-v2.stl"
    );

    harness.perform(Action::Undo);
    harness.settle();

    assert!(volume_about(&harness, body, 1000.0));

    let drawing = dir.path().join("plan.dxf");
    write_drawing(
        &drawing,
        Some(4),
        "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n40\n21\n0\n",
    );
    harness.answer_dialog(Some(drawing));
    harness.command(FileCommand::ReplaceImport(body));
    harness.wait_until("the drawing is refused", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.contains("cannot replace an imported body"))
    });

    assert_eq!(
        harness.model.notice().unwrap().text,
        "“plan.dxf” is not a STEP, STL, OBJ or 3MF file, so it cannot replace an imported body."
    );
}
