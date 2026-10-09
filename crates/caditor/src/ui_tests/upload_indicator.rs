use std::time::Duration;

use egui::accesskit::Role;

use super::Harness;
use crate::{status_bar, upload_badge};

const SETTLING_FRAMES: usize = 5;

fn nodes_reading(harness: &Harness, text: &str) -> usize {
    harness
        .accessible
        .iter()
        .filter(|(_, node)| node.role() == Role::Label && node.value() == Some(text))
        .count()
}

fn uploading(harness: &mut Harness, on: bool) {
    harness.workspace.viewport.set_uploading(on);
    harness.frame();
}

#[test]
fn the_graphics_card_indicator_shows_while_meshes_upload_and_not_otherwise() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    for _ in 0..SETTLING_FRAMES {
        harness.frame();
    }
    let idle = harness.shows(upload_badge::TEXT);

    uploading(&mut harness, true);
    let during = harness.shows(upload_badge::TEXT);
    let spoken = nodes_reading(&harness, upload_badge::TEXT);

    uploading(&mut harness, false);
    let after = harness.shows(upload_badge::TEXT);
    let still_spoken = nodes_reading(&harness, upload_badge::TEXT);

    assert!(!idle);
    assert!(during);
    assert_eq!(spoken, 2);
    assert!(!after);
    assert_eq!(still_spoken, 0);
    assert!(harness.shows(status_bar::UP_TO_DATE));
}

#[test]
fn the_graphics_card_indicator_asks_for_no_repaint_once_the_uploads_end() {
    let mut harness = Harness::new();
    harness.frame();
    for _ in 0..SETTLING_FRAMES {
        harness.frame();
    }
    let idle = harness.repaint_after;

    uploading(&mut harness, true);
    let during = harness.repaint_after;

    uploading(&mut harness, false);
    let after = harness.repaint_after;

    assert_eq!(idle, Duration::MAX);
    assert!(during < Duration::MAX);
    assert_eq!(after, Duration::MAX);
}
