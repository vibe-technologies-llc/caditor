use caditor_document::{ExtrudeEnd, ExtrudeExtent, FeatureId, FeatureKind, SolidFeature};
use caditor_geometry::Point2;
use egui::{Id, Key, Modifiers};

use super::{CAMERA_SETTLE, Harness, draft_volume, extruded_plate};
use crate::solid_panel::CUT_THROUGH_ALL;

fn open_cut_on_plate(harness: &mut Harness) -> (FeatureId, FeatureId) {
    let (plate, top) = extruded_plate(harness);
    harness.select([top]);
    harness.click("New sketch");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(20.0, 30.0));
    harness.settle();
    harness.click("Extrude");
    harness.settle();
    let cut = harness.workspace.editing.solid().expect("the cut is open");
    (plate, cut)
}

#[test]
fn a_cut_typed_far_past_the_body_still_cuts_through_it() {
    let mut harness = Harness::new();
    let (plate, cut) = open_cut_on_plate(&mut harness);
    let field = Id::unique(("solid-field", "distance", cut));

    for distance in ["20 mm", "1000 mm", "100000 mm", "999999 mm"] {
        harness.type_into_field(field, distance);
        harness.settle();
        let scene = harness.built_with_overlays(1);

        assert!(
            (harness.body_volume(plate) - 14_000.0).abs() < 1.0,
            "{distance}: {}",
            harness.body_volume(plate)
        );
        assert_eq!(harness.model.evaluation().failed_count(), 0, "{distance}");
        assert!(!scene.scene.meshes.is_empty(), "{distance}");
    }
}

#[test]
fn a_cut_being_typed_far_past_the_body_is_previewed_cutting_through_it() {
    let mut harness = Harness::new();
    let (plate, cut) = open_cut_on_plate(&mut harness);
    let field = Id::unique(("solid-field", "distance", cut));

    for distance in ["20 mm", "100000 mm"] {
        harness.draft_into_field(field, distance);
        harness.wait_until("the typed distance is previewed", |harness| {
            draft_volume(harness, plate).is_some_and(|volume| (volume - 14_000.0).abs() < 1.0)
        });
    }

    harness.key(Key::Enter, Modifiers::NONE);
    harness.settle();
    assert!((harness.body_volume(plate) - 14_000.0).abs() < 1.0);
}

#[test]
fn a_cut_typed_far_past_the_body_offers_through_all() {
    let mut harness = Harness::new();
    let (plate, cut) = open_cut_on_plate(&mut harness);
    let field = Id::unique(("solid-field", "distance", cut));

    harness.type_into_field(field, "20 mm");
    harness.settle();
    let near = harness.shows(CUT_THROUGH_ALL);
    harness.type_into_field(field, "100000 mm");
    harness.settle();
    let far = harness.shows(CUT_THROUGH_ALL);
    harness.let_animations_finish();
    harness.click(CUT_THROUGH_ALL);
    harness.settle();
    let end = match &harness.document().feature(cut).unwrap().kind {
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => extrude.extent.clone(),
        other => panic!("{other:?}"),
    };

    assert!(!near);
    assert!(far);
    assert!(matches!(
        end,
        ExtrudeExtent::OneSide {
            end: ExtrudeEnd::ThroughAll,
            ..
        }
    ));
    assert!((harness.body_volume(plate) - 14_000.0).abs() < 1.0);
    assert!(!harness.shows(CUT_THROUGH_ALL));
}
