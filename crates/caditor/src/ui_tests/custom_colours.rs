use caditor_document::{FeatureId, Rgb};
use egui::{Event, Key, Modifiers, Pos2, Rect, accesskit::Role};

use super::{Harness, extruded_plate, hold_drag, release_drag, run_from_palette, scroll_in_view};
use crate::{
    body_appearance::{
        CUSTOM_COLOUR_NAME, DEFAULT_COLOUR, face_swatch_name, recent_swatch_name, selector_id,
    },
    colour_selector::{self, HEX_HINT, HUE_NAME, Hsv, SQUARE_NAME},
    model::Action,
    preferences::Preferences,
};

const COLOUR_UNDO: &str = "Change the colour of Extrude 1";
const FACES_UNDO: &str = "Change the face colours of Extrude 1";

fn open_card(harness: &mut Harness) -> (FeatureId, crate::selection::Pickable) {
    let (plate, top) = extruded_plate(harness);
    harness.select([top]);
    run_from_palette(harness, "body colour and material");
    harness.settle();
    harness
        .events
        .push(Event::PointerMoved(Pos2::new(150.0, 400.0)));
    scroll_in_view(harness, egui::vec2(0.0, 400.0));
    (plate, top)
}

fn open_selector(harness: &mut Harness) -> FeatureId {
    let (plate, _) = open_card(harness);
    harness.select([]);
    harness.click_button(CUSTOM_COLOUR_NAME);
    harness.settle();
    plate
}

fn node_rect(harness: &mut Harness, role: Role, label: &str) -> Rect {
    if harness.accessible.is_empty() {
        harness.context.enable_accesskit();
        harness.frame();
        harness.frame();
    }
    let bounds = harness
        .accessible
        .iter()
        .filter(|(_, node)| node.role() == role && node.label() == Some(label))
        .find_map(|(_, node)| node.bounds())
        .unwrap_or_else(|| panic!("no {role:?} named '{label}'"));
    Rect::from_min_max(
        Pos2::new(bounds.x0 as f32, bounds.y0 as f32),
        Pos2::new(bounds.x1 as f32, bounds.y1 as f32),
    )
}

fn body_colour(harness: &Harness, body: FeatureId) -> Option<Rgb> {
    harness.document().feature(body).unwrap().appearance.colour
}

fn press_repeatedly(harness: &mut Harness, key: Key, times: usize) {
    for _ in 0..times {
        harness.key(key, Modifiers::NONE);
        harness.frame();
    }
}

fn brightest_of_default() -> Rgb {
    Hsv {
        saturation: 1.0,
        value: 1.0,
        ..Hsv::of(DEFAULT_COLOUR)
    }
    .colour()
}

#[test]
fn a_custom_colour_applies_once_when_the_pointer_is_released() {
    let mut harness = Harness::new();
    let plate = open_selector(&mut harness);
    let before = harness.model.undo_label().map(str::to_owned);

    let square = node_rect(&mut harness, Role::Unknown, SQUARE_NAME);
    let corner = square.right_top() + egui::vec2(8.0, -8.0);
    hold_drag(&mut harness, square.center(), corner);
    let during = harness.model.undo_label().map(str::to_owned);
    let unchanged = body_colour(&harness, plate);
    let previewed = harness.shows(&brightest_of_default().hex());
    release_drag(&mut harness, corner);

    assert_eq!(during, before);
    assert_eq!(unchanged, None);
    assert!(previewed);
    assert_eq!(body_colour(&harness, plate), Some(brightest_of_default()));
    assert_eq!(harness.model.undo_label(), Some(COLOUR_UNDO));

    harness.perform(Action::Undo);
    harness.settle();

    assert_eq!(body_colour(&harness, plate), None);
}

#[test]
fn the_hue_strip_turns_the_colour_and_applies_when_released() {
    let mut harness = Harness::new();
    let plate = open_selector(&mut harness);

    let strip = node_rect(&mut harness, Role::Slider, HUE_NAME);
    let start = strip.left_center() + egui::vec2(2.0, 0.0);
    let end = strip.left_center() + egui::vec2(-8.0, 0.0);
    hold_drag(&mut harness, start, end);
    let during = body_colour(&harness, plate);
    release_drag(&mut harness, end);

    let default = Hsv::of(DEFAULT_COLOUR);
    let red = Hsv {
        hue: 0.0,
        ..default
    }
    .colour();

    assert_eq!(during, None);
    assert_eq!(body_colour(&harness, plate), Some(red));
    assert_eq!(harness.model.undo_label(), Some(COLOUR_UNDO));
}

#[test]
fn the_hex_field_follows_the_selector_and_a_typed_colour_applies() {
    let mut harness = Harness::new();
    let plate = open_selector(&mut harness);
    let hex = colour_selector::hex_id(selector_id(plate, false));

    let square = node_rect(&mut harness, Role::Unknown, SQUARE_NAME);
    let corner = square.right_top() + egui::vec2(8.0, -8.0);
    hold_drag(&mut harness, square.center(), corner);
    let follows = harness.shows(&brightest_of_default().hex());
    release_drag(&mut harness, corner);

    assert!(follows);

    harness.type_into_field(hex, "#336699");
    harness.settle();

    assert_eq!(body_colour(&harness, plate), Some(Rgb::new(51, 102, 153)));
    assert_eq!(harness.model.undo_label(), Some(COLOUR_UNDO));

    harness.type_into_field(hex, "nonsense");
    harness.settle();

    assert_eq!(body_colour(&harness, plate), Some(Rgb::new(51, 102, 153)));
    assert!(harness.shows(HEX_HINT));
}

#[test]
fn the_arrow_keys_move_the_square_and_the_strip_and_apply_once_when_done() {
    let mut harness = Harness::new();
    let plate = open_selector(&mut harness);
    let before = harness.model.undo_label().map(str::to_owned);

    press_repeatedly(&mut harness, Key::ArrowLeft, 3);
    press_repeatedly(&mut harness, Key::ArrowDown, 2);
    let during = harness.model.undo_label().map(str::to_owned);
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    let start = Hsv::of(DEFAULT_COLOUR);
    let moved = Hsv {
        saturation: start.saturation - 0.03,
        value: start.value - 0.02,
        ..start
    }
    .colour();

    assert_eq!(during, before);
    assert_eq!(body_colour(&harness, plate), Some(moved));
    assert_eq!(harness.model.undo_label(), Some(COLOUR_UNDO));

    harness.key(Key::Tab, Modifiers::NONE);
    harness.frame();
    harness.frame();
    press_repeatedly(&mut harness, Key::ArrowRight, 10);
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    let turned = Hsv {
        hue: Hsv::of(moved).hue + 10.0,
        ..Hsv::of(moved)
    }
    .colour();

    assert_eq!(body_colour(&harness, plate), Some(turned));

    harness.perform(Action::Undo);
    harness.settle();

    assert_eq!(body_colour(&harness, plate), Some(moved));
}

#[test]
fn the_selector_names_its_colours_for_screen_readers() {
    let mut harness = Harness::new();
    open_selector(&mut harness);
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();

    let described = colour_selector::describe(DEFAULT_COLOUR);

    assert!(harness.accessible_named(Role::Unknown, SQUARE_NAME));
    assert!(harness.accessible_named(Role::Slider, HUE_NAME));
    assert!(harness.accessible_named(Role::Image, &format!("New colour, {described}")));
    assert!(harness.accessible_named(Role::Image, &format!("Current colour, {described}")));
    assert!(harness.unreadable_nodes().is_empty());
}

#[test]
fn custom_colours_join_the_recent_row_kept_in_preferences() {
    let mut harness = Harness::new();
    let plate = open_selector(&mut harness);
    let hex = colour_selector::hex_id(selector_id(plate, false));
    let first = Rgb::new(51, 102, 153);
    let second = Rgb::new(153, 51, 102);

    harness.type_into_field(hex, "#336699");
    harness.type_into_field(hex, "#993366");
    harness.type_into_field(hex, "#4682b4");
    harness.settle();

    assert_eq!(
        harness.workspace.preferences.recent_colours,
        [second, first]
    );
    assert_eq!(harness.model.recent_colours(), [second, first]);
    assert_eq!(
        Preferences::from_settings(harness.workspace.preferences.settings()).recent_colours,
        [second, first]
    );

    harness.click_button(&recent_swatch_name(first));
    harness.settle();

    assert_eq!(body_colour(&harness, plate), Some(first));
    assert_eq!(
        harness.workspace.preferences.recent_colours,
        [first, second]
    );
}

#[test]
fn selected_faces_take_a_custom_colour_from_the_same_swatch() {
    let mut harness = Harness::new();
    let (plate, _) = open_card(&mut harness);
    let hex = colour_selector::hex_id(selector_id(plate, true));
    let purple = Rgb::new(153, 51, 102);

    harness.click_button(&face_swatch_name(CUSTOM_COLOUR_NAME));
    harness.settle();
    harness.type_into_field(hex, "#993366");
    harness.settle();

    let appearance = &harness.document().feature(plate).unwrap().appearance;

    assert_eq!(harness.model.undo_label(), Some(FACES_UNDO));
    assert_eq!(appearance.colour, None);
    assert_eq!(
        appearance
            .faces
            .iter()
            .map(|face| face.colour)
            .collect::<Vec<_>>(),
        [purple]
    );
    assert_eq!(harness.workspace.preferences.recent_colours, [purple]);
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();

    assert!(harness.accessible_named(Role::Button, &face_swatch_name(&recent_swatch_name(purple))));
}
