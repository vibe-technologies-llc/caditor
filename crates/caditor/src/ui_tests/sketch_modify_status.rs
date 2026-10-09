use caditor_document::FeatureId;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, EntityId, Sketch};
use egui::{Key, Modifiers, accesskit::Role};

use super::{
    Harness, drag_screen, edit_base_sketch, edit_free_sketch, entities_of_kind, entity_pickables,
    mirror_fixture, rectangle, run_from_palette, sketch_bar, sketch_bar_problems, type_point,
};
use crate::{
    editing::Tool,
    filleting, mirroring, patterning,
    preferences::{PreferenceChange, PreferencesCommand},
    selection::Pickable,
    sketch_status, sketch_toolbar,
};

fn arc_radii(sketch: &Sketch) -> Vec<f64> {
    entities_of_kind(sketch, "Arc")
        .into_iter()
        .filter_map(|arc| sketch.arc(arc))
        .map(|arc| arc.radius)
        .collect()
}

fn selected(harness: &Harness) -> Vec<Pickable> {
    let mut chosen: Vec<Pickable> = harness.workspace.viewport.selection().iter().collect();
    chosen.sort();
    chosen
}

fn sorted(mut pickables: Vec<Pickable>) -> Vec<Pickable> {
    pickables.sort();
    pickables
}

fn plate(harness: &mut Harness) -> FeatureId {
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::ZERO, Point2::new(40.0, 20.0));
    edit_free_sketch(harness, sketch)
}

#[test]
fn a_sketch_fillet_rounds_every_selected_corner_in_one_undoable_step_and_names_what_it_left_out() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::ZERO, Point2::new(40.0, 20.0));
    let lone = sketch.add_point(Point2::new(60.0, 10.0));
    let lone_label = sketch.entity_label(lone);
    let lines = entities_of_kind(&sketch, "Line");
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    let chosen: Vec<EntityId> = lines.iter().copied().chain([lone]).collect();
    harness.select(entity_pickables(feature, &chosen));
    harness.use_tool(Key::B);
    harness.frame();

    assert_eq!(harness.tool(), Some(Tool::Fillet));
    assert!(harness.shows(filleting::RADIUS_PROMPT));
    assert!(harness.shows_containing("Round 4 corners"));
    assert!(harness.shows_containing(&format!(
        "1 selected item left out: {lone_label} is not where two lines or arcs meet"
    )));

    type_point(&mut harness, "3");
    let radii = arc_radii(harness.sketch(feature));

    assert_eq!(radii.len(), 4);
    assert!(radii.iter().all(|radius| (radius - 3.0).abs() < 1e-6));
    assert_eq!(
        harness.model.undo_label(),
        Some(filleting::CORNERS_TRANSACTION)
    );

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn a_sketch_chamfer_cuts_both_corners_between_two_selected_lines_and_their_neighbours() {
    let mut harness = Harness::new();
    let feature = plate(&mut harness);
    let lines = entities_of_kind(harness.sketch(feature), "Line");

    harness.select(entity_pickables(feature, &lines[..3]));
    harness.use_tool_with(Key::B, Modifiers::SHIFT);
    harness.frame();
    assert!(harness.shows_containing("Cut 2 corners"));
    type_point(&mut harness, "2");

    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 6);
    assert_eq!(
        harness.model.undo_label(),
        Some(filleting::CHAMFER_CORNERS_TRANSACTION)
    );
}

#[test]
fn a_sketch_fillet_offers_the_last_radius_again_and_enter_reuses_it_after_a_tool_change() {
    let mut harness = Harness::new();
    let feature = plate(&mut harness);

    harness.use_tool(Key::B);
    harness.click_at(Point2::new(40.0, 0.0));
    type_point(&mut harness, "4");
    harness.click_at(Point2::new(0.0, 20.0));

    assert!(harness.shows_hint("Enter: round it with 4, as last time"));

    harness.key(Key::Enter, Modifiers::NONE);
    harness.settle();
    harness.use_tool(Key::L);
    harness.use_tool(Key::B);
    harness.click_at(Point2::new(0.0, 0.0));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.settle();
    let radii = arc_radii(harness.sketch(feature));

    assert_eq!(radii.len(), 3);
    assert!(radii.iter().all(|radius| (radius - 4.0).abs() < 1e-6));
}

#[test]
fn mirror_with_nothing_selected_takes_clicks_and_a_box_then_the_line_after_enter() {
    let mut harness = Harness::new();
    let (sketch, mirror, line, circle) = mirror_fixture();
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.use_tool(Key::Y);
    assert!(harness.shows(mirroring::SELECT_FIRST));
    harness.click_pickable(
        Plane::XY,
        Point2::new(10.0, 5.0),
        Pickable::SketchEntity {
            feature,
            entity: line,
        },
    );
    assert_eq!(selected(&harness), entity_pickables(feature, &[line]));
    assert!(harness.shows(mirroring::SELECT_MORE));

    let from = harness.on_screen(Point2::new(5.0, -5.0));
    let to = harness.on_screen(Point2::new(15.0, -15.0));
    drag_screen(&mut harness, from, to);
    assert_eq!(
        selected(&harness),
        sorted(entity_pickables(feature, &[line, circle]))
    );

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(mirroring::PROMPT));
    harness.click_at(Point2::new(0.0, 20.0));

    let mirrored = harness.sketch(feature);
    assert_eq!(entities_of_kind(mirrored, "Line").len(), 3);
    assert_eq!(entities_of_kind(mirrored, "Circle").len(), 2);
    assert_eq!(harness.model.undo_label(), Some(mirroring::TRANSACTION));
    assert!(!selected(&harness).contains(&Pickable::SketchEntity {
        feature,
        entity: mirror
    }));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(mirroring::SELECT_MORE));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert!(harness.workspace.viewport.selection().is_empty());
    assert_eq!(harness.tool(), Some(Tool::Mirror));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Select));
}

#[test]
fn a_circular_pattern_selects_in_the_tool_then_takes_its_centre_after_enter() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(10.0, 0.0), 2.0);
    let feature = edit_free_sketch(&mut harness, sketch);

    run_from_palette(&mut harness, "repeat sketch geometry about a point");
    assert!(harness.shows(patterning::CIRCULAR_SELECT_FIRST));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    assert!(
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.contains("select what to repeat first"))
    );

    harness.click_pickable(
        Plane::XY,
        Point2::new(12.0, 0.0),
        Pickable::SketchEntity {
            feature,
            entity: circle,
        },
    );
    assert!(harness.shows(patterning::CIRCULAR_SELECT_MORE));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(patterning::CENTRE_PROMPT));
    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Space, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(patterning::CIRCULAR_PROMPT));
    type_point(&mut harness, "4");

    assert_eq!(entities_of_kind(harness.sketch(feature), "Circle").len(), 4);
    assert_eq!(harness.model.undo_label(), Some(patterning::TRANSACTION));
}

#[test]
fn a_rectangular_pattern_takes_what_a_box_drawn_in_the_tool_holds() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_circle(Point2::new(5.0, 5.0), 2.0);
    let feature = edit_free_sketch(&mut harness, sketch);

    run_from_palette(&mut harness, "repeat sketch geometry in a grid");
    assert!(harness.shows(patterning::RECTANGULAR_SELECT_FIRST));
    let from = harness.on_screen(Point2::new(2.6, 7.4));
    let to = harness.on_screen(Point2::new(7.4, 2.6));
    drag_screen(&mut harness, from, to);
    assert!(harness.shows(patterning::RECTANGULAR_PROMPT));
    type_point(&mut harness, "3 x 20");

    assert_eq!(entities_of_kind(harness.sketch(feature), "Circle").len(), 3);
}

fn open_corner(harness: &mut Harness) -> FeatureId {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::ZERO, Point2::new(20.0, 0.0));
    let feature = edit_free_sketch(harness, sketch);
    harness.context.enable_accesskit();
    harness.settle();
    harness.frame();
    feature
}

#[test]
fn the_freedom_pill_selects_what_is_still_free_and_the_open_ends_pill_selects_and_frames_them() {
    let mut harness = Harness::new();
    let feature = open_corner(&mut harness);
    let line = entities_of_kind(harness.sketch(feature), "Line")[0];
    let free = format!(
        "4 degrees of freedom left: {}",
        sketch_status::SELECT_FREE.to_lowercase()
    );
    let open = format!(
        "2 open ends: {}",
        sketch_status::SELECT_OPEN_ENDS.to_lowercase()
    );

    assert!(harness.accessible_named(Role::Button, &free));
    assert!(harness.accessible_named(Role::Button, &open));

    harness.click_button(&free);
    assert_eq!(selected(&harness), entity_pickables(feature, &[line]));

    harness.click_button(&open);
    let ends = harness
        .sketch(feature)
        .entity(line)
        .map(|entity| sorted(entity_pickables(feature, &entity.points())));
    assert_eq!(Some(selected(&harness)), ends);
}

#[test]
fn the_redundant_pill_selects_the_constraint_that_repeats_another_ready_to_delete() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    let base = edit_base_sketch(&mut harness);
    let line = entities_of_kind(harness.sketch(base), "Line")[0];
    harness.add_stored_constraint(base, Constraint::Horizontal(line));
    harness.settle();
    harness.frame();
    let name = format!(
        "1 redundant constraint: {}",
        sketch_status::SELECT_REDUNDANT.to_lowercase()
    );

    harness.click_button(&name);
    let chosen = selected(&harness);

    assert_eq!(chosen.len(), 1);
    assert!(matches!(
        chosen.first(),
        Some(Pickable::SketchConstraint { feature, .. }) if *feature == base
    ));
}

#[test]
fn the_open_ends_and_redundant_selections_are_palette_commands_saying_when_there_are_none() {
    let mut harness = Harness::new();
    let feature = open_corner(&mut harness);
    let line = entities_of_kind(harness.sketch(feature), "Line")[0];

    run_from_palette(&mut harness, "select the open ends");
    harness.frame();
    let ends = harness
        .sketch(feature)
        .entity(line)
        .map(|entity| sorted(entity_pickables(feature, &entity.points())));
    assert_eq!(Some(selected(&harness)), ends);

    run_from_palette(&mut harness, "select the redundant constraints");
    assert!(harness.shows_containing(sketch_status::NOTHING_REDUNDANT));
}

#[test]
fn the_sketch_bar_keeps_its_height_when_open_ends_and_points_beyond_appear() {
    let mut harness = Harness::new();
    let feature = plate(&mut harness);
    harness.settle();
    let bar = sketch_bar(&harness);

    let mut sketch = harness.sketch(feature).clone();
    let tail = sketch.add_line(Point2::new(50.0, 0.0), Point2::new(60.0, 0.0));
    let past = sketch.add_point(Point2::new(70.0, 0.0));
    sketch
        .add_constraint(Constraint::Coincident(past, tail))
        .unwrap();
    let changed = harness.add_sketch(sketch);
    harness.edit(changed);
    harness.settle();
    harness.frame();

    assert_eq!(sketch_bar(&harness).height(), bar.height());
    assert_eq!(
        sketch_bar_problems(&harness, super::SCREEN),
        Vec::<String>::new()
    );
}

#[test]
fn the_tools_off_the_sketch_bar_are_in_the_corner_menus_of_their_partners_at_75_percent() {
    let mut harness = Harness::new();
    plate(&mut harness);
    harness.perform(crate::model::Action::Preferences(
        PreferencesCommand::Change(PreferenceChange::Scale(0.75)),
    ));
    harness.frame();
    harness.frame();
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();

    let menus = [
        (sketch_toolbar::CORNER_TOOLS, Tool::Chamfer),
        (sketch_toolbar::CURVE_FROM_GEOMETRY_TOOLS, Tool::BlendCurve),
        (sketch_toolbar::COPYING_TOOLS, Tool::CircularPattern),
        (sketch_toolbar::MODEL_GEOMETRY_TOOLS, Tool::Intersect),
    ];
    for (partners, tool) in menus {
        harness.click_button(partners.name);
        harness.click_lowest(tool.label());
        harness.frame();
        assert_eq!(harness.tool(), Some(tool), "{}", partners.name);
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
    }
    let visible = egui::Rect::from_min_size(egui::Pos2::ZERO, super::SCREEN.size() / 0.75);
    assert_eq!(sketch_bar_problems(&harness, visible), Vec::<String>::new());
}
