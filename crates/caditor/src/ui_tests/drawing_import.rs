use std::path::Path;

use caditor_document::{Edit, Transaction};
use caditor_expression::Expression;
use caditor_file::DrawingUnit;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Entity, Sketch};
use egui::{Key, Modifiers};
use tempfile::TempDir;

use super::{Harness, canonical, sample_document, square_drawing, write_drawing};
use crate::{
    app,
    files::FileCommand,
    import,
    import::Placement,
    import_options::{Arrangement, ImportOptionsCommand, PlaneChoice},
    model::Action,
};

#[test]
fn importing_a_drawing_fills_a_new_sketch_or_the_one_being_edited() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let square = dir.path().join("bracket.dxf");
    write_drawing(
        &square,
        Some(4),
        "0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n\
         10\n0\n20\n0\n10\n30\n20\n0\n10\n30\n20\n30\n10\n0\n20\n30\n",
    );
    let features = harness.document().features().len();
    harness.answer_dialog(Some(square));
    harness.key(Key::I, Modifiers::COMMAND);
    harness.frame();
    harness.confirm_import("bracket.dxf");
    harness.wait_until("the drawing is imported", |harness| {
        harness.document().features().len() == features + 1
    });
    let sketch = harness.document().features().last().unwrap().id();
    assert_eq!(harness.document().feature(sketch).unwrap().name, "bracket");
    assert_eq!(harness.editing(), Some(sketch));
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Imported 4 curves from “bracket.dxf” into bracket."
    );
    assert!(!harness.files.is_blocking());
    harness.settle();
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    let before = harness.sketch(sketch).entities().len();

    let hole = dir.path().join("hole.dxf");
    write_drawing(
        &hole,
        None,
        "0\nCIRCLE\n8\n0\n10\n15\n20\n15\n40\n5\n0\nTEXT\n8\n0\n1\nNote\n",
    );
    harness.answer_dialog(Some(hole));
    harness.command(FileCommand::Import {
        into: harness.editing(),
    });
    harness.confirm_import("hole.dxf");
    harness.wait_until("the report is shown", |harness| {
        harness.shows("Imported “hole.dxf” into bracket")
    });
    assert_eq!(harness.sketch(sketch).entities().len(), before + 2);
    assert_eq!(harness.document().features().len(), features + 1);
    assert!(harness.shows(
        "The drawing does not say which unit it uses, so its numbers were read as millimetres."
    ));
    harness.click_button(crate::widgets::CLOSE_DIALOG);
    assert!(!harness.files.is_blocking());
    harness.perform(Action::Undo);
    assert_eq!(harness.sketch(sketch).entities().len(), before);

    let label = dir.path().join("label.dxf");
    write_drawing(&label, Some(4), "0\nTEXT\n8\n0\n1\nNote\n");
    harness.answer_dialog(Some(label));
    harness.command(FileCommand::Import {
        into: harness.editing(),
    });
    harness.wait_until("the empty import is reported", |harness| {
        harness.shows("Nothing was imported from “label.dxf”")
    });
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Nothing in “label.dxf” could be imported: the drawing has no lines, arcs, circles or \
         splines to import."
    );
    assert_eq!(harness.sketch(sketch).entities().len(), before);
    harness.click_button(crate::widgets::CLOSE_DIALOG);

    let picture = dir.path().join("photo.dxf");
    std::fs::write(&picture, b"\x89PNG\r\n\x1a\n").unwrap();
    harness.answer_dialog(Some(picture));
    harness.command(FileCommand::Import { into: None });
    harness.wait_until("the failure is reported", |harness| {
        harness.model.notice().is_some_and(|notice| {
            notice.text == "Could not import “photo.dxf”: it is not a DXF drawing."
        })
    });
    assert_eq!(harness.document().features().len(), features + 1);

    harness.edit_width("41 mm");
    assert_eq!(
        harness.model.notice().map(|notice| notice.text.as_str()),
        Some("Could not import “photo.dxf”: it is not a DXF drawing.")
    );
}

fn sketch_positions(sketch: &Sketch) -> Vec<Point2> {
    sketch
        .entities()
        .filter_map(|(_, entity)| match entity {
            Entity::Point(position) => Some(*position),
            _ => None,
        })
        .collect()
}

#[test]
fn the_import_options_scale_centre_and_place_a_drawing_before_it_is_added() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let square = dir.path().join("bracket.dxf");
    square_drawing(&square);
    let features = harness.document().features().len();

    harness.answer_dialog(Some(square));
    harness.command(FileCommand::Import { into: None });
    harness.wait_until("the options are shown", |harness| {
        harness.shows("Import “bracket.dxf”")
    });
    assert!(harness.shows("4 curves drawn, 30.000 mm wide and 30.000 mm high."));
    assert!(harness.shows("Sketch plane"));
    assert_eq!(harness.document().features().len(), features);

    for command in [
        ImportOptionsCommand::Unit(DrawingUnit::Centimetres),
        ImportOptionsCommand::Scale(2.0),
        ImportOptionsCommand::Recentre(true),
        ImportOptionsCommand::Plane(PlaneChoice::Xz),
    ] {
        harness.command(FileCommand::ImportOptions(command));
    }
    harness.frame();
    assert!(harness.shows("4 curves drawn, 600.000 mm wide and 600.000 mm high."));
    harness.click("Import");

    harness.wait_until("the drawing is imported", |harness| {
        harness.document().features().len() == features + 1
    });
    let sketch = harness.document().features().last().unwrap().id();
    let imported = harness.sketch(sketch);
    assert_eq!(imported.plane(), Plane::XZ);
    let positions = sketch_positions(imported);
    assert_eq!(positions.len(), 8);
    for position in positions {
        assert!((position.x.abs() - 300.0).abs() < 1e-9, "{position}");
        assert!((position.y.abs() - 300.0).abs() < 1e-9, "{position}");
    }
    assert!(harness.shows("Imported “bracket.dxf” into bracket"));
    assert!(harness.shows_containing("You chose to read the drawing's numbers as centimetres"));
    assert!(harness.shows("You scaled the drawing by 2."));
    harness.click_button(crate::widgets::CLOSE_DIALOG);
    assert!(!harness.files.is_blocking());
}

#[test]
fn cancelling_the_import_options_adds_nothing_and_a_sketch_being_edited_has_no_plane_choice() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let square = dir.path().join("bracket.dxf");
    square_drawing(&square);
    let features = harness.document().features().len();

    harness.answer_dialog(Some(square.clone()));
    harness.command(FileCommand::Import { into: None });
    harness.wait_for_import_options("bracket.dxf");
    assert!(harness.files.is_blocking());
    harness.click("Cancel");
    assert!(!harness.files.is_blocking());
    assert!(!harness.files.is_importing());
    assert_eq!(harness.document().features().len(), features);

    harness.answer_dialog(Some(square.clone()));
    harness.command(FileCommand::Import { into: None });
    harness.confirm_import("bracket.dxf");
    harness.wait_until("the drawing is imported", |harness| {
        harness.document().features().len() == features + 1
    });
    let sketch = harness.document().features().last().unwrap().id();
    assert_eq!(harness.editing(), Some(sketch));

    harness.answer_dialog(Some(square));
    harness.command(FileCommand::Import {
        into: harness.editing(),
    });
    harness.wait_for_import_options("bracket.dxf");
    assert!(!harness.shows("Sketch plane"));
}

#[test]
fn the_import_options_warn_when_the_chosen_layers_hold_more_than_a_sketch() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let plan = dir.path().join("dense.dxf");
    let mut entities: String = (0..=caditor_file::MAX_DRAWING_CURVES)
        .map(|index| format!("0\nLINE\n8\nHatching\n10\n{index}\n20\n0\n11\n{index}\n21\n1\n"))
        .collect();
    entities.push_str("0\nCIRCLE\n8\nOutline\n10\n20\n20\n20\n40\n3\n");
    write_drawing(&plan, Some(4), &entities);
    let warning = crate::import_options::too_many_curves(caditor_file::MAX_DRAWING_CURVES + 2);

    harness.answer_dialog(Some(plan));
    harness.command(FileCommand::Import { into: None });
    harness.wait_for_import_options("dense.dxf");

    assert!(harness.shows(&warning));

    harness.command(FileCommand::ImportOptions(ImportOptionsCommand::Layer {
        layer: 0,
        included: false,
    }));
    harness.frame();

    assert!(!harness.shows(&warning));
    assert!(harness.shows("1 curve drawn, 6.000 mm wide and 6.000 mm high."));
}

#[test]
fn the_import_options_list_the_layers_and_leave_out_the_ones_unticked() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let plan = dir.path().join("plan.dxf");
    write_drawing(
        &plan,
        Some(4),
        "0\nLINE\n8\nWalls\n10\n0\n20\n0\n11\n40\n21\n0\n\
         0\nLINE\n8\nWalls\n10\n0\n20\n5\n11\n40\n21\n5\n\
         0\nCIRCLE\n8\nNotes\n10\n20\n20\n20\n40\n3\n",
    );
    let features = harness.document().features().len();

    harness.answer_dialog(Some(plan));
    harness.command(FileCommand::Import { into: None });
    harness.wait_for_import_options("plan.dxf");
    assert!(harness.shows("Layers"));
    assert!(harness.shows("Walls (2 curves)"));
    assert!(harness.shows("Notes (1 curve)"));
    assert!(harness.shows("3 curves drawn, 40.000 mm wide and 23.000 mm high."));

    harness.command(FileCommand::ImportOptions(ImportOptionsCommand::Layer {
        layer: 1,
        included: false,
    }));
    harness.frame();
    assert!(harness.shows("2 curves drawn, 40.000 mm wide and 5.000 mm high."));

    harness.command(FileCommand::ImportOptions(ImportOptionsCommand::AllLayers(
        false,
    )));
    harness.frame();
    assert!(harness.shows("The drawing is empty."));
    assert!(harness.files.is_blocking());

    harness.command(FileCommand::ImportOptions(ImportOptionsCommand::Layer {
        layer: 0,
        included: true,
    }));
    harness.frame();
    harness.click("Import");
    harness.wait_until("the drawing is imported", |harness| {
        harness.document().features().len() == features + 1
    });
    let sketch = harness.document().features().last().unwrap().id();
    let circles = harness
        .sketch(sketch)
        .entities()
        .filter(|(_, entity)| matches!(entity, Entity::Circle { .. }))
        .count();
    let lines = harness
        .sketch(sketch)
        .entities()
        .filter(|(_, entity)| matches!(entity, Entity::Line { .. }))
        .count();
    assert_eq!((lines, circles), (2, 0));
    assert!(harness.shows("1 curve on a layer you left out was not imported."));
}

#[test]
fn a_drawing_read_while_the_model_changes_is_placed_on_the_changed_model() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let square = dir.path().join("square.dxf");
    write_drawing(
        &square,
        Some(4),
        "0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n\
         10\n0\n20\n0\n10\n30\n20\n0\n10\n30\n20\n30\n10\n0\n20\n30\n",
    );
    let features = harness.document().features().len();
    let width = harness.parameter("width");
    let drawing = caditor_file::read_dxf(&square, &caditor_document::CancelToken::never()).unwrap();
    let plan = import::plan_drawing(
        harness.model.base(),
        &square,
        None,
        drawing,
        Arrangement::default(),
    );

    harness.model.perform(Action::Apply(Transaction::single(
        "Edit width",
        Edit::SetParameterExpression {
            id: width,
            expression: Expression::parse_stored("42 mm").unwrap(),
        },
    )));
    let placement = import::place_drawing(
        &mut harness.model,
        &mut harness.workspace.editing,
        &square,
        Ok(plan),
    );
    let Placement::Stale(drawing, _) = placement else {
        panic!("a drawing planned before the edit was placed on the changed model");
    };
    assert_eq!(harness.document().features().len(), features);

    let plan = import::plan_drawing(
        harness.model.base(),
        &square,
        None,
        drawing,
        Arrangement::default(),
    );
    let placement = import::place_drawing(
        &mut harness.model,
        &mut harness.workspace.editing,
        &square,
        Ok(plan),
    );
    assert!(matches!(placement, Placement::Done(None)));
    harness.frame();

    let sketch = harness.document().features().last().unwrap().id();
    assert_eq!(harness.document().feature(sketch).unwrap().name, "square");
    assert_eq!(harness.sketch(sketch).entities().len(), 12);
    assert_eq!(harness.expression_text("width"), "42 mm");
    assert_eq!(harness.model.undo_label(), Some("Import square.dxf"));
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Imported 4 curves from “square.dxf” into square."
    );

    harness.perform(Action::Undo);
    assert_eq!(harness.document().features().len(), features);
    assert_eq!(harness.expression_text("width"), "42 mm");
    assert_eq!(harness.model.undo_label(), Some("Edit width"));
}

#[test]
fn a_drawing_named_on_the_command_line_is_imported_instead_of_opened() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let square = dir.path().join("square.dxf");
    write_drawing(
        &square,
        Some(4),
        "0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n\
         10\n0\n20\n0\n10\n30\n20\n0\n10\n30\n20\n30\n10\n0\n20\n30\n",
    );
    let features = harness.document().features().len();

    harness.files.start(Some(square), &mut harness.model);

    harness.confirm_import("square.dxf");
    harness.wait_until("the drawing is imported", |harness| {
        harness.document().features().len() == features + 1
    });
    assert_eq!(harness.model.path(), None);
}

fn hovering(paths: &[&Path]) -> Vec<egui::HoveredFile> {
    paths
        .iter()
        .map(|path| egui::HoveredFile {
            path: Some(path.to_path_buf()),
            ..egui::HoveredFile::default()
        })
        .collect()
}

#[test]
fn files_dragged_over_the_window_say_what_dropping_them_does() {
    let mut harness = Harness::new();

    harness.frame();
    assert!(!harness.shows_containing("Drop to"));

    harness.hovered_files = hovering(&[Path::new("/tmp/plate.CADITOR")]);
    harness.frame();
    harness.frame();
    assert!(harness.shows("Drop to open plate.CADITOR"));

    harness.hovered_files = hovering(&[Path::new("/tmp/outline.dxf")]);
    harness.frame();
    harness.frame();
    assert!(harness.shows("Drop to import outline.dxf"));
    assert!(harness.shows("It becomes a new sketch."));

    harness.hovered_files = hovering(&[Path::new("/tmp/a.step"), Path::new("/tmp/b.dxf")]);
    harness.frame();
    harness.frame();
    assert!(harness.shows("Drop to import 2 files"));

    harness.hovered_files = hovering(&[Path::new("/tmp/plate.caditor"), Path::new("/tmp/b.dxf")]);
    harness.frame();
    harness.frame();
    assert!(harness.shows("Not one model"));

    harness.hovered_files = hovering(&[Path::new("/tmp/notes.txt")]);
    harness.frame();
    harness.frame();
    assert!(harness.shows("notes.txt may not be a drawing or a STEP file"));

    harness.hovered_files = vec![egui::HoveredFile::default()];
    harness.frame();
    harness.frame();
    assert!(harness.shows("Drop to open or import"));

    harness.hovered_files = Vec::new();
    harness.frame();
    assert!(!harness.shows_containing("Drop to"));
}

#[test]
fn dropped_drawings_are_imported_one_after_another_and_a_dropped_model_opens() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let mut harness = Harness::with_directories(Some(root.as_path()));
    let square = root.as_path().join("square.dxf");
    write_drawing(
        &square,
        Some(4),
        "0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n\
         10\n0\n20\n0\n10\n30\n20\n0\n10\n30\n20\n30\n10\n0\n20\n30\n",
    );
    let hole = root.as_path().join("hole.dxf");
    write_drawing(&hole, Some(4), "0\nCIRCLE\n8\n0\n10\n15\n20\n15\n40\n5\n");
    let features = harness.document().features().len();

    for paths in [vec![square.clone(), hole], vec![square.clone()]] {
        app::perform(
            vec![Action::File(FileCommand::Drop { paths, into: None })],
            &mut harness.model,
            &mut harness.files,
            &mut harness.workspace,
        );
    }
    assert_eq!(
        harness.model.notice().unwrap().text,
        "An import is already running. Drop the files again once it has finished."
    );
    harness.confirm_import("square.dxf");
    harness.confirm_import("hole.dxf");
    harness.wait_until("both drawings are imported", |harness| {
        harness.document().features().len() == features + 2
    });
    let names: Vec<&str> = harness
        .document()
        .features()
        .skip(features)
        .map(|feature| feature.name.as_str())
        .collect();
    assert_eq!(names, ["square", "hole"]);

    let path = root.as_path().join("plate.CADITOR");
    caditor_file::save(&sample_document().unwrap(), &path, false).unwrap();
    harness.command(FileCommand::Drop {
        paths: vec![path.clone(), square],
        into: None,
    });
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Drop a single model to open it, or drawings and STEP files to import them."
    );
    assert_eq!(harness.model.path(), None);

    harness.command(FileCommand::Drop {
        paths: vec![path.clone()],
        into: None,
    });
    harness.frame();
    assert!(harness.shows("If you continue without saving, your changes will be lost."));
    harness.click("Continue without saving");
    harness.wait_until("the model is open", |harness| {
        harness.model.path() == Some(path.as_path())
    });
}
