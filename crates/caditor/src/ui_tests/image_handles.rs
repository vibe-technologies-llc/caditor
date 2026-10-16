use caditor_document::{FeatureId, ImageSize, ImageSpot, PlacedImage};
use caditor_expression::Expression;
use caditor_geometry::{Point3, Vector3};
use egui::{Event, Id, Key, Modifiers, Pos2};
use tempfile::TempDir;

use super::{
    CAMERA_SETTLE, Harness, drag_screen,
    reference_images::{image_of, insert, write_photo},
    run_from_palette,
};
use crate::{
    image_handles::ImageGrip,
    image_panel, image_tools,
    model::Action,
    move_manipulator::Handle,
    scene_palette::{Canvas, Contrast},
};

const PHOTO_WIDTH: f64 = 100.0;
const PHOTO_HEIGHT: f64 = 50.0;
const EXACT: f64 = 1e-6;
const ZOOMS_OUT: usize = 3;

fn open_photo(dir: &TempDir) -> (Harness, FeatureId) {
    let photo = dir.path().join("photo.png");
    write_photo(&photo);
    let mut harness = Harness::with_directories(Some(dir.path()));
    let image = insert(&mut harness, &photo);
    run_from_palette(&mut harness, "Fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    for _ in 0..ZOOMS_OUT {
        harness.key(Key::PageDown, Modifiers::NONE);
        harness.workspace.viewport.advance(CAMERA_SETTLE);
        harness.frame();
    }
    (harness, image)
}

fn placed(harness: &Harness, image: FeatureId) -> PlacedImage {
    image_tools::placed(harness.model.evaluation(), image).unwrap()
}

fn grip(harness: &Harness, grip: ImageGrip) -> Pos2 {
    let handle = Handle::Image(grip);
    harness
        .workspace
        .viewport
        .handle_position(handle, 0.0)
        .unwrap_or_else(|| panic!("{handle:?} is shown"))
}

fn shown(harness: &Harness, grip: ImageGrip) -> bool {
    harness
        .workspace
        .viewport
        .handle_position(Handle::Image(grip), 0.0)
        .is_some()
}

fn screen_of(harness: &Harness, point: Point3) -> Pos2 {
    harness
        .workspace
        .viewport
        .screen_of(point)
        .unwrap_or_else(|| panic!("{point:?} is in view"))
}

fn drag_handle(harness: &mut Harness, handle: ImageGrip, target: Point3) -> f64 {
    let from = grip(harness, handle);
    let to = screen_of(harness, target);
    let step = harness.workspace.viewport.manipulator_step().unwrap();
    assert!(
        harness.workspace.viewport.rect().unwrap().contains(to),
        "{to:?} lies outside the view"
    );
    drag_screen(harness, from, to);
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    step
}

fn hover(harness: &mut Harness, handle: ImageGrip) {
    let at = grip(harness, handle);
    harness.events.push(Event::PointerMoved(at));
    harness.frame();
    harness.frame();
}

fn undo_steps_to_insert(harness: &mut Harness) -> usize {
    let mut steps = 0;
    while harness.model.undo_label() != Some("Insert photo") {
        harness.perform(Action::Undo);
        harness.settle();
        steps += 1;
    }
    steps
}

#[test]
fn dragging_the_middle_square_moves_the_picture_on_its_plane_as_one_change() {
    let dir = TempDir::new().unwrap();
    let (mut harness, image) = open_photo(&dir);
    let start = placed(&harness, image).centre;
    let target = start + Vector3::new(20.0, 10.0, 0.0);

    let step = drag_handle(&mut harness, ImageGrip::Move, target);

    let moved = placed(&harness, image);
    assert!(
        (moved.centre - target).length() <= 2.0 * step,
        "{:?} for {target:?} with steps of {step}",
        moved.centre
    );
    assert_eq!(moved.width, PHOTO_WIDTH);
    assert_eq!(harness.model.undo_label(), Some("Edit photo"));
    assert_eq!(undo_steps_to_insert(&mut harness), 1);
    assert!((placed(&harness, image).centre - start).length() < EXACT);
}

#[test]
fn dragging_the_knob_on_its_stem_turns_the_picture_about_its_middle_in_steps() {
    let dir = TempDir::new().unwrap();
    let (mut harness, image) = open_photo(&dir);
    let centre = placed(&harness, image).centre;

    drag_handle(
        &mut harness,
        ImageGrip::Turn,
        centre + Vector3::new(-60.0, 0.0, 0.0),
    );

    let turned = placed(&harness, image);
    assert!((turned.across - Vector3::Y).length() < EXACT, "{turned:?}");
    assert!((turned.centre - centre).length() < EXACT);
    assert_eq!(harness.model.undo_label(), Some("Edit photo"));
    assert_eq!(undo_steps_to_insert(&mut harness), 1);
    assert!((placed(&harness, image).across - Vector3::X).length() < EXACT);
}

#[test]
fn dragging_the_corner_square_sizes_the_picture_about_its_middle_keeping_its_shape() {
    let dir = TempDir::new().unwrap();
    let (mut harness, image) = open_photo(&dir);
    let before = placed(&harness, image);
    let corner = before.centre + Vector3::new(PHOTO_WIDTH / 2.0, PHOTO_HEIGHT / 2.0, 0.0);

    let step = drag_handle(
        &mut harness,
        ImageGrip::Scale,
        before.centre + (corner - before.centre) * 2.0,
    );

    let sized = placed(&harness, image);
    assert!(
        (sized.width - 2.0 * PHOTO_WIDTH).abs() <= 2.0 * step,
        "{} with steps of {step}",
        sized.width
    );
    assert!((sized.height / sized.width - PHOTO_HEIGHT / PHOTO_WIDTH).abs() < EXACT);
    assert!((sized.centre - before.centre).length() < EXACT);
    assert!(matches!(
        image_of(&harness, image).size,
        ImageSize::Width(_)
    ));
    assert_eq!(harness.model.undo_label(), Some("Edit photo"));
    assert_eq!(undo_steps_to_insert(&mut harness), 1);
    assert!((placed(&harness, image).width - PHOTO_WIDTH).abs() < EXACT);
}

#[test]
fn sizing_a_calibrated_picture_scales_its_distance_and_keeps_its_two_points() {
    let dir = TempDir::new().unwrap();
    let (mut harness, image) = open_photo(&dir);
    let spots = (ImageSpot::new(0.25, 0.5), ImageSpot::new(0.75, 0.5));
    let calibrated = image_tools::calibrated(&harness.model, image, spots).unwrap();
    harness.perform(Action::Apply(calibrated));
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let before = placed(&harness, image);
    let corner = before.centre + Vector3::new(PHOTO_WIDTH / 2.0, PHOTO_HEIGHT / 2.0, 0.0);

    let step = drag_handle(
        &mut harness,
        ImageGrip::Scale,
        before.centre + (corner - before.centre) * 0.5,
    );

    let sized = placed(&harness, image);
    let ImageSize::Calibrated(calibration) = image_of(&harness, image).size else {
        panic!("the picture stays calibrated");
    };
    let apart = sized
        .point_at(calibration.from)
        .distance(sized.point_at(calibration.to));
    assert!(
        (sized.width - PHOTO_WIDTH / 2.0).abs() <= 2.0 * step,
        "{} with steps of {step}",
        sized.width
    );
    assert!((apart - sized.width / 2.0).abs() < EXACT, "{apart}");
    assert_eq!(calibration.from, spots.0);
    assert_eq!(calibration.to, spots.1);
}

#[test]
fn a_value_typed_over_a_handle_sets_the_field_it_stands_for() {
    let dir = TempDir::new().unwrap();
    let (mut harness, image) = open_photo(&dir);

    hover(&mut harness, ImageGrip::Turn);
    harness.type_text("45");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();

    let (sine, cosine) = 45.0_f64.to_radians().sin_cos();
    let turned = placed(&harness, image);
    assert!((turned.across - Vector3::new(cosine, sine, 0.0)).length() < EXACT);
    assert_eq!(harness.model.undo_label(), Some("Edit photo"));

    hover(&mut harness, ImageGrip::Move);
    harness.type_text("12, -8");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    let moved = placed(&harness, image).centre;
    assert!((moved - Point3::new(12.0, -8.0, 0.0)).length() < EXACT);

    hover(&mut harness, ImageGrip::Scale);
    harness.type_text("160");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    assert!((placed(&harness, image).width - 160.0).abs() < EXACT);
    assert_eq!(undo_steps_to_insert(&mut harness), 3);
}

#[test]
fn a_value_that_follows_a_parameter_cannot_be_dragged_and_the_hover_says_what_drives_it() {
    let dir = TempDir::new().unwrap();
    let (mut harness, image) = open_photo(&dir);
    let document = harness.document();
    let mut transaction = document.transaction("Add w");
    transaction.add_parameter("w", Expression::parse("80 mm", &|_| None).unwrap());
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    harness.type_into_field(Id::unique(("image-field", image, "width")), "w");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let before = placed(&harness, image);
    assert!((before.width - 80.0).abs() < EXACT);

    hover(&mut harness, ImageGrip::Scale);
    assert!(harness.shows_containing("Width follows w"));
    let corner = before.centre + Vector3::new(40.0, 20.0, 0.0);
    drag_handle(
        &mut harness,
        ImageGrip::Scale,
        before.centre + (corner - before.centre) * 2.0,
    );

    assert!((placed(&harness, image).width - 80.0).abs() < EXACT);
    assert_eq!(harness.model.undo_label(), Some("Edit photo"));
}

#[test]
fn the_handles_stand_only_while_the_picture_s_panel_is_open_and_not_while_choosing_in_it() {
    let dir = TempDir::new().unwrap();
    let (mut harness, _) = open_photo(&dir);
    let all = [ImageGrip::Move, ImageGrip::Turn, ImageGrip::Scale];

    assert!(all.into_iter().all(|handle| shown(&harness, handle)));

    harness.click(image_panel::CALIBRATE);
    harness.frame();
    assert!(all.into_iter().all(|handle| !shown(&harness, handle)));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(all.into_iter().all(|handle| shown(&harness, handle)));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(all.into_iter().all(|handle| !shown(&harness, handle)));
}

fn knob_colour(harness: &mut Harness) -> caditor_render::Color {
    let knob = harness
        .workspace
        .viewport
        .handle_grip(Handle::Image(ImageGrip::Turn), 0.0)
        .unwrap();
    harness
        .built()
        .scene
        .batches
        .iter()
        .flat_map(|batch| batch.markers.iter())
        .find(|marker| (marker.position - knob).length() < EXACT)
        .expect("the knob is drawn")
        .color
}

#[test]
fn the_handles_are_drawn_in_the_view_and_the_one_under_the_pointer_lights() {
    let dir = TempDir::new().unwrap();
    let (mut harness, _) = open_photo(&dir);
    let palette = Contrast::Standard.palette(Canvas::default());

    assert_eq!(knob_colour(&mut harness), palette.handle);
    assert!(
        harness
            .built()
            .scene
            .batches
            .iter()
            .flat_map(|batch| batch.fills.iter())
            .filter(|fill| fill.color.with_alpha(1.0) == palette.handle.with_alpha(1.0))
            .count()
            >= 2
    );

    hover(&mut harness, ImageGrip::Turn);

    assert_eq!(knob_colour(&mut harness), palette.handle_highlighted);
    assert!(harness.shows_containing("Drag to turn the picture"));
}
