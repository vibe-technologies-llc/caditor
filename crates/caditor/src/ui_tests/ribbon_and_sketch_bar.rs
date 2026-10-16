use caditor_geometry::Point2;
use egui::{Key, Modifiers, Pos2, Rect, accesskit::Role};

use super::{Harness, entities_of_kind, lowest_in, sketch_bar, top_level_panel};
use crate::{editing::Tool, sketch_toolbar, sweep_tools, toolbar};

fn button_in(harness: &mut Harness, area: Rect, name: &str) -> Rect {
    harness.read_screen();
    harness
        .accessible
        .iter()
        .filter(|(_, node)| node.role() == Role::Button && node.label() == Some(name))
        .filter_map(|(_, node)| node.bounds())
        .map(|bounds| {
            Rect::from_min_max(
                Pos2::new(bounds.x0 as f32, bounds.y0 as f32),
                Pos2::new(bounds.x1 as f32, bounds.y1 as f32),
            )
        })
        .find(|rect| area.contains(rect.center()))
        .unwrap_or_else(|| panic!("no button named '{name}' in {area:?}"))
}

#[test]
fn sweep_has_a_button_in_the_solid_group_and_a_row_in_the_model_menu() {
    let mut harness = Harness::new();
    harness.frame();

    let revolve = harness.button_rect("Revolve");
    let sweep = harness.button_rect(sweep_tools::TITLE);
    let hole = harness.button_rect("Hole");

    assert!(revolve.right() <= sweep.left() && sweep.right() <= hole.left());
    assert!(ribbon(&harness).contains_rect(sweep));

    harness.hover(sweep_tools::TITLE);
    assert!(harness.shows_containing(toolbar::SWEEP_HOVER));

    harness.click("Model");
    let row = |label: &str| lowest_in(&harness, Rect::EVERYTHING, label).y;
    assert_eq!(harness.count_shown(sweep_tools::TITLE), 2);
    assert!(row("Revolve") < row(sweep_tools::TITLE));
    assert!(row(sweep_tools::TITLE) < row("Hole"));
}

#[test]
fn the_sketch_bar_undoes_and_redoes_the_last_change() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::P);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.settle();
    let points = |harness: &Harness| entities_of_kind(harness.sketch(feature), "Point").len();
    let bar = sketch_bar(&harness);

    assert_eq!(points(&harness), 1);

    let undo = button_in(&mut harness, bar, "Undo");
    harness.click_screen(undo.center());
    harness.settle();
    assert_eq!(points(&harness), 0);

    let redo = button_in(&mut harness, bar, "Redo");
    harness.click_screen(redo.center());
    harness.settle();
    assert_eq!(points(&harness), 1);
    assert_eq!(harness.editing(), Some(feature));
}

#[test]
fn sketch_chamfer_has_its_own_button_beside_sketch_fillet() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    let bar = sketch_bar(&harness);

    let fillet = button_in(&mut harness, bar, Tool::Fillet.label());
    let chamfer = button_in(&mut harness, bar, Tool::Chamfer.label());

    assert!((fillet.top() - chamfer.top()).abs() < 0.5);
    assert!(chamfer.left() >= fillet.right());

    harness.click_screen(chamfer.center());
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Chamfer));
}

#[test]
fn a_corner_menu_lists_the_tools_it_holds_on_hover() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();

    let partners = &sketch_toolbar::CURVE_FROM_GEOMETRY_TOOLS;
    harness.hover_button(partners.name);
    let contents = partners.contents();

    assert!(harness.shows(&contents), "{contents}");
    for tool in partners.tools() {
        assert!(contents.contains(tool.label()), "{contents}");
    }

    harness.hover_button(sketch_toolbar::ARC_WAYS_LABEL);
    for tool in sketch_toolbar::ARC_TOOLS {
        assert!(harness.shows_containing(tool.label()), "{tool:?}");
    }
}

fn ribbon(harness: &Harness) -> Rect {
    egui::containers::panel::PanelState::load(&harness.context, top_level_panel("toolbar"))
        .expect("the ribbon is shown")
        .outer_rect
}

const RIBBON_GROUPS: [(&str, &[&str]); 7] = [
    ("History", &["Undo", "Redo"]),
    ("Sketch", &[toolbar::NEW_SKETCH_LABEL]),
    ("Solid", &["Extrude", "Revolve", sweep_tools::TITLE, "Hole"]),
    (
        "Modify",
        &[
            "Fillet",
            "Chamfer",
            "Shell",
            "Combine",
            "Move body",
            "Mirror body",
            "Scale body",
        ],
    ),
    ("Pattern", &["Linear pattern", "Circular pattern"]),
    ("Reference", &[toolbar::PLANE_LABEL, toolbar::AXIS_LABEL]),
    (
        "Analyse",
        &[toolbar::MEASURE_LABEL, toolbar::INTERFERENCE_LABEL],
    ),
];

fn ribbon_layout(harness: &Harness) -> Vec<Pos2> {
    let bar = ribbon(harness);
    RIBBON_GROUPS
        .iter()
        .flat_map(|(caption, buttons)| std::iter::once(caption).chain(buttons.iter()))
        .map(|label| lowest_in(harness, bar, label))
        .collect()
}

#[test]
fn every_ribbon_group_is_captioned_under_its_buttons() {
    let mut harness = Harness::new();
    harness.frame();

    let bar = ribbon(&harness);
    for (caption, buttons) in RIBBON_GROUPS {
        let below = lowest_in(&harness, bar, caption);
        let rects: Vec<Rect> = buttons
            .iter()
            .map(|button| harness.button_rect(button))
            .collect();
        let left = rects
            .iter()
            .map(|rect| rect.left())
            .fold(f32::MAX, f32::min);
        let right = rects
            .iter()
            .map(|rect| rect.right())
            .fold(f32::MIN, f32::max);

        assert!(below.x > left && below.x < right, "{caption}");
        assert!(
            rects.iter().all(|rect| below.y > rect.bottom()),
            "{caption}"
        );
    }
    harness.hover("Undo");
    assert_eq!(harness.count_shown("Undo"), 2);
    assert!(harness.shows("Nothing to undo"));
}

#[test]
fn the_ribbon_keeps_its_height_and_groups_while_choosing_a_plane() {
    let mut harness = Harness::new();
    harness.frame();
    let bar = ribbon(&harness);
    let layout = ribbon_layout(&harness);

    harness.click(toolbar::NEW_SKETCH_LABEL);
    harness.frame();
    assert!(harness.workspace.editing.is_choosing_plane());
    assert_eq!(ribbon(&harness), bar);
    assert_eq!(ribbon_layout(&harness), layout);

    harness.click(toolbar::NEW_SKETCH_LABEL);
    harness.frame();
    assert!(!harness.workspace.editing.is_choosing_plane());
    assert_eq!(ribbon(&harness), bar);
}

#[test]
fn the_arc_button_offers_each_way_to_draw_an_arc_and_keeps_the_last_chosen() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    let bar = sketch_bar(&harness);

    harness.click_button(sketch_toolbar::ARC_WAYS_LABEL);
    for tool in sketch_toolbar::ARC_TOOLS {
        assert!(harness.shows(tool.label()), "{tool:?}");
    }
    harness.click("Tangent arc");
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::TangentArc));

    harness.use_tool(Key::L);
    assert_eq!(harness.tool(), Some(Tool::Line));
    harness.click_button(sketch_toolbar::ARC_LABEL);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::TangentArc));
    assert!(!harness.shows("3-point arc"));

    harness.use_tool_with(Key::A, Modifiers::ALT);
    assert_eq!(harness.tool(), Some(Tool::ThreePointArc));
    harness.use_tool(Key::A);
    assert_eq!(harness.tool(), Some(Tool::Arc));
    harness.use_tool(Key::T);
    assert_eq!(harness.tool(), Some(Tool::TangentArc));
    assert_eq!(sketch_bar(&harness), bar);
}
