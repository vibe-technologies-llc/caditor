use caditor_document::{BodyOperation, ExtrudeExtent, Feature, RegionChoice, SolidFeature};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{CAMERA_SETTLE, Harness, distance_of, extruded_plate, rectangle};
use crate::{model::Action, selection::Pickable};

#[test]
fn extruding_a_drawn_rectangle_makes_a_shaded_body_that_follows_its_distance() {
    let mut harness = Harness::new();
    let sketch = harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 30.0));
    harness.settle();

    harness.click("Extrude");
    harness.settle();

    let extrude = harness
        .document()
        .features()
        .find(|feature| feature.name == "Extrude 1")
        .map(Feature::id)
        .expect("the extrusion was created");
    assert_eq!(harness.model.undo_label(), Some("Create Extrude 1"));
    assert_eq!(harness.editing(), None);
    assert_eq!(harness.workspace.editing.solid(), Some(extrude));
    assert_eq!(harness.solid(extrude).sketch(), sketch);
    assert_eq!(harness.solid(extrude).operation(), BodyOperation::NewBody);
    assert!((harness.body_volume(extrude) - 6000.0).abs() < 1.0);
    assert!(harness.shows("Click regions of the sketch to include or leave them out"));

    let built = harness.built_with_meshes(1);
    assert!(built.scene.meshes.is_empty());
    assert_eq!(built.scene.translucent_meshes.len(), 1);
    assert_eq!(built.scene.translucent_meshes[0].mesh.face_count(), 6);
    assert!(
        built.scene.translucent_meshes[0]
            .faces
            .iter()
            .all(|face| face.color.alpha < 1.0 && face.pick.is_some())
    );
    let pickables: Vec<Pickable> = built.picks.pickables().collect();
    assert_eq!(
        pickables
            .iter()
            .filter(|pickable| matches!(pickable, Pickable::Face { .. }))
            .count(),
        6
    );
    assert_eq!(
        pickables
            .iter()
            .filter(|pickable| matches!(pickable, Pickable::Edge { .. }))
            .count(),
        12
    );
    assert_eq!(
        pickables
            .iter()
            .filter(|pickable| matches!(pickable, Pickable::Region { .. }))
            .count(),
        1
    );
    assert!(built.everything.max().z >= 10.0);

    let field = Id::new(("solid-field", "distance", extrude));
    harness.type_into_field(field, "25 mm");
    assert_eq!(distance_of(&harness, extrude), "25 mm");
    harness.settle();
    assert!((harness.body_volume(extrude) - 15000.0).abs() < 1.0);

    harness.type_into_field(field, "0 mm");
    assert!(harness.shows(crate::feature_fields::NOT_ZERO_OR_REVERSE));
    assert_eq!(distance_of(&harness, extrude), "25 mm");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(distance_of(&harness, extrude), "10 mm");

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert_eq!(harness.workspace.editing.solid(), None);
    let closed = harness.built();
    assert!(
        closed
            .picks
            .pickables()
            .all(|pickable| !matches!(pickable, Pickable::Region { .. }))
    );
    assert_eq!(closed.scene.meshes.len(), 1);
    assert!(closed.scene.translucent_meshes.is_empty());
}

#[test]
fn a_flat_rectangle_is_refused_with_the_reason() {
    let mut harness = Harness::new();
    let sketch = harness.draw_on_new_sketch();

    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 10.0));
    harness.settle();

    assert_eq!(harness.sketch(sketch).entities().len(), 0);
    assert_eq!(
        harness.model.notice().map(|notice| notice.text.as_str()),
        Some("A rectangle needs its corners apart in both directions.")
    );
}

#[test]
fn both_distances_of_a_two_sided_extrusion_must_be_above_zero() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 30.0));
    harness.settle();
    harness.click("Extrude");
    harness.settle();

    let extrude = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    harness.click("One side");
    harness.click("Two sides");
    harness.settle();
    let SolidFeature::Extrude(extruded) = harness.solid(extrude) else {
        panic!("expected an extrusion");
    };
    assert!(matches!(extruded.extent, ExtrudeExtent::TwoSides { .. }));
    let before = harness.solid(extrude).clone();
    harness.type_into_field(Id::new(("solid-field", "forward", extrude)), "0 mm");
    let refused_forward = harness.shows(crate::feature_fields::ABOVE_ZERO);
    harness.type_into_field(Id::new(("solid-field", "backward", extrude)), "-3 mm");
    let refused_backward = harness.shows(crate::feature_fields::ABOVE_ZERO);

    assert!(refused_forward);
    assert!(refused_backward);
    assert_eq!(harness.solid(extrude), &before);
}

#[test]
fn clicking_a_hole_region_adds_it_and_a_face_names_the_feature_that_made_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    rectangle(
        &mut sketch,
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 20.0),
    );
    let sketch = harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    let extrude = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    assert_eq!(harness.solid(extrude).sketch(), sketch);
    assert!((harness.body_volume(extrude) - 15000.0).abs() < 1.0);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();

    let hole = harness
        .built()
        .picks
        .pickables()
        .filter_map(|pickable| match pickable {
            Pickable::Region { region, .. } => Some(region),
            _ => None,
        })
        .find(|region| {
            let RegionChoice::All = harness.solid(extrude).regions() else {
                return false;
            };
            let regions = crate::selection::swept_regions(
                harness.document(),
                harness.model.evaluation(),
                extrude,
            )
            .unwrap()
            .1;
            regions
                .iter()
                .any(|candidate| candidate.region.key() == *region && !candidate.even_depth)
        })
        .expect("the hole is a region");
    harness.click_pickable(
        Plane::XY,
        Point2::new(15.0, 15.0),
        Pickable::Region {
            feature: extrude,
            region: hole,
        },
    );
    harness.settle();

    let RegionChoice::Chosen(references) = harness.solid(extrude).regions() else {
        panic!("the regions were not chosen");
    };
    assert_eq!(references.len(), 2);
    assert!(
        references
            .iter()
            .all(|reference| !reference.boundary().is_empty() && reference.anchor().is_some())
    );
    assert_eq!(
        harness.model.undo_label(),
        Some("Choose regions of Extrude 1")
    );
    assert!((harness.body_volume(extrude) - 16000.0).abs() < 1.0);

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let top = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            matches!(pickable, Pickable::Face { .. })
                && pickable.describe(harness.document(), harness.model.evaluation())
                    == "Extrude 1 › Extrude 1 end face"
        })
        .expect("the top face is pickable");
    harness.select([top]);
    assert!(harness.shows("Extrude 1 › Extrude 1 end face"));
    let Pickable::Face { body, .. } = top else {
        panic!("expected a face");
    };
    assert_eq!(body, extrude);
}

#[test]
fn revolving_about_a_selected_line_uses_it_as_the_axis() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XZ);
    rectangle(&mut sketch, Point2::new(10.0, 0.0), Point2::new(20.0, 10.0));
    let axis = sketch.add_line(Point2::new(0.0, -5.0), Point2::new(0.0, 15.0));
    let sketch = harness.add_sketch(sketch);
    harness.select([Pickable::SketchEntity {
        feature: sketch,
        entity: axis,
    }]);

    harness.click("Revolve");
    harness.settle();

    let revolve = harness
        .workspace
        .editing
        .solid()
        .expect("the revolution is open");
    assert_eq!(harness.solid(revolve).axis_line(), Some(axis));
    let expected = std::f64::consts::PI * (20.0f64.powi(2) - 10.0f64.powi(2)) * 10.0;
    assert!((harness.body_volume(revolve) - expected).abs() / expected < 0.01);
    assert!(harness.shows("Axis"));
}

#[test]
fn an_open_cut_shows_the_body_solid_and_only_the_material_it_removes_see_through() {
    let mut harness = Harness::new();
    let (extrude, top) = extruded_plate(&mut harness);
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
    assert_eq!(
        harness.solid(cut).operation(),
        BodyOperation::Remove(extrude)
    );

    let open = harness.built_with_overlays(1);

    assert_eq!(open.scene.meshes.len(), 1);
    assert!(open.scene.translucent_meshes.is_empty());
    assert!(
        open.scene.meshes[0]
            .faces
            .iter()
            .all(|face| face.color.alpha >= 1.0 && face.pick.is_some())
    );
    assert_eq!(open.scene.overlay_meshes[0].mesh.face_count(), 6);
    assert!(
        open.scene.overlay_meshes[0]
            .faces
            .iter()
            .all(|face| face.color.alpha < 1.0 && face.pick.is_none())
    );

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert_eq!(harness.workspace.editing.solid(), None);
    let closed = harness.built();
    assert!(closed.scene.overlay_meshes.is_empty());
    assert_eq!(closed.scene.meshes.len(), 1);
}
