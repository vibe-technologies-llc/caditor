use caditor_document::{FeatureId, FeatureState, Pieces, RegionChoice};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Key, Modifiers};

use super::{Harness, choose, open_solid, rectangle, run_from_palette};
use crate::model::Action;

fn three_rectangles(harness: &mut Harness) -> FeatureId {
    let mut sketch = Sketch::new(Plane::XY);
    for x in [0.0, 20.0, 40.0] {
        rectangle(
            &mut sketch,
            Point2::new(x, 0.0),
            Point2::new(x + 10.0, 10.0),
        );
    }
    harness.add_sketch(sketch)
}

fn extruded_rectangles(harness: &mut Harness) -> FeatureId {
    three_rectangles(harness);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    open_solid(harness)
}

fn pieces_of(harness: &Harness, feature: FeatureId) -> Pieces {
    harness.document().feature(feature).unwrap().pieces
}

fn bodies(harness: &Harness) -> usize {
    harness.model.evaluation().bodies().count()
}

fn regions_of(harness: &Harness, feature: FeatureId) -> RegionChoice {
    harness.solid(feature).regions().clone()
}

#[test]
fn a_body_each_makes_three_rectangles_three_bodies_from_the_extrude_panel_and_undoes() {
    let mut harness = Harness::new();
    let extrude = extruded_rectangles(&mut harness);
    let one_body = (pieces_of(&harness, extrude), bodies(&harness));
    let offered = harness.shows(crate::solid_panel::SEPARATE_PIECES);

    choose(&mut harness, "One body", "A body each");
    let separated = (pieces_of(&harness, extrude), bodies(&harness));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let listed = harness.shows("Bodies (3)");
    harness.perform(Action::Undo);
    harness.settle();

    assert!(offered);
    assert_eq!(one_body, (Pieces::OneBody, 1));
    assert_eq!(separated, (Pieces::BodyEach, 3));
    assert!(listed);
    assert_eq!(pieces_of(&harness, extrude), Pieces::OneBody);
    assert_eq!(bodies(&harness), 1);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn each_body_of_a_separated_extrusion_hides_on_its_own_from_the_bodies_list() {
    let mut harness = Harness::new();
    let extrude = extruded_rectangles(&mut harness);
    choose(&mut harness, "One body", "A body each");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let pieces = harness.model.evaluation().pieces_made_by(extrude);
    let names: Vec<String> = pieces
        .iter()
        .map(|piece| harness.document().body_name(*piece).unwrap().into_owned())
        .collect();
    harness.click("Bodies (3)");
    harness.let_animations_finish();
    harness.frame();
    let listed = names.iter().all(|name| harness.shows(name));

    harness.click_beside(crate::icons::SHOW, &names[1]);
    harness.settle();
    let hidden: Vec<bool> = pieces
        .iter()
        .map(|piece| harness.document().body_hidden(*piece))
        .collect();
    let label = harness.model.undo_label().map(str::to_owned);

    assert_eq!(pieces.len(), 3);
    assert!(listed);
    assert_eq!(hidden, [false, true, false]);
    assert_eq!(label, Some(format!("Hide {}", names[1])));
}

#[test]
fn select_all_takes_every_region_again_after_clear_from_the_panel_and_the_palette() {
    let mut harness = Harness::new();
    let extrude = extruded_rectangles(&mut harness);
    choose(&mut harness, "One body", "A body each");

    run_from_palette(&mut harness, "clear the chosen regions");
    harness.settle();
    let cleared = regions_of(&harness, extrude);
    let waiting = matches!(
        harness
            .model
            .evaluation()
            .feature(extrude)
            .map(|status| &status.state),
        Some(FeatureState::Waiting(_))
    );
    run_from_palette(&mut harness, "select all regions of the sketch");
    harness.settle();
    let from_palette = (regions_of(&harness, extrude), bodies(&harness));

    run_from_palette(&mut harness, "clear the chosen regions");
    harness.settle();
    harness.click("Select all");
    harness.settle();
    let from_button = (regions_of(&harness, extrude), bodies(&harness));
    harness.perform(Action::Undo);
    harness.settle();
    let undone = regions_of(&harness, extrude);

    assert_eq!(cleared, RegionChoice::Chosen(Vec::new()));
    assert!(waiting);
    assert_eq!(from_palette, (RegionChoice::All, 3));
    assert_eq!(from_button, (RegionChoice::All, 3));
    assert_eq!(undone, RegionChoice::Chosen(Vec::new()));
}
