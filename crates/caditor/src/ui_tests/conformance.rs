use egui::{Key, Modifiers, Pos2, Rect, accesskit::Role};

use super::{Harness, feature_named};
use crate::{
    commands::Command,
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand, PreferencesTab},
};

const LEAST_TARGET: f32 = 24.0;
const SKETCH: &str = "Base sketch";

#[derive(Debug, Clone)]
struct Target {
    name: String,
    role: Role,
    rect: Rect,
}

fn is_target(role: Role) -> bool {
    matches!(
        role,
        Role::Button
            | Role::CheckBox
            | Role::RadioButton
            | Role::Tab
            | Role::Link
            | Role::Slider
            | Role::ComboBox
            | Role::TextInput
            | Role::MenuItem
            | Role::TreeItem
    )
}

fn targets(harness: &Harness) -> Vec<Target> {
    let screen = harness.context.content_rect();
    harness
        .accessible
        .iter()
        .filter(|(_, node)| is_target(node.role()) && !node.is_disabled() && !node.is_hidden())
        .filter_map(|(_, node)| {
            let bounds = node.bounds()?;
            let rect = Rect::from_min_max(
                Pos2::new(bounds.x0 as f32, bounds.y0 as f32),
                Pos2::new(bounds.x1 as f32, bounds.y1 as f32),
            );
            (rect.is_positive() && screen.contains_rect(rect)).then(|| Target {
                name: node.label().unwrap_or_default().to_owned(),
                role: node.role(),
                rect,
            })
        })
        .collect()
}

fn distance_to(rect: Rect, point: Pos2) -> f32 {
    let dx = (rect.left() - point.x).max(point.x - rect.right()).max(0.0);
    let dy = (rect.top() - point.y).max(point.y - rect.bottom()).max(0.0);
    dx.hypot(dy)
}

fn is_undersized(rect: Rect) -> bool {
    rect.width() < LEAST_TARGET || rect.height() < LEAST_TARGET
}

fn fails_target_size(target: &Target, all: &[Target]) -> bool {
    if !is_undersized(target.rect) {
        return false;
    }
    let centre = target.rect.center();
    all.iter()
        .filter(|other| other.rect != target.rect)
        .any(|other| {
            let close = distance_to(other.rect, centre) < LEAST_TARGET / 2.0;
            let crowded =
                is_undersized(other.rect) && (other.rect.center() - centre).length() < LEAST_TARGET;
            close || crowded
        })
}

fn prepared() -> Harness {
    let mut harness = Harness::new();
    harness.settle();
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();
    harness
}

fn screens() -> Vec<(String, Harness)> {
    let mut screens = Vec::new();

    screens.push(("the model".to_owned(), prepared()));

    let mut sketching = Harness::new();
    let sketch = feature_named(&sketching, SKETCH);
    sketching.edit(sketch);
    sketching.settle();
    sketching.context.enable_accesskit();
    sketching.frame();
    sketching.frame();
    screens.push(("a sketch being edited".to_owned(), sketching));

    let mut palette = prepared();
    palette.workspace.queue_command(Command::Palette);
    palette.frame();
    palette.frame();
    screens.push(("the command palette".to_owned(), palette));

    for tab in PreferencesTab::ALL {
        let mut preferences = prepared();
        preferences.key(Key::Comma, Modifiers::COMMAND);
        preferences.show_new_windows();
        preferences.click(tab.label());
        preferences.frame();
        preferences.frame();
        screens.push((format!("the {} preferences", tab.label()), preferences));
    }
    screens
}

fn undersized_with_others_near(harness: &Harness) -> Vec<String> {
    let all = targets(harness);
    all.iter()
        .filter(|target| fails_target_size(target, &all))
        .map(|target| {
            format!(
                "{:?} '{}' {:.0}x{:.0} at {:.0},{:.0}",
                target.role,
                target.name,
                target.rect.width(),
                target.rect.height(),
                target.rect.left(),
                target.rect.top()
            )
        })
        .collect()
}

fn at_interface_size(scale: f32) {
    for (name, mut harness) in screens() {
        harness.perform(Action::Preferences(PreferencesCommand::Change(
            PreferenceChange::Scale(scale),
        )));
        harness.frame();
        harness.frame();
        let failures = undersized_with_others_near(&harness);
        assert!(
            failures.is_empty(),
            "{name} at {:.0}% has targets under {LEAST_TARGET} points with another target within reach: {failures:#?}",
            scale * 100.0
        );
    }
}

#[test]
fn every_target_is_at_least_24_points_or_clear_of_the_others_at_the_usual_size() {
    at_interface_size(1.0);
}

#[test]
fn every_target_is_at_least_24_points_or_clear_of_the_others_at_double_size() {
    at_interface_size(2.0);
}

#[test]
fn the_ribbon_group_titles_are_24_points_tall_on_both_ribbons() {
    for (name, harness) in screens()
        .into_iter()
        .filter(|(name, _)| name == "the model" || name == "a sketch being edited")
    {
        let titles: Vec<Target> = targets(&harness)
            .into_iter()
            .filter(|target| target.role == Role::Button && target.name.ends_with(" commands"))
            .collect();
        let short: Vec<&Target> = titles
            .iter()
            .filter(|title| is_undersized(title.rect))
            .collect();

        assert!(
            titles.len() >= 5,
            "{name} has {} group titles",
            titles.len()
        );
        assert!(short.is_empty(), "{name}: {short:#?}");
    }
}

fn focused(harness: &Harness) -> Option<(egui::Id, Option<Rect>)> {
    let id = harness.context.memory(|memory| memory.focused())?;
    let node = id.accesskit_id();
    let rect = harness
        .accessible
        .iter()
        .find(|(candidate, _)| *candidate == node)
        .and_then(|(_, node)| node.bounds())
        .map(|bounds| {
            Rect::from_min_max(
                Pos2::new(bounds.x0 as f32, bounds.y0 as f32),
                Pos2::new(bounds.x1 as f32, bounds.y1 as f32),
            )
        });
    Some((id, rect))
}

const MOST_TAB_STOPS: usize = 900;

fn walk_tab_order(harness: &mut Harness) -> (usize, Vec<String>) {
    let screen = harness.context.content_rect();
    let mut seen = Vec::new();
    let mut problems = Vec::new();
    for _ in 0..MOST_TAB_STOPS {
        harness.key(Key::Tab, Modifiers::NONE);
        harness.frame();
        harness.let_animations_finish();
        let Some((id, rect)) = focused(harness) else {
            continue;
        };
        if seen.contains(&id) {
            return (seen.len(), problems);
        }
        seen.push(id);
        match rect {
            Some(rect) if !screen.intersects(rect) => {
                problems.push(format!("focus left the screen at {rect:?}"));
            }
            Some(rect) if rect.width() < 1.0 || rect.height() < 1.0 => {
                problems.push(format!("focus on an empty target at {rect:?}"));
            }
            _ => {}
        }
    }
    problems.push(format!(
        "tab did not come round within {MOST_TAB_STOPS} stops"
    ));
    (seen.len(), problems)
}

#[test]
fn tab_comes_round_on_every_screen_without_a_trap_or_focus_off_screen() {
    for (name, mut harness) in screens() {
        let (stops, problems) = walk_tab_order(&mut harness);
        assert!(problems.is_empty(), "{name}: {problems:#?}");
        assert!(stops >= 1, "{name} offers no tab stop");
    }
}

fn spoken_name(harness: &Harness, node: &egui::accesskit::Node) -> String {
    let own = node.label().or_else(|| node.value()).unwrap_or_default();
    if !own.trim().is_empty() {
        return own.to_owned();
    }
    node.labelled_by()
        .iter()
        .filter_map(|label| harness.accessible.iter().find(|(id, _)| id == label))
        .filter_map(|(_, label)| label.label().or_else(|| label.value()))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn every_control_on_every_screen_has_a_name_a_screen_reader_can_speak() {
    for (name, harness) in screens() {
        let unnamed = unnamed_controls(&harness);

        assert!(
            unnamed.is_empty(),
            "{name} has unnamed controls: {unnamed:#?}"
        );
    }
}

const SHARES: usize = 4;
const SETTLE_FRAMES: usize = 4;

fn unnamed_controls(harness: &Harness) -> Vec<String> {
    harness
        .accessible
        .iter()
        .filter(|(_, node)| is_target(node.role()) && !node.is_hidden())
        .filter(|(_, node)| {
            node.bounds()
                .is_some_and(|bounds| bounds.x1 > bounds.x0 && bounds.y1 > bounds.y0)
        })
        .filter(|(_, node)| spoken_name(harness, node).trim().is_empty())
        .map(|(_, node)| format!("{:?} at {:?}", node.role(), node.bounds()))
        .collect()
}

fn opened_by(command: Command) -> Harness {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    harness.workspace.queue_command(command);
    for _ in 0..SETTLE_FRAMES {
        harness.frame();
    }
    harness.settle();
    harness
}

const FEWEST_CONTROLS_PER_SHARE: usize = 500;

fn every_window_is_named_and_sized(share: usize) {
    let mut problems = Vec::new();
    let mut checked = 0;
    for command in Command::all().skip(share).step_by(SHARES) {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let harness = opened_by(command);
            (
                targets(&harness).len(),
                unnamed_controls(&harness),
                undersized_with_others_near(&harness),
            )
        }));
        if let Ok((count, unnamed, small)) = outcome {
            checked += count;
            let id = command.id();
            problems.extend(
                unnamed
                    .into_iter()
                    .map(|node| format!("{id}: unnamed {node}")),
            );
            problems.extend(small.into_iter().map(|node| format!("{id}: small {node}")));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
    assert!(
        checked >= FEWEST_CONTROLS_PER_SHARE,
        "only {checked} controls were checked"
    );
}

#[test]
fn every_window_of_the_first_quarter_names_its_controls_and_sizes_its_targets() {
    every_window_is_named_and_sized(0);
}

#[test]
fn every_window_of_the_second_quarter_names_its_controls_and_sizes_its_targets() {
    every_window_is_named_and_sized(1);
}

#[test]
fn every_window_of_the_third_quarter_names_its_controls_and_sizes_its_targets() {
    every_window_is_named_and_sized(2);
}

#[test]
fn every_window_of_the_fourth_quarter_names_its_controls_and_sizes_its_targets() {
    every_window_is_named_and_sized(3);
}
