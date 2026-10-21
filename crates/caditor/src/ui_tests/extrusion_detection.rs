use caditor_document::{BodyOperation, ExtrudeEnd, ExtrudeExtent, FeatureId, SolidFeature};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::Sketch;
use egui::{Id, Key};

use super::{CAMERA_SETTLE, Harness, extruded_plate, rectangle};
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
        harness.click_at(Point2::new(x, y));
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
    harness.type_into_field(Id::unique(("solid-field", "distance", plate)), "4 mm");
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
    assert!(
        harness
            .shows_containing("The cut passes right through Extrude 1, so it ends at Through all.")
    );
    assert!(harness.shows_containing("End in the panel sets a distance instead."));
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
    assert!(!harness.shows_containing("passes right through"));
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

#[test]
fn a_sketch_on_a_face_larger_than_it_adds_to_the_body_instead_of_cutting_it() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);

    let flange = extruded_from_the_top_face(&mut harness, top, [(-10.0, -10.0), (50.0, 50.0)]);

    assert_eq!(harness.solid(flange).operation(), BodyOperation::Add(plate));
    assert!(matches!(
        extent(&harness, flange),
        ExtrudeExtent::OneSide {
            reversed: false,
            ..
        }
    ));
    assert!(harness.shows_containing("spans the whole face"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn a_sketch_larger_than_the_boss_it_lies_on_adds_even_though_the_body_is_wider() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let mut raised = Sketch::new(
        Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap(),
    );
    rectangle(
        &mut raised,
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 20.0),
    );
    harness.add_sketch(raised);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let boss = harness.workspace.editing.solid().expect("the boss is open");
    harness.key(Key::Escape, egui::Modifiers::NONE);
    harness.frame();
    harness.frame();
    let boss_top = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            pickable.describe(harness.document(), harness.model.evaluation())
                == "Extrude 1 › Extrude 2 end face"
        })
        .expect("the boss top is pickable");

    let flange = extruded_from_the_top_face(&mut harness, boss_top, [(5.0, 5.0), (25.0, 25.0)]);

    assert_eq!(harness.solid(boss).operation(), BodyOperation::Add(plate));
    assert_eq!(harness.solid(flange).operation(), BodyOperation::Add(plate));
    assert!(harness.shows_containing("spans the whole face"));
}

#[test]
fn a_sketch_standing_on_a_plane_that_touches_a_body_says_it_adds_to_it_and_where_to_change_that() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let mut raised = Sketch::new(
        Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap(),
    );
    raised.add_circle(Point2::new(20.0, 20.0), 6.0);
    harness.add_sketch(raised);
    harness.select([]);

    harness.click_tool("Extrude");
    harness.settle();
    let boss = harness.workspace.editing.solid().expect("the boss is open");

    assert_eq!(harness.solid(boss).operation(), BodyOperation::Add(plate));
    assert!(harness.shows_containing(
        "The extrusion meets Extrude 1, so it is added to it. Result in the panel changes that."
    ));
}

fn boss_on_the_plate(harness: &mut Harness) -> (FeatureId, f64) {
    let (plate, _) = extruded_plate(harness);
    let mut raised = Sketch::new(
        Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap(),
    );
    raised.add_circle(Point2::new(20.0, 20.0), 6.0);
    harness.add_sketch(raised);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    harness.key(Key::Escape, egui::Modifiers::NONE);
    harness.frame();
    harness.settle();
    let top = harness
        .model
        .evaluation()
        .body(plate)
        .and_then(|solid| solid.bounding_box())
        .expect("the plate has a body")
        .max()
        .z;
    (plate, top)
}

#[test]
fn a_ring_round_the_top_of_a_boss_hangs_down_from_its_rim_instead_of_meeting_it_along_an_edge() {
    let mut harness = Harness::new();
    let (plate, top) = boss_on_the_plate(&mut harness);
    let before = harness.body_volume(plate);
    let mut ring =
        Sketch::new(Plane::from_frame(Point3::new(0.0, 0.0, top), Vector3::Z, Vector3::X).unwrap());
    ring.add_circle(Point2::new(20.0, 20.0), 6.0);
    ring.add_circle(Point2::new(20.0, 20.0), 9.0);
    let sketch = harness.add_sketch(ring);
    harness.settle();
    let annulus = crate::selection::sketch_regions(harness.model.evaluation(), sketch)
        .expect("the ring has regions")
        .iter()
        .find(|region| region.region.contains(Point2::new(27.5, 20.0)))
        .map(|region| region.region.key())
        .expect("a region lies between the circles");
    harness.select([Pickable::SketchRegion {
        feature: sketch,
        region: annulus,
    }]);

    harness.click("Extrude");
    harness.settle();
    let ring = harness.workspace.editing.solid().expect("the ring is open");

    assert_eq!(harness.solid(ring).operation(), BodyOperation::Add(plate));
    assert!(matches!(
        extent(&harness, ring),
        ExtrudeExtent::OneSide { reversed: true, .. }
    ));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(harness.body_volume(plate) > before);
    assert!(harness.shows_containing("only along an edge"));
}
