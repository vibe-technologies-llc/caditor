use caditor_document::UndoMark;
use egui::{Key, Modifiers};

use super::{Harness, extruded_plate, run_from_palette, top_edge_along_x};
use crate::{
    editing::{EditingCommand, Tool},
    icons,
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand},
    principal_tree::GROUP_TITLE,
    reference_picking::Slot,
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

fn set_when_needed(harness: &mut Harness, when_needed: bool) {
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::PrincipalWhenNeeded(when_needed),
    )));
    harness.settle();
}

#[derive(Debug, PartialEq)]
struct ModelState {
    revision: u64,
    undo: UndoMark,
    dirty: bool,
}

fn model_state(harness: &Harness) -> ModelState {
    ModelState {
        revision: harness.model.revision(),
        undo: harness.model.undo_mark(),
        dirty: harness.model.is_dirty(),
    }
}

#[test]
fn principal_geometry_shows_until_the_first_body_and_then_only_while_a_sketch_plane_is_chosen() {
    let mut harness = Harness::new();
    harness.settle();
    let before_a_body = principal_pickables(&mut harness).len();

    extruded_plate(&mut harness);
    harness.settle();
    let after_the_body = principal_pickables(&mut harness);
    let model_after_the_body = model_state(&harness);

    harness.perform(Action::Editing(EditingCommand::NewSketch(None)));
    harness.settle();
    let while_choosing = principal_pickables(&mut harness).len();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let after_escape = principal_pickables(&mut harness);
    let model_after_escape = model_state(&harness);

    harness.perform(Action::Editing(EditingCommand::NewSketch(None)));
    harness.settle();
    harness.perform(Action::Editing(EditingCommand::NewSketch(Some(
        PrincipalPlane::Xy,
    ))));
    harness.settle();
    let sketching = harness.editing().is_some();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let after_sketching = principal_pickables(&mut harness);

    assert_eq!(before_a_body, 7);
    assert!(after_the_body.is_empty());
    assert_eq!(while_choosing, 7);
    assert!(after_escape.is_empty());
    assert_eq!(model_after_escape, model_after_the_body);
    assert!(sketching);
    assert!(after_sketching.is_empty());
    assert!(harness.document().hidden_principal().next().is_none());
}

#[test]
fn an_open_mirror_shows_the_principal_geometry_and_takes_a_plane_clicked_until_it_closes() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    harness.select([Pickable::Edge {
        body: plate,
        edge: top_edge_along_x(&harness, plate, 0.0),
    }]);
    harness.click("Mirror body");
    harness.settle();
    harness.select([]);
    let mirror_open = principal_pickables(&mut harness).len();
    harness.click_pickable(
        caditor_geometry::Plane::XY,
        caditor_geometry::Point2::new(5.0, 5.0),
        Pickable::Plane(PrincipalPlane::Xz),
    );
    harness.settle();
    let across_clicked = open_mirror_plane(&harness);

    harness.click_beside(icons::CHOOSE_IN_VIEW, "Mirror across");
    harness.settle();
    let choosing = harness
        .workspace
        .editing
        .picking()
        .map(|picking| picking.slot);
    let while_choosing = principal_pickables(&mut harness).len();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let after_choosing = principal_pickables(&mut harness).len();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let after_closing = principal_pickables(&mut harness);

    assert_eq!(mirror_open, 7);
    assert_eq!(
        across_clicked,
        caditor_document::PlaneReference::Principal(caditor_document::PrincipalPlane::Xz)
    );
    assert_eq!(choosing, Some(Slot::MirrorPlane));
    assert_eq!(while_choosing, 7);
    assert_eq!(after_choosing, 7);
    assert!(harness.workspace.editing.solid().is_none());
    assert!(after_closing.is_empty());
}

#[test]
fn showing_principal_geometry_yourself_keeps_it_shown_without_changing_the_model() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.settle();
    let before = model_state(&harness);
    let tucked_away = principal_pickables(&mut harness).is_empty();

    harness.click_beside(icons::HIDE, GROUP_TITLE);
    harness.settle();
    let kept = principal_pickables(&mut harness).len();
    let after_showing = model_state(&harness);
    harness.perform(Action::Editing(EditingCommand::NewSketch(None)));
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let after_choosing = principal_pickables(&mut harness).len();

    harness.click_beside(icons::SHOW, GROUP_TITLE);
    harness.settle();
    let hidden = principal_pickables(&mut harness);
    let hide_label = harness.model.undo_label().map(str::to_owned);
    harness.perform(Action::Undo);
    harness.settle();
    let after_undo = principal_pickables(&mut harness).len();

    assert!(tucked_away);
    assert_eq!(kept, 7);
    assert_eq!(after_showing, before);
    assert_eq!(after_choosing, 7);
    assert!(hidden.is_empty());
    assert_eq!(
        hide_label.as_deref(),
        Some("Hide principal planes, axes and origin")
    );
    assert_eq!(harness.document().hidden_principal().count(), 0);
    assert_eq!(after_undo, 7);
}

#[test]
fn a_principal_plane_chosen_in_the_tree_shows_while_it_is_selected() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.settle();

    harness.click_button("Show details of Principal planes, axes and origin");
    harness.let_animations_finish();
    harness.click("XY plane");
    harness.settle();
    let xy = Pickable::Plane(PrincipalPlane::Xy);
    let shown = principal_pickables(&mut harness);

    assert!(harness.workspace.viewport.selection().contains(xy));
    assert_eq!(shown, [xy]);
}

#[test]
fn with_the_preference_off_principal_geometry_never_hides_by_itself() {
    let mut harness = Harness::new();
    set_when_needed(&mut harness, false);

    extruded_plate(&mut harness);
    harness.settle();
    let after_the_body = principal_pickables(&mut harness).len();

    run_from_palette(&mut harness, "show principal planes only when needed");
    harness.settle();
    let toggled_on = harness
        .workspace
        .preferences
        .view_details
        .principal_when_needed;
    let after_toggling = principal_pickables(&mut harness);

    assert_eq!(after_the_body, 7);
    assert!(toggled_on);
    assert!(after_toggling.is_empty());
}

fn open_mirror_plane(harness: &Harness) -> caditor_document::PlaneReference {
    let feature = harness
        .workspace
        .editing
        .solid()
        .expect("the mirror is open");
    match &harness.document().feature(feature).unwrap().kind {
        caditor_document::FeatureKind::Mirror(mirror) => mirror.plane.clone(),
        other => panic!("expected a mirror, found {other:?}"),
    }
}

#[test]
fn mirror_body_shows_the_principal_planes_while_choosing_and_takes_a_plane_clicked_there() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    harness.select([]);
    harness.settle();
    let before = principal_pickables(&mut harness);

    harness.click("Mirror body");
    harness.settle();
    let choosing_bodies = principal_pickables(&mut harness).len();
    harness.click_pickable(
        caditor_geometry::Plane::XY,
        caditor_geometry::Point2::new(5.0, 5.0),
        Pickable::Plane(PrincipalPlane::Xz),
    );
    harness.settle();
    let still_choosing = harness.workspace.editing.is_choosing_mirrored();
    harness.click_pickable(
        caditor_geometry::Plane::XY,
        caditor_geometry::Point2::new(5.0, 5.0),
        Pickable::Edge {
            body: plate,
            edge: top_edge_along_x(&harness, plate, 0.0),
        },
    );
    harness.settle();
    let across_clicked = open_mirror_plane(&harness);
    let choosing_more = principal_pickables(&mut harness).len();
    harness.click_pickable(
        caditor_geometry::Plane::XY,
        caditor_geometry::Point2::new(5.0, 5.0),
        Pickable::Plane(PrincipalPlane::Xy),
    );
    harness.settle();
    let across_changed = open_mirror_plane(&harness);
    let still_picking = harness
        .workspace
        .editing
        .picking()
        .map(|picking| picking.slot);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    harness.select([]);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let after = principal_pickables(&mut harness);

    assert!(before.is_empty());
    assert_eq!(choosing_bodies, 7);
    assert!(still_choosing);
    assert_eq!(
        across_clicked,
        caditor_document::PlaneReference::Principal(caditor_document::PrincipalPlane::Xz)
    );
    assert_eq!(choosing_more, 7);
    assert_eq!(
        across_changed,
        caditor_document::PlaneReference::Principal(caditor_document::PrincipalPlane::Xy)
    );
    assert_eq!(still_picking, Some(Slot::MirrorBodies));
    assert!(after.is_empty());
}
