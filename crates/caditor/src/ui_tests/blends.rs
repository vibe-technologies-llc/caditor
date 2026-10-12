use caditor_document::{
    BodyOperation, Edit, ExtrudeExtent, FeatureKind, RegionChoice, SolidFeature,
};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, blend_of, draft_volume, extruded_plate, rectangle, removed_about,
    run_from_palette, top_edge_along_x,
};
use crate::{model::Action, selection::Pickable};

#[test]
fn a_fillet_radius_being_typed_is_previewed_before_it_is_entered() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let front = top_edge_along_x(&harness, plate, 0.0);
    let spandrel = |radius: f64| (1.0 - std::f64::consts::PI / 4.0) * radius * radius;
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
    let field = Id::new(("blend-size", fillet));

    harness.draft_into_field(field, "3 mm");
    harness.wait_until("the typed radius is previewed", |harness| {
        draft_volume(harness, plate).is_some()
    });

    let removed = 16000.0 - draft_volume(&harness, plate).unwrap();
    assert!((removed - 40.0 * spandrel(3.0)).abs() < 0.1 * 40.0 * spandrel(3.0));
    assert_eq!(harness.model.undo_label(), Some("Create Fillet 1"));
    assert!(removed_about(&harness, plate, 40.0 * spandrel(1.0)));
    let built = harness.built_with_meshes(1);
    assert!(built.scene.meshes.iter().any(|mesh| {
        mesh.mesh.face_count() == 7 && mesh.faces.iter().all(|face| face.pick.is_none())
    }));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(harness.model.draft_evaluation().is_none());
    assert_eq!(harness.model.undo_label(), Some("Create Fillet 1"));

    harness.draft_into_field(field, "2 mm");
    harness.wait_until("the typed radius is previewed", |harness| {
        draft_volume(harness, plate).is_some()
    });
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Edit Fillet 1"));
    assert!(removed_about(&harness, plate, 40.0 * spandrel(2.0)));
    harness.wait_until("the preview gives way to the result", |harness| {
        harness.model.draft_evaluation().is_none()
    });
}

#[test]
fn an_open_fillet_listing_long_edge_names_keeps_the_side_panel_width() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let front = top_edge_along_x(&harness, plate, 0.0);
    let view = harness
        .workspace
        .viewport
        .rect()
        .expect("the view is shown");

    harness.select([Pickable::Edge {
        body: plate,
        edge: front,
    }]);
    harness.click("Fillet");
    harness.settle();
    harness.let_animations_finish();
    let opened = harness
        .workspace
        .viewport
        .rect()
        .expect("the view is shown");

    assert!(harness.workspace.editing.solid().is_some());
    assert!(
        (opened.min.x - view.min.x).abs() < 0.5,
        "{view:?} became {opened:?}"
    );
}

#[test]
fn a_fillet_starts_from_the_selected_edge_and_takes_more_edges_clicked_in_the_view() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let front = top_edge_along_x(&harness, plate, 0.0);
    let back = top_edge_along_x(&harness, plate, 40.0);
    let spandrel = |radius: f64| (1.0 - std::f64::consts::PI / 4.0) * radius * radius;

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
    assert_eq!(harness.model.undo_label(), Some("Create Fillet 1"));
    assert_eq!(blend_of(&harness, fillet).edges.len(), 1);
    assert!(removed_about(&harness, plate, 40.0 * spandrel(1.0)));
    assert!(harness.shows("Click edges to add them or leave them out"));
    assert!(harness.shows("Radius"));

    let built = harness.built_with_meshes(1);
    assert_eq!(built.scene.meshes.len(), 1);
    assert_eq!(built.scene.meshes[0].mesh.face_count(), 7);
    assert!(
        built.scene.meshes[0]
            .faces
            .iter()
            .all(|face| face.pick.is_none())
    );
    let blend_edges = built
        .picks
        .pickables()
        .filter(|pickable| matches!(pickable, Pickable::BlendEdge { .. }))
        .count();
    assert_eq!(blend_edges, 12);

    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.click_pickable(
        Plane::XY,
        Point2::new(20.0, 20.0),
        Pickable::BlendEdge {
            feature: fillet,
            edge: back,
        },
    );
    harness.settle();
    assert_eq!(blend_of(&harness, fillet).edges.len(), 2);
    assert_eq!(harness.model.undo_label(), Some("Add an edge to Fillet 1"));
    assert!(removed_about(&harness, plate, 80.0 * spandrel(1.0)));

    harness.type_into_field(Id::new(("blend-size", fillet)), "2 mm");
    harness.settle();
    assert!(removed_about(&harness, plate, 80.0 * spandrel(2.0)));

    harness.type_into_field(Id::new(("blend-size", fillet)), "0 mm");
    assert!(harness.shows(crate::feature_fields::ABOVE_ZERO));
    assert_eq!(harness.workspace.editing.solid(), Some(fillet));

    harness.click_pickable(
        Plane::XY,
        Point2::new(20.0, 20.0),
        Pickable::BlendEdge {
            feature: fillet,
            edge: front,
        },
    );
    harness.settle();
    assert_eq!(blend_of(&harness, fillet).edges.len(), 1);
    assert_eq!(
        harness.model.undo_label(),
        Some("Leave an edge out of Fillet 1")
    );

    harness.click_leftmost("Fillet");
    harness.click_leftmost("Chamfer");
    harness.settle();
    assert_eq!(
        blend_of(&harness, fillet).kind,
        caditor_document::BlendKind::Chamfer
    );
    assert!(removed_about(&harness, plate, 40.0 * 2.0));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert_eq!(harness.workspace.editing.solid(), None);
    let chamfer_face = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            pickable.describe(harness.document(), harness.model.evaluation())
                == "Extrude 1 › Fillet 1 face"
        })
        .expect("the chamfer face is pickable and named after its feature");
    assert!(matches!(chamfer_face, Pickable::Face { .. }));
}

#[test]
fn a_chamfer_takes_two_distances_or_an_angle_and_flips_from_its_panel_or_the_palette() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let front = top_edge_along_x(&harness, plate, 0.0);
    harness.select([Pickable::Edge {
        body: plate,
        edge: front,
    }]);
    harness.click("Chamfer");
    harness.settle();
    let chamfer = harness
        .workspace
        .editing
        .solid()
        .expect("the chamfer is open");
    let value = Id::new(("blend-form", chamfer));
    let form =
        |harness: &Harness| crate::blend_tools::FormChoice::of(&blend_of(harness, chamfer).form);
    let tangent = 30f64.to_radians().tan();

    harness.click("Two");
    harness.settle();
    assert_eq!(form(&harness), crate::blend_tools::FormChoice::TwoDistances);
    assert!(harness.shows("First distance"));
    assert!(harness.shows("Second distance"));
    assert!(removed_about(&harness, plate, 40.0 * 0.5));
    harness.type_into_field(value, "3 mm");
    harness.settle();
    assert!(removed_about(&harness, plate, 40.0 * 1.5));
    harness.click("Measure from the other face");
    harness.settle();
    assert!(blend_of(&harness, chamfer).flipped);
    assert!(removed_about(&harness, plate, 40.0 * 1.5));

    run_from_palette(&mut harness, "Chamfer by a distance and an angle");
    harness.settle();
    assert_eq!(
        form(&harness),
        crate::blend_tools::FormChoice::DistanceAngle
    );
    assert!(harness.shows("Angle"));
    assert!(removed_about(&harness, plate, 40.0 * 0.5));
    harness.type_into_field(value, "30 deg");
    harness.settle();
    assert!(removed_about(&harness, plate, 40.0 * 0.5 * tangent));
    run_from_palette(&mut harness, "Flip the chamfer");
    harness.settle();
    assert!(!blend_of(&harness, chamfer).flipped);
    assert_eq!(harness.model.undo_label(), Some("Edit Chamfer 1"));
    harness.type_into_field(value, "180 deg");
    assert!(harness.shows("Enter an angle above 0° and below 180°"));

    run_from_palette(&mut harness, "Chamfer by one distance");
    harness.settle();
    assert_eq!(form(&harness), crate::blend_tools::FormChoice::Equal);
    assert!(!harness.shows("Measure from the other face"));
    assert!(removed_about(&harness, plate, 40.0 * 0.5));
}

#[test]
fn enter_confirms_a_fillet_whose_row_then_closes_and_choosing_in_the_view_keeps_the_selection() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let front = top_edge_along_x(&harness, plate, 0.0);
    let back = top_edge_along_x(&harness, plate, 40.0);
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

    assert!(harness.shows("Radius"));

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness.settle();

    assert_eq!(harness.workspace.editing.solid(), None);
    assert!(!harness.shows("Radius"));
    assert_eq!(blend_of(&harness, fillet).edges.len(), 1);

    harness.select([Pickable::Edge {
        body: plate,
        edge: back,
    }]);
    harness.click_button("Show details of Fillet 1");
    harness.frame();
    harness.click("Choose in the view");
    harness.settle();

    assert_eq!(harness.workspace.editing.solid(), Some(fillet));
    assert_eq!(blend_of(&harness, fillet).edges.len(), 2);
    assert_eq!(
        harness.model.undo_label(),
        Some("Add the selected edges to Fillet 1")
    );
    assert!(removed_about(
        &harness,
        plate,
        80.0 * (1.0 - std::f64::consts::PI / 4.0)
    ));
}

#[test]
fn a_fillet_lists_an_edge_split_by_an_earlier_cut_as_its_pieces() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let front = top_edge_along_x(&harness, plate, 0.0);
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
    let whole = crate::bodies::describe_edge(
        harness.document(),
        crate::bodies::input(harness.model.evaluation(), fillet).unwrap(),
        front,
    );
    let listed_whole = harness.shows(&whole);

    let top = Plane::from_frame(
        caditor_geometry::Point3::new(0.0, 0.0, 10.0),
        caditor_geometry::Vector3::Z,
        caditor_geometry::Vector3::X,
    )
    .unwrap();
    let mut slot = Sketch::new(top);
    rectangle(&mut slot, Point2::new(18.0, -5.0), Point2::new(22.0, 45.0));
    let last = harness.document().features().len() + 1;
    let mut transaction = harness.document().transaction("Cut a slot");
    let sketch = transaction.add_feature("Slot sketch", FeatureKind::from(slot));
    transaction.add_feature(
        "Slot",
        FeatureKind::Solid(SolidFeature::Extrude(caditor_document::Extrude {
            sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("2 mm").unwrap(), true),
            operation: BodyOperation::Remove(plate),
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    transaction.edit(Edit::MoveFeature {
        id: fillet,
        index: last,
    });
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    let input = crate::bodies::input(harness.model.evaluation(), fillet).unwrap();
    let pieces = blend_of(&harness, fillet).resolutions(harness.document(), &input.solid);
    let [caditor_document::Resolution::Pieces(pieces)] = pieces.as_slice() else {
        panic!("the slot splits the chosen edge in two: {pieces:?}");
    };
    let split = format!(
        "{}, split into 2 pieces",
        crate::bodies::describe_edge_id(harness.document(), input, pieces[0])
    );

    assert!(listed_whole);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert_eq!(pieces.len(), 2);
    assert!(harness.shows(&split), "{split} is listed");
    assert!(!harness.shows("An edge that is no longer there"));

    harness.perform(Action::Undo);
    harness.settle();
    assert!(harness.shows(&whole));
    assert!(!harness.shows(&split));
}
