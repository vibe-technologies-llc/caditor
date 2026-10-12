use caditor_document::{FeatureId, FeatureKind, FeatureState};
use caditor_kernel::FaceOrigin;
use egui::{Event, Key, Modifiers};

use super::{Harness, blend_of, extruded_plate, run_from_palette, top_edge_along_x};
use crate::{pasting, selection::Pickable};

fn filleted_plate(harness: &mut Harness) -> [FeatureId; 3] {
    let (plate, _) = extruded_plate(harness);
    let sketch = match &harness.document().feature(plate).unwrap().kind {
        FeatureKind::Solid(solid) => solid.sketch(),
        other => panic!("an extrusion, found {other:?}"),
    };
    let front = top_edge_along_x(harness, plate, 0.0);
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
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    harness.select([]);
    [sketch, plate, fillet]
}

fn copy_features(harness: &mut Harness, features: &[FeatureId]) -> String {
    harness.workspace.panels.choose_all(features);
    harness.frame();
    harness.events.push(Event::Copy);
    harness.settle();
    harness.frame();
    harness.clipboard.clone().expect("the features were copied")
}

#[test]
fn a_fillet_pasted_with_its_extrusion_rounds_the_copy_after_finding_its_edge_in_the_background() {
    let mut harness = Harness::new();
    let copied = filleted_plate(&mut harness);
    let before = harness.document().features().len();
    let text = copy_features(&mut harness, &copied);

    harness.model.hold_pastes(true);
    harness.events.push(Event::Paste(text));
    harness.frame();
    harness.frame();
    let waiting = harness.document().features().len();
    let shown = harness.shows_containing("Pasting 3 features");
    harness.model.hold_pastes(false);
    harness.model.finish_pasting();
    harness.settle();
    harness.frame();

    let document = harness.document();
    let pasted: Vec<FeatureId> = document
        .features()
        .skip(before)
        .map(|feature| feature.id())
        .collect();
    let [_, plate_copy, fillet_copy] = pasted[..] else {
        panic!("three features pasted, found {pasted:?}");
    };
    let picked = blend_of(&harness, fillet_copy).edges[0];
    let state = harness
        .model
        .evaluation()
        .feature(fillet_copy)
        .map(|status| status.state.clone());
    let rounded = harness
        .model
        .evaluation()
        .body(plate_copy)
        .unwrap()
        .faces()
        .any(|(_, face)| {
            face.origin()
                == Some(FaceOrigin::Fillet {
                    feature: fillet_copy.raw(),
                })
        });

    assert_eq!(waiting, before);
    assert!(shown, "the status bar says what is being pasted");
    assert_eq!(harness.model.undo_label(), Some("Paste 3 features"));
    assert_eq!(state, Some(FeatureState::UpToDate));
    assert!(rounded);
    assert!(
        picked
            .origins()
            .iter()
            .flatten()
            .all(|origin| origin.feature() == plate_copy.raw())
    );
    assert_eq!(harness.workspace.panels.chosen(), pasted);
    assert!(
        harness.model.notice().is_none(),
        "{:?}",
        harness.model.notice()
    );

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    assert_eq!(harness.document().features().len(), before);
}

#[test]
fn a_paste_still_finding_its_picks_can_be_cancelled_and_adds_nothing() {
    let mut harness = Harness::new();
    let copied = filleted_plate(&mut harness);
    let before = harness.document().features().len();
    let text = copy_features(&mut harness, &copied);

    harness.model.hold_pastes(true);
    harness.events.push(Event::Paste(text.clone()));
    harness.frame();
    harness.events.push(Event::Paste(text));
    harness.frame();
    harness.frame();
    let refused = harness.shows_containing(pasting::STILL_PASTING);
    run_from_palette(&mut harness, "Cancel the paste");
    harness.frame();
    harness.model.hold_pastes(false);
    harness.settle();

    assert!(refused, "a second paste waits for the first");
    assert!(harness.shows(pasting::STOPPED));
    assert!(!harness.model.is_pasting());
    assert_eq!(harness.document().features().len(), before);
}
