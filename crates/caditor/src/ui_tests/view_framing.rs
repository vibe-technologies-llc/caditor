use caditor_document::{Edit, Transaction};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, edit_free_sketch, extruded_plate, rectangle, run_from_palette,
    sample_document,
};
use crate::{app::Workspace, commands::Command, model::Action, preferences::Preferences};

#[test]
fn fitting_the_view_frames_the_rows_chosen_in_the_tree_when_nothing_is_selected() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(10.0, 10.0));
    harness.add_sketch(sketch);
    let mut far = Sketch::new(Plane::XY);
    rectangle(
        &mut far,
        Point2::new(500.0, 500.0),
        Point2::new(510.0, 510.0),
    );
    let near = harness.add_sketch(far);
    harness.select([]);
    harness.frame();

    harness.workspace.panels.choose_only(near);
    harness.key(Key::F, Modifiers::NONE);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let target = harness.workspace.viewport.viewpoint().target;

    assert!(
        target.x > 400.0,
        "the view is framed on the chosen sketch: {target:?}"
    );
}

#[test]
fn looking_at_a_flat_face_turns_the_view_to_face_it_head_on() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);

    harness.key(Key::V, Modifiers::ALT);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();

    let toward_eye =
        harness.workspace.viewport.viewpoint().orientation * caditor_geometry::Vector3::Z;
    assert!(
        (toward_eye - caditor_geometry::Vector3::Z).length() < 1e-6,
        "{toward_eye}"
    );
}

#[test]
fn o_switches_the_view_to_orthographic_and_back_and_the_preference_remembers_it() {
    let mut harness = Harness::new();
    harness.settle();
    let view = |harness: &Harness| harness.workspace.viewport.current_view().unwrap();
    let before = view(&harness);

    harness.key(Key::O, Modifiers::NONE);
    harness.settle();
    let switched = view(&harness);
    let remembered = harness.workspace.preferences.navigation.projection;
    let offered = harness
        .workspace
        .last_offers
        .iter()
        .any(|offer| offer.command == Command::ToggleProjection);
    harness.key(Key::O, Modifiers::NONE);
    harness.settle();
    let back = view(&harness);

    assert!(!before.is_orthographic());
    assert!(switched.is_orthographic());
    assert_eq!(remembered, caditor_render::ProjectionMode::Orthographic);
    assert!(
        offered,
        "{:?}",
        harness
            .texts
            .iter()
            .map(|(text, _)| text.clone())
            .filter(|text| text.contains("lick") || text.contains("opy"))
            .collect::<Vec<_>>()
    );
    assert_eq!(switched.viewpoint(), before.viewpoint());
    assert!(
        (switched.units_per_pixel_at(1.0) - before.units_per_pixel_at(before.viewpoint().distance))
            .abs()
            < 1e-9
    );
    assert!(!back.is_orthographic());
    assert_eq!(
        harness.workspace.preferences.navigation.projection,
        caditor_render::ProjectionMode::Perspective
    );
}

#[test]
fn the_automatic_projection_is_a_command_and_turns_orthographic_in_a_standard_view() {
    let mut harness = Harness::new();
    harness.settle();
    let view = |harness: &Harness| harness.workspace.viewport.current_view().unwrap();

    run_from_palette(&mut harness, "perspective that turns orthographic");
    harness.settle();
    let isometric = view(&harness);
    let chosen = harness.workspace.preferences.navigation.projection;

    harness.key(Key::Num2, Modifiers::ALT);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let top = view(&harness);

    harness.key(Key::Num0, Modifiers::ALT);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let back_to_isometric = view(&harness);

    assert_eq!(chosen, caditor_render::ProjectionMode::Automatic);
    assert!(!isometric.is_orthographic());
    assert!(top.is_orthographic());
    assert!(!back_to_isometric.is_orthographic());

    run_from_palette(&mut harness, "perspective that turns orthographic");
    harness.settle();
    assert_eq!(
        harness.workspace.preferences.navigation.projection,
        caditor_render::ProjectionMode::Perspective
    );
}

#[test]
fn an_orthographic_preference_survives_the_first_fit_and_o_switches_back() {
    let mut preferences = Preferences::default();
    preferences.onboarding = crate::onboarding::Onboarding::finished();
    preferences.navigation.projection = caditor_render::ProjectionMode::Orthographic;
    let mut harness = Harness::starting(
        None,
        sample_document().unwrap(),
        Workspace::with_preferences(preferences),
    );
    harness.settle();
    let started = harness.workspace.viewport.current_view().unwrap();

    harness.key(Key::O, Modifiers::NONE);
    harness.settle();

    assert!(started.is_orthographic());
    assert!(
        !harness
            .workspace
            .viewport
            .current_view()
            .unwrap()
            .is_orthographic()
    );
    assert_eq!(
        harness.workspace.preferences.navigation.projection,
        caditor_render::ProjectionMode::Perspective
    );
}

#[test]
fn a_suppressed_sketch_counts_toward_fitting_the_view_no_more() {
    let mut harness = Harness::new();
    harness.settle();
    let mut far = Sketch::new(Plane::XY);
    far.add_line(Point2::new(900.0, 900.0), Point2::new(1000.0, 1000.0));
    let sketch = harness.add_sketch(far);
    harness.settle();
    let reaching = harness.built().fit_all().max().x;

    harness.perform(Action::Apply(Transaction::single(
        "Suppress",
        Edit::SetFeatureSuppressed {
            id: sketch,
            suppressed: true,
        },
    )));
    harness.settle();

    assert!(reaching >= 1000.0, "{reaching}");
    assert!(harness.built().fit_all().max().x < 900.0);
}

#[test]
fn look_at_sketch_turns_the_view_square_on_to_the_edited_sketch_again() {
    let mut harness = Harness::new();
    let mut slanted = Sketch::new(Plane::XZ);
    slanted.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 10.0));
    edit_free_sketch(&mut harness, slanted);
    let facing = *harness
        .workspace
        .viewport
        .current_view()
        .unwrap()
        .viewpoint();
    let orbit = Command::Camera(crate::commands::CameraMove::OrbitLeft).default_shortcuts()[0];
    for _ in 0..3 {
        harness.key(orbit.logical_key, orbit.modifiers);
    }
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let orbited = *harness
        .workspace
        .viewport
        .current_view()
        .unwrap()
        .viewpoint();

    harness.key(Key::V, Modifiers::ALT | Modifiers::SHIFT);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let back = *harness
        .workspace
        .viewport
        .current_view()
        .unwrap()
        .viewpoint();

    let turned = |viewpoint: caditor_render::Viewpoint| {
        viewpoint.orientation.angle_between(facing.orientation)
    };
    assert!(turned(orbited) > 0.1);
    assert!(turned(back) < 1e-6, "{}", turned(back));
}
