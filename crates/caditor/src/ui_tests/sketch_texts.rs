use caditor_document::placed_text;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{EntityId, Faceting, Sketch, TextAlign};
use egui::{Key, Modifiers};

use super::{Harness, constraints_of_kind, edit_free_sketch, run_from_palette};
use crate::{
    editing::Tool,
    selection::Pickable,
    solid_tools, text_panel,
    texting::{self, TextField},
};

const TEXT_TOOL: &str = "write or edit sketch text";
const START: Point2 = Point2::new(10.0, 5.0);

fn only_text(sketch: &Sketch) -> EntityId {
    let texts: Vec<EntityId> = sketch.texts().map(|(id, _)| id).collect();
    assert_eq!(texts.len(), 1, "{texts:?}");
    texts[0]
}

#[test]
fn text_is_written_where_clicked_after_a_blank_one_is_refused_in_words() {
    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));
    let before = harness.sketch(feature).clone();

    run_from_palette(&mut harness, TEXT_TOOL);
    assert_eq!(harness.tool(), Some(Tool::Text));
    assert!(harness.shows(text_panel::TITLE));
    assert!(harness.shows(texting::PROMPT));
    harness.type_lines_into_field(text_panel::field_id(TextField::Content), &["   "]);
    assert!(harness.shows_containing("Text needs at least one letter"));
    harness.click_at(START);
    harness.settle();
    assert!(harness.sketch(feature).same_content(&before));

    harness.type_lines_into_field(text_panel::field_id(TextField::Content), &["M8 bolt"]);
    harness.type_into_field(text_panel::field_id(TextField::Height), "2 * 3 mm");
    harness.point_at(START);
    assert!(harness.shows_containing("Text “M8 bolt”, 6 mm high"));
    harness.click_at(START);
    harness.settle();

    let sketch = harness.sketch(feature);
    let text = only_text(sketch);
    let held = sketch.text(text).unwrap();
    let anchor = sketch.point(held.anchor).unwrap();
    assert_eq!(held.content, "M8 bolt");
    assert!(anchor.distance(START) < 1.0, "{anchor:?}");
    assert_eq!(harness.model.undo_label(), Some(texting::TRANSACTION));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn text_written_at_the_origin_extrudes_into_its_letters() {
    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));

    run_from_palette(&mut harness, TEXT_TOOL);
    harness.type_lines_into_field(text_panel::field_id(TextField::Content), &["HOLA"]);
    harness.type_into_field(text_panel::field_id(TextField::Height), "8 mm");
    harness.click(&text_panel::write_label(None, None));
    harness.settle();
    let sketch = harness.sketch(feature).clone();
    let text = only_text(&sketch);
    let coincident = constraints_of_kind(&sketch, "Coincident");
    let shown = harness.shown(feature);
    let placed = placed_text(&shown, text).unwrap();
    let area: f64 = placed
        .polylines(Faceting::within(1e-4))
        .iter()
        .map(|line| {
            line.iter()
                .zip(line.iter().skip(1))
                .map(|(a, b)| a.perp_dot(*b))
                .sum::<f64>()
                / 2.0
        })
        .sum::<f64>()
        .abs();
    harness.click("Extrude");
    harness.settle();
    let extrude = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");

    let volume = harness.body_volume(extrude);
    assert_eq!(coincident.len(), 1);
    assert!(coincident[0].entities().contains(&EntityId::ORIGIN));
    assert!(area > 0.0);
    assert!(
        (volume - area * solid_tools::DEFAULT_DISTANCE).abs() < 1e-2 * volume,
        "{volume} for {area}"
    );
}

#[test]
fn a_selected_text_is_edited_in_place_keeping_its_point() {
    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));
    run_from_palette(&mut harness, TEXT_TOOL);
    harness.type_lines_into_field(text_panel::field_id(TextField::Content), &["Draft"]);
    harness.click_at(START);
    harness.settle();
    let text = only_text(harness.sketch(feature));
    let anchor = harness.sketch(feature).text(text).unwrap().anchor;
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.select([Pickable::SketchEntity {
        feature,
        entity: text,
    }]);
    run_from_palette(&mut harness, TEXT_TOOL);
    let update = text_panel::update_label(Some(harness.sketch(feature)), text);
    assert!(harness.shows(&update));
    assert!(harness.shows(texting::EDIT_PROMPT));
    harness.type_lines_into_field(text_panel::field_id(TextField::Content), &["Final"]);
    harness.type_into_field(text_panel::field_id(TextField::Angle), "90 deg");
    harness.click(&update);
    harness.settle();

    let sketch = harness.sketch(feature);
    let held = sketch.text(only_text(sketch)).unwrap();
    assert_eq!(held.content, "Final");
    assert_eq!(held.anchor, anchor);
    assert_eq!(harness.model.undo_label(), Some(texting::EDIT_TRANSACTION));
    assert_eq!(harness.document().expression_text(&held.angle), "90 deg");
}

#[test]
fn deleting_a_selected_text_takes_its_point() {
    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));
    run_from_palette(&mut harness, TEXT_TOOL);
    harness.click_at(START);
    harness.settle();
    let text = only_text(harness.sketch(feature));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.select([Pickable::SketchEntity {
        feature,
        entity: text,
    }]);
    harness.key(Key::Delete, Modifiers::NONE);
    harness.settle();

    let sketch = harness.sketch(feature);
    assert_eq!(sketch.texts().len(), 0);
    assert_eq!(sketch.entities().len(), 0);
}

#[test]
fn several_lines_and_an_alignment_are_written_and_edited_in_the_panel() {
    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));
    run_from_palette(&mut harness, TEXT_TOOL);

    harness.type_lines_into_field(
        text_panel::field_id(TextField::Content),
        &["Top", "Bottom line"],
    );
    harness.click("Centre");
    harness.click_at(START);
    harness.settle();

    let sketch = harness.sketch(feature);
    let text = only_text(sketch);
    let held = sketch.text(text).unwrap();
    let placed = placed_text(sketch, text).unwrap();
    let lowest = placed
        .polylines(Faceting::within(1e-4))
        .into_iter()
        .flatten()
        .map(|point| point.y)
        .fold(f64::INFINITY, f64::min);
    assert_eq!(held.content, "Top\nBottom line");
    assert_eq!(held.align, TextAlign::Centre);
    assert!(lowest < START.y - 5.0, "{lowest}");
    assert_eq!(harness.model.undo_label(), Some(texting::TRANSACTION));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.select([Pickable::SketchEntity {
        feature,
        entity: text,
    }]);
    run_from_palette(&mut harness, TEXT_TOOL);
    harness.click("Right");
    harness.click(&text_panel::update_label(
        Some(harness.sketch(feature)),
        text,
    ));
    harness.settle();

    let sketch = harness.sketch(feature);
    let held = sketch.text(only_text(sketch)).unwrap();
    assert_eq!(held.content, "Top\nBottom line");
    assert_eq!(held.align, TextAlign::Right);
    assert_eq!(harness.model.undo_label(), Some(texting::EDIT_TRANSACTION));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    let sketch = harness.sketch(feature);
    assert_eq!(
        sketch.text(only_text(sketch)).unwrap().align,
        TextAlign::Centre
    );
}
