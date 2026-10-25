use std::panic::{self, AssertUnwindSafe};

use egui::{LayerId, Rect};

use super::{
    Harness, WINDOW_SETTLE_FRAMES,
    large_interface::{SLACK, overlaps, shown_within},
    sample_document,
};
use crate::{app::Workspace, commands::Command};

const SHARES: usize = 4;
const SMALLEST: f32 = 0.75;
const LARGEST: f32 = 2.0;
const FEWEST_WINDOWS_PER_SHARE: usize = 1;

fn at_scale(scale: f32) -> Harness {
    let mut workspace = Workspace::new();
    workspace.preferences.appearance.scale = scale;
    let mut harness = Harness::starting(None, sample_document().unwrap(), workspace);
    harness.settle();
    harness
}

fn layers(harness: &Harness) -> Vec<(LayerId, Rect)> {
    harness.context.memory(|memory| {
        memory
            .areas()
            .visible_layer_ids()
            .into_iter()
            .filter_map(|layer| Some((layer, memory.area_rect(layer.id)?)))
            .collect()
    })
}

fn run(harness: &mut Harness, command: Command) -> Vec<Rect> {
    let before = layers(harness);
    harness.workspace.queue_command(command);
    for _ in 0..WINDOW_SETTLE_FRAMES {
        harness.frame();
    }
    harness.settle();
    layers(harness)
        .into_iter()
        .filter(|(layer, _)| !before.iter().any(|(earlier, _)| earlier == layer))
        .map(|(_, rect)| rect)
        .filter(|rect| rect.is_positive())
        .collect()
}

fn is_unexplained(harness: &Harness, command: Command) -> bool {
    harness
        .workspace
        .last_offers
        .iter()
        .find(|offer| offer.command == command)
        .is_some_and(|offer| {
            offer
                .availability
                .as_ref()
                .err()
                .is_some_and(|reason| reason.trim().is_empty())
        })
}

fn problems(harness: &Harness, window: Rect) -> Vec<String> {
    let screen = harness.context.content_rect();
    let shown = shown_within(harness, window);
    let outside = (!screen.expand(SLACK).contains_rect(window))
        .then(|| format!("the window {window:?} runs off the screen {screen:?}"));
    let cut = shown
        .iter()
        .filter(|(_, rect, clip)| {
            rect.min.x < clip.min.x - SLACK || rect.max.x > clip.max.x + SLACK
        })
        .map(|(text, rect, clip)| format!("{text:?} {rect:?} is cut off at the side by {clip:?}"));
    outside
        .into_iter()
        .chain(cut)
        .chain(overlaps(&shown))
        .collect()
}

fn sweep_share(share: usize) {
    let mut panicked = Vec::new();
    let mut unexplained = Vec::new();
    let mut broken = Vec::new();
    let mut opening_windows = 0;

    for command in Command::all().skip(share).step_by(SHARES) {
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
            let mut harness = Harness::new();
            let opened = run(&mut harness, command);
            (is_unexplained(&harness, command), !opened.is_empty())
        }));
        match outcome {
            Err(_) => panicked.push(command.id()),
            Ok((explained_badly, opens_a_window)) => {
                if explained_badly {
                    unexplained.push(command.id());
                }
                if opens_a_window {
                    opening_windows += 1;
                    for scale in [SMALLEST, LARGEST] {
                        let mut harness = at_scale(scale);
                        for window in run(&mut harness, command) {
                            for problem in problems(&harness, window) {
                                broken.push(format!("{} at {scale}: {problem}", command.id()));
                            }
                        }
                    }
                }
            }
        }
    }

    assert!(panicked.is_empty(), "these commands panicked: {panicked:?}");
    assert!(
        unexplained.is_empty(),
        "these commands are unavailable without saying why: {unexplained:?}"
    );
    assert!(broken.is_empty(), "{broken:#?}");
    assert!(
        opening_windows >= FEWEST_WINDOWS_PER_SHARE,
        "only {opening_windows} commands opened a window, so the layout sweep checks too little"
    );
}

#[test]
fn every_command_in_the_first_quarter_runs_or_is_refused_in_words_and_its_windows_fit() {
    sweep_share(0);
}

#[test]
fn every_command_in_the_second_quarter_runs_or_is_refused_in_words_and_its_windows_fit() {
    sweep_share(1);
}

#[test]
fn every_command_in_the_third_quarter_runs_or_is_refused_in_words_and_its_windows_fit() {
    sweep_share(2);
}

#[test]
fn every_command_in_the_fourth_quarter_runs_or_is_refused_in_words_and_its_windows_fit() {
    sweep_share(3);
}
