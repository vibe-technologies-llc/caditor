use std::panic::{self, AssertUnwindSafe};

use egui::accesskit::Role;

use super::{Harness, feature_named};

const SHARES: usize = 4;
const FEWEST_BUTTONS: usize = 30;
const SKETCH: &str = "Base sketch";

#[derive(Clone, Copy, Debug)]
enum State {
    Idle,
    Sketching,
}

fn prepared(state: State) -> Harness {
    let mut harness = Harness::new();

    if matches!(state, State::Sketching) {
        let sketch = feature_named(&harness, SKETCH);

        harness.edit(sketch);
    }
    harness.settle();
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();
    harness
}

fn enabled_buttons(harness: &Harness) -> Vec<(String, usize)> {
    let screen = harness.context.content_rect();
    let mut labels: Vec<String> = harness
        .accessible
        .iter()
        .filter(|(_, node)| node.role() == Role::Button && !node.is_disabled())
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .filter(|label| !label.trim().is_empty())
        .collect();

    labels.sort();
    labels.dedup();
    labels
        .into_iter()
        .flat_map(|label| {
            harness
                .enabled_button_rects(&label)
                .into_iter()
                .enumerate()
                .filter(|(_, rect)| rect.is_positive() && screen.contains_rect(*rect))
                .map(|(index, _)| (label.clone(), index))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn click_share(state: State, share: usize) -> usize {
    let buttons = enabled_buttons(&prepared(state));
    let mut failed = Vec::new();
    let mut clicked = 0;

    for (label, index) in buttons.iter().skip(share).step_by(SHARES) {
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
            let mut harness = prepared(state);

            harness.click_nth_button(label, *index);
            harness.frame();
            harness.settle();
        }));

        clicked += 1;
        if outcome.is_err() {
            failed.push(format!("{label} (number {index})"));
        }
    }

    assert!(
        failed.is_empty(),
        "clicking these buttons panicked or dropped a command in {state:?}: {failed:?}"
    );
    clicked
}

fn click_every_button_of(share: usize) {
    let clicked = click_share(State::Idle, share) + click_share(State::Sketching, share);

    assert!(
        clicked >= FEWEST_BUTTONS / SHARES,
        "only {clicked} buttons were clicked, so the sweep checks too little"
    );
}

#[test]
fn clicking_every_enabled_button_in_the_first_quarter_neither_panics_nor_drops_a_command() {
    click_every_button_of(0);
}

#[test]
fn clicking_every_enabled_button_in_the_second_quarter_neither_panics_nor_drops_a_command() {
    click_every_button_of(1);
}

#[test]
fn clicking_every_enabled_button_in_the_third_quarter_neither_panics_nor_drops_a_command() {
    click_every_button_of(2);
}

#[test]
fn clicking_every_enabled_button_in_the_fourth_quarter_neither_panics_nor_drops_a_command() {
    click_every_button_of(3);
}
