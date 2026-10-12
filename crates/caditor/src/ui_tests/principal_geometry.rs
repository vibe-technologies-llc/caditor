use egui::{Key, Modifiers};

use super::Harness;
use crate::{
    editing::{EditingCommand, Tool},
    model::Action,
    selection::{Pickable, PrincipalPlane},
};

#[test]
fn p_hides_the_principal_geometry_outside_a_sketch_and_places_points_inside_one() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().id();

    harness.key(Key::P, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.document().hidden_principal().next().is_some());

    harness.key(Key::P, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let shown_again = harness.document().hidden_principal().next().is_none();
    harness.edit(base);
    harness.key(Key::P, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(shown_again);
    assert_eq!(
        harness.workspace.editing.active().map(|active| active.tool),
        Some(Tool::Point)
    );
    assert!(harness.document().hidden_principal().next().is_none());
}

fn principal_pickables(harness: &mut Harness) -> Vec<Pickable> {
    harness
        .built()
        .picks
        .pickables()
        .filter(|pickable| crate::visibility::principal(*pickable).is_some())
        .collect()
}

#[test]
fn principal_planes_axes_and_origin_hide_and_planes_return_while_choosing_one() {
    let mut harness = Harness::new();
    harness.settle();
    let shown_at_first = principal_pickables(&mut harness).len();

    harness.click_beside(crate::icons::SHOW, crate::principal_tree::GROUP_TITLE);
    harness.settle();
    let hidden_label = harness.model.undo_label().map(str::to_owned);
    let left_after_hiding = principal_pickables(&mut harness);

    harness.perform(Action::Editing(EditingCommand::NewSketch(None)));
    harness.settle();
    let offered_while_choosing = principal_pickables(&mut harness);
    harness.perform(Action::Editing(EditingCommand::CancelNewSketch));
    harness.settle();
    let left_after_choosing = principal_pickables(&mut harness).len();

    harness.key(Key::H, Modifiers::ALT);
    harness.settle();
    let shown_again = principal_pickables(&mut harness).len();
    let xy = Pickable::Plane(PrincipalPlane::Xy);
    harness.select([xy]);
    harness.key(Key::H, Modifiers::NONE);
    harness.settle();
    let after_hiding_xy = principal_pickables(&mut harness);

    assert_eq!(shown_at_first, 7);
    assert_eq!(
        hidden_label.as_deref(),
        Some("Hide principal planes, axes and origin")
    );
    assert!(left_after_hiding.is_empty());
    assert_eq!(
        offered_while_choosing,
        PrincipalPlane::ALL.map(Pickable::Plane).to_vec()
    );
    assert_eq!(left_after_choosing, 0);
    assert_eq!(shown_again, 7);
    assert_eq!(after_hiding_xy.len(), 6);
    assert!(!after_hiding_xy.contains(&xy));
    assert_eq!(harness.model.undo_label(), Some("Hide XY plane"));
    assert!(harness.workspace.viewport.selection().is_empty());
}

#[test]
fn a_principal_plane_chosen_in_the_tree_is_selected_in_the_view() {
    let mut harness = Harness::new();
    harness.click("Side sketch");

    harness.click_button("Show details of Principal planes, axes and origin");
    harness.let_animations_finish();
    harness.click("XY plane");

    assert!(
        harness
            .workspace
            .viewport
            .selection()
            .contains(Pickable::Plane(PrincipalPlane::Xy))
    );
    assert!(harness.workspace.panels.chosen().is_empty());
}
