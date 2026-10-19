use caditor_document::{
    Document, Edit, Feature, FeatureId, FeatureKind, PlaneReference, RollbackBar, SolidFeature,
    Split, SplitAlong, Transaction,
};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Event, Id, Key, Modifiers, Pos2, accesskit::Role};

use super::{
    CAMERA_SETTLE, Harness, click_with, extruded_plate, feature_named, feature_names, hold_drag,
    pickable_described, rectangle, release_drag, run_from_palette, top_edge_along_x,
};
use crate::{
    app::Workspace,
    editing::EditingCommand,
    icons,
    model::Action,
    panels::{Focus, REVEAL_FRAMES},
    selection::Pickable,
};

#[test]
fn a_face_clicked_in_the_view_marks_its_body_row_selected_in_the_tree() {
    let mut harness = Harness::new();
    let (extrude, top) = extruded_plate(&mut harness);
    harness.select([]);

    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    harness.frame();
    let marked = harness.workspace.panels.reads_selected(extrude);
    let chosen = harness.workspace.panels.chosen();

    harness.select([]);
    harness.frame();
    let cleared = harness.workspace.panels.reads_selected(extrude);

    assert!(marked);
    assert!(
        chosen.is_empty(),
        "marking a row does not choose it: {chosen:?}"
    );
    assert!(!cleared);
    assert!(harness.workspace.panels.in_view.is_empty());
}

#[test]
fn a_row_chosen_in_the_tree_highlights_its_body_in_the_view_without_selecting_it() {
    let mut harness = Harness::new();
    let (extrude, top) = extruded_plate(&mut harness);
    harness.select([]);
    harness.frame();
    let face_colour = |harness: &mut Harness| {
        let built = harness.built();
        let pick = built.picks.id_of(top);
        built
            .scene
            .meshes
            .iter()
            .flat_map(|instance| instance.faces.iter())
            .find(|face| face.pick == pick)
            .map(|face| face.color)
            .expect("the top face is drawn")
    };
    let plain = face_colour(&mut harness);

    harness.workspace.panels.choose_only(extrude);
    harness.frame();
    harness.frame();
    let highlighted = face_colour(&mut harness);
    let selected = harness.workspace.viewport.selection().is_empty();

    harness.workspace.panels.selected = None;
    harness.frame();
    harness.frame();
    let cleared = face_colour(&mut harness);

    assert_ne!(highlighted, plain);
    assert!(selected, "highlighting does not select");
    assert_eq!(cleared, plain);
}

#[test]
fn features_chosen_in_the_tree_copy_and_paste_as_one_change_following_each_other() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    let sketch = match &harness.document().feature(extrude).unwrap().kind {
        FeatureKind::Solid(solid) => solid.sketch(),
        other => panic!("an extrusion, found {other:?}"),
    };
    let before = harness.document().features().len();

    harness.workspace.panels.choose_all(&[sketch, extrude]);
    harness.frame();
    harness.events.push(Event::Copy);
    harness.settle();
    harness.frame();
    assert!(harness.shows("Copied 2 features."));
    let copied = harness.clipboard.clone().unwrap();
    assert!(copied.starts_with("caditor clipboard: features, version 1\n"));
    harness.events.push(Event::Paste(copied));
    harness.settle();

    let document = harness.document();
    assert_eq!(document.features().len(), before + 2);
    assert_eq!(harness.model.undo_label(), Some("Paste 2 features"));
    let pasted: Vec<&Feature> = document.features().skip(before).collect();
    assert_eq!(pasted[1].name, "Extrude 1 copy");
    let Some(SolidFeature::Extrude(copy)) = pasted[1].kind.solid() else {
        panic!("an extrusion");
    };
    assert_eq!(copy.sketch, pasted[0].id());
    assert_ne!(pasted[0].id(), sketch);

    harness
        .events
        .push(Event::Paste("not a feature".to_owned()));
    harness.settle();
    harness.frame();
    assert!(
        harness
            .shows("Nothing was pasted: the clipboard holds text that did not come from caditor.")
    );
    assert_eq!(harness.document().features().len(), before + 2);
}

#[test]
fn a_modifying_features_row_frames_the_body_it_changes_and_lights_the_faces_it_made() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let mut far = Sketch::new(Plane::XY);
    rectangle(
        &mut far,
        Point2::new(500.0, 500.0),
        Point2::new(510.0, 510.0),
    );
    harness.add_sketch(far);
    let front = top_edge_along_x(&harness, plate, 0.0);
    harness.select([Pickable::Edge {
        body: plate,
        edge: front,
    }]);
    harness.click("Fillet");
    harness.settle();
    let fillet = harness
        .workspace
        .editing
        .solid()
        .expect("the fillet is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    harness.select([]);
    let rounded = pickable_described(&mut harness, "Extrude 1 › Fillet 1 face");
    let face_colour = |harness: &mut Harness, face: Pickable| {
        let built = harness.built();
        let pick = built.picks.id_of(face);
        built
            .scene
            .meshes
            .iter()
            .flat_map(|instance| instance.faces.iter())
            .find(|drawn| drawn.pick == pick)
            .map(|drawn| drawn.color)
            .expect("the face is drawn")
    };
    let plain = face_colour(&mut harness, top);

    harness.workspace.panels.choose_only(fillet);
    harness.frame();
    harness.frame();
    let made = face_colour(&mut harness, rounded);
    let untouched = face_colour(&mut harness, top);
    harness.key(Key::F, Modifiers::NONE);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let target = harness.workspace.viewport.viewpoint().target;

    assert!(harness.document().feature(fillet).is_some());
    assert_ne!(made, plain);
    assert_eq!(untouched, plain);
    assert!(
        target.x < 100.0,
        "the view is framed on the filleted body: {target:?}"
    );
}

#[test]
fn a_split_off_body_s_row_lights_that_body_alone_not_the_body_it_was_split_from() {
    let mut harness = Harness::new();
    let mut blocks = Sketch::new(Plane::XY);
    rectangle(
        &mut blocks,
        Point2::new(-30.0, 0.0),
        Point2::new(-10.0, 20.0),
    );
    rectangle(&mut blocks, Point2::new(10.0, 0.0), Point2::new(30.0, 20.0));
    harness.add_sketch(blocks);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let extrude = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let mut transaction = harness.document().transaction("Split");
    let split = transaction.add_feature(
        "Split 1",
        FeatureKind::Split(Split {
            body: extrude,
            along: SplitAlong::Plane(PlaneReference::Principal(
                caditor_document::PrincipalPlane::Yz,
            )),
            flipped: false,
        }),
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    harness.select([]);
    harness.frame();
    let colours = |harness: &mut Harness, wanted: FeatureId| {
        let built = harness.built();
        let picks: Vec<_> = built
            .picks
            .pickables()
            .filter(|pickable| matches!(pickable, Pickable::Face { body, .. } if *body == wanted))
            .map(|pickable| built.picks.id_of(pickable))
            .collect();
        let mut found: Vec<_> = built
            .scene
            .meshes
            .iter()
            .flat_map(|instance| instance.faces.iter())
            .filter(|face| picks.contains(&face.pick))
            .map(|face| format!("{:?}", face.color))
            .collect();
        found.sort();
        found
    };
    let kept_plain = colours(&mut harness, extrude);
    let split_plain = colours(&mut harness, split);

    harness.workspace.panels.choose_only(split);
    harness.frame();
    harness.frame();
    let kept_lit = colours(&mut harness, extrude);
    let split_lit = colours(&mut harness, split);

    assert!(!kept_plain.is_empty() && !split_plain.is_empty());
    assert_eq!(kept_lit, kept_plain, "the body split from stays unlit");
    assert_ne!(split_lit, split_plain, "the split-off body is lit");
}

#[test]
fn a_feature_chosen_in_the_tree_is_moved_renamed_and_deleted_from_the_keyboard() {
    let mut harness = Harness::new();
    harness.click("Side sketch");
    harness.frame();
    run_from_palette(&mut harness, "move feature up");
    harness.settle();
    assert_eq!(feature_names(&harness), ["Side sketch", "Base sketch"]);

    harness.key(Key::F2, Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness.replace_text("Profile");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.settle();
    assert_eq!(feature_names(&harness), ["Profile", "Base sketch"]);

    harness.key(Key::Delete, Modifiers::NONE);
    harness.settle();
    assert_eq!(feature_names(&harness), ["Base sketch"]);
    harness.key(Key::Delete, Modifiers::NONE);
    harness.settle();
    assert_eq!(feature_names(&harness), ["Base sketch"]);
}

#[test]
fn chosen_features_are_grouped_named_folded_and_ungrouped_from_the_tree() {
    let mut harness = Harness::new();
    let ids: Vec<FeatureId> = harness
        .document()
        .features()
        .map(|feature| feature.id())
        .collect();
    let groups = |harness: &Harness| -> Vec<Option<String>> {
        harness
            .document()
            .features()
            .map(|feature| feature.group.clone())
            .collect()
    };
    harness.workspace.panels.choose_all(&ids);
    harness.frame();

    harness.key(Key::G, Modifiers::COMMAND);
    harness.settle();
    harness.frame();
    let first = groups(&harness);
    harness.replace_text("Profiles");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.settle();
    let renamed = groups(&harness);
    let shown_open = harness.shows("Base sketch");
    harness.click_button("Hide details of Profiles");
    harness.frame();
    harness.frame();
    let shown_folded = harness.shows("Base sketch");
    let folder_shown = harness.shows("Profiles");
    run_from_palette(&mut harness, "ungroup");
    harness.settle();

    let named = |name: &str| vec![Some(name.to_owned()); ids.len()];
    assert_eq!(first, named("Group 1"));
    assert_eq!(renamed, named("Profiles"));
    assert!(shown_open);
    assert!(!shown_folded);
    assert!(folder_shown);
    assert_eq!(groups(&harness), vec![None; ids.len()]);
    assert!(harness.shows("Base sketch"));
}

#[test]
fn the_feature_tree_is_filtered_by_name_from_the_keyboard() {
    let mut harness = Harness::new();
    let hidden_at_first = !harness.shows(crate::feature_tree::FILTER_HINT);

    harness.key(Key::F, Modifiers::COMMAND);
    harness.frame();
    let focused = harness.focused() == Some(Focus::TreeFilter.field_id());
    harness.type_text("SIDE");
    let side_only = harness.shows("Side sketch") && !harness.shows("Base sketch");
    harness.replace_text("bracket");
    let none_named =
        harness.shows("No feature is named like “bracket” or is of that kind or state.");
    harness.click(crate::feature_tree::CLEAR_FILTER_LABEL);
    harness.frame();

    assert!(hidden_at_first);
    assert!(focused);
    assert!(side_only);
    assert!(none_named);
    assert!(harness.shows("Side sketch") && harness.shows("Base sketch"));
    assert!(harness.workspace.panels.tree_filter.is_empty());
}

#[test]
fn the_feature_tree_filter_matches_kinds_as_well_as_names() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    for _ in 0..REVEAL_FRAMES {
        harness.frame();
    }
    harness.context.enable_accesskit();
    let row = |harness: &Harness, name: &str| {
        harness.accessible_named(Role::Button, &format!("More actions for {name}"))
    };

    harness.key(Key::F, Modifiers::COMMAND);
    harness.frame();
    harness.type_text("extrusion");
    harness.frame();
    let extrusions_only = row(&harness, "Extrude 1") && !row(&harness, "Base sketch");
    harness.replace_text("sketch");
    harness.frame();
    let sketches_only =
        row(&harness, "Base sketch") && row(&harness, "Side sketch") && !row(&harness, "Extrude 1");

    assert!(extrusions_only);
    assert!(sketches_only);
}

fn feature_order(harness: &Harness) -> Vec<String> {
    harness
        .document()
        .features()
        .map(|feature| feature.name.clone())
        .collect()
}

#[test]
fn features_move_up_and_down_from_their_menu() {
    let mut harness = Harness::new();
    harness.click_beside(crate::icons::MORE, "Side sketch");
    harness.click("Move up");
    harness.settle();
    let moved_up = feature_order(&harness);
    let up_label = harness.model.undo_label().map(str::to_owned);
    harness.click_beside(crate::icons::MORE, "Side sketch");
    let first_cannot_rise = harness.shows("Move up") && {
        harness.click("Move up");
        harness.settle();
        feature_order(&harness) == moved_up
    };
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.click_beside(crate::icons::MORE, "Side sketch");
    harness.click("Move down");
    harness.settle();

    assert_eq!(moved_up, ["Side sketch", "Base sketch"]);
    assert_eq!(up_label.as_deref(), Some("Move up Side sketch"));
    assert!(first_cannot_rise);
    assert_eq!(feature_order(&harness), ["Base sketch", "Side sketch"]);
    assert_eq!(harness.model.undo_label(), Some("Move down Side sketch"));
}

#[test]
fn the_curves_of_a_suppressed_or_rolled_back_sketch_leave_the_selection() {
    let mut harness = Harness::new();
    harness.settle();
    let mut far = Sketch::new(Plane::XY);
    let line = far.add_line(Point2::new(900.0, 900.0), Point2::new(1000.0, 1000.0));
    let sketch = harness.add_sketch(far);
    harness.settle();
    let curve = Pickable::SketchEntity {
        feature: sketch,
        entity: line,
    };

    for (label, edit, back) in [
        (
            "Suppress",
            Edit::SetFeatureSuppressed {
                id: sketch,
                suppressed: true,
            },
            Edit::SetFeatureSuppressed {
                id: sketch,
                suppressed: false,
            },
        ),
        (
            "Roll back",
            Edit::SetRollbackBar {
                bar: RollbackBar::Before(sketch),
            },
            Edit::SetRollbackBar {
                bar: RollbackBar::AtEnd,
            },
        ),
    ] {
        harness.select([curve]);
        let selected = harness.workspace.viewport.selection().contains(curve);
        harness.perform(Action::Apply(Transaction::single(label, edit)));
        harness.settle();
        let after = harness.workspace.viewport.selection().contains(curve);
        harness.perform(Action::Apply(Transaction::single(label, back)));
        harness.settle();

        assert!(selected, "{label}");
        assert!(!after, "{label}");
    }
}

fn is_suppressed(harness: &Harness, name: &str) -> bool {
    harness
        .document()
        .feature(feature_named(harness, name))
        .is_some_and(|feature| feature.suppressed)
}

fn upper_edge_of(harness: &Harness, label: &str) -> Pos2 {
    let (_, rect) = harness
        .texts
        .iter()
        .find(|(shown, _)| shown == label)
        .unwrap_or_else(|| panic!("'{label}' is not on screen"));
    Pos2::new(rect.center().x, rect.top() + 1.0)
}

#[test]
fn suppressing_a_sketch_fails_its_extrusion_until_the_callout_unsuppresses_it() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    let volume = harness.body_volume(extrude);

    harness.click_beside(icons::MORE, "Plate");
    harness.click("Suppress");
    harness.settle();
    let suppressed_label = harness.model.undo_label().map(str::to_owned);
    let failed = harness.shows("It uses Plate, which is suppressed.");
    let body_stale = harness.model.evaluation().is_stale(extrude);
    let muted = harness.color_of("Plate");
    harness.click_beside(icons::MORE, "Extrude 1");
    harness.click("Suppress");
    harness.settle();
    harness.click("Extrude 1");
    harness.key(Key::E, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let refused = harness.model.notice().map(|notice| notice.text.clone());
    harness.perform(Action::Undo);
    harness.settle();

    assert_eq!(suppressed_label.as_deref(), Some("Suppress Plate"));
    assert!(failed);
    assert!(body_stale);
    assert_ne!(muted, harness.color_of("Side sketch"));
    assert_eq!(
        refused.as_deref(),
        Some("Edit feature: Extrude 1 is suppressed; unsuppress it to edit it")
    );
    assert!(is_suppressed(&harness, "Plate"));
    assert!(!is_suppressed(&harness, "Extrude 1"));

    harness.click("Unsuppress Plate");
    harness.settle();

    assert!(!is_suppressed(&harness, "Plate"));
    assert_eq!(harness.model.undo_label(), Some("Unsuppress Plate"));
    assert!(harness.shows("Up to date"));
    assert!((harness.body_volume(extrude) - volume).abs() < 1e-6);
}

#[test]
fn features_chosen_together_in_the_tree_are_suppressed_in_one_change_from_the_palette() {
    let mut harness = Harness::new();

    harness.click("Base sketch");
    click_with(&mut harness, "Side sketch", Modifiers::COMMAND);
    run_from_palette(&mut harness, "suppress or unsuppress");
    harness.settle();
    let both = is_suppressed(&harness, "Base sketch") && is_suppressed(&harness, "Side sketch");
    let label = harness.model.undo_label().map(str::to_owned);
    harness.perform(Action::Undo);
    harness.settle();

    assert!(both);
    assert_eq!(label.as_deref(), Some("Suppress 2 features"));
    assert!(!is_suppressed(&harness, "Base sketch"));
    assert!(!is_suppressed(&harness, "Side sketch"));
}

#[test]
fn the_rollback_bar_moves_from_the_menu_and_keyboard_and_new_features_go_in_above_it() {
    let mut harness = Harness::new();

    harness.click_beside(icons::MORE, "Base sketch");
    harness.click("Roll back to here");
    harness.settle();
    let rolled_label = harness.model.undo_label().map(str::to_owned);
    let side_rolled_back = harness
        .document()
        .is_rolled_back(feature_named(&harness, "Side sketch"));
    let caption = harness.shows("1 feature rolled back");
    let sketch = harness.draw_on_new_sketch();
    let order = feature_names(&harness);
    harness.key(Key::ArrowDown, Modifiers::ALT);
    harness.settle();
    let at_end = harness.document().rollback_bar();
    harness.key(Key::ArrowUp, Modifiers::ALT);
    harness.settle();
    harness.key(Key::ArrowUp, Modifiers::ALT);
    harness.settle();
    let editing_after = harness.editing();
    let rolled_past_sketch = harness.document().is_rolled_back(sketch);
    run_from_palette(&mut harness, "roll to end");
    harness.settle();

    assert_eq!(rolled_label.as_deref(), Some("Roll back to Base sketch"));
    assert!(side_rolled_back);
    assert!(caption);
    assert_eq!(order, ["Base sketch", "Sketch 1", "Side sketch"]);
    assert_eq!(at_end, RollbackBar::AtEnd);
    assert!(rolled_past_sketch);
    assert_eq!(editing_after, None);
    assert_eq!(harness.document().rollback_bar(), RollbackBar::AtEnd);
    assert_eq!(harness.model.undo_label(), Some("Roll to end"));
    assert!(harness.unreadable_nodes().is_empty());
}

#[test]
fn dragging_a_feature_reorders_the_tree_and_a_refused_place_says_why_while_dragging() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);

    let from = harness.position_of("Extrude 1");
    let onto = upper_edge_of(&harness, "Plate");
    hold_drag(&mut harness, from, onto);
    let refusal = harness.shows("Extrude 1 cannot move above Plate, which it uses");
    release_drag(&mut harness, onto);
    let unchanged = feature_names(&harness);

    let from = harness.position_of("Side sketch");
    let onto = upper_edge_of(&harness, "Base sketch");
    hold_drag(&mut harness, from, onto);
    let allowed = !harness.shows("Side sketch cannot move above Base sketch, which it uses");
    release_drag(&mut harness, onto);

    assert!(refusal);
    assert_eq!(
        unchanged,
        ["Base sketch", "Side sketch", "Plate", "Extrude 1"]
    );
    assert!(allowed);
    assert_eq!(
        feature_names(&harness),
        ["Side sketch", "Base sketch", "Plate", "Extrude 1"]
    );
    assert_eq!(harness.model.undo_label(), Some("Move Side sketch"));
    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(
        feature_names(&harness),
        ["Base sketch", "Side sketch", "Plate", "Extrude 1"]
    );
}

#[test]
fn dragging_one_of_several_chosen_features_moves_them_all_together() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);

    harness.click("Base sketch");
    click_with(&mut harness, "Plate", Modifiers::COMMAND);
    let from = harness.position_of("Plate");
    let onto = upper_edge_of(&harness, "Base sketch");
    hold_drag(&mut harness, from, onto);
    let refused = harness.shows_containing("cannot move");
    release_drag(&mut harness, onto);

    assert!(!refused);
    assert_eq!(
        feature_names(&harness),
        ["Base sketch", "Plate", "Side sketch", "Extrude 1"]
    );
    assert_eq!(harness.model.undo_label(), Some("Move 2 features"));

    harness.perform(Action::Undo);
    harness.settle();

    assert_eq!(
        feature_names(&harness),
        ["Base sketch", "Side sketch", "Plate", "Extrude 1"]
    );
}

#[test]
fn dragging_the_rollback_bar_shows_the_model_as_of_where_it_is_dropped() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    harness.let_animations_finish();

    let bar = harness
        .button_rect(crate::feature_tree::ROLLBACK_BAR_NAME)
        .center();
    let onto = upper_edge_of(&harness, "Plate");
    hold_drag(&mut harness, bar, onto);
    release_drag(&mut harness, onto);

    assert_eq!(
        harness.document().rollback_bar(),
        RollbackBar::Before(feature_named(&harness, "Plate"))
    );
    assert!(harness.model.evaluation().body(extrude).is_none());
    assert!(harness.shows("2 features rolled back"));
    assert_eq!(harness.model.undo_label(), Some("Move the rollback bar"));
    harness.perform(Action::Undo);
    harness.settle();
    assert!(harness.model.evaluation().body(extrude).is_some());
}

#[test]
fn deleting_a_feature_others_use_asks_whether_to_take_or_keep_them() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let everything = feature_names(&harness);

    harness.click("Plate");
    harness.key(Key::Delete, Modifiers::NONE);
    harness.show_new_windows();
    let asked = harness.shows("Delete “Plate”?") && harness.shows("1 feature depends on “Plate”:");
    let listed = harness.shows("uses Plate");
    let with_dependents = harness.position_of(crate::feature_tree::DELETE_WITH_DEPENDENTS);
    let keep = harness.position_of(crate::feature_tree::KEEP_DEPENDENTS);
    let cancel = harness.position_of("Cancel");
    harness.click("Cancel");
    let cancelled = feature_names(&harness) == everything && !harness.shows("Delete “Plate”?");

    harness.click("Plate");
    harness.key(Key::Delete, Modifiers::NONE);
    harness.show_new_windows();
    harness.click(crate::feature_tree::KEEP_DEPENDENTS);
    harness.settle();
    let kept = feature_names(&harness);
    let failing = harness.shows("It uses a feature that no longer exists.");
    harness.perform(Action::Undo);
    harness.settle();

    harness.click("Plate");
    harness.key(Key::Delete, Modifiers::NONE);
    harness.show_new_windows();
    harness.click(crate::feature_tree::DELETE_WITH_DEPENDENTS);
    harness.settle();
    let taken = feature_names(&harness);
    let label = harness.model.undo_label().map(str::to_owned);
    harness.perform(Action::Undo);
    harness.settle();

    assert!(asked);
    assert!(listed);
    assert!(with_dependents.x < keep.x && keep.x < cancel.x);
    assert!(cancelled);
    assert_eq!(kept, ["Base sketch", "Side sketch", "Extrude 1"]);
    assert!(failing);
    assert_eq!(taken, ["Base sketch", "Side sketch"]);
    assert_eq!(label.as_deref(), Some("Delete Plate and its dependents"));
    assert_eq!(feature_names(&harness), everything);
    assert!(harness.shows("Up to date"));
}

#[test]
fn a_click_on_a_feature_name_selects_it_without_opening_or_expanding_it() {
    let mut harness = Harness::new();
    let base = feature_named(&harness, "Base sketch");
    let side = feature_named(&harness, "Side sketch");

    harness.click("Side sketch");
    let chosen = harness.workspace.panels.chosen();
    let expanded = harness.shows(crate::feature_tree::DIMENSIONS_TITLE);
    click_with(&mut harness, "Base sketch", Modifiers::COMMAND);

    assert_eq!(chosen, vec![side]);
    assert!(!expanded);
    assert_eq!(harness.editing(), None);
    assert_eq!(harness.workspace.panels.chosen(), vec![side, base]);
    assert!(!harness.shows(crate::feature_tree::DIMENSIONS_TITLE));
}

#[test]
fn the_chevron_alone_shows_and_hides_a_feature_s_details() {
    let mut harness = Harness::new();

    harness.click_button("Show details of Side sketch");
    harness.let_animations_finish();
    let shown = harness.shows(crate::feature_tree::DIMENSIONS_TITLE);
    let chosen = harness.workspace.panels.chosen();
    harness.click_button("Hide details of Side sketch");
    harness.let_animations_finish();

    assert!(shown);
    assert!(chosen.is_empty());
    assert_eq!(harness.editing(), None);
    assert!(!harness.shows(crate::feature_tree::DIMENSIONS_TITLE));
}

#[test]
fn a_double_click_or_enter_on_a_feature_row_opens_it() {
    let mut harness = Harness::new();
    let base = feature_named(&harness, "Base sketch");
    let side = feature_named(&harness, "Side sketch");

    harness.double_click("Side sketch");
    harness.settle();
    let double_clicked = harness.editing();
    harness.perform(Action::Editing(EditingCommand::Finish));
    harness.settle();
    harness
        .context
        .memory_mut(|memory| memory.request_focus(Id::unique(("feature-row", base))));
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.settle();

    assert_eq!(double_clicked, Some(side));
    assert_eq!(harness.workspace.panels.chosen(), vec![base]);
    assert_eq!(harness.editing(), Some(base));
    assert!(harness.shows(crate::feature_tree::DIMENSIONS_TITLE));
}

#[test]
fn an_empty_tree_offers_a_new_sketch_and_the_samples() {
    let mut harness = Harness::starting(None, Document::default(), Workspace::new());
    assert!(harness.shows(crate::feature_tree::EMPTY_TREE));
    harness.button_rect(crate::feature_tree::OPEN_SAMPLE_LABEL);
    let new_sketch = harness
        .accessible
        .iter()
        .filter(|(_, node)| node.role() == Role::Button && node.label() == Some("New sketch"))
        .filter_map(|(_, node)| node.bounds())
        .max_by(|a, b| a.y0.total_cmp(&b.y0))
        .map(|bounds| {
            Pos2::new(
                ((bounds.x0 + bounds.x1) / 2.0) as f32,
                ((bounds.y0 + bounds.y1) / 2.0) as f32,
            )
        })
        .expect("the empty tree has a New sketch button");

    harness.click_screen(new_sketch);
    harness.show_new_windows();
    let choosing = harness.workspace.editing.is_choosing_plane();
    harness.perform(Action::Editing(EditingCommand::CancelNewSketch));
    harness.click(crate::feature_tree::OPEN_SAMPLE_LABEL);

    assert!(choosing);
    assert!(harness.workspace.welcome_open);
}
