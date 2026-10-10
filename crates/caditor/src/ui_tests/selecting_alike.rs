use caditor_geometry::{Plane, Point2, Point3};
use caditor_sketch::Sketch;
use egui::{Key, Modifiers};

use super::{
    Harness, extruded_plate, is_edge, is_face, rectangle, run_from_palette, selected_of, vertex_at,
};
use crate::{body_selection, selection::Pickable};

struct Drilled {
    big: Vec<Pickable>,
    small: Vec<Pickable>,
}

fn drilled_plate(harness: &mut Harness) -> Drilled {
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(
        &mut sketch,
        Point2::new(-30.0, -30.0),
        Point2::new(30.0, 30.0),
    );
    let first = sketch.add_circle(Point2::new(-15.0, 0.0), 5.0);
    let second = sketch.add_circle(Point2::new(15.0, 0.0), 5.0);
    let third = sketch.add_circle(Point2::new(0.0, 15.0), 3.0);
    let labels = [first, second, third].map(|circle| sketch.entity_label(circle));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    assert!(harness.workspace.editing.solid().is_some());
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness.settle();
    let faces: Vec<Pickable> = harness.built().picks.pickables().filter(is_face).collect();
    let walls_of = |label: &str| -> Vec<Pickable> {
        faces
            .iter()
            .copied()
            .filter(|face| {
                face.describe(harness.document(), harness.model.evaluation())
                    .contains(label)
            })
            .collect()
    };
    let big = [&labels[0], &labels[1]]
        .into_iter()
        .flat_map(|label| walls_of(label))
        .collect();
    let small = walls_of(&labels[2]);
    Drilled { big, small }
}

#[test]
fn selecting_similar_from_a_hole_wall_takes_the_walls_of_every_hole_of_that_size() {
    let mut harness = Harness::new();
    let drilled = drilled_plate(&mut harness);
    harness.select([drilled.big[0]]);

    run_from_palette(&mut harness, "Select faces or edges like the selected ones");
    harness.frame();

    assert!(!drilled.big.is_empty() && !drilled.small.is_empty());
    assert_eq!(selected_of(&harness, is_face), drilled.big.len());
    assert!(
        drilled
            .big
            .iter()
            .all(|wall| harness.workspace.viewport.selection().contains(*wall))
    );
    assert!(
        !drilled
            .small
            .iter()
            .any(|wall| harness.workspace.viewport.selection().contains(*wall))
    );
    assert!(harness.shows_containing("hole radius 5"));
}

#[test]
fn the_shortcut_grows_edges_to_every_edge_of_the_same_length() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let edge = Pickable::Edge {
        body: plate,
        edge: super::top_edge_along_x(&harness, plate, 0.0),
    };
    harness.select([edge]);

    harness.key(Key::A, Modifiers::ALT | Modifiers::SHIFT);
    harness.frame();
    harness.frame();

    assert_eq!(selected_of(&harness, is_edge), 8);
    assert!(harness.workspace.viewport.selection().contains(edge));
    assert!(harness.shows_containing("8 edges of length 40"));
}

#[test]
fn flat_faces_match_by_area_and_a_lone_shape_says_nothing_else_is_like_it() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);

    run_from_palette(&mut harness, "Select faces or edges like the selected ones");
    harness.frame();
    let faces = selected_of(&harness, is_face);
    run_from_palette(&mut harness, "Select faces or edges like the selected ones");
    harness.frame();

    assert_eq!(faces, 2);
    assert!(harness.shows_containing(body_selection::NO_SIMILAR));
}

#[test]
fn selecting_similar_needs_a_face_or_an_edge() {
    let mut harness = Harness::new();
    let _ = extruded_plate(&mut harness);
    harness.select([]);

    run_from_palette(&mut harness, "Select faces or edges like the selected ones");
    harness.frame();

    assert!(harness.shows_containing(body_selection::NO_SIMILAR_SELECTED));
}

#[test]
fn two_selected_items_show_the_distance_between_them_in_the_status_bar_without_measure() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let corner = vertex_at(&harness, plate, Point3::ZERO);
    let far = vertex_at(&harness, plate, Point3::new(40.0, 0.0, 0.0));

    harness.select([corner]);
    harness.frame();
    let alone = harness.shows_containing("Distance");
    harness.select([corner, far]);
    harness.wait_until("the distance is read", |harness| {
        harness.shows_containing("Distance 40.000 mm")
    });
    let measure_open = harness.workspace.measure.open;
    harness.select([corner]);
    harness.frame();

    assert!(!alone);
    assert!(!measure_open);
    assert!(!harness.shows_containing("Distance 40.000 mm"));
}

#[test]
fn two_faces_at_a_right_angle_show_the_angle_beside_the_distance() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let side = harness
        .built()
        .picks
        .pickables()
        .filter(is_face)
        .find(|face| {
            face.describe(harness.document(), harness.model.evaluation())
                .contains("side")
                && matches!(face, Pickable::Face { body, .. } if *body == plate)
        })
        .expect("a side face is pickable");

    harness.select([top, side]);
    harness.wait_until("the angle is read", |harness| {
        harness.shows_containing("Angle 90")
    });

    assert!(harness.shows_containing("Distance"));
}
