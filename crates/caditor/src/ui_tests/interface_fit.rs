use egui::{Id, Key, Modifiers, Pos2, Rect, accesskit::Role};
use tempfile::TempDir;

use super::{
    Harness, SCREEN, assert_readable, drag_screen, edit_base_sketch, entities_of_kind, lowest_in,
    open_menus, sketch_bar, sketch_bar_problems,
};
use crate::{
    commands::Command,
    editing::Tool,
    model::{Action, Notice},
    preferences::{PreferenceChange, PreferencesCommand},
    ribbon,
    selection::Pickable,
    sketch_toolbar,
    sketch_tools::ConstraintTool,
    status_bar,
};

fn in_sketch_bar(harness: &Harness, label: &str) -> Pos2 {
    lowest_in(harness, sketch_bar(harness), label)
}

fn sketch_bar_buttons() -> Vec<String> {
    Tool::ALL
        .into_iter()
        .filter(|tool| {
            !sketch_toolbar::ARC_TOOLS.contains(tool)
                && !sketch_toolbar::CURVE_TOOLS.contains(tool)
                && !sketch_toolbar::OFF_RIBBON.contains(tool)
                && *tool != Tool::Gear
        })
        .map(Tool::label)
        .chain(
            ConstraintTool::ALL
                .into_iter()
                .filter(|tool| !sketch_toolbar::OFF_RIBBON_CONSTRAINTS.contains(tool))
                .map(ConstraintTool::label),
        )
        .chain([
            sketch_toolbar::ARC_LABEL,
            sketch_toolbar::ARC_WAYS_LABEL,
            sketch_toolbar::CURVE_LABEL,
            sketch_toolbar::CURVE_WAYS_LABEL,
            sketch_toolbar::CONSTRUCTION_LABEL,
            sketch_toolbar::MOVE_LABEL,
            sketch_toolbar::SELECT_ALL_LABEL,
            sketch_toolbar::DELETE_LABEL,
            sketch_toolbar::FINISH_LABEL,
            sketch_toolbar::CURVE_FROM_GEOMETRY_TOOLS.name,
            sketch_toolbar::COPYING_TOOLS.name,
            sketch_toolbar::MODEL_GEOMETRY_TOOLS.name,
        ])
        .map(str::to_owned)
        .chain(
            [Tool::Rectangle, Tool::Circle, Tool::Polygon, Tool::Slot]
                .map(sketch_toolbar::modes_label),
        )
        .collect()
}

#[test]
fn at_200_percent_the_side_panels_leave_the_view_a_usable_width() {
    let mut harness = Harness::new();
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(2.0),
    )));
    harness.workspace.measure.toggle();
    harness.workspace.interference.toggle();
    for _ in 0..4 {
        harness.frame();
    }

    let window = SCREEN.width() / 2.0;
    let view = harness.workspace.viewport.rect().unwrap().width();

    assert!(harness.workspace.measure.open && harness.workspace.interference.open);
    assert!(view >= window * 0.39, "{view} of {window}");
}

#[test]
fn the_sketch_bar_fits_one_row_wraps_at_200_percent_and_names_every_button() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    let base = edit_base_sketch(&mut harness);
    harness.frame();
    let line = entities_of_kind(harness.sketch(base), "Line")[0];

    let row = in_sketch_bar(&harness, "Draw").y;
    let bar = sketch_bar(&harness);
    let finish = harness.position_of(sketch_toolbar::FINISH_LABEL);
    assert!(harness.shows("Editing Base sketch"));
    assert_eq!(sketch_bar_problems(&harness, SCREEN), Vec::<String>::new());
    for caption in ["Select", "Modify", "Constrain", "Dimension"] {
        assert!(
            (in_sketch_bar(&harness, caption).y - row).abs() < 0.5,
            "{caption}"
        );
    }
    assert!(harness.position_of("Select").y < row);
    assert!(finish.x > in_sketch_bar(&harness, "Dimension").x);
    assert!(finish.y < row);
    assert_readable(&harness, "The sketch bar");
    for name in sketch_bar_buttons() {
        assert!(harness.accessible_named(Role::Button, &name), "{name}");
    }

    harness.select([Pickable::SketchEntity {
        feature: base,
        entity: line,
    }]);
    harness.frame();
    harness.frame();
    assert_eq!(sketch_bar(&harness), bar);
    let delete = harness.button_rect(sketch_toolbar::DELETE_LABEL);
    assert!(bar.contains_rect(delete));

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(2.0),
    )));
    harness.frame();
    harness.frame();
    harness.frame();
    let visible = Rect::from_min_size(Pos2::ZERO, SCREEN.size() / 2.0);
    assert_eq!(sketch_bar_problems(&harness, visible), Vec::<String>::new());
    assert!(harness.button_rect("Parallel").top() > harness.button_rect("Point").bottom());
    assert!(!harness.shows("Constrain"));
    assert!(!harness.shows("Analyse"));
    assert!(harness.shows(sketch_toolbar::FINISH_LABEL));
    assert_readable(&harness, "The sketch bar at 200%");
    for name in sketch_bar_buttons() {
        assert!(harness.accessible_named(Role::Button, &name), "{name}");
    }
}

#[test]
fn every_menu_stays_on_screen_at_the_largest_interface_size() {
    let mut harness = Harness::new();
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(2.0),
    )));
    harness.frame();
    harness.frame();
    let visible = Rect::from_min_size(Pos2::ZERO, SCREEN.size() / 2.0);

    for menu in ["File", "Edit", "View", "Model", "Sketch", "Help"] {
        harness.click(menu);
        harness.frame();
        harness.frame();
        let menus = open_menus(&harness);

        assert!(!menus.is_empty(), "{menu} did not open");
        for rect in menus {
            assert!(
                visible.expand(0.5).contains_rect(rect),
                "{menu} reaches {rect:?}, past {visible:?}"
            );
        }

        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
    }

    harness.click("Model");
    harness.frame();

    assert!(harness.shows("Bodies"));

    harness.click("Bodies");
    harness.frame();
    harness.frame();

    assert!(harness.shows("Move body"));
    for rect in open_menus(&harness) {
        assert!(visible.expand(0.5).contains_rect(rect), "{rect:?}");
    }
}

#[test]
fn dialogs_fit_the_screen_at_the_largest_interface_size() {
    let mut harness = Harness::new();
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(2.0),
    )));
    harness.frame();
    harness.perform(Action::Preferences(PreferencesCommand::ShowShortcuts));
    harness.frame();
    harness.show_new_windows();
    let visible = SCREEN.size() / 2.0;
    for label in ["Close", "Reset all shortcuts", "Keyboard shortcuts"] {
        let rect = harness
            .texts
            .iter()
            .find(|(shown, _)| shown == label)
            .unwrap_or_else(|| panic!("{label} is not on screen"))
            .1;
        assert!(
            rect.min.x >= 0.0
                && rect.min.y >= 0.0
                && rect.max.x <= visible.x
                && rect.max.y <= visible.y,
            "{label} at {rect:?}"
        );
    }
}

#[test]
fn the_bars_and_the_parameter_grid_wrap_or_shrink_rather_than_overlap_at_200_percent() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::new();

    let document = harness.document().clone();
    let name = "Bracket for the front suspension, revised after the second test.caditor";
    harness
        .model
        .replace(document, Some(dir.path().join(name)), None, false);
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(2.0),
    )));
    let notice = "The sketch could not be placed on the selected face, because the face is curved; \
                  choose a flat face or a plane";
    harness.perform(Action::Inform(Notice::error(notice)));
    harness.frame();
    harness.frame();
    let panel_id = Id::new("model");
    let panel = egui::containers::panel::PanelState::load(&harness.context, panel_id)
        .unwrap()
        .outer_rect;
    drag_screen(
        &mut harness,
        Pos2::new(panel.max.x - 1.0, 300.0),
        Pos2::new(60.0, 300.0),
    );
    let narrowed = egui::containers::panel::PanelState::load(&harness.context, panel_id)
        .unwrap()
        .outer_rect;
    for _ in 0..5 {
        harness.frame();
    }
    let settled = egui::containers::panel::PanelState::load(&harness.context, panel_id)
        .unwrap()
        .outer_rect;
    let visible = Rect::from_min_size(Pos2::ZERO, SCREEN.size() / 2.0);
    let rect_of = |label: &str| {
        harness
            .texts
            .iter()
            .find(|(shown, _)| shown == label)
            .unwrap_or_else(|| panic!("{label} is not on screen"))
            .1
    };
    let line = rect_of(status_bar::UP_TO_DATE).height();
    let shown: Vec<(&str, Rect)> = harness
        .texts
        .iter()
        .zip(&harness.text_clips)
        .map(|((text, rect), clip)| (text.as_str(), clip.intersect(*rect)))
        .filter(|(_, rect)| rect.is_positive())
        .collect();
    let overlapping: Vec<(&str, &str)> = shown
        .iter()
        .enumerate()
        .flat_map(|(index, (first, a))| {
            shown[index + 1..]
                .iter()
                .filter(move |(_, b)| a.shrink(0.5).intersects(b.shrink(0.5)))
                .map(move |(second, _)| (*first, *second))
        })
        .collect();
    let outside: Vec<&str> = harness
        .texts
        .iter()
        .filter(|(_, rect)| !visible.expand(0.5).contains_rect(*rect))
        .map(|(shown, _)| shown.as_str())
        .collect();

    assert!(narrowed.width() < panel.width(), "{narrowed:?}");
    assert_eq!(settled, narrowed);
    assert!(overlapping.is_empty(), "{overlapping:#?}");
    assert!(outside.is_empty(), "{outside:#?}");
    assert!(rect_of(notice).height() > 1.5 * line, "the notice wraps");
    assert!(rect_of(name).max.x <= rect_of("Search commands").min.x);
    harness.button_rect("Delete width");
    let deletes: Vec<Rect> = ["width", "height"]
        .iter()
        .filter_map(|name| {
            let label = format!("Delete {name}");
            harness
                .accessible
                .iter()
                .filter(|(_, node)| node.role() == Role::Button && node.label() == Some(&label))
                .find_map(|(_, node)| node.bounds())
        })
        .map(|bounds| {
            Rect::from_min_max(
                Pos2::new(bounds.x0 as f32, bounds.y0 as f32),
                Pos2::new(bounds.x1 as f32, bounds.y1 as f32),
            )
        })
        .collect();
    assert!(!deletes.is_empty());
    assert!(deletes.iter().all(|rect| rect.max.x <= narrowed.max.x));
}

#[test]
fn the_ribbons_settle_on_one_layout_at_every_interface_size() {
    let mut harness = Harness::new();
    edit_base_sketch(&mut harness);

    let unsettled: Vec<u8> = (75_u8..=200)
        .filter(|&percent| {
            harness.perform(Action::Preferences(PreferencesCommand::Change(
                PreferenceChange::Scale(f32::from(percent) / 100.0),
            )));
            harness.frame();
            harness.frame();
            harness.frame();
            let passes = harness.context.cumulative_pass_nr();
            harness.frame();
            harness.context.cumulative_pass_nr() - passes > 1
        })
        .collect();
    assert_eq!(unsettled, Vec::<u8>::new());
}

fn sketch_bar_button_starting(harness: &Harness, name: &str) -> Rect {
    let bar = sketch_bar(harness);
    harness
        .accessible
        .iter()
        .filter(|(_, node)| {
            node.role() == Role::Button && node.label().is_some_and(|label| label.starts_with(name))
        })
        .filter_map(|(_, node)| node.bounds())
        .map(|bounds| {
            Rect::from_min_max(
                Pos2::new(bounds.x0 as f32, bounds.y0 as f32),
                Pos2::new(bounds.x1 as f32, bounds.y1 as f32),
            )
        })
        .find(|rect| bar.contains(rect.center()))
        .unwrap_or_else(|| panic!("no button named '{name}' in the sketch bar"))
}

#[test]
fn the_buttons_beside_finish_sketch_keep_a_gap_from_it_at_every_interface_size() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    edit_base_sketch(&mut harness);

    let look = Command::LookAtSketch.title();
    let half_gap = ribbon::TOOL_GAP / 2.0 - 0.25;
    let overlapping: Vec<String> = (75_u8..=200)
        .step_by(5)
        .filter(|&percent| {
            harness.perform(Action::Preferences(PreferencesCommand::Change(
                PreferenceChange::Scale(f32::from(percent) / 100.0),
            )));
            harness.frame();
            harness.frame();
            harness.frame();
            let buttons = [
                sketch_bar_button_starting(&harness, sketch_toolbar::FINISH_LABEL),
                sketch_bar_button_starting(&harness, sketch_toolbar::CANCEL_LABEL),
                sketch_bar_button_starting(&harness, &look),
            ];
            buttons.iter().enumerate().any(|(index, first)| {
                buttons[index + 1..]
                    .iter()
                    .any(|second| first.expand(half_gap).intersects(second.expand(half_gap)))
            })
        })
        .map(|percent| format!("{percent}%"))
        .collect();
    assert_eq!(overlapping, Vec::<String>::new());
}
