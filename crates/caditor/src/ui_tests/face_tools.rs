use std::f64::consts::PI;

use caditor_document::{BlendKind, FeatureId, RevolveAxis, SolidFeature, face_plane};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::Sketch;
use egui::{Key, Modifiers};

use super::{CAMERA_SETTLE, Harness, click_with, extruded_plate, rectangle};
use crate::{
    blend_tools, bodies, look_at, measure,
    selection::{Pickable, Selection},
    solid_tools::{self, Sweep},
};

fn faces_facing(harness: &Harness, body: FeatureId, normal: Vector3, offset: f64) -> Vec<Pickable> {
    let shown = bodies::shown(harness.model.evaluation(), body).expect("the body is shown");
    bodies::face_keys(&shown.solid)
        .into_iter()
        .filter(|(id, _)| {
            face_plane(&shown.solid, *id).is_some_and(|plane| {
                (plane.normal() - normal).length() < 1e-9
                    && (plane.origin().dot(normal) - offset).abs() < 1e-9
            })
        })
        .map(|(_, face)| Pickable::Face { body, face })
        .collect()
}

fn edge_through(harness: &Harness, body: FeatureId, middle: Point3) -> Pickable {
    let solid = harness.model.evaluation().body(body).unwrap();
    let edge = solid
        .edges()
        .find(|(_, edge)| (edge.curve().point(edge.interval().middle()) - middle).length() < 1e-6)
        .map(|(_, edge)| edge.name())
        .expect("the body has that edge");
    Pickable::Edge { body, edge }
}

fn open_feature(harness: &Harness) -> FeatureId {
    harness
        .workspace
        .editing
        .solid()
        .expect("a feature is open")
}

fn blend_edges(harness: &Harness, feature: FeatureId) -> (FeatureId, usize) {
    let blend = harness
        .document()
        .feature(feature)
        .unwrap()
        .kind
        .blend()
        .unwrap()
        .clone();
    (blend.body, blend.edges.len())
}

fn close_feature(harness: &mut Harness) {
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
}

#[test]
fn fillet_with_a_face_selected_rounds_every_edge_around_it() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let side = edge_through(&harness, plate, Point3::new(0.0, 0.0, 5.0));

    harness.select([top, side]);
    harness.click_tool(BlendKind::Fillet.title());
    harness.settle();

    assert_eq!(blend_edges(&harness, open_feature(&harness)), (plate, 5));
}

#[test]
fn fillet_with_a_body_chosen_in_the_tree_rounds_every_edge_of_it() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    harness.select([]);

    click_with(&mut harness, "Extrude 1", Modifiers::NONE);
    harness.click_tool(BlendKind::Chamfer.title());
    harness.settle();

    assert_eq!(blend_edges(&harness, open_feature(&harness)), (plate, 12));
}

#[test]
fn a_fillet_takes_faces_of_one_body_and_says_what_it_leaves_out() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let corner = Pickable::Origin;
    let mut selection = Selection::default();
    selection.toggle(top);
    selection.toggle(corner);

    let source = blend_tools::selected_edges(&harness.model, &selection, &[]).unwrap();

    assert_eq!(source.body, plate);
    assert_eq!(source.edges.len(), 4);
    assert_eq!(source.left_out, vec![corner]);
    assert_eq!(
        blend_tools::selected_edges(&harness.model, &Selection::default(), &[]),
        Err(blend_tools::NOTHING_TO_BLEND)
    );
}

#[test]
fn revolve_with_a_flat_face_and_an_edge_selected_turns_the_face_about_the_edge() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    close_feature(&mut harness);
    let side = faces_facing(&harness, plate, -Vector3::X, 0.0);
    let axis = edge_through(&harness, plate, Point3::new(0.0, 0.0, 5.0));

    harness.select(side.iter().copied().chain([axis]));
    harness.click_tool("Revolve");
    harness.settle();
    let revolve = open_feature(&harness);
    let Some(SolidFeature::Revolve(definition)) = harness
        .document()
        .feature(revolve)
        .unwrap()
        .kind
        .solid()
        .cloned()
    else {
        panic!("the feature is a revolve");
    };
    let cylinder = PI * 40.0 * 40.0 * 10.0;
    let corners_outside = 16000.0 - cylinder / 4.0;

    assert_eq!(side.len(), 1);
    assert!(matches!(definition.axis, RevolveAxis::Model(_)));
    assert_eq!(
        definition.operation,
        caditor_document::BodyOperation::Add(plate)
    );
    assert!(
        harness
            .document()
            .feature(definition.sketch)
            .unwrap()
            .hidden
    );
    assert!(harness.shows_containing("Revolve 1 revolves the selected face out of its body"));
    let volume = harness.body_volume(plate);
    let expected = cylinder + corners_outside;
    assert!((volume - expected).abs() < expected * 0.01, "{volume}");
}

#[test]
fn a_face_alone_is_not_revolved_and_faces_of_two_planes_are_not_swept() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let side = faces_facing(&harness, plate, -Vector3::X, 0.0);
    let offer = |selected: &[Pickable], sweep: Sweep| {
        let mut selection = Selection::default();
        for pickable in selected {
            selection.toggle(*pickable);
        }
        solid_tools::faces_to_sweep(
            &harness.model,
            &selection,
            &harness.workspace.editing,
            sweep,
            &[],
        )
        .map(|offer| offer.map(|profile| profile.faces.len()))
    };

    assert_eq!(
        offer(&[top], Sweep::Revolve),
        Some(Err(solid_tools::NO_AXIS_FOR_FACE))
    );
    assert_eq!(
        offer(&[top, side[0]], Sweep::Extrude),
        Some(Err(solid_tools::NOT_IN_ONE_PLANE))
    );
    assert_eq!(offer(&[top], Sweep::Extrude), Some(Ok(1)));
}

#[test]
fn extrude_with_faces_of_one_plane_selected_extrudes_them_all_as_one_profile() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(10.0, 10.0));
    rectangle(&mut sketch, Point2::new(20.0, 0.0), Point2::new(30.0, 10.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let body = open_feature(&harness);
    close_feature(&mut harness);
    let tops = faces_facing(&harness, body, Vector3::Z, 10.0);

    harness.select(tops.iter().copied());
    harness.click_tool("Extrude");
    harness.settle();

    assert_eq!(tops.len(), 2);
    assert_ne!(open_feature(&harness), body);
    assert!(harness.shows_containing("extrudes the selected faces out of its body"));
    assert!((harness.body_volume(body) - 4000.0).abs() < 1.0);
}

fn settle_camera(harness: &mut Harness) {
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.let_animations_finish();
}

fn look(harness: &mut Harness) -> caditor_render::Viewpoint {
    harness.key(Key::V, Modifiers::ALT);
    harness.frame();
    settle_camera(harness);
    harness.workspace.viewport.destination()
}

#[test]
fn looking_straight_at_a_face_again_turns_the_view_a_quarter_turn() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    close_feature(&mut harness);
    settle_camera(&mut harness);

    harness.select([top]);
    let first = look(&mut harness);
    let second = look(&mut harness);

    assert!(look_at::faces(&first, Vector3::Z));
    assert!(look_at::faces(&second, Vector3::Z));
    assert!((second.up() + first.right()).length() < 1e-6);

    let edge = edge_through(&harness, plate, Point3::new(20.0, 0.0, 10.0));
    harness.select([edge]);
    let along = look(&mut harness);
    let direction = measure::direction_of(&harness.model, edge).unwrap();

    assert!(along.forward().cross(direction).length() < 1e-6);
    assert_eq!(
        look_at::look_target(&harness.model, &Selection::default()),
        Err(look_at::NOTHING_TO_LOOK_AT)
    );
}
