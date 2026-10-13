use caditor_document::Document;
use caditor_geometry::Point2;
use egui::{Color32, Event, Key, Modifiers, PointerButton, Rect, Shape, accesskit::Role};

use super::{Harness, Painted, set_high_contrast, viewport_centre};
use crate::{
    app::Workspace,
    canvas::{FEWER_KEYS, MORE_KEYS, more_keys_words},
    drawing::Refusal,
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand},
    scene_palette::Canvas,
    themes::ViewChoice,
    viewport::{EMPTY_MODEL_TITLE, SELECT_PROMPT, START_WITH_SKETCH},
};

const HELD_SNAP: &str = "snap to the grid or nearby geometry";
const NOTE_FRAMES: usize = 60;
const LINGER_FRAMES: usize = 12;

fn noted_in_the_view(harness: &Harness, text: &str) -> bool {
    let view = harness.workspace.viewport.rect().unwrap();
    harness
        .texts
        .iter()
        .any(|(shown, rect)| shown.contains(text) && view.contains_rect(*rect))
}

#[test]
fn a_long_list_of_keys_folds_behind_more_keys_and_the_chip_shows_the_rest() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    harness.draw_on_new_sketch();
    harness.use_tool(Key::G);

    assert!(harness.shows("Click the hexagon's centre"));
    assert!(harness.shows_containing(MORE_KEYS));
    assert!(!harness.shows(HELD_SNAP));
    assert!(harness.accessible_named(Role::Button, MORE_KEYS));

    harness.click_button(MORE_KEYS);
    harness.frame();

    assert!(harness.shows(HELD_SNAP));
    assert!(harness.shows(FEWER_KEYS));
    assert!(harness.accessible_named(Role::Button, FEWER_KEYS));

    harness.click_button(FEWER_KEYS);
    harness.frame();

    assert!(!harness.shows(HELD_SNAP));
}

#[test]
fn a_short_list_of_keys_shows_whole_without_a_chip() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    harness.use_tool(Key::L);

    assert!(harness.shows(HELD_SNAP));
    assert!(!harness.shows_containing(MORE_KEYS));
    assert!(!harness.shows(&more_keys_words(1)));
}

#[test]
fn an_empty_model_says_where_to_start_and_a_start_runs_its_command() {
    let mut harness = Harness::starting(None, Document::default(), Workspace::new());
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();

    assert!(harness.shows(EMPTY_MODEL_TITLE));
    assert!(harness.accessible_named(Role::Button, START_WITH_SKETCH));

    harness.click_button(START_WITH_SKETCH);
    harness.frame();
    harness.frame();

    assert!(harness.workspace.editing.is_choosing_plane());
    assert!(!harness.shows(EMPTY_MODEL_TITLE));
}

#[test]
fn select_in_an_edited_sketch_has_a_short_prompt() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    harness.frame();

    assert!(harness.shows(SELECT_PROMPT));
    assert!(harness.shows("finish the sketch"));

    harness.use_tool(Key::L);

    assert!(!harness.shows(SELECT_PROMPT));
}

#[test]
fn a_refused_click_leaves_a_note_beside_the_pointer_that_fades() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(30.0, 10.0));

    let reason = Refusal::Rectangle.reason();
    let notice = harness.model.notice().map(|notice| notice.text.clone());
    assert!(
        notice.as_deref().is_some_and(|text| text.contains(reason)),
        "{notice:?}"
    );
    assert!(noted_in_the_view(&harness, reason));

    for _ in 0..NOTE_FRAMES {
        harness.frame();
    }

    assert!(!noted_in_the_view(&harness, reason));
}

#[test]
fn orbiting_marks_its_pivot_until_shortly_after_the_drag() {
    let mut harness = Harness::new();
    harness.frame();
    let from = viewport_centre(&harness);
    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: from,
        button: PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    for step in 1..=4 {
        harness.events.push(Event::PointerMoved(
            from + egui::vec2(20.0 * step as f32, 0.0),
        ));
        harness.frame();
    }

    let held = harness.workspace.viewport.orbit_mark();
    assert!(held.is_some_and(|mark| mark.held), "{held:?}");

    harness.events.push(Event::PointerButton {
        pos: from + egui::vec2(80.0, 0.0),
        button: PointerButton::Secondary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    for _ in 0..LINGER_FRAMES {
        harness.frame();
    }

    assert_eq!(harness.workspace.viewport.orbit_mark(), None);
}

fn fills_behind(shape: &Shape, text: Rect, fills: &mut Vec<Color32>) {
    match shape {
        Shape::Rect(rect) if rect.rect.contains_rect(text) && rect.fill != Color32::TRANSPARENT => {
            fills.push(rect.fill);
        }
        Shape::Vec(shapes) => {
            for shape in shapes {
                fills_behind(shape, text, fills);
            }
        }
        _ => {}
    }
}

#[test]
fn the_key_hint_panel_is_opaque_on_every_canvas_so_no_label_shows_through() {
    let mut harness = Harness::new();
    harness.painted = Some(Painted {
        shapes: Vec::new(),
        pixels_per_point: 1.0,
    });
    harness.draw_on_new_sketch();
    harness.use_tool(Key::G);
    harness.expand_keys();

    for high_contrast in [false, true] {
        set_high_contrast(&mut harness, high_contrast);
        for canvas in Canvas::ALL {
            harness.perform(Action::Preferences(PreferencesCommand::Change(
                PreferenceChange::View(ViewChoice::Canvas(canvas)),
            )));
            harness.frame();
            harness.frame();

            let text = harness
                .texts
                .iter()
                .find(|(shown, _)| shown == FEWER_KEYS)
                .map(|(_, rect)| *rect)
                .expect("the fewer keys chip is shown");
            let mut fills = Vec::new();
            for clipped in harness.painted.iter().flat_map(|painted| &painted.shapes) {
                fills_behind(&clipped.shape, text, &mut fills);
            }

            assert!(fills.len() >= 2, "{canvas:?} {high_contrast}: {fills:?}");
            assert!(
                fills.iter().all(|fill| fill.a() == 255),
                "{canvas:?} high contrast {high_contrast}: {fills:?}"
            );
        }
    }
}
