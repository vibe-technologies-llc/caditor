use caditor_document::{BodyOperation, ExtrudeEnd, ExtrudeExtent, FeatureId, SolidFeature};
use egui::{Id, Key};

use super::{CAMERA_SETTLE, Harness, extruded_plate};
use crate::{last_values::LastValues, selection::Pickable};

fn extent(harness: &Harness, feature: FeatureId) -> ExtrudeExtent {
    match harness.solid(feature) {
        SolidFeature::Extrude(extrude) => extrude.extent.clone(),
        SolidFeature::Revolve(_) | SolidFeature::Sweep(_) => panic!("expected an extrusion"),
    }
}

fn extruded_from_the_top_face(
    harness: &mut Harness,
    top: Pickable,
    corners: [(f64, f64); 2],
) -> FeatureId {
    harness.select([top]);
    harness.click("New sketch");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.use_tool(Key::R);
    for (x, y) in corners {
        harness.click_at(caditor_geometry::Point2::new(x, y));
    }
    harness.settle();
    harness.click("Extrude");
    harness.settle();
    harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open")
}

fn top_face(harness: &mut Harness) -> Pickable {
    harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            pickable.describe(harness.document(), harness.model.evaluation())
                == "Extrude 1 › Extrude 1 end face"
        })
        .expect("the top face is pickable")
}

#[test]
fn a_pocket_the_default_distance_clears_the_body_with_runs_through_all() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    harness.perform(crate::model::Action::Editing(
        crate::editing::EditingCommand::OpenSolid(plate),
    ));
    harness.type_into_field(Id::new(("solid-field", "distance", plate)), "4 mm");
    harness.settle();
    harness.key(Key::Escape, egui::Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness.model.set_last_values(&LastValues::default());
    let top = top_face(&mut harness);

    let pocket = extruded_from_the_top_face(&mut harness, top, [(10.0, 10.0), (20.0, 30.0)]);

    assert_eq!(
        harness.solid(pocket).operation(),
        BodyOperation::Remove(plate)
    );
    assert!(matches!(
        extent(&harness, pocket),
        ExtrudeExtent::OneSide {
            end: ExtrudeEnd::ThroughAll,
            reversed: true
        }
    ));
    assert!((harness.body_volume(plate) - (6400.0 - 800.0)).abs() < 1.0);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn a_pocket_shallower_than_the_body_keeps_its_distance() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);

    let pocket = extruded_from_the_top_face(&mut harness, top, [(10.0, 10.0), (20.0, 30.0)]);

    assert_eq!(
        harness.solid(pocket).operation(),
        BodyOperation::Remove(plate)
    );
    assert!(matches!(
        extent(&harness, pocket),
        ExtrudeExtent::OneSide {
            end: ExtrudeEnd::Distance(_),
            reversed: true
        }
    ));
}

#[test]
fn a_sketch_on_a_face_drawn_off_the_body_does_not_cut_it() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);

    let beside = extruded_from_the_top_face(&mut harness, top, [(50.0, 10.0), (60.0, 20.0)]);

    assert_eq!(harness.solid(beside).operation(), BodyOperation::NewBody);
    assert!(matches!(
        extent(&harness, beside),
        ExtrudeExtent::OneSide {
            reversed: false,
            ..
        }
    ));
    assert!(harness.shows_containing("so the extrusion does not cut into it"));
    assert!((harness.body_volume(plate) - 16000.0).abs() < 1.0);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}
