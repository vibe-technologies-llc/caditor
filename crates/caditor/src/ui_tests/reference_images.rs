use std::path::Path;

use caditor_document::{FeatureId, FeatureKind, ImageSize, ImageSpot, ReferenceImage};
use caditor_geometry::{Point2, Point3};
use egui::{Id, Key, Modifiers, Pos2};
use tempfile::TempDir;

use super::{FILE_TIMEOUT, Harness, entities_of_kind, run_from_palette};
use crate::{files::FileCommand, image_panel, image_tools, model::Action};

const PICTURE_WIDTH: u32 = 40;
const PICTURE_HEIGHT: u32 = 20;
const DRAWING_WIDTH: u32 = 300;
const DRAWING_HEIGHT: u32 = 200;

pub(super) fn write_drawing_photo(path: &Path) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, DRAWING_WIDTH, DRAWING_HEIGHT);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let pixels: Vec<u8> = (0..DRAWING_WIDTH * DRAWING_HEIGHT)
        .flat_map(|index| {
            let (x, y) = (index % DRAWING_WIDTH, index / DRAWING_WIDTH);
            let line = x % 30 < 2 || y % 30 < 2;
            let part = (60..240).contains(&x) && (50..150).contains(&y);
            match (line, part) {
                (true, _) => [70, 110, 170, 255],
                (false, true) => [215, 205, 180, 255],
                (false, false) => [245, 243, 236, 255],
            }
        })
        .collect();
    writer.write_image_data(&pixels).unwrap();
}

pub(super) fn write_photo(path: &Path) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, PICTURE_WIDTH, PICTURE_HEIGHT);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let pixels: Vec<u8> = (0..PICTURE_WIDTH * PICTURE_HEIGHT)
        .flat_map(|index| {
            let shade = u8::try_from(index % 256).unwrap();
            [shade, 255 - shade, 90, 255]
        })
        .collect();
    writer.write_image_data(&pixels).unwrap();
}

pub(super) fn image_of(harness: &Harness, feature: FeatureId) -> ReferenceImage {
    match &harness.document().feature(feature).unwrap().kind {
        FeatureKind::ReferenceImage(image) => image.as_ref().clone(),
        _ => panic!("not a reference image"),
    }
}

pub(super) fn spot_on_screen(harness: &Harness, feature: FeatureId, spot: ImageSpot) -> Pos2 {
    let placed = image_tools::placed(harness.model.evaluation(), feature).unwrap();
    let plane = placed.plane().unwrap();
    let local = plane.to_local(placed.point_at(spot));
    harness
        .workspace
        .viewport
        .screen_position(plane, local)
        .expect("the spot is in view")
}

pub(super) fn insert(harness: &mut Harness, path: &Path) -> FeatureId {
    harness.select([]);
    harness.answer_dialog(Some(path.to_path_buf()));
    run_from_palette(harness, "Insert reference image");
    harness.wait_until("the picture is inserted", |harness| {
        harness
            .document()
            .features()
            .any(|feature| matches!(feature.kind, FeatureKind::ReferenceImage(_)))
    });
    harness.settle();
    harness
        .document()
        .features()
        .find(|feature| matches!(feature.kind, FeatureKind::ReferenceImage(_)))
        .unwrap()
        .id()
}

#[test]
fn a_picture_is_inserted_calibrated_shown_locked_traced_saved_and_undone() {
    let dir = TempDir::new().unwrap();
    let photo = dir.path().join("photo.png");
    write_photo(&photo);
    let mut harness = Harness::with_directories(Some(dir.path()));

    let image = insert(&mut harness, &photo);

    assert_eq!(harness.model.undo_label(), Some("Insert photo"));
    assert_eq!(harness.workspace.editing.solid(), Some(image));
    let inserted = image_of(&harness, image);
    assert_eq!(inserted.picture.size().width, PICTURE_WIDTH);
    assert!(
        harness
            .model
            .pictures()
            .texture(inserted.picture.digest())
            .is_some()
    );
    let built = harness.built();
    assert_eq!(built.scene.pictures.len(), 1);
    assert!((built.scene.pictures[0].alpha - 0.6).abs() < 1e-6);
    assert!(built.scene.pictures[0].pick.is_some());
    assert!(harness.shows("A PNG picture, 40 by 20 pixels, to trace over"));

    harness.click(image_panel::CALIBRATE);
    harness.frame();
    let from = ImageSpot::new(0.25, 0.5);
    let to = ImageSpot::new(0.75, 0.5);
    let first = spot_on_screen(&harness, image, from);
    let second = spot_on_screen(&harness, image, to);
    harness.click_screen(first);
    harness.frame();
    assert!(harness.shows_containing("Click the second point on the picture"));
    harness.click_screen(second);
    harness.frame();
    harness.settle();
    let ImageSize::Calibrated(calibration) = image_of(&harness, image).size else {
        panic!("the picture should be calibrated");
    };
    assert!((calibration.from.across - from.across).abs() < 0.02);
    assert!((calibration.to.across - to.across).abs() < 0.02);
    assert_eq!(
        harness.document().expression_text(&calibration.distance),
        "50 mm"
    );

    harness.type_into_field(Id::new(("image-field", image, "distance")), "30 mm");
    harness.settle();
    let placed = image_tools::placed(harness.model.evaluation(), image).unwrap();
    let calibrated = image_of(&harness, image);
    let ImageSize::Calibrated(calibration) = &calibrated.size else {
        panic!("still calibrated");
    };
    let apart = placed
        .point_at(calibration.from)
        .distance(placed.point_at(calibration.to));
    assert!((apart - 30.0).abs() < 1e-6, "{apart}");

    harness.click("60%");
    harness.frame();
    harness.click("20%");
    harness.settle();
    assert_eq!(image_of(&harness, image).opacity, 20);
    assert!((harness.built().scene.pictures[0].alpha - 0.2).abs() < 1e-6);

    harness.click(image_panel::BEHIND);
    harness.frame();
    harness.click(image_panel::IN_FRONT);
    harness.settle();
    assert!(image_of(&harness, image).in_front);
    assert!(harness.built().scene.pictures[0].in_front);

    harness.click(image_panel::LOCKED);
    harness.settle();
    assert!(image_of(&harness, image).locked);
    assert!(harness.built().scene.pictures[0].pick.is_none());

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let sketch = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(-5.0, -2.0));
    harness.click_at(Point2::new(8.0, 3.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(entities_of_kind(harness.sketch(sketch), "Line").len(), 1);
    assert_eq!(harness.built().scene.pictures.len(), 1);

    let saved = dir.path().join("traced.caditor");
    harness.answer_dialog(Some(saved.clone()));
    harness.command(FileCommand::SaveAs);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    let reopened = caditor_file::load(&saved).unwrap();
    assert!(reopened.issues.is_empty(), "{:?}", reopened.issues);
    assert_eq!(reopened.document, *harness.model.document());

    while harness.model.undo_label() != Some("Insert photo") {
        harness.perform(Action::Undo);
    }
    harness.settle();
    assert_eq!(image_of(&harness, image).opacity, 60);
    assert!(!image_of(&harness, image).locked);
    harness.perform(Action::Undo);
    harness.settle();
    assert!(harness.document().feature(image).is_none());
    assert!(harness.built().scene.pictures.is_empty());
    harness.perform(Action::Redo);
    harness.settle();
    assert_eq!(harness.built().scene.pictures.len(), 1);

    assert!(harness.files.wait_for_jobs(FILE_TIMEOUT));
    let closing = harness.model.close().unwrap();
    assert!(closing.wait(FILE_TIMEOUT));
    drop(harness);
    let mut fresh = Harness::with_directories(Some(dir.path()));
    fresh.command(FileCommand::OpenPath(saved));
    fresh.wait_until("the saved model opens", |harness| {
        harness
            .document()
            .features()
            .any(|feature| feature.id() == image)
    });
    fresh.settle();
    assert_eq!(image_of(&fresh, image), calibrated_with_lock(&calibrated));
    assert_eq!(fresh.built().scene.pictures.len(), 1);
}

fn calibrated_with_lock(calibrated: &ReferenceImage) -> ReferenceImage {
    ReferenceImage {
        opacity: 20,
        in_front: true,
        locked: true,
        ..calibrated.clone()
    }
}

#[test]
fn a_picture_caditor_cannot_read_is_refused_in_words_and_adds_nothing() {
    let dir = TempDir::new().unwrap();
    let heic = dir.path().join("holiday.heic");
    std::fs::write(&heic, b"\0\0\0\x18ftypheic\0\0\0\0mif1heic").unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let features = harness.document().features().count();
    harness.select([]);
    harness.answer_dialog(Some(heic));

    run_from_palette(&mut harness, "Insert reference image");
    harness.wait_until("the refusal is shown", |harness| {
        harness.model.notice().is_some()
    });

    assert_eq!(
        harness.model.notice().unwrap().text,
        "Could not insert the picture “holiday.heic”: it is an HEIC picture, which caditor \
         cannot read yet; save it as PNG or JPEG."
    );
    assert_eq!(harness.document().features().count(), features);
}

#[test]
fn a_picture_inserted_while_sketching_lies_on_the_sketch_and_drawing_goes_on_over_it() {
    let dir = TempDir::new().unwrap();
    let photo = dir.path().join("plan.png");
    write_photo(&photo);
    let mut harness = Harness::with_directories(Some(dir.path()));
    let sketch = harness.draw_on_new_sketch();

    harness.answer_dialog(Some(photo));
    harness.command(FileCommand::InsertImage(image_tools::ImageTarget::Sketch(
        sketch,
    )));
    harness.wait_until("the picture is inserted", |harness| {
        harness
            .document()
            .features()
            .any(|feature| matches!(feature.kind, FeatureKind::ReferenceImage(_)))
    });
    harness.settle();
    let image = harness
        .document()
        .features()
        .find(|feature| matches!(feature.kind, FeatureKind::ReferenceImage(_)))
        .unwrap()
        .id();

    assert_eq!(harness.editing(), Some(sketch));
    assert_eq!(
        image_of(&harness, image).plane,
        caditor_document::ImagePlane::Sketch(sketch)
    );
    assert!(harness.built().scene.pictures[0].pick.is_none());
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(-10.0, 0.0));
    harness.click_at(Point2::new(10.0, 4.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(entities_of_kind(harness.sketch(sketch), "Line").len(), 1);
}

fn marked_at(harness: &mut Harness, point: Point3) -> bool {
    harness.built().scene.batches.iter().any(|batch| {
        batch
            .markers
            .iter()
            .any(|marker| (marker.position - point).length() < 1e-6 && marker.diameter > 12.0)
    })
}

fn banded_from(harness: &mut Harness, start: Point3) -> Option<Point3> {
    harness
        .built()
        .scene
        .batches
        .iter()
        .flat_map(|batch| batch.lines.iter())
        .find(|line| (line.start - start).length() < 1e-6 && line.color == calibration_colour())
        .map(|line| line.end)
}

fn calibration_colour() -> caditor_render::Color {
    crate::scene_palette::Contrast::Standard
        .palette(crate::scene_palette::Canvas::default())
        .measured
}

#[test]
fn calibrating_marks_the_first_point_and_a_band_with_its_length_follows_the_pointer_until_the_second()
 {
    let dir = TempDir::new().unwrap();
    let photo = dir.path().join("photo.png");
    write_photo(&photo);
    let mut harness = Harness::with_directories(Some(dir.path()));
    let image = insert(&mut harness, &photo);
    let placed = image_tools::placed(harness.model.evaluation(), image).unwrap();
    let from = ImageSpot::new(0.25, 0.5);
    let to = ImageSpot::new(0.75, 0.5);
    let start = placed.point_at(from);

    harness.click(image_panel::CALIBRATE);
    harness.frame();
    assert!(harness.workspace.viewport.calibration().is_none());
    assert!(!marked_at(&mut harness, start));
    assert!(banded_from(&mut harness, start).is_none());

    harness.click_screen(spot_on_screen(&harness, image, from));
    harness.frame();
    let guide = harness.workspace.viewport.calibration().cloned().unwrap();
    assert!((guide.from - start).length() < 1.0);
    let start = guide.from;
    assert!(marked_at(&mut harness, start));

    let ahead = spot_on_screen(&harness, image, to);
    harness.events.push(egui::Event::PointerMoved(ahead));
    harness.frame();
    harness.frame();
    let guide = harness.workspace.viewport.calibration().cloned().unwrap();
    let end = guide.to.unwrap();
    let label = guide.label.clone().unwrap();
    assert!((end - placed.point_at(to)).length() < 1.0, "{end:?}");
    assert_eq!(
        label,
        harness
            .model
            .units()
            .length
            .measured_length((end - start).length())
    );
    assert!((end - start).length() > 49.0);
    assert!(harness.shows(&label));
    let banded = banded_from(&mut harness, start).unwrap();
    assert!((banded - end).length() < 1e-6);

    let beside = harness.workspace.viewport.rect().unwrap().min + egui::vec2(4.0, 4.0);
    harness.events.push(egui::Event::PointerMoved(beside));
    harness.frame();
    harness.frame();
    let guide = harness.workspace.viewport.calibration().cloned().unwrap();
    assert!(guide.to.is_none() && guide.label.is_none());
    assert!(banded_from(&mut harness, start).is_none());
    assert!(marked_at(&mut harness, start));

    harness.click_screen(ahead);
    harness.frame();
    harness.settle();
    assert!(harness.workspace.viewport.calibration().is_none());
    assert!(!marked_at(&mut harness, start));
    assert!(image_of(&harness, image).size.calibration().is_some());
}

#[test]
fn the_thumbnail_of_an_exported_model_holds_its_pictures() {
    let dir = TempDir::new().unwrap();
    let photo = dir.path().join("photo.png");
    write_photo(&photo);
    let mut harness = Harness::with_directories(Some(dir.path()));
    let image = insert(&mut harness, &photo);
    let size = caditor_render::SurfaceSize {
        width: 256,
        height: 160,
    };

    let thumbnail = harness
        .workspace
        .viewport
        .thumbnail(&harness.model, &[], size);

    assert_eq!(thumbnail.scene.pictures.len(), 1);
    let drawn = &thumbnail.scene.pictures[0];
    let placed = image_tools::placed(harness.model.evaluation(), image).unwrap();
    assert_eq!(drawn.corners, placed.corners());
    assert!((drawn.alpha - 0.6).abs() < 1e-6);
}
