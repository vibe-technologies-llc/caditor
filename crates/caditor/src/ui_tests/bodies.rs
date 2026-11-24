use egui::{Id, Key, Modifiers};

use super::{
    Harness, add_peg, choose, extruded_plate, feature_named, hidden, rows_named, run_from_palette,
};
use crate::{
    model::Action,
    preferences::{DoubleClick, PreferenceChange, PreferencesCommand},
    selection::Pickable,
};

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

    let named = harness.document().body_name(plate).as_deref() == Some("Base plate");
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

    harness.type_into_field(Id::unique(("move-field", "offset", 1usize, copy)), "60 mm");
    harness.settle();

    let feature = harness.document().feature(copy).unwrap();

    assert_eq!(feature.name, "Copy 1");
    assert!(feature.makes_body());
    assert!(harness.shows(crate::move_panel::RESULT));
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

    choose(
        &mut harness,
        crate::move_panel::NEW_BODY,
        crate::move_panel::MOVE_BODY,
    );

    assert_eq!(harness.model.undo_label(), Some("Edit Copy 1"));
    assert!(!harness.document().feature(copy).unwrap().makes_body());
    assert!(harness.model.evaluation().body(copy).is_none());

    let moved = harness
        .model
        .evaluation()
        .body(plate)
        .unwrap()
        .bounding_box()
        .unwrap();

    assert!((moved.min().y - 60.0).abs() < 1e-6);

    harness.perform(Action::Undo);
    harness.settle();

    assert!(harness.document().feature(copy).unwrap().makes_body());
    assert!((harness.body_volume(copy) - volume).abs() < 1e-6 * volume);
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

fn body_row(harness: &Harness, name: &str) -> egui::Rect {
    rows_named(harness, name)
        .into_iter()
        .min_by(|one, other| one.top().total_cmp(&other.top()))
        .expect("the body is listed")
}

#[test]
fn a_double_click_on_a_body_row_opens_its_feature_or_renames_the_body_as_the_preference_says() {
    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);

    harness.settle();
    harness.click("Bodies (2)");
    harness.settle();

    let listed = body_row(&harness, "Peg");

    harness.click_screen(listed.center());
    harness.press(listed.center());
    harness.settle();

    let opened = harness.workspace.editing.solid();

    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::DoubleClick(DoubleClick::Rename),
    )));
    harness.frame();

    let listed = body_row(&harness, "Peg");

    harness.click_screen(listed.center());
    harness.press(listed.center());
    harness.frame();
    harness.frame();

    assert_eq!(opened, Some(peg));
    assert_eq!(harness.workspace.editing.solid(), None);
    assert_eq!(
        harness.focused(),
        Some(crate::body_appearance::name_field_id(peg))
    );
    assert!(harness.document().feature(plate).is_some());
}

#[test]
fn preparing_bodies_for_display_can_be_cancelled_leaving_the_rest_hidden_until_shown() {
    use caditor_geometry::Point2;

    use super::block_into;
    use crate::{
        bodies::{Hold, Holds},
        status_bar,
    };

    let mut harness = Harness::new();
    let holds = Holds {
        converts: Hold::holding(),
        ..Holds::default()
    };

    harness.model.hold_body_work(&holds);

    let mut transaction = harness.document().transaction("Add two blocks");

    block_into(
        &mut transaction,
        "Left",
        [Point2::new(0.0, 0.0), Point2::new(10.0, 10.0)],
        "5 mm",
    );
    block_into(
        &mut transaction,
        "Right",
        [Point2::new(20.0, 0.0), Point2::new(30.0, 10.0)],
        "5 mm",
    );
    harness.perform(Action::Apply(transaction.finish()));

    let preparing = format!("{}…", status_bar::showing_bodies_text(0, 2));

    harness.wait_until("the bodies are being prepared", |harness| {
        harness.shows(&preparing)
    });

    harness.hover("Cancel");

    let explained = harness.shows_containing("stay hidden until Show the rest");

    harness.click("Cancel");
    harness.frame();
    harness.frame();

    assert!(explained);
    assert!(!harness.model.bodies_pending());
    assert_eq!(harness.model.bodies_stopped(), Some((0, 2)));
    assert!(harness.shows(&status_bar::showing_bodies_text(0, 2)));
    assert!(!harness.shows(&preparing));
    assert_eq!(harness.workspace.viewport.bodies().iter().count(), 0);

    holds.converts.release();
    harness.click(status_bar::SHOW_THE_REST);
    harness.settle();

    assert_eq!(harness.model.bodies_stopped(), None);
    assert!(!harness.shows(status_bar::SHOW_THE_REST));
    assert_eq!(harness.workspace.viewport.bodies().iter().count(), 2);
}

#[test]
fn a_bodys_edges_are_one_styled_run_over_its_mesh_polylines_not_a_line_per_segment() {
    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);

    harness.select([]);
    harness.frame();

    let built = harness.built();
    let edge_picks: Vec<_> = built
        .picks
        .pickables()
        .filter(|pickable| matches!(pickable, Pickable::Edge { body, .. } if *body == plate))
        .filter_map(|pickable| built.picks.id_of(pickable))
        .collect();

    let batch = built
        .scene
        .batches
        .iter()
        .find(|batch| {
            batch
                .runs
                .iter()
                .flat_map(|run| &run.polylines)
                .any(|drawn| drawn.pick.is_some_and(|pick| edge_picks.contains(&pick)))
        })
        .expect("the plate's edges are drawn");
    let run = batch.runs.first().expect("the plate has a run");
    let lines: Vec<_> = batch.all_lines().collect();

    assert_eq!(edge_picks.len(), 12);
    assert!(batch.lines.is_empty());
    assert_eq!(batch.runs.len(), 1);
    assert_eq!(run.polylines.len(), 12);
    assert_eq!(lines.len(), run.segment_count());
    assert!(lines.len() >= 12);
    assert!(
        edge_picks
            .iter()
            .all(|pick| lines.iter().any(|line| line.pick == Some(*pick)))
    );
}

#[test]
fn isolating_a_body_shows_only_it_for_the_session_without_undo_and_clears_on_exit_or_new_model() {
    let mut harness = Harness::new();

    let (plate, top) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);

    harness.settle();

    let undo_before = harness.model.undo_label().map(str::to_owned);

    harness.select([top]);
    harness.frame();
    run_from_palette(&mut harness, "Isolate body in view");
    harness.frame();
    harness.frame();

    let isolated = harness.workspace.viewport.isolated().clone();
    let picks_isolated: Vec<Pickable> = harness.built().picks.pickables().collect();
    let has_plate_isolated = picks_isolated
        .iter()
        .any(|pickable| matches!(pickable, Pickable::Face { body, .. } if *body == plate));
    let has_peg_isolated = picks_isolated
        .iter()
        .any(|pickable| matches!(pickable, Pickable::Face { body, .. } if *body == peg));
    let undo_during = harness.model.undo_label().map(str::to_owned);
    let shows_chip = harness.shows_containing("Isolated:");

    run_from_palette(&mut harness, "Exit isolate");
    harness.frame();
    harness.frame();

    let restored = harness.workspace.viewport.isolated().is_empty();
    let picks_restored: Vec<Pickable> = harness.built().picks.pickables().collect();
    let has_peg_restored = picks_restored
        .iter()
        .any(|pickable| matches!(pickable, Pickable::Face { body, .. } if *body == peg));

    harness.select([top]);
    harness.frame();
    run_from_palette(&mut harness, "Isolate body in view");
    harness.frame();
    harness
        .model
        .replace(caditor_document::Document::default(), None, None, false);
    harness.settle();

    assert!(isolated.contains(&plate));
    assert!(!isolated.contains(&peg));
    assert!(has_plate_isolated);
    assert!(!has_peg_isolated);
    assert_eq!(undo_during, undo_before);
    assert!(shows_chip);
    assert!(restored);
    assert!(has_peg_restored);
    assert!(harness.workspace.viewport.isolated().is_empty());
}

#[test]
fn bodies_group_header_eye_toggles_all_bodies_and_rows_offer_a_menu_button() {
    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);

    harness.settle();
    harness.click("Bodies (2)");
    harness.settle();
    harness.click_button("Hide every body");
    harness.settle();

    let both_hidden = hidden(&harness, plate) && hidden(&harness, peg);

    harness.click_button("Show every body");
    harness.settle();

    let both_shown = !hidden(&harness, plate) && !hidden(&harness, peg);

    harness.click_button("More actions for Peg");
    harness.frame();

    let shows_isolate = harness.shows("Isolate in view");

    assert!(both_hidden);
    assert!(both_shown);
    assert!(shows_isolate);
}
