use caditor_document::{Edit, FeatureId, SetMember, Transaction};
use caditor_geometry::Point2;
use egui::{Key, Modifiers};

use super::{Harness, add_block, extruded_plate, run_from_palette, top_edge_along_x};
use crate::{
    bodies,
    model::Action,
    selection::Pickable,
    selection_sets::{NOTHING_TO_KEEP, SAVE_LABEL, name_field_id, rename_field_id},
};

fn notice_text(harness: &Harness) -> String {
    harness
        .model
        .notice()
        .map(|notice| notice.text.clone())
        .unwrap_or_default()
}

fn every_face(harness: &Harness, body: FeatureId) -> Vec<Pickable> {
    let result = bodies::shown(harness.model.evaluation(), body).unwrap();
    bodies::face_keys(&result.solid)
        .into_iter()
        .map(|(_, face)| Pickable::Face { body, face })
        .collect()
}

fn plate_edge_and_block(harness: &mut Harness) -> (FeatureId, Pickable, Pickable, FeatureId) {
    let (plate, top) = extruded_plate(harness);
    let block = add_block(
        harness,
        "Block",
        [Point2::new(60.0, 0.0), Point2::new(80.0, 20.0)],
        "10 mm",
    );
    harness.frame();
    let edge = Pickable::Edge {
        body: plate,
        edge: top_edge_along_x(harness, plate, 0.0),
    };
    (plate, top, edge, block)
}

#[test]
fn a_saved_set_selects_its_faces_edges_and_bodies_again_and_says_what_is_gone() {
    let mut harness = Harness::new();
    let (plate, top, edge, block) = plate_edge_and_block(&mut harness);
    let mut chosen = vec![top, edge];
    chosen.extend(every_face(&harness, block));
    harness.select(chosen.clone());

    run_from_palette(&mut harness, "save the selection as a set");
    harness.frame();
    let sets = harness.document().selection_sets().clone();
    harness.select([]);
    run_from_palette(&mut harness, "set 1");
    harness.frame();
    let selected: Vec<Pickable> = harness.workspace.viewport.selection().iter().collect();

    harness.perform(Action::Apply(Transaction::single(
        "Hide",
        Edit::SetFeatureHidden {
            id: block,
            hidden: true,
        },
    )));
    harness.settle();
    harness.select([]);
    run_from_palette(&mut harness, "set 1");
    harness.frame();
    let without_hidden = harness.workspace.viewport.selection().len();
    let hidden_told = notice_text(&harness);

    assert_eq!(sets.sets.len(), 1);
    assert_eq!(sets.sets[0].name, "Set 1");
    assert_eq!(sets.sets[0].members.len(), 3);
    assert!(sets.sets[0].members.contains(&SetMember::Body(block)));
    assert!(
        sets.sets[0]
            .members
            .iter()
            .any(|member| matches!(member, SetMember::Face { body, .. } if *body == plate))
    );
    assert!(
        sets.sets[0]
            .members
            .iter()
            .any(|member| matches!(member, SetMember::Edge { body, .. } if *body == plate))
    );
    assert_eq!(selected.len(), chosen.len());
    assert!(chosen.iter().all(|pickable| selected.contains(pickable)));
    assert_eq!(without_hidden, 2);
    assert!(hidden_told.contains("Block is hidden"), "{hidden_told}");
}

#[test]
fn saving_a_set_with_nothing_it_can_keep_is_refused_with_the_reason() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    plate_edge_and_block(&mut harness);
    harness.select([]);

    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.show_new_windows();
    harness.type_text("save the selection as a set");
    harness.frame();
    let explained = harness.shows_containing(NOTHING_TO_KEEP);
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();

    assert!(explained);
    assert!(harness.document().selection_sets().is_empty());
}

#[test]
fn the_selection_sets_dialog_saves_renames_replaces_selects_and_deletes_sets() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    let (_, top, edge, _) = plate_edge_and_block(&mut harness);
    harness.select([top]);

    run_from_palette(&mut harness, "selection sets");
    harness.frame();
    let focused = harness.focused() == Some(name_field_id());
    harness.type_into_field(name_field_id(), "Mating faces");
    harness.frame();
    let first = harness.document().selection_sets().sets.clone();
    harness.type_into_field(name_field_id(), "mating faces");
    harness.frame();
    let refused = harness.shows_containing("There is already a selection set named");

    harness.click_button("Rename it");
    harness.frame();
    harness.type_into_field(rename_field_id(), "Top");
    harness.frame();
    let renamed = harness.document().selection_sets().sets[0].name.clone();

    harness.workspace.viewport.selection_mut().clear();
    harness.workspace.viewport.selection_mut().toggle(edge);
    harness.click_button("Replace it with the current selection");
    harness.frame();
    let replaced = harness.document().selection_sets().sets[0].members.clone();

    harness.workspace.viewport.selection_mut().clear();
    harness.click_button("Select what this set holds");
    harness.frame();
    harness.frame();
    let selected: Vec<Pickable> = harness.workspace.viewport.selection().iter().collect();

    harness.click_button("Delete it");
    harness.frame();
    let left = harness.document().selection_sets().sets.len();
    harness.click(SAVE_LABEL);
    harness.frame();
    let saved_again = harness.document().selection_sets().sets.len();

    assert!(focused);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].name, "Mating faces");
    assert!(refused);
    assert_eq!(renamed, "Top");
    assert!(matches!(replaced.as_slice(), [SetMember::Edge { .. }]));
    assert_eq!(selected, vec![edge]);
    assert_eq!(left, 0);
    assert_eq!(saved_again, 1);
}
