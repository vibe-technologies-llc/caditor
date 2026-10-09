use std::time::Instant;

use caditor_document::Document;
use caditor_geometry::Point2;
use egui::{Event, Key, Pos2};

use super::{CAMERA_SETTLE, Harness, feature_named};
use crate::{
    app::Workspace,
    viewport::timing::{large_sketch, plate_document, sketch_document},
};

const FRAMES: u32 = 100;
const SMALL_MODEL_FRAMES: u32 = 2000;
const WARM_UP: u32 = 3;

fn started_with(document: Document) -> Harness {
    let started = Instant::now();
    let harness = Harness::starting(None, document, Workspace::new());
    eprintln!("started and settled in {:?}", started.elapsed());
    harness
}

fn time(harness: &mut Harness, name: &str, frames: u32, mut change: impl FnMut(&mut Harness, u32)) {
    for frame in 0..WARM_UP {
        change(harness, frame);
        harness.pass();
    }
    let started = Instant::now();
    for frame in 0..frames {
        change(harness, frame);
        harness.pass();
    }
    eprintln!("{name}: {:?} per frame", started.elapsed() / frames);
}

fn moving_between(positions: [Pos2; 2]) -> impl FnMut(&mut Harness, u32) {
    move |harness, frame| {
        let position = positions
            .get(frame as usize % positions.len())
            .copied()
            .unwrap_or_default();
        harness.events.push(Event::PointerMoved(position));
    }
}

#[test]
#[ignore = "a timing benchmark: cargo test --release -p caditor app_frame_costs -- --ignored --nocapture"]
fn app_frame_costs_while_editing_a_large_sketch_and_with_a_row_chosen() {
    let (document, sketch) = sketch_document(large_sketch());
    let mut harness = started_with(document);
    harness.edit(sketch);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.select([]);
    let positions = [
        harness.on_screen(Point2::new(101.0, 101.5)),
        harness.on_screen(Point2::new(103.5, 102.0)),
    ];
    time(
        &mut harness,
        "large sketch, nothing chosen, idle",
        FRAMES,
        |_, _| {},
    );
    time(
        &mut harness,
        "large sketch, nothing chosen, pointer moving",
        FRAMES,
        moving_between(positions),
    );
    harness.use_tool(Key::L);
    time(
        &mut harness,
        "large sketch, line tool, pointer moving",
        FRAMES,
        moving_between(positions),
    );

    let mut harness = started_with(plate_document());
    let plate = feature_named(&harness, "Plate");
    harness.workspace.panels.choose_only(plate);
    harness.frame();
    time(
        &mut harness,
        "plate, a tree row chosen, idle",
        SMALL_MODEL_FRAMES,
        |_, _| {},
    );
    let centre = super::SCREEN.center();
    time(
        &mut harness,
        "plate, a tree row chosen, pointer moving",
        SMALL_MODEL_FRAMES,
        moving_between([centre, centre + egui::vec2(3.0, 2.0)]),
    );
}
