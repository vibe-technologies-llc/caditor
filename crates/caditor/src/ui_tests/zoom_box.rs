use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};
use caditor_render::Viewpoint;
use caditor_sketch::Sketch;
use egui::{Key, Modifiers, Pos2, Rect, vec2};

use super::{
    CAMERA_SETTLE, Harness, drag_screen, edit_free_sketch, extruded_plate, hold_drag, rectangle,
    release_drag, run_from_palette,
};
use crate::{
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand, Transitions},
    viewport::{ZOOM_BOX_PROMPT, ZOOM_BOX_RELEASE_PROMPT},
};

const EDGE_SLACK_POINTS: f32 = 1.0;

fn settle_camera(harness: &mut Harness) {
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.frame();
}

fn view_rect(harness: &Harness) -> Rect {
    harness.workspace.viewport.rect().unwrap()
}

fn same_view(a: Viewpoint, b: Viewpoint) -> bool {
    a.target.distance(b.target) < 1e-9 * a.distance
        && (a.distance - b.distance).abs() < 1e-9 * a.distance
        && a.orientation.angle_between(b.orientation) < 1e-9
}

fn arm(harness: &mut Harness) {
    run_from_palette(harness, "Zoom to box");
    harness.frame();
}

fn under_box(harness: &Harness, from: Pos2, to: Pos2, depth_plane: &Plane) -> [Point3; 3] {
    let view = harness.workspace.viewport.current_view().unwrap();
    let rect = view_rect(harness);
    let pixels_per_point = f64::from(harness.context.pixels_per_point());

    let pixel = |at: Pos2| {
        let local = at - rect.min;
        Vector2::new(f64::from(local.x), f64::from(local.y)) * pixels_per_point
    };

    let middle = (pixel(from) + pixel(to)) * 0.5;
    let ray = view.ray_through(middle).unwrap();
    let depth = view.view_depth(ray.at(ray.intersect_plane(depth_plane).unwrap()));

    [pixel(from), pixel(to), middle].map(|at| view.unproject(at, depth).unwrap())
}

fn assert_spread(harness: &Harness, [first, second, middle]: [Point3; 3]) {
    let rect = view_rect(harness);
    let viewport = &harness.workspace.viewport;

    let centre = viewport.screen_of(middle).unwrap();
    let span = (viewport.screen_of(first).unwrap() - viewport.screen_of(second).unwrap()).abs();

    assert!(
        centre.distance(rect.center()) < EDGE_SLACK_POINTS,
        "{centre:?} is not the view's centre {:?}",
        rect.center()
    );
    assert!(
        span.x <= rect.width() + EDGE_SLACK_POINTS && span.y <= rect.height() + EDGE_SLACK_POINTS,
        "{span:?} leaves the view {rect:?}"
    );
    assert!(
        (span.x - rect.width()).abs() < EDGE_SLACK_POINTS
            || (span.y - rect.height()).abs() < EDGE_SLACK_POINTS,
        "{span:?} does not reach across the view {rect:?}"
    );
}

fn plate_harness(orthographic: bool) -> Harness {
    let mut harness = Harness::new();

    extruded_plate(&mut harness);
    if orthographic {
        harness.key(Key::O, Modifiers::NONE);
        harness.frame();
    }
    settle_camera(&mut harness);
    harness
}

fn top_box(harness: &Harness) -> (Pos2, Pos2) {
    let middle = harness
        .workspace
        .viewport
        .screen_of(Point3::new(20.0, 20.0, 10.0))
        .unwrap();
    (middle - vec2(60.0, 25.0), middle + vec2(30.0, 35.0))
}

fn zooms_onto_the_plate_top(orthographic: bool) {
    let mut harness = plate_harness(orthographic);
    let top = Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap();

    let before = harness.workspace.viewport.destination();
    let (from, to) = top_box(&harness);

    let under = under_box(&harness, from, to, &top);

    arm(&mut harness);

    let armed = harness.shows(ZOOM_BOX_PROMPT);

    drag_screen(&mut harness, from, to);
    settle_camera(&mut harness);

    assert!(armed, "arming says what to do");
    assert!(!harness.shows(ZOOM_BOX_PROMPT));
    assert!(harness.workspace.viewport.selection().is_empty());

    assert_spread(&harness, under);

    assert!(
        harness.workspace.viewport.destination().distance < before.distance,
        "the camera moved in"
    );

    harness.key(Key::ArrowLeft, Modifiers::ALT);
    harness.frame();
    settle_camera(&mut harness);

    assert!(same_view(harness.workspace.viewport.destination(), before));
}

#[test]
fn a_box_dragged_in_perspective_fills_the_view_about_the_face_under_its_middle() {
    zooms_onto_the_plate_top(false);
}

#[test]
fn a_box_dragged_in_orthographic_fills_the_view_about_its_middle() {
    zooms_onto_the_plate_top(true);
}

#[test]
fn with_transitions_off_the_zoom_lands_at_once() {
    let mut harness = plate_harness(false);

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Transitions(Transitions::Off),
    )));
    harness.frame();

    let before = harness.workspace.viewport.viewpoint();
    let (from, to) = top_box(&harness);

    arm(&mut harness);
    drag_screen(&mut harness, from, to);

    let shown = harness.workspace.viewport.viewpoint();

    assert!(!same_view(shown, before));
    assert!(same_view(shown, harness.workspace.viewport.destination()));
}

#[test]
fn a_box_too_small_to_tell_changes_nothing_and_ends_the_zoom() {
    let mut harness = plate_harness(false);

    let before = harness.workspace.viewport.destination();
    let (from, _) = top_box(&harness);

    arm(&mut harness);
    drag_screen(&mut harness, from, from + vec2(2.0, 1.0));
    settle_camera(&mut harness);

    assert!(same_view(harness.workspace.viewport.destination(), before));
    assert!(!harness.shows(ZOOM_BOX_PROMPT));
    assert!(harness.workspace.viewport.selection().is_empty());
}

#[test]
fn escape_before_the_drag_leaves_it_to_box_selection() {
    let mut harness = plate_harness(false);

    let before = harness.workspace.viewport.destination();
    let (from, to) = top_box(&harness);

    arm(&mut harness);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    let disarmed = !harness.shows(ZOOM_BOX_PROMPT);

    drag_screen(&mut harness, to, from);
    settle_camera(&mut harness);

    assert!(disarmed);
    assert!(same_view(harness.workspace.viewport.destination(), before));
    assert!(!harness.workspace.viewport.selection().is_empty());
}

#[test]
fn escape_during_the_drag_drops_the_box_without_zooming() {
    let mut harness = plate_harness(false);

    let before = harness.workspace.viewport.destination();
    let (from, to) = top_box(&harness);

    arm(&mut harness);
    hold_drag(&mut harness, from, to);

    let dragging = harness.shows(ZOOM_BOX_RELEASE_PROMPT);

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    release_drag(&mut harness, to);
    settle_camera(&mut harness);

    assert!(dragging, "the drag says how to finish");
    assert!(same_view(harness.workspace.viewport.destination(), before));
    assert!(!harness.shows(ZOOM_BOX_PROMPT) && !harness.shows(ZOOM_BOX_RELEASE_PROMPT));
    assert!(harness.workspace.viewport.selection().is_empty());
}

#[test]
fn a_box_dragged_over_an_edited_sketch_fills_the_view_with_that_part_of_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);

    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));

    edit_free_sketch(&mut harness, sketch);
    settle_camera(&mut harness);

    let from = harness.on_screen(Point2::new(4.0, 6.0));
    let to = harness.on_screen(Point2::new(18.0, 12.0));

    let under = under_box(&harness, from, to, &Plane::XY);

    arm(&mut harness);
    drag_screen(&mut harness, from, to);
    settle_camera(&mut harness);

    assert_spread(&harness, under);

    assert!(harness.workspace.viewport.selection().is_empty());
    assert!(harness.editing().is_some(), "the sketch stays open");
}
