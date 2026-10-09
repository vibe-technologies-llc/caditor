use caditor_document::{
    AxisReference, Datum, DatumFrame, FeatureId, FeatureKind, PointReference, PrincipalAxis,
    PrincipalPlane, displayed_frame,
};
use caditor_geometry::{Plane, Point3, Vector3};
use egui::{Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, drag_screen, extruded_plate, move_of, open_combo, opened_move,
    run_from_palette, top_edge_along_x, vertex_at,
};
use crate::{
    move_manipulator::Handle,
    selection::{Axis, Pickable},
};

const CLOSE: f64 = 1e-6;

fn frame_of(harness: &Harness, feature: FeatureId) -> DatumFrame {
    match harness
        .document()
        .feature(feature)
        .map(|feature| &feature.kind)
    {
        Some(FeatureKind::Datum(Datum::Frame(frame))) => frame.as_ref().clone(),
        other => panic!("a coordinate system was expected, found {other:?}"),
    }
}

fn placed(harness: &Harness, feature: FeatureId) -> Plane {
    displayed_frame(harness.model.evaluation(), feature).expect("the coordinate system is placed")
}

fn near(found: Vector3, expected: [f64; 3]) -> bool {
    (found - Vector3::from_array(expected)).length() < CLOSE
}

fn coordinate_system_at_far_corner(harness: &mut Harness, body: FeatureId) -> FeatureId {
    let corner = vertex_at(harness, body, Point3::new(40.0, 40.0, 10.0));
    harness.select([corner, Pickable::Axis(Axis::Y)]);
    harness.frame();
    run_from_palette(harness, "Coordinate system");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness
        .document()
        .features()
        .find(|feature| matches!(feature.kind, FeatureKind::Datum(Datum::Frame(_))))
        .map(|feature| feature.id())
        .expect("the coordinate system was created")
}

#[test]
fn a_coordinate_system_takes_the_selection_and_its_axes_and_planes_are_picked_and_used() {
    let mut harness = Harness::new();
    let (body, _) = extruded_plate(&mut harness);
    let corner = vertex_at(&harness, body, Point3::new(40.0, 40.0, 10.0));
    let edge = Pickable::Edge {
        body,
        edge: top_edge_along_x(&harness, body, 0.0),
    };

    harness.select([corner, edge]);
    harness.frame();
    run_from_palette(&mut harness, "Coordinate system");
    harness.settle();
    let system = harness
        .workspace
        .editing
        .solid()
        .expect("the coordinate system is open");
    let frame = frame_of(&harness, system);
    let origin = placed(&harness, system);

    assert!(matches!(frame.origin, PointReference::Vertex { .. }));
    assert!(matches!(frame.x_axis, AxisReference::Edge { .. }));
    assert!(harness.shows(crate::datum_panel::FRAME_DESCRIPTION));
    assert!(harness.shows("X axis"));
    assert!(harness.shows("XY plane"));
    assert!(origin.origin().distance(Point3::new(40.0, 40.0, 10.0)) < CLOSE);
    assert!(origin.x_axis().cross(Vector3::X).length() < CLOSE);
    assert!(near(origin.normal(), [0.0, 0.0, 1.0]));

    harness.click(crate::datum_panel::REVERSE_Z);
    harness.settle();
    assert!(frame_of(&harness, system).reverse_z);
    assert!(near(placed(&harness, system).normal(), [0.0, 0.0, -1.0]));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let pickables: Vec<Pickable> = harness.built().picks.pickables().collect();
    let axis = Pickable::FrameAxis {
        feature: system,
        axis: PrincipalAxis::Y,
    };
    let plane = Pickable::FramePlane {
        feature: system,
        plane: PrincipalPlane::Yz,
    };
    assert!(pickables.contains(&axis));
    assert!(pickables.contains(&plane));
    assert!(pickables.contains(&Pickable::Datum(system)));
    assert_eq!(
        axis.describe(harness.document(), harness.model.evaluation()),
        "Coordinate system 1 › Y axis"
    );

    harness.select([corner, plane]);
    harness.click("Mirror body");
    harness.settle();
    let mirror = harness
        .workspace
        .editing
        .solid()
        .expect("the mirror is open");
    let Some(FeatureKind::Mirror(mirrored)) = harness
        .document()
        .feature(mirror)
        .map(|feature| feature.kind.clone())
    else {
        panic!("a mirror was made");
    };
    assert_eq!(
        mirrored.plane,
        caditor_document::PlaneReference::Frame {
            frame: system,
            plane: PrincipalPlane::Yz,
        }
    );
}

#[test]
fn measure_reads_positions_and_offsets_in_the_chosen_coordinate_system() {
    let mut harness = Harness::new();
    let (body, _) = extruded_plate(&mut harness);
    let system = coordinate_system_at_far_corner(&mut harness, body);
    let frame = placed(&harness, system);
    assert!(near(frame.x_axis(), [0.0, 1.0, 0.0]));
    let corner = vertex_at(&harness, body, Point3::ZERO);
    let middle = vertex_at(&harness, body, Point3::new(40.0, 0.0, 0.0));

    harness.key(Key::I, Modifiers::NONE);
    harness.frame();
    harness.select([corner]);
    harness.wait_until("the corner is measured", |harness| {
        harness.shows("0.000, 0.000, 0.000 mm")
    });
    open_combo(&mut harness, crate::measure_panel::RELATIVE_TO);
    let rightmost = harness
        .texts
        .iter()
        .filter(|(shown, _)| shown == "Coordinate system 1")
        .map(|(_, rect)| rect.center())
        .max_by(|a, b| a.x.total_cmp(&b.x))
        .expect("the coordinate system is offered");
    harness.click_screen(rightmost);
    harness.frame();

    assert_eq!(harness.workspace.measure.relative_to, Some(system));
    harness.wait_until("the corner is read in the coordinate system", |harness| {
        harness.shows("-40.000, 40.000, -10.000 mm")
    });
    harness.select([corner, middle]);
    harness.wait_until("the offsets are read along its axes", |harness| {
        harness.shows("Along Y")
    });
    let readout = harness
        .workspace
        .measure
        .measurements
        .readout()
        .cloned()
        .expect("measured");
    let along = |label: &str| {
        readout
            .groups
            .iter()
            .flat_map(|group| &group.readings)
            .find(|reading| reading.label == label)
            .and_then(|reading| match reading.value {
                crate::measure::Value::Length(length) => Some(length),
                _ => None,
            })
            .expect("the offset is read")
    };
    assert!(along("Along X").abs() < CLOSE);
    assert!((along("Along Y") - 40.0).abs() < CLOSE);
    assert!(along("Along Z").abs() < CLOSE);
}

#[test]
fn a_move_in_a_coordinate_system_shifts_along_its_axes_from_the_panel_and_the_arrows() {
    let mut harness = Harness::new();
    let (body, top) = extruded_plate(&mut harness);
    let system = coordinate_system_at_far_corner(&mut harness, body);
    let movement = opened_move(&mut harness, top);

    open_combo(&mut harness, crate::move_panel::DIRECTIONS);
    harness.click_lowest("Coordinate system 1");
    harness.settle();
    assert_eq!(move_of(&harness, movement).frame, Some(system));
    assert!(harness.shows(crate::move_panel::FRAME_DESCRIPTION));

    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let along_x = Handle::Along(caditor_document::MoveAxis::X);
    let from = harness
        .workspace
        .viewport
        .handle_position(along_x, 0.0)
        .expect("the X arrow is shown");
    let to = harness
        .workspace
        .viewport
        .handle_position(along_x, 15.0)
        .unwrap();
    drag_screen(&mut harness, from, to);
    harness.frame();
    harness.settle();

    let moved = move_of(&harness, movement);
    let parameters = harness.model.parameters();
    let x = moved.offset[0]
        .evaluate_as(caditor_expression::Dimension::LENGTH, &|id| {
            parameters.value(id)
        })
        .unwrap();
    let bounds = harness
        .model
        .evaluation()
        .body(body)
        .unwrap()
        .bounding_box()
        .unwrap();
    assert!(x > 0.0, "{x}");
    assert!((bounds.min().y - x).abs() < CLOSE, "{bounds:?} after {x}");
    assert!(bounds.min().x.abs() < CLOSE);
}

#[test]
fn a_coordinate_system_origin_is_a_point_and_a_scale_centre_can_be_measured_in_it() {
    let mut harness = Harness::new();
    let (body, top) = extruded_plate(&mut harness);
    let system = coordinate_system_at_far_corner(&mut harness, body);

    harness.select([Pickable::Datum(system)]);
    harness.click("Point");
    harness.settle();
    let point = harness
        .workspace
        .editing
        .solid()
        .expect("the point is open");
    let based = match harness
        .document()
        .feature(point)
        .map(|feature| &feature.kind)
    {
        Some(FeatureKind::Datum(Datum::Point(point))) => point.base.clone(),
        other => panic!("a datum point was expected, found {other:?}"),
    };
    assert_eq!(based, PointReference::Frame(system));
    assert!(harness.shows_containing("origin of Coordinate system 1"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.select([top]);
    harness.click(crate::scale_tools::TITLE);
    harness.settle();
    let scale = harness
        .workspace
        .editing
        .solid()
        .expect("the scale is open");
    open_combo(&mut harness, crate::scale_panel::CENTRE_IN);
    harness.click_lowest("Coordinate system 1");
    harness.settle();

    let framed = harness
        .document()
        .feature(scale)
        .and_then(|feature| feature.kind.scale())
        .and_then(|scale| scale.frame);
    assert_eq!(framed, Some(system));
    assert!(harness.shows(crate::scale_panel::FRAME_DESCRIPTION));
}
