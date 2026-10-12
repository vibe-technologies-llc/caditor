use caditor_document::FeatureId;
use caditor_geometry::Vector3;
use egui::{
    Event, Key, Modifiers, Rect,
    accesskit::{NodeId, Role},
};

use super::{CAMERA_SETTLE, Harness, with_type_hint};
use crate::{
    canvas,
    files::FileCommand,
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand},
    view_cube,
};

#[test]
fn the_canvas_says_how_far_apart_the_grid_lines_are_and_names_it_for_screen_readers() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();

    let shown: Vec<&str> = harness
        .texts
        .iter()
        .map(|(text, _)| text.as_str())
        .filter(|text| text.starts_with("Grid "))
        .collect();
    let [label] = shown.as_slice() else {
        panic!("the grid spacing should show once: {shown:?}");
    };

    assert!(label.ends_with(" mm"), "{label}");
    assert!(
        harness
            .accessible
            .iter()
            .any(|(_, node)| { node.role() == Role::Label && node.value() == Some(label) })
    );
}

fn view_cube_node(harness: &Harness) -> Option<(NodeId, String)> {
    harness
        .accessible
        .iter()
        .find(|(_, node)| {
            node.role() == Role::Button
                && node
                    .label()
                    .is_some_and(|label| label.starts_with(view_cube::NAME))
        })
        .and_then(|(id, node)| Some((*id, node.label()?.to_owned())))
}

#[test]
fn the_view_cube_names_its_view_and_steps_to_a_neighbour_from_the_keyboard() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.let_animations_finish();

    let (cube, name) = view_cube_node(&harness).expect("the view cube is named");
    assert_eq!(name, "View cube: View from top, front and right");
    assert!(harness.accessible_named(Role::Button, "Fit all"));
    assert!(harness.shows("Fit all"));

    harness.events.push(Event::AccessKitActionRequest(
        egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Focus,
            target_node: cube,
            target_tree: egui::accesskit::TreeId::ROOT,
            data: None,
        },
    ));
    harness.frame();
    harness.frame();
    harness.key(Key::ArrowRight, Modifiers::NONE);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.let_animations_finish();

    let looking_from = harness.workspace.viewport.viewpoint().orientation * Vector3::Z;
    let expected = Vector3::new(1.0, 0.0, 1.0).normalize();
    assert!(looking_from.distance(expected) < 1e-6, "{looking_from}");
    let (_, name) = view_cube_node(&harness).expect("the view cube is named");
    assert_eq!(name, "View cube: View from top and right");
}

fn bracket_profile(harness: &mut Harness) -> FeatureId {
    let session = harness.model.session();
    harness.command(FileCommand::OpenSample(crate::samples::Sample::Bracket));
    if harness.shows("Continue Without Saving") {
        harness.click("Continue Without Saving");
    }
    harness.wait_until("the sample opens", |harness| {
        harness.model.session() != session
    });
    harness.settle();
    let profile = harness
        .document()
        .features()
        .find(|feature| feature.name == "Bracket profile")
        .map(|feature| feature.id())
        .expect("the bracket has its profile sketch");
    harness.edit(profile);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.let_animations_finish();
    profile
}

#[test]
fn constraint_glyphs_keep_clear_of_dimension_labels_in_the_bracket_profile() {
    let mut harness = Harness::new();
    bracket_profile(&mut harness);
    let view = harness.workspace.viewport.rect().unwrap();
    let in_view = |rect: &Rect| view.contains_rect(*rect);

    let labels: Vec<Rect> = harness
        .texts
        .iter()
        .filter(|(text, rect)| text.contains(" = ") && in_view(rect))
        .map(|(_, rect)| rect.expand2(canvas::PADDING))
        .collect();
    let glyphs: Vec<Rect> = harness
        .texts
        .iter()
        .filter(|(text, rect)| (text == "H" || text == "V") && in_view(rect))
        .map(|(_, rect)| Rect::from_center_size(rect.center(), egui::Vec2::splat(15.0)))
        .collect();

    assert_eq!(labels.len(), 4);
    assert_eq!(glyphs.len(), 6);
    for glyph in &glyphs {
        for label in &labels {
            assert!(!glyph.intersects(*label), "{glyph:?} overlaps {label:?}");
        }
    }
}

#[test]
fn the_prompt_and_key_hints_wrap_clear_of_the_view_cube_in_a_small_view() {
    let mut harness = Harness::new();
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(1.5),
    )));
    harness.frame();
    harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    harness.frame();

    let view = harness.workspace.viewport.rect().unwrap();
    let cube = view_cube::area(view);
    let prompt = "Click the rectangle's first corner";
    let keys = with_type_hint(
        "Rectangle from two corners   R: from its centre",
        "Esc: back to Select",
    );
    assert!(harness.shows(prompt));
    assert!(harness.shows_hint(&keys));
    let hinted: Vec<&str> = std::iter::once(prompt)
        .chain(canvas::hint_texts(&keys))
        .collect();
    let rows: Vec<f32> = harness
        .texts
        .iter()
        .filter(|(text, _)| hinted.contains(&text.as_str()))
        .map(|(text, rect)| {
            assert!(!rect.intersects(cube), "'{text}' runs under the view cube");
            assert!(
                view.contains_rect(*rect),
                "'{text}' leaves the view {view:?} at {rect:?}"
            );
            rect.top()
        })
        .collect();
    let first = rows.iter().copied().fold(f32::INFINITY, f32::min);
    assert!(rows.iter().any(|top| *top > first + 30.0), "{rows:?}");
}
