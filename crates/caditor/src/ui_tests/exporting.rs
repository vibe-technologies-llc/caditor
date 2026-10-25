use std::path::Path;

use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2, Vector2};
use caditor_render::ImageError;
use caditor_sketch::{Constraint, Sketch};
use egui::{Id, Key, Modifiers};
use tempfile::TempDir;

use super::{CAMERA_SETTLE, Harness, add_peg, extruded_plate, rectangle, run_from_palette};
use crate::{
    commands::Command,
    export::ExportCommand,
    files::FileCommand,
    image_export::ImageCommand,
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand},
};

#[test]
fn exporting_writes_the_chosen_bodies_in_the_chosen_format_beside_the_model() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.command(FileCommand::Export(ExportCommand::Show));
    assert!(
        harness
            .shows("There are no bodies to export yet. Extrude or revolve a sketch to make one.")
    );
    assert!(!harness.shows("Close"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert!(!harness.files.is_blocking());

    let (extrude, _) = extruded_plate(&mut harness);
    harness.key(Key::E, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    assert!(harness.shows("Export"));
    harness.click("3MF");
    harness.answer_dialog(Some(dir.path().join("plate")));
    harness.click("Export…");
    harness.wait_until("the 3MF is written", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.starts_with("Exported 1 body to “plate.3mf”"))
    });
    let package = std::fs::read(dir.path().join("plate.3mf")).unwrap();
    assert!(package.starts_with(b"PK\x03\x04"));
    assert!(
        package
            .windows(b"Metadata/thumbnail.png".len())
            .any(|name| name == b"Metadata/thumbnail.png")
    );

    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("STL");
    harness.command(FileCommand::Export(ExportCommand::Include {
        body: extrude,
        included: false,
    }));
    assert!(harness.shows("Choose at least one body to export."));
    harness.command(FileCommand::Export(ExportCommand::Include {
        body: extrude,
        included: true,
    }));
    harness.answer_dialog(Some(dir.path().join("plate.caditor")));
    harness.click("Export…");
    harness.wait_until("the STL is written", |harness| {
        harness.model.notice().is_some_and(|notice| {
            notice
                .text
                .starts_with("Exported 1 body to “plate.caditor.stl”")
        })
    });
    let stl = std::fs::read(dir.path().join("plate.caditor.stl")).unwrap();
    let triangles = u32::from_le_bytes(stl[80..84].try_into().unwrap()) as usize;
    assert_eq!(stl.len(), 84 + 50 * triangles);
    assert!(!dir.path().join("plate.caditor").exists());

    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("STEP");
    assert!(!harness.shows("Resolution"));
    harness.answer_dialog(Some(dir.path().join("plate.stp")));
    harness.click("Export…");
    harness.wait_until("the STEP file is written", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text == "Exported 1 body to “plate.stp”.")
    });
    let step = std::fs::read_to_string(dir.path().join("plate.stp")).unwrap();
    assert!(step.starts_with("ISO-10303-21;"));
    assert!(step.contains("MANIFOLD_SOLID_BREP("));
}

#[test]
fn a_text_stl_is_chosen_in_the_export_dialog_and_keeps_each_body_named() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    extruded_plate(&mut harness);
    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("STEP");
    assert!(!harness.shows("Encoding"));
    harness.click("STL");
    assert!(harness.shows("Encoding"));
    harness.click("Text");
    assert!(harness.shows(
        "Each body a named solid with exact coordinates, about five times larger; some programs \
         read the first solid only."
    ));
    harness.answer_dialog(Some(dir.path().join("plate.stl")));

    harness.click("Export…");
    harness.wait_until("the STL is written", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.starts_with("Exported 1 body to “plate.stl”"))
    });

    let text = std::fs::read_to_string(dir.path().join("plate.stl")).unwrap();
    assert!(text.starts_with("solid "));
    assert!(
        text.trim_end()
            .lines()
            .last()
            .unwrap()
            .starts_with("endsolid ")
    );
}

#[test]
fn cancelling_the_export_file_picker_keeps_the_dialog_and_its_settings() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    extruded_plate(&mut harness);
    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("STEP");
    harness.answer_dialog(None);

    harness.click("Export…");
    harness.settle();

    assert!(harness.shows("Format"));
    assert!(!harness.shows("Resolution"));
    assert!(harness.files.is_blocking());
    assert!(std::fs::read_dir(dir.path()).unwrap().all(|entry| {
        let name = entry.unwrap().file_name();
        name != "plate.step"
    }));
}

#[test]
fn exporting_asks_before_replacing_a_file_the_picker_did_not_name() {
    let dir = TempDir::new().unwrap();
    let existing = dir.path().join("plate.stl");
    std::fs::write(&existing, b"precious").unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    extruded_plate(&mut harness);
    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("STL");
    harness.answer_dialog(Some(dir.path().join("plate")));

    harness.click("Export…");
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “plate.stl”?")
    });
    assert!(harness.shows(&format!(
        "A file named “plate.stl” already exists in “{}”. Replacing it overwrites what it holds.",
        dir.path().display()
    )));
    harness.click("Cancel");
    assert_eq!(std::fs::read(&existing).unwrap(), b"precious");
    assert!(!harness.shows("Replace “plate.stl”?"));
    assert!(harness.shows("Resolution"));

    harness.click("Export…");
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “plate.stl”?")
    });
    harness.click("Replace");
    harness.wait_until("the STL is written", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.starts_with("Exported 1 body to “plate.stl”"))
    });
    assert_ne!(std::fs::read(&existing).unwrap(), b"precious");
    assert!(!harness.shows("Resolution"));
}

#[test]
fn exporting_replaces_a_file_the_picker_named_itself_without_asking_again() {
    let dir = TempDir::new().unwrap();
    let existing = dir.path().join("plate.stl");
    std::fs::write(&existing, b"precious").unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    extruded_plate(&mut harness);
    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("STL");
    harness.answer_dialog(Some(existing.clone()));

    harness.click("Export…");
    harness.wait_until("the STL is written", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.starts_with("Exported 1 body to “plate.stl”"))
    });
    assert!(!harness.shows("Replace “plate.stl”?"));
    assert_ne!(std::fs::read(&existing).unwrap(), b"precious");
}

#[test]
fn exporting_an_image_keeps_its_dialog_when_the_picker_is_cancelled_and_asks_before_replacing() {
    let dir = TempDir::new().unwrap();
    let existing = dir.path().join("plate.png");
    std::fs::write(&existing, b"precious").unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.command(FileCommand::ExportImage(ImageCommand::Show));
    harness.click("Transparent");
    harness.answer_dialog(None);

    harness.click("Export…");
    harness.settle();
    assert!(harness.shows("Export image"));
    assert!(!harness.files.is_exporting_image());

    harness.answer_dialog(Some(dir.path().join("plate")));
    harness.click("Export…");
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “plate.png”?")
    });
    harness.click("Cancel");
    assert_eq!(std::fs::read(&existing).unwrap(), b"precious");
    assert!(harness.shows("Export image"));

    harness.click("Export…");
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “plate.png”?")
    });
    harness.click("Replace");
    harness.wait_until("the image is written", |harness| {
        !harness.files.is_exporting_image() && !harness.shows("Replace “plate.png”?")
    });
    harness.settle();
    assert_ne!(std::fs::read(&existing).unwrap(), b"precious");
    assert!(!harness.shows("Export image"));
}

#[test]
fn a_chosen_sketch_exports_to_a_dxf_of_its_curves_and_replacing_asks_first() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 20.0));
    let id = harness.add_sketch(sketch);
    harness.settle();
    let name = harness.document().feature(id).unwrap().name.clone();
    let availability = |harness: &Harness| {
        harness
            .workspace
            .last_offers
            .iter()
            .find(|offer| offer.command == Command::ExportSketch)
            .map(|offer| offer.availability.clone())
    };
    harness.workspace.panels.selected = None;
    harness.frame();
    assert_eq!(
        availability(&harness),
        Some(Err(
            "Choose a sketch in the feature tree, or edit one, to export it".into()
        ))
    );

    harness.workspace.panels.choose_only(id);
    harness.frame();
    assert_eq!(availability(&harness), Some(Ok(())));
    harness.answer_dialog(Some(dir.path().join("outline")));
    run_from_palette(&mut harness, "export sketch");
    harness.click("Export…");
    let written = dir.path().join("outline.dxf");
    harness.wait_until("the drawing is written", |_| written.exists());
    harness.wait_until("the export is announced", |harness| {
        harness.shows(&format!("Exported 4 objects of “{name}” to “outline.dxf”."))
    });

    let drawing =
        caditor_file::read_dxf(&written, &caditor_document::CancelToken::never()).unwrap();
    assert_eq!(drawing.curve_count(), 4);
    assert!(drawing.notes.is_empty(), "{:?}", drawing.notes);

    let cut = dir.path().join("cut.svg");
    harness.answer_dialog(Some(cut.clone()));
    run_from_palette(&mut harness, "export sketch");
    harness.click("Export…");
    harness.wait_until("the SVG is written", |_| cut.exists());
    let svg = std::fs::read_to_string(&cut).unwrap();
    assert!(svg.starts_with("<svg "));
    assert_eq!(svg.matches("<line ").count(), 4);

    harness.answer_dialog(Some(dir.path().join("outline")));
    std::fs::write(&written, b"precious").unwrap();
    run_from_palette(&mut harness, "export sketch");
    harness.click("Export…");
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “outline.dxf”?")
    });
    harness.click("Cancel");
    harness.settle();
    assert_eq!(std::fs::read(&written).unwrap(), b"precious");
}

#[test]
fn construction_geometry_is_exported_on_its_own_layer_once_kept_from_the_palette() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 20.0));
    let guide = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(40.0, 10.0));
    sketch.set_construction(guide, true).unwrap();
    let id = harness.add_sketch(sketch);
    harness.settle();
    harness.workspace.panels.choose_only(id);
    harness.frame();

    run_from_palette(&mut harness, "keep construction geometry");
    harness.frame();
    let kept = harness.files.keeps_drawing_construction();
    harness.answer_dialog(Some(dir.path().join("guided")));
    run_from_palette(&mut harness, "export sketch");
    harness.click("Export…");
    let written = dir.path().join("guided.dxf");
    harness.wait_until("the drawing is written", |_| written.exists());
    harness.wait_until("the export is announced", |harness| {
        harness.shows_containing("1 construction curve is on the Construction layer.")
    });
    let drawing =
        caditor_file::read_dxf(&written, &caditor_document::CancelToken::never()).unwrap();

    assert!(kept);
    assert_eq!(drawing.curve_count(), 5);
    assert_eq!(drawing.construction.len(), 1);
}

#[test]
fn chosen_sketches_nest_on_one_sheet_with_their_dimensions_and_names() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let mut frame = Sketch::new(Plane::XY);
    rectangle(
        &mut frame,
        Point2::new(-50.0, -50.0),
        Point2::new(-10.0, -30.0),
    );
    let mut washer = Sketch::new(Plane::XY);
    let hole = washer.add_circle(Point2::new(200.0, 0.0), 5.0);
    washer
        .add_constraint(Constraint::Radius {
            entity: hole,
            value: Expression::Measure(5.0, Unit::Millimetre),
        })
        .unwrap();
    let frame = harness.add_sketch(frame);
    let washer = harness.add_sketch(washer);
    harness.settle();
    let names: Vec<String> = [frame, washer]
        .map(|id| harness.document().feature(id).unwrap().name.clone())
        .to_vec();
    harness.workspace.panels.choose_all(&[frame, washer]);
    harness.frame();
    let written = dir.path().join("sheet.dxf");

    harness.answer_dialog(Some(written.clone()));
    run_from_palette(&mut harness, "export sketch");
    let titled = harness.shows("Export sketches");
    harness.click("Nested on a sheet");
    harness.click("Dimensions and names");
    harness.click("Export…");
    harness.wait_until("the drawing is written", |_| written.exists());
    harness.wait_until("the export is announced", |harness| {
        harness.shows("Exported 5 objects of 2 sketches with 1 dimension to “sheet.dxf”.")
    });
    let text = std::fs::read_to_string(&written).unwrap();
    let drawing =
        caditor_file::read_dxf(&written, &caditor_document::CancelToken::never()).unwrap();
    let lowest = drawing
        .curves
        .iter()
        .flat_map(|curve| match curve {
            caditor_file::DrawingCurve::Line { start, end } => vec![*start, *end],
            caditor_file::DrawingCurve::Circle { center, radius } => {
                vec![*center - Vector2::splat(*radius)]
            }
            _ => Vec::new(),
        })
        .fold(Point2::splat(f64::INFINITY), Point2::min);

    assert!(titled);
    assert!(!harness.shows("Export sketches"));
    assert_eq!(drawing.curve_count(), 5);
    assert!(lowest.x >= -1e-9 && lowest.y >= -1e-9, "{lowest:?}");
    assert!(text.contains("\nDIMENSION\n"));
    assert!(text.contains("  1\nR5\n"));
    for name in &names {
        assert!(text.contains(&format!("  1\n{name}\n")), "{name}");
    }
    assert!(
        drawing.notes.iter().any(|note| note.contains("dimension")),
        "{:?}",
        drawing.notes
    );
}

#[test]
fn a_selected_flat_face_exports_to_a_dxf_of_its_outline() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let (_, top) = extruded_plate(&mut harness);
    let availability = |harness: &Harness| {
        harness
            .workspace
            .last_offers
            .iter()
            .find(|offer| offer.command == Command::ExportFace)
            .map(|offer| offer.availability.clone())
    };

    harness.select([]);
    harness.frame();

    assert_eq!(
        availability(&harness),
        Some(Err(
            "Select one or more flat faces of bodies to export their outlines".into()
        ))
    );

    harness.select([top]);
    harness.frame();

    assert_eq!(availability(&harness), Some(Ok(())));

    harness.answer_dialog(Some(dir.path().join("plate")));
    run_from_palette(&mut harness, "export face");
    harness.click("Export…");
    let written = dir.path().join("plate.dxf");
    harness.wait_until("the drawing is written", |_| written.exists());
    harness.wait_until("the export is announced", |harness| {
        harness.shows("Exported 4 curves of “Extrude 1 end face” to “plate.dxf”, in 1 loop.")
    });

    let drawing =
        caditor_file::read_dxf(&written, &caditor_document::CancelToken::never()).unwrap();

    assert_eq!(drawing.curve_count(), 4);
    assert_eq!(drawing.layers, vec!["Outline".to_owned()]);

    let bottom = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            pickable.describe(harness.document(), harness.model.evaluation())
                == "Extrude 1 › Extrude 1 start face"
        })
        .expect("the bottom face is pickable");
    harness.select([top, bottom]);
    harness.frame();
    harness.answer_dialog(Some(dir.path().join("both")));
    run_from_palette(&mut harness, "export face");
    harness.click("Export…");
    let both = dir.path().join("both.dxf");
    harness.wait_until("the drawing of both faces is written", |_| both.exists());
    harness.wait_until("the export of both is announced", |harness| {
        harness.shows("Exported 8 curves of 2 faces to “both.dxf”, in 2 loops, side by side.")
    });
    assert_eq!(
        caditor_file::read_dxf(&both, &caditor_document::CancelToken::never())
            .unwrap()
            .curve_count(),
        8
    );
}

#[test]
fn a_chosen_sketch_and_a_selected_face_nest_into_one_drawing() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let (_, top) = extruded_plate(&mut harness);
    let mut washer = Sketch::new(Plane::XY);
    washer.add_circle(Point2::new(100.0, 0.0), 5.0);
    let washer = harness.add_sketch(washer);
    harness.settle();
    let name = harness.document().feature(washer).unwrap().name.clone();
    harness.workspace.panels.choose_only(washer);
    harness.select([top]);
    harness.frame();
    let written = dir.path().join("mixed.dxf");

    harness.answer_dialog(Some(written.clone()));
    run_from_palette(&mut harness, "export sketch");
    let titled = harness.shows("Export sketches and faces");
    let offered = harness.shows("The 1 flat face selected in the view");
    harness.click("Nested on a sheet");
    harness.click("Export…");
    harness.wait_until("the drawing is written", |_| written.exists());
    harness.wait_until("the export is announced", |harness| {
        harness.shows(&format!(
            "Exported “{name}” and “Extrude 1 end face” to “mixed.dxf”, nested on a sheet: 1 \
             object from the sketches and 4 curves in 1 loop from the faces."
        ))
    });
    let drawing =
        caditor_file::read_dxf(&written, &caditor_document::CancelToken::never()).unwrap();

    assert!(titled);
    assert!(offered);
    assert_eq!(drawing.curve_count(), 5);
    assert!(drawing.layers.contains(&"0".to_owned()));
    assert!(drawing.layers.contains(&"Outline".to_owned()));
}

#[test]
fn export_face_takes_the_sketches_chosen_in_the_tree_too() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let (_, top) = extruded_plate(&mut harness);
    let mut washer = Sketch::new(Plane::XY);
    washer.add_circle(Point2::new(100.0, 0.0), 5.0);
    let washer = harness.add_sketch(washer);
    harness.settle();
    harness.workspace.panels.choose_only(washer);
    harness.select([top]);
    harness.frame();
    let written = dir.path().join("face and sketch.dxf");

    harness.answer_dialog(Some(written.clone()));
    run_from_palette(&mut harness, "export face");
    let titled = harness.shows("Export sketches and faces");
    harness.click("Export…");
    harness.wait_until("the drawing is written", |_| written.exists());
    let drawing =
        caditor_file::read_dxf(&written, &caditor_document::CancelToken::never()).unwrap();

    assert!(titled);
    assert_eq!(drawing.curve_count(), 5);
}

fn png_size(path: &Path) -> (u32, u32, u8) {
    const RGBA: u8 = 6;
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(&bytes[12..16], b"IHDR");
    let side = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    assert_eq!(bytes[25], RGBA);
    (side(16), side(20), bytes[24])
}

#[test]
fn an_exported_image_is_drawn_in_the_palette_of_the_light_view_it_shows() {
    let mut harness = Harness::new();
    let mut free = Sketch::new(Plane::XY);
    free.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 10.0));
    harness.add_sketch(free);
    harness.settle();
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::View(crate::themes::ViewChoice::Canvas(
            crate::scene_palette::Canvas::Light,
        )),
    )));
    harness.frame();
    harness.frame();
    let view = harness.workspace.viewport.view_pixels().unwrap();

    let image =
        harness
            .workspace
            .viewport
            .image(&harness.model, &harness.workspace.editing, view, &[]);
    let mut shown = harness.built().scene;
    shown.grid = None;

    assert_eq!(
        harness.workspace.viewport.canvas(),
        crate::scene_palette::Canvas::Light
    );
    assert_eq!(image.scene, shown);
}

#[test]
fn exporting_an_image_writes_a_png_of_the_view_without_highlights_at_the_chosen_size() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let (_, top) = extruded_plate(&mut harness);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.select([top]);
    let view = harness.workspace.viewport.view_pixels().unwrap();
    let image =
        harness
            .workspace
            .viewport
            .image(&harness.model, &harness.workspace.editing, view, &[]);
    harness.select([]);
    let mut plain = harness.built().scene;
    plain.grid = None;

    assert_eq!(image.scene, plain);
    assert_eq!(image.pixels_per_point, 1.0);
    assert_eq!(
        image.view.size(),
        (harness.workspace.viewport.current_view().unwrap().size()).round()
    );

    harness.key(Key::E, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();
    harness.show_new_windows();
    assert!(harness.shows("Export image"));
    assert!(harness.shows(&format!("View size ({} × {})", view.width, view.height)));
    harness.click("2×");
    assert!(harness.shows(&format!(
        "The image will be {} × {} pixels.",
        view.width * 2,
        view.height * 2
    )));
    harness.answer_dialog(Some(dir.path().join("plate")));
    harness.click("Export…");
    harness.wait_until("the image is written", |harness| {
        !harness.files.is_exporting_image() && harness.model.notice().is_some()
    });
    assert_eq!(
        harness.model.notice().unwrap().text,
        format!(
            "Exported a {} × {} image to “plate.png”.",
            view.width * 2,
            view.height * 2
        )
    );
    assert_eq!(
        png_size(&dir.path().join("plate.png")),
        (view.width * 2, view.height * 2, 8)
    );

    harness.command(FileCommand::ExportImage(ImageCommand::Show));
    assert!(harness.shows(&format!(
        "The image will be {} × {} pixels.",
        view.width * 2,
        view.height * 2
    )));
    harness.click("Custom");
    harness.type_into_field(Id::unique(("image-side", "Width")), "5000");
    harness.type_into_field(Id::unique(("image-side", "Height")), "none");
    assert!(harness.shows("Enter a whole number of pixels from 1 to 8192"));
    harness.type_into_field(Id::unique(("image-side", "Height")), "900");
    harness.click("4×");
    assert!(harness.shows(
        "At 4× the image would be 20000 × 3600 pixels, more than the 8192 pixels a side caditor \
         draws. Choose a smaller size or scale."
    ));
    harness.click("1×");
    harness.click("Transparent");
    assert!(harness.shows("The image will be 5000 × 900 pixels."));
    harness.answer_dialog(Some(dir.path().join("wide.PNG")));
    harness.click("Export…");
    harness.wait_until("the wide image is written", |harness| {
        !harness.files.is_exporting_image() && dir.path().join("wide.PNG").exists()
    });
    assert_eq!(png_size(&dir.path().join("wide.PNG")), (5000, 900, 8));

    harness.image_failure = Some(ImageError::OutOfMemory);
    harness.key(Key::E, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();
    harness.show_new_windows();
    harness.answer_dialog(Some(dir.path().join("huge.png")));
    harness.click("Export…");
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Could not export “huge.png”: the graphics card does not have enough memory for an image \
         this large. Choose a smaller size or scale, or lower the anti-aliasing in Preferences › \
         Graphics."
    );
    assert!(!dir.path().join("huge.png").exists());
    assert!(!harness.files.is_exporting_image());
    assert!(!harness.files.is_blocking());
}

#[test]
fn an_exported_image_leaves_out_the_bodies_unticked_in_its_dialog() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    harness.settle();
    harness.command(FileCommand::ExportImage(ImageCommand::Show));
    harness.frame();

    assert!(!harness.shows("Bodies"));

    harness.command(FileCommand::ExportImage(ImageCommand::Hide));
    let peg = add_peg(&mut harness);
    harness.settle();
    harness.command(FileCommand::ExportImage(ImageCommand::Show));
    harness.frame();
    harness.frame();

    assert!(harness.shows("Bodies"));
    assert!(harness.shows("Peg"));

    let view = harness.workspace.viewport.view_pixels().unwrap();
    let all =
        harness
            .workspace
            .viewport
            .image(&harness.model, &harness.workspace.editing, view, &[]);
    let without_peg =
        harness
            .workspace
            .viewport
            .image(&harness.model, &harness.workspace.editing, view, &[peg]);
    let without_both = harness.workspace.viewport.image(
        &harness.model,
        &harness.workspace.editing,
        view,
        &[plate, peg],
    );

    assert_eq!(all.scene.meshes.len(), 2);
    assert_eq!(without_peg.scene.meshes.len(), 1);
    assert!(without_both.scene.meshes.is_empty());
}
