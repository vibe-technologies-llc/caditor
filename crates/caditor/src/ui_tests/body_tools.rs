use std::f64::consts::PI;

use caditor_document::{BodyOperation, FeatureId, FeatureKind};
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{
    Harness, add_peg, choose, extruded, extruded_plate, pickable_described, plate_bounds,
    rectangle, run_from_palette, top_edge_along_x, vertex_at, volume_about,
};
use crate::{
    mate_panel, mirror_panel, model::Action, selection::Pickable, solid_panel, split_panel,
};

#[test]
fn a_body_is_mirrored_across_a_plane_and_keeps_or_leaves_out_its_original_from_the_panel() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);

    harness.select([Pickable::Edge {
        body: plate,
        edge: top_edge_along_x(&harness, plate, 0.0),
    }]);
    harness.click("Mirror body");
    harness.settle();
    let mirror = harness
        .workspace
        .editing
        .solid()
        .expect("the mirror is open");
    assert_eq!(harness.model.undo_label(), Some("Create Mirror body 1"));
    assert!(harness.shows("Mirror across"));
    assert!(volume_about(&harness, plate, 2.0 * 16000.0));
    let (low, high) = plate_bounds(&harness, plate);
    assert!((low.x + 40.0).abs() < 1e-6 && (high.x - 40.0).abs() < 1e-6);

    harness.click(mirror_panel::KEEP_ORIGINAL);
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Edit Mirror body 1"));
    assert!(volume_about(&harness, plate, 16000.0));
    let (low, high) = plate_bounds(&harness, plate);
    assert!((low.x + 40.0).abs() < 1e-6 && high.x.abs() < 1e-6);

    choose(&mut harness, "The YZ plane", "The XY plane");
    let (low, high) = plate_bounds(&harness, plate);
    assert!(
        (low.z + 10.0).abs() < 1e-6 && high.z.abs() < 1e-6,
        "{low:?} {high:?}"
    );

    harness.select([top]);
    run_from_palette(&mut harness, "mirror across selected");
    harness.settle();
    let (low, high) = plate_bounds(&harness, plate);
    assert!(
        (low.z - 10.0).abs() < 1e-6 && (high.z - 20.0).abs() < 1e-6,
        "{low:?} {high:?}"
    );
    assert_eq!(harness.workspace.editing.solid(), Some(mirror));

    for _ in 0..3 {
        harness.perform(Action::Undo);
    }
    harness.settle();
    assert!(volume_about(&harness, plate, 2.0 * 16000.0));
}

#[test]
fn a_body_is_split_along_a_plane_into_two_bodies_from_the_panel() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(
        &mut sketch,
        Point2::new(-20.0, -20.0),
        Point2::new(20.0, 20.0),
    );
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    let plate = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let top = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| matches!(pickable, Pickable::Edge { body, .. } if *body == plate))
        .expect("an edge of the plate is pickable");

    harness.select([top]);
    harness.use_tool_with(Key::K, Modifiers::ALT);
    harness.settle();
    let split = harness
        .workspace
        .editing
        .solid()
        .expect("the split is open");
    assert_eq!(harness.model.undo_label(), Some("Create Split 1"));
    assert!(harness.shows("Split along"));
    assert!(harness.shows("Split-off body"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    let (low, high) = plate_bounds(&harness, plate);
    assert!(low.x.abs() < 1e-6 && (high.x - 20.0).abs() < 1e-6);
    let (low, high) = plate_bounds(&harness, split);
    assert!((low.x + 20.0).abs() < 1e-6 && high.x.abs() < 1e-6);
    assert!(volume_about(&harness, split, 8000.0));

    harness.click(split_panel::KEEP_OTHER_SIDE);
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Edit Split 1"));
    let (low, high) = plate_bounds(&harness, plate);
    assert!((low.x + 20.0).abs() < 1e-6 && high.x.abs() < 1e-6);

    choose(&mut harness, "The YZ plane", "The XZ plane");
    let (low, high) = plate_bounds(&harness, plate);
    assert!(
        low.y.abs() < 1e-6 && (high.y - 20.0).abs() < 1e-6,
        "{low:?} {high:?}"
    );
    let (low, high) = plate_bounds(&harness, split);
    assert!(
        (low.y + 20.0).abs() < 1e-6 && high.y.abs() < 1e-6,
        "{low:?} {high:?}"
    );

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert_eq!(harness.workspace.editing.solid(), None);
    assert_eq!(harness.built_with_meshes(2).scene.meshes.len(), 2);
}

#[test]
fn a_body_is_split_along_a_sketch_curve_or_another_body_chosen_in_the_panel() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);
    let mut line = Sketch::new(Plane::XY);
    line.add_line(Point2::new(20.0, 10.0), Point2::new(20.0, 30.0));
    let mut transaction = harness.document().transaction("Add a cut line");
    transaction.add_feature("Cut line", FeatureKind::from(line));
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    let edge = |harness: &mut Harness, of: FeatureId| {
        harness
            .built()
            .picks
            .pickables()
            .find(|pickable| matches!(pickable, Pickable::Edge { body, .. } if *body == of))
            .expect("an edge of the body is pickable")
    };
    let plate_edge = edge(&mut harness, plate);
    let peg_edge = edge(&mut harness, peg);

    harness.select([plate_edge, peg_edge]);
    harness.use_tool_with(Key::K, Modifiers::ALT);
    harness.settle();
    let split = harness
        .workspace
        .editing
        .solid()
        .expect("the split is open");
    assert_eq!(harness.model.undo_label(), Some("Create Split 1"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(volume_about(&harness, plate, 16000.0 - 1000.0));
    assert!(volume_about(&harness, split, 1000.0));
    assert!(volume_about(&harness, peg, 3000.0));

    choose(&mut harness, "Peg", "The curve of Cut line");
    assert_eq!(harness.model.undo_label(), Some("Edit Split 1"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    let (low, high) = plate_bounds(&harness, plate);
    assert!(low.x.abs() < 1e-6 && (high.x - 20.0).abs() < 1e-6);
    let (low, high) = plate_bounds(&harness, split);
    assert!((low.x - 20.0).abs() < 1e-6 && (high.x - 40.0).abs() < 1e-6);

    harness.click(split_panel::KEEP_OTHER_SIDE);
    harness.settle();
    let (low, high) = plate_bounds(&harness, plate);
    assert!((low.x - 20.0).abs() < 1e-6 && (high.x - 40.0).abs() < 1e-6);
}

#[test]
fn a_body_is_mated_face_to_face_from_the_palette_and_its_panel_sets_the_distance() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);
    let bottom = pickable_described(&mut harness, "Peg › Peg start face");

    harness.select([bottom, top]);
    run_from_palette(&mut harness, "mate body");
    harness.settle();
    let mate = harness.workspace.editing.solid().expect("the mate is open");
    assert_eq!(harness.model.undo_label(), Some("Create Mate 1"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(harness.shows("Face to mate"));
    assert!(harness.shows("Onto"));
    let (low, high) = plate_bounds(&harness, peg);
    assert!((low.z - 10.0).abs() < 1e-6 && (high.z - 15.0).abs() < 1e-6);
    assert!((low.x - 30.0).abs() < 1e-6);

    harness.type_into_field(Id::unique(("mate-field", "distance", mate)), "2 mm");
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Edit Mate 1"));
    let (low, _) = plate_bounds(&harness, peg);
    assert!((low.z - 12.0).abs() < 1e-6, "{low:?}");

    harness.click(mate_panel::FACE_SAME_WAY);
    harness.settle();
    let (low, high) = plate_bounds(&harness, peg);
    assert!(
        (high.z - 12.0).abs() < 1e-6 && (low.z - 7.0).abs() < 1e-6,
        "{low:?} {high:?}"
    );

    for _ in 0..3 {
        harness.perform(Action::Undo);
    }
    harness.settle();
    let (low, _) = plate_bounds(&harness, peg);
    assert!(low.z.abs() < 1e-6);
    assert!(volume_about(&harness, plate, 16000.0));
}

#[test]
fn a_body_is_mated_at_an_angle_and_by_a_corner_from_the_palette() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);
    let bottom = pickable_described(&mut harness, "Peg › Peg start face");

    harness.select([bottom, top]);
    run_from_palette(&mut harness, "mate body at an angle");
    harness.settle();
    let mate = harness.workspace.editing.solid().expect("the mate is open");
    assert_eq!(harness.model.undo_label(), Some("Create Mate 1"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(harness.shows(mate_panel::ANGLE));
    let (low, high) = plate_bounds(&harness, peg);
    assert!((high.z - low.z - 5.0).abs() > 1.0, "{low:?} {high:?}");

    harness.type_into_field(Id::unique(("mate-field", "angle", mate)), "0 deg");
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Edit Mate 1"));
    let (low, high) = plate_bounds(&harness, peg);
    assert!((high.z - low.z - 5.0).abs() < 1e-6, "{low:?} {high:?}");
    assert!((low.x - 30.0).abs() < 1e-6, "{low:?}");

    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let corner = vertex_at(
        &harness,
        peg,
        caditor_geometry::Point3::new(30.0, 10.0, 0.0),
    );
    harness.select([corner, top]);
    run_from_palette(&mut harness, "mate body");
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Create Mate 2"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(harness.shows("Point to mate"));
    let (low, _) = plate_bounds(&harness, peg);
    assert!((low.z - 10.0).abs() < 1e-6, "{low:?}");
    assert!((low.x - 30.0).abs() < 1e-6, "{low:?}");
}

fn upright_edge_at(harness: &Harness, body: FeatureId, x: f64, y: f64) -> Pickable {
    let solid = harness.model.evaluation().body(body).unwrap();
    let edge = solid
        .edges()
        .find(|(_, edge)| {
            let middle = edge.curve().point(edge.interval().middle());
            (middle.x - x).abs() < 1e-6 && (middle.y - y).abs() < 1e-6
        })
        .map(|(_, edge)| edge.name())
        .expect("the body has an upright edge there");
    Pickable::Edge { body, edge }
}

#[test]
fn four_picks_mate_a_body_flush_and_concentric_in_one_feature() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);
    let bottom = pickable_described(&mut harness, "Peg › Peg start face");
    let peg_edge = upright_edge_at(&harness, peg, 30.0, 10.0);
    let plate_edge = upright_edge_at(&harness, plate, 40.0, 40.0);

    harness.select([bottom, peg_edge, top, plate_edge]);
    run_from_palette(&mut harness, "mate body");
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Create Mate 1"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(harness.shows(mate_panel::AXIS_TO_MATE));
    assert!(harness.shows(mate_panel::ONTO_AXIS));
    let (low, high) = plate_bounds(&harness, peg);
    assert!(
        (low - caditor_geometry::Point3::new(40.0, 40.0, 10.0)).length() < 1e-6,
        "{low:?}"
    );
    assert!(
        (high - caditor_geometry::Point3::new(70.0, 60.0, 15.0)).length() < 1e-6,
        "{high:?}"
    );
}

#[test]
fn one_cut_removes_material_from_every_body_chosen_in_its_panel() {
    let mut harness = Harness::new();
    let first = extruded(&mut harness, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let second = extruded(
        &mut harness,
        Point2::new(50.0, 0.0),
        Point2::new(90.0, 40.0),
    );
    assert_eq!(harness.solid(second).operation(), BodyOperation::NewBody);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert!(volume_about(&harness, second, 16000.0));

    let cut = extruded(
        &mut harness,
        Point2::new(30.0, 10.0),
        Point2::new(60.0, 20.0),
    );
    choose(&mut harness, "Add to body", "Remove from body");
    assert_eq!(
        harness.solid(cut).operation(),
        BodyOperation::Remove(second)
    );
    assert!(harness.shows(solid_panel::ALSO_CUTS));
    assert!(volume_about(&harness, second, 15000.0));
    assert!(volume_about(&harness, first, 16000.0));

    choose(&mut harness, solid_panel::ADD_CUT_BODY, "Extrude 1");
    assert_eq!(harness.solid(cut).other_bodies(), [first]);
    assert!(volume_about(&harness, first, 15000.0));
    assert!(volume_about(&harness, second, 15000.0));
    assert_eq!(harness.model.evaluation().failed_count(), 0);

    harness.click_button("Stop cutting Extrude 1");
    harness.settle();
    assert!(harness.solid(cut).other_bodies().is_empty());
    assert!(volume_about(&harness, first, 16000.0));

    choose(&mut harness, solid_panel::ADD_CUT_BODY, "Extrude 1");
    choose(&mut harness, "Remove from body", "Add to body");
    assert!(harness.solid(cut).other_bodies().is_empty());
}

#[test]
fn a_body_is_scaled_by_a_factor_about_a_centre_typed_in_the_panel() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);

    harness.select([top]);
    harness.use_tool_with(Key::S, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    let scale = harness
        .workspace
        .editing
        .solid()
        .expect("the scale is open");
    assert_eq!(harness.model.undo_label(), Some("Create Scale body 1"));
    assert!(harness.shows("Factor"));
    assert!(harness.shows("Centre X"));
    assert!(volume_about(&harness, plate, 16000.0));

    harness.type_into_field(Id::unique(("scale-field", ("factor", 0usize), scale)), "2");
    harness.settle();
    assert!(volume_about(&harness, plate, 8.0 * 16000.0));

    harness.type_into_field(
        Id::unique(("scale-field", ("factor", 0usize), scale)),
        "0.5",
    );
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Edit Scale body 1"));
    assert!(volume_about(&harness, plate, 16000.0 / 8.0));

    harness.type_into_field(
        Id::unique(("scale-field", ("center", 0usize), scale)),
        "40 mm",
    );
    harness.settle();
    let (low, high) = plate_bounds(&harness, plate);
    assert!(
        (low.x - 20.0).abs() < 1e-6 && (high.x - 40.0).abs() < 1e-6,
        "{low:?} {high:?}"
    );

    harness.type_into_field(Id::unique(("scale-field", ("factor", 0usize), scale)), "-1");
    assert!(harness.shows_containing("above zero"));
    assert_eq!(harness.workspace.editing.solid(), Some(scale));

    for _ in 0..4 {
        harness.perform(Action::Undo);
    }
    harness.settle();
    assert!(volume_about(&harness, plate, 16000.0));
}

fn add_round(
    harness: &mut Harness,
    name: &str,
    shape: caditor_document::PrimitiveShape,
    at: [f64; 2],
    reversed: bool,
) -> FeatureId {
    use caditor_document::{PlaneReference, Primitive, PrimitiveAnchor, PrincipalPlane};
    let mut transaction = harness.document().transaction("Add a round body");
    let body = transaction.add_feature(
        name,
        FeatureKind::Primitive(Primitive {
            shape,
            plane: PlaneReference::Principal(PrincipalPlane::Xy),
            at: at.map(|value| Expression::measure(value, Unit::Millimetre)),
            anchor: PrimitiveAnchor::BaseCentre,
            reversed,
            operation: BodyOperation::NewBody,
        }),
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    body
}

#[test]
fn a_body_is_split_along_the_surface_of_a_curved_face_of_another_body() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let whole = harness.body_volume(plate);
    let height = whole / 1600.0;
    add_round(
        &mut harness,
        "Rod",
        caditor_document::PrimitiveShape::Cylinder {
            diameter: Expression::measure(10.0, Unit::Millimetre),
            height: Expression::measure(5.0, Unit::Millimetre),
        },
        [20.0, 20.0],
        true,
    );
    let plate_edge = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| matches!(pickable, Pickable::Edge { body, .. } if *body == plate))
        .expect("an edge of the plate is pickable");
    let wall = pickable_described(&mut harness, "Rod › Rod wall");

    harness.select([plate_edge, wall]);
    harness.use_tool_with(Key::K, Modifiers::ALT);
    harness.settle();
    let split = harness
        .workspace
        .editing
        .solid()
        .expect("the split is open");

    assert_eq!(harness.model.undo_label(), Some("Create Split 1"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(harness.shows("The surface of Rod wall"));
    let inside = PI * 25.0 * height;
    let roughly = |harness: &Harness, body: FeatureId, expected: f64| {
        let found = harness.body_volume(body);
        assert!(
            (found - expected).abs() < 2e-2 * expected,
            "{found} {expected}"
        );
    };
    roughly(&harness, split, inside);
    roughly(&harness, plate, whole - inside);

    harness.click(split_panel::KEEP_OTHER_SIDE);
    harness.settle();
    roughly(&harness, plate, inside);
}

#[test]
fn a_flat_face_is_mated_resting_on_a_sphere_of_another_body_from_the_palette() {
    let mut harness = Harness::new();
    let peg = add_peg(&mut harness);
    add_round(
        &mut harness,
        "Ball",
        caditor_document::PrimitiveShape::Sphere {
            diameter: Expression::measure(10.0, Unit::Millimetre),
        },
        [100.0, 0.0],
        false,
    );
    let bottom = pickable_described(&mut harness, "Peg › Peg start face");
    let ball = pickable_described(&mut harness, "Ball › Ball surface");

    harness.select([bottom, ball]);
    run_from_palette(&mut harness, "mate body");
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Create Mate 1"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(harness.shows("Flat face"));
    assert!(harness.shows("Rests on"));
    let (low, high) = plate_bounds(&harness, peg);
    assert!(
        (low.z - 10.0).abs() < 1e-6 && (high.z - 15.0).abs() < 1e-6,
        "{low:?} {high:?}"
    );

    harness.click(mate_panel::OTHER_SIDE);
    harness.settle();
    let (low, _) = plate_bounds(&harness, peg);
    assert!(low.z.abs() < 1e-6, "{low:?}");
}
