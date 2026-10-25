use caditor_document::{PrincipalAxis, PrincipalGeometry, PrincipalShowing};
use egui::{Key, Modifiers};
use tempfile::TempDir;

use super::{
    CAMERA_SETTLE, Harness, canonical, extruded_plate, run_from_palette, top_edge_along_x,
};
use crate::{
    editing::{EditingCommand, Tool},
    files::FileCommand,
    icons,
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand},
    principal_tree::GROUP_TITLE,
    reference_picking::Slot,
    scene_palette::Contrast,
    selection::{Axis, Pickable, PrincipalPlane},
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

fn planes() -> Vec<Pickable> {
    PrincipalPlane::ALL.map(Pickable::Plane).to_vec()
}

fn axes() -> Vec<Pickable> {
    Axis::ALL.map(Pickable::Axis).to_vec()
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
    assert_eq!(offered_while_choosing, planes());
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

fn showing(harness: &Harness) -> Vec<PrincipalShowing> {
    PrincipalGeometry::ALL
        .into_iter()
        .map(|geometry| harness.document().principal_showing(geometry))
        .collect()
}

#[test]
fn principal_geometry_steps_aside_once_a_body_shows_and_returns_when_none_does() {
    let mut harness = Harness::new();
    harness.settle();
    let before_a_body = principal_pickables(&mut harness).len();

    extruded_plate(&mut harness);
    harness.select([]);
    harness.settle();
    let after_the_body = principal_pickables(&mut harness);
    run_from_palette(&mut harness, "hide or show every body");
    harness.settle();
    let bodies_hidden = principal_pickables(&mut harness).len();
    harness.perform(Action::Undo);
    harness.settle();
    let body_shown_again = principal_pickables(&mut harness);

    assert_eq!(before_a_body, 7);
    assert!(after_the_body.is_empty());
    assert_eq!(bodies_hidden, 7);
    assert!(body_shown_again.is_empty());
    assert!(
        showing(&harness)
            .iter()
            .all(|set| *set == PrincipalShowing::WhenNeeded)
    );
}

#[test]
fn a_tool_choosing_in_the_view_brings_back_only_the_kind_it_takes() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.select([]);
    harness.settle();
    let revision = harness.model.revision();

    harness.perform(Action::Editing(EditingCommand::NewSketch(None)));
    harness.settle();
    let choosing_a_sketch_plane = principal_pickables(&mut harness);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let after_escape = principal_pickables(&mut harness);

    run_from_palette(&mut harness, "circular pattern");
    harness.settle();
    let choosing = harness
        .workspace
        .editing
        .picking()
        .map(|picking| picking.slot);
    let choosing_an_axis = principal_pickables(&mut harness);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let open_without_choosing = principal_pickables(&mut harness);

    assert_eq!(choosing_a_sketch_plane, planes());
    assert!(after_escape.is_empty());
    assert_eq!(choosing, Some(Slot::PatternDirection));
    assert_eq!(choosing_an_axis, axes());
    assert!(open_without_choosing.is_empty());
    assert!(harness.model.revision() > revision);
}

#[test]
fn an_open_axis_list_shows_the_axes_and_measuring_shows_everything() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    run_from_palette(&mut harness, "circular pattern");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();

    harness.click("None chosen");
    harness.frame();
    harness.settle();
    let listing_axes = principal_pickables(&mut harness);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    harness.select([]);
    harness.click_tool("Measure");
    harness.settle();
    let measuring = principal_pickables(&mut harness).len();

    assert_eq!(listing_axes, axes());
    assert!(harness.workspace.measure.open);
    assert_eq!(measuring, 7);
}

#[test]
fn hidden_principal_geometry_is_offered_only_while_a_tool_asks_for_its_kind() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let xz = Pickable::Plane(PrincipalPlane::Xz);
    harness.perform(Action::Editing(EditingCommand::NewSketch(None)));
    harness.settle();
    harness.select([xz]);
    harness.perform(Action::Editing(EditingCommand::CancelNewSketch));
    harness.settle();
    harness.select([xz]);
    harness.settle();
    harness.key(Key::H, Modifiers::NONE);
    harness.settle();
    let hidden = harness
        .document()
        .principal_showing(PrincipalGeometry::Plane(PrincipalPlane::Xz));

    harness.select([]);
    harness.click_tool("Measure");
    harness.settle();
    let measuring = principal_pickables(&mut harness);
    harness.click_tool("Measure");
    harness.settle();
    harness.select([Pickable::Edge {
        body: plate,
        edge: top_edge_along_x(&harness, plate, 0.0),
    }]);
    harness.click("Mirror body");
    harness.settle();
    harness.click_beside(icons::CHOOSE_IN_VIEW, "Mirror across");
    harness.settle();
    let choosing_a_mirror_plane = principal_pickables(&mut harness);

    assert_eq!(hidden, PrincipalShowing::Hidden);
    assert_eq!(measuring.len(), 6);
    assert!(!measuring.contains(&xz));
    assert_eq!(choosing_a_mirror_plane, planes());
}

#[test]
fn an_open_mirror_shows_the_principal_planes_and_takes_a_plane_clicked_until_it_closes() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    harness.select([Pickable::Edge {
        body: plate,
        edge: top_edge_along_x(&harness, plate, 0.0),
    }]);
    harness.click("Mirror body");
    harness.settle();
    harness.select([]);
    let mirror_open = principal_pickables(&mut harness);
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
    let while_choosing = principal_pickables(&mut harness);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let after_choosing = principal_pickables(&mut harness);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let after_closing = principal_pickables(&mut harness);

    assert_eq!(mirror_open, planes());
    assert_eq!(
        across_clicked,
        caditor_document::PlaneReference::Principal(caditor_document::PrincipalPlane::Xz)
    );
    assert_eq!(choosing, Some(Slot::MirrorPlane));
    assert_eq!(while_choosing, planes());
    assert_eq!(after_choosing, planes());
    assert!(harness.workspace.editing.solid().is_none());
    assert!(after_closing.is_empty());
}

#[test]
fn showing_principal_geometry_yourself_is_an_undoable_change_kept_with_the_model() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let path = root.join("shown.caditor");
    let mut harness = Harness::with_directories(Some(root.as_path()));
    extruded_plate(&mut harness);
    harness.select([]);
    harness.settle();
    let tucked_away = principal_pickables(&mut harness).is_empty();

    harness.click_beside(icons::HIDE, GROUP_TITLE);
    harness.settle();
    let kept = principal_pickables(&mut harness).len();
    let show_label = harness.model.undo_label().map(str::to_owned);
    harness.perform(Action::Editing(EditingCommand::NewSketch(None)));
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let after_choosing = principal_pickables(&mut harness).len();
    harness.perform(Action::Undo);
    harness.settle();
    let after_undo = principal_pickables(&mut harness);
    harness.perform(Action::Redo);
    harness.settle();
    harness.answer_dialog(Some(path.clone()));
    harness.command(FileCommand::SaveAs);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });

    let reopened = caditor_file::load(&path).unwrap().document;
    let reopened_showing: Vec<PrincipalShowing> = PrincipalGeometry::ALL
        .into_iter()
        .map(|geometry| reopened.principal_showing(geometry))
        .collect();

    assert!(tucked_away);
    assert_eq!(kept, 7);
    assert_eq!(
        show_label.as_deref(),
        Some("Show principal planes, axes and origin")
    );
    assert_eq!(after_choosing, 7);
    assert!(after_undo.is_empty());
    assert!(
        reopened_showing
            .iter()
            .all(|set| *set == PrincipalShowing::Shown)
    );
}

#[test]
fn hiding_by_hand_holds_until_show_only_when_needed_returns_to_stepping_aside() {
    let mut harness = Harness::new();
    let x = PrincipalGeometry::Axis(PrincipalAxis::X);
    harness.key(Key::P, Modifiers::NONE);
    harness.settle();
    let hidden_without_a_body = principal_pickables(&mut harness).is_empty();
    extruded_plate(&mut harness);
    harness.select([]);
    harness.click_tool("Measure");
    harness.settle();
    let measuring_hidden = principal_pickables(&mut harness).is_empty();
    harness.click_tool("Measure");
    harness.settle();

    run_from_palette(
        &mut harness,
        "let principal planes, axes and origin show only when needed",
    );
    harness.settle();
    let label = harness.model.undo_label().map(str::to_owned);
    let after = showing(&harness);
    let tucked = principal_pickables(&mut harness).is_empty();
    harness.click_tool("Measure");
    harness.settle();
    let measuring = principal_pickables(&mut harness).len();
    run_from_palette(
        &mut harness,
        "let principal planes, axes and origin show only when needed",
    );
    harness.settle();

    assert!(hidden_without_a_body);
    assert!(measuring_hidden);
    assert_eq!(
        label.as_deref(),
        Some("Show principal planes, axes and origin only when needed")
    );
    assert!(after.iter().all(|set| *set == PrincipalShowing::WhenNeeded));
    assert!(tucked);
    assert_eq!(measuring, 7);
    assert_eq!(
        harness.document().principal_showing(x),
        PrincipalShowing::WhenNeeded
    );
    assert_eq!(harness.model.undo_label(), label.as_deref());
}

#[test]
fn a_principal_row_hovered_or_chosen_in_the_tree_shows_while_it_is() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.select([]);
    harness.settle();
    let xy = Pickable::Plane(PrincipalPlane::Xy);

    harness.click_button("Show details of Principal planes, axes and origin");
    harness.let_animations_finish();
    harness.hover("YZ plane");
    harness.settle();
    let hovered = principal_pickables(&mut harness);
    harness.click("XY plane");
    harness.settle();
    let shown = principal_pickables(&mut harness);

    assert_eq!(hovered, [Pickable::Plane(PrincipalPlane::Yz)]);
    assert!(harness.workspace.viewport.selection().contains(xy));
    assert!(shown.contains(&xy));
    assert!(harness.document().hidden_principal().next().is_none());
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

#[test]
fn principal_geometry_stepping_aside_fades_out_unpicked_and_then_is_gone() {
    let mut harness = Harness::new();
    let origin = Contrast::Standard
        .palette(harness.workspace.viewport.canvas())
        .origin;
    let drawn_origin = |harness: &mut Harness| {
        harness
            .built()
            .scene
            .markers()
            .find(|marker| {
                marker.color.red == origin.red
                    && marker.color.green == origin.green
                    && marker.color.blue == origin.blue
            })
            .map(|marker| marker.color.alpha)
    };
    harness.settle();
    let settled = drawn_origin(&mut harness);

    extruded_plate(&mut harness);
    harness.select([]);
    harness.settle();
    let fading = drawn_origin(&mut harness);
    let animating = harness.workspace.viewport.is_animating();
    let picked = principal_pickables(&mut harness);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let faded = drawn_origin(&mut harness);

    assert_eq!(settled, Some(origin.alpha));
    assert!(fading.is_some());
    assert!(animating);
    assert!(picked.is_empty());
    assert_eq!(faded, None);
    assert!(!harness.workspace.viewport.is_animating());
}

#[test]
fn a_principal_plane_seen_edge_on_is_neither_drawn_nor_picked_unless_it_is_selected() {
    let mut harness = Harness::starting(
        None,
        caditor_document::Document::default(),
        crate::app::Workspace::new(),
    );
    let xz = Pickable::Plane(PrincipalPlane::Xz);
    harness
        .workspace
        .viewport
        .look_toward(caditor_geometry::Vector3::Z);
    harness.settle();
    let from_above = principal_pickables(&mut harness);
    let edges_drawn = |harness: &mut Harness| {
        let edge = Contrast::Standard
            .palette(harness.workspace.viewport.canvas())
            .plane_edge;
        harness
            .built()
            .scene
            .lines()
            .filter(|line| line.color.red == edge.red && line.color.blue == edge.blue)
            .count()
    };
    let drawn_from_above = edges_drawn(&mut harness);
    harness.select([xz]);
    harness.settle();
    let selected = harness.workspace.viewport.selection().contains(xz);
    let lit_lines = harness.built().scene.lines().count();
    harness.select([]);
    harness.settle();
    let unlit_lines = harness.built().scene.lines().count();

    assert_eq!(from_above.len(), 5);
    assert!(from_above.contains(&Pickable::Plane(PrincipalPlane::Xy)));
    assert!(!from_above.contains(&xz));
    assert_eq!(drawn_from_above, 4);
    assert!(selected);
    assert_eq!(lit_lines, unlit_lines + 4);
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
    let choosing_bodies = principal_pickables(&mut harness);
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
    let choosing_more = principal_pickables(&mut harness);
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
    assert_eq!(choosing_bodies, planes());
    assert!(still_choosing);
    assert_eq!(
        across_clicked,
        caditor_document::PlaneReference::Principal(caditor_document::PrincipalPlane::Xz)
    );
    assert_eq!(choosing_more, planes());
    assert_eq!(
        across_changed,
        caditor_document::PlaneReference::Principal(caditor_document::PrincipalPlane::Xy)
    );
    assert_eq!(still_picking, Some(Slot::MirrorBodies));
    assert!(after.is_empty());
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
