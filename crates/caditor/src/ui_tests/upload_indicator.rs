use std::time::Duration;

use egui::accesskit::Role;

use super::{Harness, offer, run_from_palette};
use crate::{commands::Command, status_bar, upload_badge, upload_badge::UploadRequest};

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
    assert_eq!(spoken, 1);
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

fn upload_stopped(harness: &mut Harness, stopped: bool) {
    harness.workspace.viewport.set_upload_stopped(stopped);
    harness.frame();
}

#[test]
fn the_graphics_card_indicator_cancels_and_a_pill_offers_to_continue() {
    let mut harness = Harness::new();
    for _ in 0..SETTLING_FRAMES {
        harness.frame();
    }
    uploading(&mut harness, true);

    harness.hover("Cancel");
    let explained = harness.shows_containing("stay hidden until Continue loading");
    harness.click("Cancel");
    harness.frame();
    let stopping = harness.workspace.viewport.take_upload_request();

    harness.workspace.viewport.set_uploading(false);
    upload_stopped(&mut harness, true);
    let during_pill = harness.shows(upload_badge::STOPPED);
    let during_progress = harness.shows(upload_badge::TEXT);
    harness.click(upload_badge::CONTINUE);
    harness.frame();
    let resuming = harness.workspace.viewport.take_upload_request();

    upload_stopped(&mut harness, false);
    let after_pill = harness.shows(upload_badge::STOPPED);
    let after_button = harness.shows(upload_badge::CONTINUE);

    assert!(explained);
    assert_eq!(stopping, Some(UploadRequest::Stop));
    assert!(during_pill);
    assert!(!during_progress);
    assert_eq!(resuming, Some(UploadRequest::Resume));
    assert!(!after_pill);
    assert!(!after_button);
}

#[test]
fn loading_to_the_graphics_card_is_cancelled_and_continued_from_the_palette() {
    let mut harness = Harness::new();
    for _ in 0..SETTLING_FRAMES {
        harness.frame();
    }
    let idle = offer(&harness, Command::CancelUploading).availability;
    uploading(&mut harness, true);
    let cancelling = offer(&harness, Command::CancelUploading).availability;
    let continuing_early = offer(&harness, Command::ContinueUploading).availability;

    run_from_palette(&mut harness, "cancel loading to the graphics card");
    let stopping = harness.workspace.viewport.take_upload_request();
    harness.workspace.viewport.set_uploading(false);
    upload_stopped(&mut harness, true);
    let continuing = offer(&harness, Command::ContinueUploading).availability;
    let cancelled = offer(&harness, Command::CancelUploading).availability;

    run_from_palette(&mut harness, "continue loading to the graphics card");
    let resuming = harness.workspace.viewport.take_upload_request();

    assert!(idle.is_err());
    assert!(cancelling.is_ok());
    assert!(continuing_early.is_err());
    assert_eq!(stopping, Some(UploadRequest::Stop));
    assert!(continuing.is_ok());
    assert!(cancelled.is_err());
    assert_eq!(resuming, Some(UploadRequest::Resume));
}
