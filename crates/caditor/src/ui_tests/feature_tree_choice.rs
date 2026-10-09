use caditor_document::FeatureId;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Key, Modifiers, Pos2, accesskit::Role};

use super::{
    CAMERA_SETTLE, Harness, add_block, click_with, extruded_plate, feature_named, offer, rectangle,
    run_from_palette, top_edge_along_x,
};
use crate::{commands::Command, icons, model::Action, selection::Pickable, solid_tools};

fn chosen(harness: &Harness) -> Vec<FeatureId> {
    harness.workspace.panels.chosen()
}

fn is_hidden(harness: &Harness, feature: FeatureId) -> bool {
    harness
        .document()
        .feature(feature)
        .is_some_and(|feature| feature.hidden)
}

fn clear_regions(harness: &mut Harness, feature: FeatureId) {
    let feature = harness.document().feature(feature).unwrap().clone();
    let cleared = solid_tools::clear_regions(&harness.model, &feature).unwrap();
    harness.perform(Action::Apply(cleared));
    harness.settle();
}

fn rightmost(harness: &Harness, label: &str) -> Pos2 {
    harness
        .texts
        .iter()
        .filter(|(shown, _)| shown == label)
        .map(|(_, rect)| rect.center())
        .max_by(|a, b| a.x.total_cmp(&b.x))
        .unwrap_or_else(|| panic!("'{label}' is not on screen"))
}

fn top_face_colour(harness: &mut Harness, top: Pickable) -> caditor_render::Color {
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
}

#[test]
fn a_shift_click_in_a_filtered_tree_chooses_only_the_rows_shown() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    let mut second = Sketch::new(Plane::XY);
    rectangle(&mut second, Point2::new(60.0, 0.0), Point2::new(70.0, 10.0));
    let later = harness.add_sketch(second);
    let plate = feature_named(&harness, "Plate");
    let later_name = harness.document().feature(later).unwrap().name.clone();

    harness.workspace.panels.tree_filter = "plate".to_owned();
    harness.frame();
    harness.frame();
    harness.click("Plate");
    click_with(&mut harness, &later_name, Modifiers::SHIFT);

    let range = chosen(&harness);
    assert!(range.contains(&plate) && range.contains(&later));
    assert!(
        !range.contains(&extrude),
        "the extrusion the filter hides between them is not chosen"
    );
}

#[test]
fn a_failure_the_feature_fixes_itself_offers_to_edit_it_until_it_is_open() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    clear_regions(&mut harness, extrude);
    harness.frame();

    assert!(harness.shows("No region of the sketch is chosen."));
    assert!(harness.shows("Edit Extrude 1"));
    assert!(!harness.shows("Go to Extrude 1"));

    harness.click("Edit Extrude 1");
    harness.settle();

    assert_eq!(harness.workspace.editing.solid(), Some(extrude));
    assert!(!harness.shows("Edit Extrude 1"));
}

#[test]
fn the_card_of_a_suppressed_or_rolled_back_feature_says_why_and_brings_it_back() {
    let mut harness = Harness::new();
    let side = feature_named(&harness, "Side sketch");
    let suppressed = harness
        .document()
        .suppression(&[side], true, "Suppress Side sketch");
    harness.perform(Action::Apply(suppressed));
    harness.settle();

    harness.click_button("Show details of Side sketch");
    harness.frame();
    assert!(harness.shows("Side sketch is suppressed; unsuppress it to edit it."));
    harness.click("Unsuppress");
    harness.settle();
    assert!(!harness.document().feature(side).unwrap().suppressed);
    assert_eq!(harness.model.undo_label(), Some("Unsuppress Side sketch"));

    harness.click_beside(icons::MORE, "Base sketch");
    harness.click("Roll back to here");
    harness.settle();
    harness.frame();
    assert!(
        harness.shows("Side sketch is below the rollback bar; roll forward past it to edit it.")
    );
    harness.click("Roll forward to here");
    harness.settle();
    assert!(!harness.document().is_rolled_back(side));
}

#[test]
fn hiding_takes_the_rows_chosen_in_the_tree_and_the_row_menu_shows_them_all() {
    let mut harness = Harness::new();
    let base = feature_named(&harness, "Base sketch");
    let side = feature_named(&harness, "Side sketch");
    harness.select([]);

    harness.click("Base sketch");
    click_with(&mut harness, "Side sketch", Modifiers::COMMAND);
    harness.key(Key::H, Modifiers::NONE);
    harness.settle();
    let both_hidden = is_hidden(&harness, base) && is_hidden(&harness, side);
    let hide_label = harness.model.undo_label().map(str::to_owned);

    harness.click_beside(icons::MORE, "Side sketch");
    harness.click("Show");
    harness.settle();

    assert!(both_hidden);
    assert_eq!(hide_label.as_deref(), Some("Hide 2 features"));
    assert!(!is_hidden(&harness, base) && !is_hidden(&harness, side));
    assert_eq!(harness.model.undo_label(), Some("Show 2 features"));

    run_from_palette(&mut harness, "hide or show feature");
    harness.settle();
    assert!(is_hidden(&harness, base) && is_hidden(&harness, side));
}

#[test]
fn hiding_everything_else_keeps_the_body_of_the_row_chosen() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    let base = feature_named(&harness, "Base sketch");
    harness.select([]);

    harness.workspace.panels.choose_only(extrude);
    harness.frame();
    harness.key(Key::H, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();

    assert!(!is_hidden(&harness, extrude));
    assert!(is_hidden(&harness, base));
    assert_eq!(
        harness.model.undo_label(),
        Some("Hide everything but Extrude 1")
    );
}

#[test]
fn escape_or_a_click_on_nothing_with_nothing_selected_lets_go_of_the_tree_rows() {
    let mut harness = Harness::new();
    let base = feature_named(&harness, "Base sketch");
    harness.select([]);

    harness.workspace.panels.choose_only(base);
    harness.frame();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let after_escape = chosen(&harness);

    harness.workspace.panels.choose_only(base);
    harness.frame();
    let empty = harness
        .workspace
        .viewport
        .rect()
        .expect("the view is laid out")
        .left_bottom()
        + egui::vec2(40.0, -40.0);
    harness.click_screen(empty);
    harness.frame();
    let after_click = chosen(&harness);

    assert!(after_escape.is_empty());
    assert!(after_click.is_empty());
}

#[test]
fn bodies_are_chosen_several_at_a_time_with_ctrl_and_shift() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let block = add_block(
        &mut harness,
        "Block",
        [Point2::new(60.0, 0.0), Point2::new(80.0, 20.0)],
        "10 mm",
    );
    harness.click("Bodies (2)");
    harness.frame();

    harness.click("Extrude 1");
    click_with(&mut harness, "Block", Modifiers::COMMAND);
    let toggled = chosen(&harness);
    click_with(&mut harness, "Block", Modifiers::COMMAND);
    let untoggled = chosen(&harness);
    click_with(&mut harness, "Block", Modifiers::SHIFT);
    let ranged = chosen(&harness);

    assert_eq!(toggled, vec![plate, block]);
    assert_eq!(untoggled, vec![plate]);
    assert_eq!(ranged, vec![plate, block]);
}

#[test]
fn the_row_menu_lists_what_a_feature_uses_and_what_uses_it() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    let plate = feature_named(&harness, "Plate");

    harness.click_beside(icons::MORE, "Extrude 1");
    harness.hover("Uses");
    let listed = rightmost(&harness, "Plate");
    harness.click_screen(listed);
    harness.frame();
    harness.frame();
    assert_eq!(harness.workspace.panels.selected, Some(plate));

    harness.click_beside(icons::MORE, "Plate");
    harness.hover("Used by");
    let listed = rightmost(&harness, "Extrude 1");
    harness.click_screen(listed);
    harness.frame();
    harness.frame();
    assert_eq!(harness.workspace.panels.selected, Some(extrude));

    harness.click_beside(icons::MORE, "Extrude 1");
    harness.click(&Command::CopyFeatures.title());
    harness.settle();
    harness.frame();
    assert!(harness.shows("Copied “Extrude 1”."));
    assert!(
        harness
            .clipboard
            .as_deref()
            .is_some_and(|text| text.starts_with("caditor clipboard: features"))
    );
}

#[test]
fn hovering_a_row_lights_its_body_and_hovering_a_fillet_edge_lights_the_edge() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    harness.select([]);
    harness.frame();
    let plain = top_face_colour(&mut harness, top);

    harness.hover("Extrude 1");
    let lit = top_face_colour(&mut harness, top);
    assert_ne!(lit, plain);

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
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let edge = Pickable::BlendEdge {
        feature: fillet,
        edge: front,
    };
    let edge_width = |harness: &mut Harness| {
        let built = harness.built();
        let pick = built.picks.id_of(edge);
        built
            .scene
            .batches
            .iter()
            .flat_map(|batch| batch.lines.iter())
            .filter(|line| line.pick.is_some() && line.pick == pick)
            .map(|line| line.width)
            .fold(0.0_f32, f32::max)
    };
    let before = edge_width(&mut harness);
    let row = harness
        .texts
        .iter()
        .find(|(text, _)| text.starts_with("Edge between"))
        .map(|(text, _)| text.clone())
        .expect("the fillet lists its edge");
    harness.hover(&row);
    let hovered = edge_width(&mut harness);

    assert!(hovered > before, "{hovered} > {before}");
}

#[test]
fn f8_and_shift_f8_step_through_the_failed_features_and_the_filter_finds_them() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let block = add_block(
        &mut harness,
        "Block",
        [Point2::new(60.0, 0.0), Point2::new(80.0, 20.0)],
        "10 mm",
    );
    clear_regions(&mut harness, plate);
    clear_regions(&mut harness, block);
    harness.select([]);
    let step = |harness: &mut Harness, modifiers: Modifiers| {
        harness.key(Key::F8, modifiers);
        harness.frame();
        harness.frame();
        harness.workspace.panels.selected
    };

    let first = step(&mut harness, Modifiers::NONE);
    let next = step(&mut harness, Modifiers::NONE);
    let wrapped = step(&mut harness, Modifiers::NONE);
    let back = step(&mut harness, Modifiers::SHIFT);

    assert_eq!(first, Some(plate));
    assert_eq!(next, Some(block));
    assert_eq!(wrapped, Some(plate));
    assert_eq!(back, Some(block));
    assert!(
        offer(&harness, Command::ShowPreviousFailed)
            .availability
            .is_ok()
    );

    harness.context.enable_accesskit();
    harness.workspace.panels.tree_filter = "failed".to_owned();
    harness.frame();
    harness.frame();
    let row = |harness: &Harness, name: &str| {
        harness.accessible_named(Role::Button, &format!("More actions for {name}"))
    };
    assert!(row(&harness, "Extrude 1") && row(&harness, "Block"));
    assert!(!row(&harness, "Base sketch"));
}

#[test]
fn edit_feature_opens_the_feature_of_the_one_thing_selected_when_no_row_is_chosen() {
    let mut harness = Harness::new();
    let (extrude, top) = extruded_plate(&mut harness);
    harness.workspace.panels.selected = None;

    harness.select([top]);
    harness.key(Key::E, Modifiers::NONE);
    harness.settle();
    let opened = harness.workspace.editing.solid();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();

    let base = feature_named(&harness, "Base sketch");
    let line = harness
        .document()
        .feature(base)
        .and_then(|feature| feature.kind.sketch())
        .and_then(|sketch| sketch.entities().next())
        .map(|(entity, _)| entity)
        .unwrap();
    harness.workspace.panels.selected = None;
    harness.select([Pickable::SketchEntity {
        feature: base,
        entity: line,
    }]);
    harness.key(Key::E, Modifiers::NONE);
    harness.settle();

    assert_eq!(opened, Some(extrude));
    assert_eq!(harness.editing(), Some(base));
}
