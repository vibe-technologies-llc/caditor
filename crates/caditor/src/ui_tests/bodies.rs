use egui::{Id, Key, Modifiers};

use super::{
    Harness, add_peg, extruded_plate, feature_named, hidden, rows_named, run_from_palette,
};
use crate::{model::Action, selection::Pickable};

#[test]
fn a_body_is_renamed_selected_whole_and_removed_from_the_palette() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.frame();

    run_from_palette(&mut harness, "rename body");
    harness.frame();
    let focused = harness.focused() == Some(crate::body_appearance::name_field_id(plate));
    harness.type_text("Base plate");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.settle();
    let named = harness.document().body_name(plate) == Some("Base plate");
    let listed = harness.shows("Base plate");

    harness.select([top]);
    run_from_palette(&mut harness, "select the whole body");
    harness.frame();
    let faces = harness
        .workspace
        .viewport
        .selection()
        .iter()
        .filter(|pickable| matches!(pickable, Pickable::Face { body, .. } if *body == plate))
        .count();

    run_from_palette(&mut harness, "remove body");
    harness.settle();
    let removed = harness.model.evaluation().body(plate).is_none();
    let told = harness.shows_containing("Removed Base plate with Remove 1");
    harness.perform(Action::Undo);
    harness.settle();

    assert!(focused);
    assert!(named);
    assert!(listed);
    assert_eq!(faces, 6);
    assert!(removed);
    assert!(told);
    assert!(harness.model.evaluation().body(plate).is_some());
}

#[test]
fn copy_body_makes_a_placed_copy_beside_the_original() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let volume = harness.body_volume(plate);
    harness.select([top]);
    harness.frame();

    run_from_palette(&mut harness, "copy body");
    harness.settle();
    let copy = harness.workspace.editing.solid().expect("the copy is open");
    harness.type_into_field(Id::new(("move-field", "offset", 1usize, copy)), "60 mm");
    harness.settle();

    let feature = harness.document().feature(copy).unwrap();
    assert_eq!(feature.name, "Copy 1");
    assert!(feature.makes_body());
    assert!(harness.shows(crate::move_panel::MAKE_A_COPY));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!((harness.body_volume(copy) - volume).abs() < 1e-6 * volume);
    let bounds = harness
        .model
        .evaluation()
        .body(copy)
        .unwrap()
        .bounding_box()
        .unwrap();
    assert!((bounds.min().y - 60.0).abs() < 1e-6);
    let original = harness
        .model
        .evaluation()
        .body(plate)
        .unwrap()
        .bounding_box()
        .unwrap();
    assert!(original.min().y.abs() < 1e-6);
}

#[test]
fn swept_sketches_hide_and_bodies_and_sketches_hide_and_show_again() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let sketch = feature_named(&harness, "Plate");
    let sketch_hidden_by_extrude = hidden(&harness, sketch);
    let pickables =
        |harness: &mut Harness| -> Vec<Pickable> { harness.built().picks.pickables().collect() };
    let sketch_pickable = pickables(&mut harness).iter().any(
        |pickable| matches!(pickable, Pickable::SketchEntity { feature, .. } if *feature == sketch),
    );

    harness.let_animations_finish();
    harness.click_beside(crate::icons::HIDE, "Plate");
    harness.settle();
    let shown_from_tree = !hidden(&harness, sketch);
    let show_label = harness.model.undo_label().map(str::to_owned);

    let face = pickables(&mut harness)
        .into_iter()
        .find(|pickable| matches!(pickable, Pickable::Face { .. }))
        .expect("the body has faces");
    harness.select([face]);
    harness.key(Key::H, Modifiers::NONE);
    harness.settle();
    let body_hidden = hidden(&harness, plate);
    let faces_left = pickables(&mut harness)
        .iter()
        .any(|pickable| matches!(pickable, Pickable::Face { .. }));
    let selection_cleared = harness.workspace.viewport.selection().is_empty();

    harness.key(Key::H, Modifiers::ALT);
    harness.settle();

    assert!(sketch_hidden_by_extrude);
    assert!(!sketch_pickable);
    assert!(shown_from_tree);
    assert_eq!(show_label.as_deref(), Some("Show Plate"));
    assert!(body_hidden);
    assert!(!faces_left);
    assert!(selection_cleared);
    assert!(!hidden(&harness, plate) && !hidden(&harness, sketch));
    assert_eq!(harness.model.undo_label(), Some("Show everything"));
    harness.perform(Action::Undo);
    harness.settle();
    assert!(hidden(&harness, plate));
}

#[test]
fn hiding_others_keeps_only_the_selected_body_and_undoes_in_one_step() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let sketch = feature_named(&harness, "Plate");
    harness.click_beside(crate::icons::HIDE, "Plate");
    harness.settle();
    let sketch_shown = !hidden(&harness, sketch);

    harness.select([top]);
    harness.key(Key::H, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();

    assert!(sketch_shown);
    assert!(hidden(&harness, sketch));
    assert!(!hidden(&harness, plate));
    assert_eq!(
        harness.model.undo_label(),
        Some("Hide everything but Extrude 1")
    );
    harness.perform(Action::Undo);
    harness.settle();
    assert!(!hidden(&harness, sketch));
}

#[test]
fn the_bodies_group_lists_every_body_and_hides_or_selects_one() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);
    harness.settle();

    assert!(harness.shows("Bodies (2)"));
    assert_eq!(rows_named(&harness, "Peg").len(), 1);
    harness.click("Bodies (2)");
    harness.settle();
    let rows = rows_named(&harness, "Peg");
    assert_eq!(rows.len(), 2);

    let listed = rows[1];
    harness.click_screen(listed.center());
    harness.settle();
    assert_eq!(harness.workspace.panels.selected, Some(peg));

    let eye = harness
        .texts
        .iter()
        .filter(|(shown, rect)| {
            shown == crate::icons::SHOW && (rect.center().y - listed.center().y).abs() < 4.0
        })
        .map(|(_, rect)| rect.center())
        .next()
        .expect("the listed body has an eye");
    harness.click_screen(eye);
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Hide Peg"));
    assert!(hidden(&harness, peg));
    assert!(!hidden(&harness, plate));

    harness.perform(Action::Undo);
    harness.settle();
    assert!(!hidden(&harness, peg));
}
