use egui::{Id, Key, Modifiers, Pos2, Rect};

use super::{Harness, lowest_in, sketch_bar};
use crate::{editing::Tool, sketch_toolbar, toolbar};

fn ribbon(harness: &Harness) -> Rect {
    egui::containers::panel::PanelState::load(&harness.context, Id::new("toolbar"))
        .expect("the ribbon is shown")
        .outer_rect
}

const RIBBON_GROUPS: [(&str, &[&str]); 7] = [
    ("History", &["Undo", "Redo"]),
    ("Sketch", &[toolbar::NEW_SKETCH_LABEL]),
    ("Solid", &["Extrude", "Revolve", "Hole"]),
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
        "Inspect",
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
