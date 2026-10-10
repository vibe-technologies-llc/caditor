use caditor_document::FeatureId;
use caditor_geometry::Point2;
use egui::{Key, Modifiers};

use super::{
    Harness, add_block, extruded_plate, is_edge, is_face, run_from_palette, selected_of,
    top_edge_along_x,
};
use crate::{
    body_selection,
    commands::Command,
    repeating::NOTHING_TO_REPEAT,
    selection::{Pickable, SelectionFilter},
};

fn boxes(harness: &Harness) -> usize {
    harness
        .document()
        .features()
        .filter(|feature| feature.kind.primitive().is_some())
        .count()
}

fn plate_faces(harness: &Harness, plate: FeatureId) -> usize {
    harness
        .model
        .evaluation()
        .body(plate)
        .map_or(0, |solid| solid.faces().count())
}

#[test]
fn repeating_the_last_command_runs_it_again_on_the_new_selection() {
    let mut harness = Harness::new();
    harness.select([]);
    run_from_palette(&mut harness, "Box");
    harness.settle();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();
    let offered = harness
        .workspace
        .last_offers
        .iter()
        .find(|offer| offer.command == Command::RepeatLast)
        .cloned()
        .expect("the repeat command is offered");

    harness.key(Key::F4, Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness.settle();

    assert_eq!(boxes(&harness), 2);
    assert!(offered.availability.is_ok());
    assert!(
        offered.title().starts_with("Repeat "),
        "{}",
        offered.title()
    );
    assert!(harness.workspace.editing.solid().is_some());
}

#[test]
fn repeating_with_nothing_run_says_so() {
    let mut harness = Harness::new();

    harness.key(Key::F4, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.shows_containing(NOTHING_TO_REPEAT));
    assert_eq!(boxes(&harness), 0);
}

#[test]
fn repeat_is_reached_from_the_palette_too() {
    let mut harness = Harness::new();
    harness.select([]);
    run_from_palette(&mut harness, "Box");
    harness.settle();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();

    run_from_palette(&mut harness, "Repeat");
    harness.frame();
    harness.settle();

    assert_eq!(boxes(&harness), 2);
}

#[test]
fn inverting_the_selection_swaps_the_selected_faces_for_the_others() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let faces = plate_faces(&harness, plate);
    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::Faces);
    harness.select([top]);

    harness.key(Key::I, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();
    let inverted = selected_of(&harness, is_face);
    let kept_top = harness.workspace.viewport.selection().contains(top);
    harness.key(Key::I, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();

    assert_eq!(inverted, faces - 1);
    assert!(!kept_top);
    assert_eq!(selected_of(&harness, is_face), 1);
    assert!(harness.workspace.viewport.selection().contains(top));
}

#[test]
fn inverting_with_nothing_of_a_kind_to_go_by_asks_for_one() {
    let mut harness = Harness::new();
    let (_, _) = extruded_plate(&mut harness);
    harness.select([]);

    run_from_palette(&mut harness, "Invert the selection");
    harness.frame();

    assert!(harness.shows_containing(body_selection::NO_KIND_TO_SELECT));
}

#[test]
fn the_faces_of_one_feature_are_selected_together_and_not_those_of_another_body() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let faces = plate_faces(&harness, plate);
    let block = add_block(
        &mut harness,
        "Block",
        [Point2::new(60.0, 0.0), Point2::new(80.0, 20.0)],
        "10 mm",
    );
    harness.select([top]);

    harness.key(Key::F, Modifiers::ALT | Modifiers::SHIFT);
    harness.frame();

    let others = harness
        .workspace
        .viewport
        .selection()
        .iter()
        .filter(|pickable| matches!(pickable, Pickable::Face { body, .. } if *body == block))
        .count();
    assert_eq!(selected_of(&harness, is_face), faces);
    assert_eq!(others, 0);
}

#[test]
fn the_loop_of_an_edge_needs_a_face_it_bounds() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let edge = Pickable::Edge {
        body: plate,
        edge: top_edge_along_x(&harness, plate, 0.0),
    };
    harness.select([edge]);

    run_from_palette(&mut harness, "Select the loop of the selected edge");
    harness.frame();

    assert!(harness.shows_containing(body_selection::NO_LOOP_PAIR_SELECTED));
    assert_eq!(selected_of(&harness, is_edge), 1);
}

#[test]
fn the_loop_of_an_edge_is_the_loop_of_the_selected_face_holding_it() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let edge = Pickable::Edge {
        body: plate,
        edge: top_edge_along_x(&harness, plate, 0.0),
    };
    harness.select([edge, top]);

    harness.key(Key::G, Modifiers::ALT | Modifiers::SHIFT);
    harness.frame();

    assert_eq!(selected_of(&harness, is_edge), 4);
    assert_eq!(selected_of(&harness, is_face), 0);
    assert!(harness.workspace.viewport.selection().contains(edge));
}
