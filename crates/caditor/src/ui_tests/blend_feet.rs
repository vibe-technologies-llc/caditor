use caditor_document::{ChamferForm, FeatureId};
use caditor_geometry::{Plane, Point2, Point3};
use caditor_kernel::{EdgeName, FaceOrigin, Solid};
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{
    Harness, blend_of, drag_value_arrow, extruded_plate, length_value, run_from_palette,
    top_edge_along_x,
};
use crate::{move_manipulator::Handle, selection::Pickable, value_gauges::Measured};

fn open_feature(harness: &Harness) -> FeatureId {
    harness
        .workspace
        .editing
        .solid()
        .expect("a feature is open")
}

fn l_shaped_body(harness: &mut Harness) -> FeatureId {
    let corners = [
        Point2::new(0.0, 0.0),
        Point2::new(40.0, 0.0),
        Point2::new(40.0, 20.0),
        Point2::new(20.0, 20.0),
        Point2::new(20.0, 40.0),
        Point2::new(0.0, 40.0),
    ];
    let mut sketch = Sketch::new(Plane::XY);
    for (index, corner) in corners.iter().enumerate() {
        sketch.add_line(*corner, corners[(index + 1) % corners.len()]);
    }
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let extrude = open_feature(harness);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    extrude
}

fn vertical_edge_at(harness: &Harness, body: FeatureId, x: f64, y: f64) -> EdgeName {
    let solid = harness.model.evaluation().body(body).unwrap();
    solid
        .edges()
        .find(|(_, edge)| {
            let middle = edge.curve().point(edge.interval().middle());
            (middle - Point3::new(x, y, 5.0)).length() < 1e-6
        })
        .map(|(_, edge)| edge.name())
        .expect("the body has that vertical edge")
}

fn blend_with_edge(
    harness: &mut Harness,
    label: &str,
    body: FeatureId,
    edge: EdgeName,
) -> FeatureId {
    harness.select([Pickable::Edge { body, edge }]);
    harness.click(label);
    harness.settle();
    harness.select([]);
    harness.workspace.viewport.advance(super::CAMERA_SETTLE);
    harness.frame();
    open_feature(harness)
}

fn distance_to_blend_faces(solid: &Solid, feature: FeatureId, point: Point3) -> f64 {
    let feature = feature.raw();
    solid
        .faces()
        .filter(|(_, face)| {
            matches!(
                face.origin(),
                Some(FaceOrigin::Fillet { feature: made } | FaceOrigin::Chamfer { feature: made })
                    if made == feature
            )
        })
        .map(|(_, face)| {
            let surface = face.surface();
            let at = surface.project(point, None);
            (surface.point(at.x, at.y) - point).length()
        })
        .fold(f64::INFINITY, f64::min)
}

fn result_of(harness: &Harness, feature: FeatureId) -> &Solid {
    harness
        .model
        .evaluation()
        .feature(feature)
        .and_then(|status| status.result.as_deref())
        .and_then(|result| result.solid())
        .map(|solid| &solid.solid)
        .expect("the feature has a body")
}

#[test]
fn a_fillet_s_arrow_on_a_concave_edge_stands_on_the_rounded_face() {
    let mut harness = Harness::new();
    let body = l_shaped_body(&mut harness);
    let edge = vertical_edge_at(&harness, body, 20.0, 20.0);
    let fillet = blend_with_edge(&mut harness, "Fillet", body, edge);
    let foot = harness
        .workspace
        .viewport
        .handle_foot(Handle::Value(Measured::BlendSize))
        .expect("the fillet's arrow is shown");
    let distance = distance_to_blend_faces(result_of(&harness, fillet), fillet, foot);

    assert!(
        distance < 1e-3,
        "{foot:?} is {distance} mm off the rounded face"
    );
    assert_eq!(
        blend_of(&harness, fillet).kind,
        caditor_document::BlendKind::Fillet
    );
}

fn plate_with_boss(harness: &mut Harness) -> FeatureId {
    let (plate, top) = extruded_plate(harness);
    harness.select([top]);
    harness.use_tool_with(Key::Y, Modifiers::ALT);
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();
    plate
}

fn circle_at(harness: &Harness, body: FeatureId, level: f64) -> EdgeName {
    let solid = harness.model.evaluation().body(body).unwrap();
    solid
        .edges()
        .find(|(_, edge)| {
            matches!(edge.curve(), caditor_kernel::Curve::Circle(circle)
                if (circle.center().z - level).abs() < 1e-6)
        })
        .map(|(_, edge)| edge.name())
        .expect("the body has that round edge")
}

#[test]
fn a_fillet_s_arrow_on_a_concave_round_edge_stands_on_the_rounded_face() {
    let mut harness = Harness::new();
    let plate = plate_with_boss(&mut harness);
    let edge = circle_at(&harness, plate, 10.0);
    let fillet = blend_with_edge(&mut harness, "Fillet", plate, edge);
    let foot = harness
        .workspace
        .viewport
        .handle_foot(Handle::Value(Measured::BlendSize))
        .expect("the fillet's arrow is shown");
    let distance = distance_to_blend_faces(result_of(&harness, fillet), fillet, foot);

    assert!(
        distance < 1e-3,
        "{foot:?} is {distance} mm off the rounded face"
    );
}

fn on_an_edge(solid: &Solid, point: Point3) -> bool {
    solid.edges().any(|(_, edge)| {
        let parameter = edge.curve().closest_parameter(point, edge.interval());
        edge.curve().point(parameter).distance(point) < 1e-6
    })
}

fn foot_of(harness: &Harness, measured: Measured) -> Point3 {
    harness
        .workspace
        .viewport
        .handle_foot(Handle::Value(measured))
        .unwrap_or_else(|| panic!("{measured:?} is shown"))
}

fn open_chamfer(harness: &mut Harness) -> (FeatureId, FeatureId) {
    let (plate, _) = extruded_plate(harness);
    let front = top_edge_along_x(harness, plate, 0.0);
    let chamfer = blend_with_edge(harness, "Chamfer", plate, front);
    (plate, chamfer)
}

fn second_distance(harness: &Harness, chamfer: FeatureId) -> f64 {
    let ChamferForm::TwoDistances { second } = &blend_of(harness, chamfer).form else {
        panic!("the chamfer takes two distances");
    };
    length_value(harness, second)
}

fn chamfer_angle(harness: &Harness, chamfer: FeatureId) -> f64 {
    let ChamferForm::DistanceAngle { angle } = &blend_of(harness, chamfer).form else {
        panic!("the chamfer takes a distance and an angle");
    };
    harness
        .model
        .parameters()
        .evaluate_expression(angle)
        .unwrap()
        .value
}

fn settled_view(harness: &mut Harness) {
    harness.workspace.viewport.advance(super::CAMERA_SETTLE);
    harness.frame();
}

#[test]
fn a_two_distance_chamfer_shows_an_arrow_on_each_face_where_its_distance_ends() {
    let mut harness = Harness::new();
    let (_, chamfer) = open_chamfer(&mut harness);
    harness.click("Two");
    harness.settle();
    harness.type_into_field(Id::new(("blend-form", chamfer)), "3 mm");
    harness.settle();
    settled_view(&mut harness);
    let result = |harness: &Harness| result_of(harness, chamfer).clone();
    let first = foot_of(&harness, Measured::BlendSize);
    let second = foot_of(&harness, Measured::BlendSecond);

    assert!(on_an_edge(&result(&harness), first), "{first:?}");
    assert!(on_an_edge(&result(&harness), second), "{second:?}");
    assert!(first.distance(second) > 2.0, "{first:?} {second:?}");

    harness.click("Measure from the other face");
    harness.settle();
    settled_view(&mut harness);
    let swapped_first = foot_of(&harness, Measured::BlendSize);
    let swapped_second = foot_of(&harness, Measured::BlendSecond);

    assert!(
        on_an_edge(&result(&harness), swapped_first),
        "{swapped_first:?}"
    );
    assert!(
        on_an_edge(&result(&harness), swapped_second),
        "{swapped_second:?}"
    );
    assert!(swapped_first.distance(first) > 0.5);
}

#[test]
fn dragging_a_chamfer_s_second_distance_arrow_changes_only_that_distance() {
    let mut harness = Harness::new();
    let (_, chamfer) = open_chamfer(&mut harness);
    harness.click("Two");
    harness.settle();
    let size_before = length_value(&harness, &blend_of(&harness, chamfer).size);
    let second_before = second_distance(&harness, chamfer);

    let step = drag_value_arrow(&mut harness, Measured::BlendSecond, 2.0);
    let second_after = second_distance(&harness, chamfer);
    let name = harness.document().feature(chamfer).unwrap().name.clone();

    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Edit {name}").as_str())
    );
    assert!(
        (second_after - second_before - 2.0).abs() <= step,
        "{second_before} to {second_after} with steps of {step}"
    );
    assert_eq!(
        length_value(&harness, &blend_of(&harness, chamfer).size),
        size_before
    );
}

#[test]
fn dragging_a_chamfer_s_first_distance_arrow_follows_its_face_in_the_two_distance_form() {
    let mut harness = Harness::new();
    let (_, chamfer) = open_chamfer(&mut harness);
    harness.click("Two");
    harness.settle();
    let before = length_value(&harness, &blend_of(&harness, chamfer).size);
    let second_before = second_distance(&harness, chamfer);

    let step = drag_value_arrow(&mut harness, Measured::BlendSize, 2.0);
    let after = length_value(&harness, &blend_of(&harness, chamfer).size);

    assert!(
        (after - before - 2.0).abs() <= step,
        "{before} to {after} with steps of {step}"
    );
    assert_eq!(second_distance(&harness, chamfer), second_before);
}

#[test]
fn an_angled_chamfer_shows_its_angle_arrow_where_the_cut_meets_the_other_face() {
    let mut harness = Harness::new();
    let (_, chamfer) = open_chamfer(&mut harness);
    run_from_palette(&mut harness, "Chamfer by a distance and an angle");
    harness.settle();
    settled_view(&mut harness);
    let first = foot_of(&harness, Measured::BlendSize);
    let other = foot_of(&harness, Measured::BlendAngle);

    assert!(on_an_edge(result_of(&harness, chamfer), first), "{first:?}");
    assert!(on_an_edge(result_of(&harness, chamfer), other), "{other:?}");

    let before = chamfer_angle(&harness, chamfer);
    let step = drag_value_arrow(&mut harness, Measured::BlendAngle, 10.0);
    let after = chamfer_angle(&harness, chamfer);

    assert!(
        (after - before - 10.0).abs() <= step,
        "{before} to {after} with steps of {step}"
    );
    let moved = foot_of(&harness, Measured::BlendAngle);

    assert!(on_an_edge(result_of(&harness, chamfer), moved), "{moved:?}");
}

#[test]
fn a_two_distance_chamfer_on_a_concave_edge_shows_an_arrow_on_each_face_where_its_distance_ends() {
    let mut harness = Harness::new();
    let body = l_shaped_body(&mut harness);
    let edge = vertical_edge_at(&harness, body, 20.0, 20.0);
    let chamfer = blend_with_edge(&mut harness, "Chamfer", body, edge);
    harness.click("Two");
    harness.settle();
    harness.type_into_field(Id::new(("blend-form", chamfer)), "3 mm");
    harness.settle();
    settled_view(&mut harness);
    let first = foot_of(&harness, Measured::BlendSize);
    let second = foot_of(&harness, Measured::BlendSecond);

    assert!(on_an_edge(result_of(&harness, chamfer), first), "{first:?}");
    assert!(
        on_an_edge(result_of(&harness, chamfer), second),
        "{second:?}"
    );
    assert!(first.distance(second) > 2.0, "{first:?} {second:?}");
}

fn d_shaped_body(harness: &mut Harness) -> FeatureId {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(0.0, 0.0), Point2::new(40.0, 0.0));
    sketch.add_arc(
        Point2::new(20.0, 0.0),
        Point2::new(40.0, 0.0),
        Point2::new(0.0, 0.0),
    );
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let extrude = open_feature(harness);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    extrude
}

fn off_the_round_face(point: Point3) -> f64 {
    (Point2::new(point.x, point.y).distance(Point2::new(20.0, 0.0)) - 20.0).abs()
}

fn on_the_round_face(harness: &Harness) -> Measured {
    [Measured::BlendSize, Measured::BlendSecond]
        .into_iter()
        .find(|measured| foot_of(harness, *measured).y > 1e-3)
        .expect("one foot stands on the round face")
}

fn chamfer_on_the_round_edge(harness: &mut Harness) -> FeatureId {
    let body = d_shaped_body(harness);
    let edge = vertical_edge_at(harness, body, 40.0, 0.0);
    blend_with_edge(harness, "Chamfer", body, edge)
}

#[test]
fn a_two_distance_chamfer_stands_its_foot_on_a_round_face_where_the_cut_meets_it() {
    let mut harness = Harness::new();
    let chamfer = chamfer_on_the_round_edge(&mut harness);
    harness.click("Two");
    harness.settle();
    harness.type_into_field(Id::new(("blend-form", chamfer)), "6 mm");
    harness.settle();
    settled_view(&mut harness);
    let round = on_the_round_face(&harness);
    let foot = foot_of(&harness, round);
    let flat = foot_of(
        &harness,
        if round == Measured::BlendSize {
            Measured::BlendSecond
        } else {
            Measured::BlendSize
        },
    );

    assert!(off_the_round_face(foot) < 1e-6, "{foot:?}");
    assert!(on_an_edge(result_of(&harness, chamfer), foot), "{foot:?}");
    assert!(on_an_edge(result_of(&harness, chamfer), flat), "{flat:?}");
}

#[test]
fn dragging_a_chamfer_s_arrow_on_a_round_face_follows_the_face() {
    let mut harness = Harness::new();
    let chamfer = chamfer_on_the_round_edge(&mut harness);
    harness.click("Two");
    harness.settle();
    settled_view(&mut harness);
    let round = on_the_round_face(&harness);
    let value = |harness: &Harness| match round {
        Measured::BlendSize => length_value(harness, &blend_of(harness, chamfer).size),
        _ => second_distance(harness, chamfer),
    };
    let before = value(&harness);

    let step = drag_value_arrow(&mut harness, round, 4.0);
    settled_view(&mut harness);
    let after = value(&harness);
    let foot = foot_of(&harness, round);

    assert!(
        (after - before - 4.0).abs() <= step,
        "{before} to {after} with steps of {step}"
    );
    assert!(off_the_round_face(foot) < 1e-6, "{foot:?}");
    assert!(on_an_edge(result_of(&harness, chamfer), foot), "{foot:?}");
}

#[test]
fn an_angled_chamfer_measured_on_a_round_face_turns_its_arrow_about_the_foot_on_it() {
    let mut harness = Harness::new();
    let chamfer = chamfer_on_the_round_edge(&mut harness);
    run_from_palette(&mut harness, "Chamfer by a distance and an angle");
    harness.settle();
    settled_view(&mut harness);
    if foot_of(&harness, Measured::BlendSize).y <= 1e-3 {
        harness.click("Measure from the other face");
        harness.settle();
        settled_view(&mut harness);
    }
    let first = foot_of(&harness, Measured::BlendSize);
    let other = foot_of(&harness, Measured::BlendAngle);

    assert!(off_the_round_face(first) < 1e-6, "{first:?}");
    assert!(on_an_edge(result_of(&harness, chamfer), first), "{first:?}");
    assert!(on_an_edge(result_of(&harness, chamfer), other), "{other:?}");
}
