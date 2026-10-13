use std::time::Instant;

use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Key, Modifiers};

use super::{
    CAMERA_SETTLE, FILE_TIMEOUT, Harness, add_block, add_peg, extruded_plate, pickable_described,
    run_from_palette, vertex_at,
};
use crate::{
    selection::{Pickable, PrincipalPlane},
    units::LengthUnit,
};

#[test]
fn two_picked_vertices_are_measured_in_the_panel_and_the_view() {
    let mut harness = Harness::new();
    let (body, _) = extruded_plate(&mut harness);
    let revision = harness.model.revision();
    let corner_point = caditor_geometry::Point3::ZERO;
    let far_point = caditor_geometry::Point3::new(40.0, 40.0, 10.0);
    let corner = vertex_at(&harness, body, corner_point);
    let far = vertex_at(&harness, body, far_point);

    harness.key(Key::I, Modifiers::NONE);
    harness.frame();
    assert!(harness.workspace.measure.open);
    assert!(harness.shows(crate::measure_panel::TITLE));

    harness.click_pickable(Plane::XY, Point2::ZERO, corner);
    assert_eq!(
        harness
            .workspace
            .viewport
            .selection()
            .iter()
            .collect::<Vec<_>>(),
        vec![corner]
    );
    let Pickable::Vertex { vertex: key, .. } = corner else {
        panic!("a vertex was picked");
    };
    let shown = crate::bodies::shown(harness.model.evaluation(), body).unwrap();
    let id = crate::bodies::find_vertex(shown, key).expect("the name resolves");
    assert_eq!(shown.names().vertex_name(id), Some(key.name));
    assert_eq!(shown.solid.vertex(id).unwrap().point(), corner_point);
    harness.wait_until("the vertex is measured", |harness| {
        harness.shows("0.000, 0.000, 0.000 mm")
    });

    harness.select([corner, far]);
    let expected = LengthUnit::Millimetre.measured_length(corner_point.distance(far_point));
    harness.wait_until("the distance is measured", |harness| {
        harness.shows(&expected)
    });

    assert!(
        harness.count_shown(&expected) >= 2,
        "the panel and the view"
    );
    assert!(harness.shows("Between them"));
    assert!(harness.shows("Along X"));
    assert_eq!(harness.count_shown("40.000 mm"), 2);
    assert!(harness.shows("10.000 mm"));
    assert_eq!(harness.model.revision(), revision);

    harness.key(Key::I, Modifiers::NONE);
    harness.frame();
    assert!(!harness.workspace.measure.open);
    assert!(!harness.shows(&expected));
}

#[test]
fn centres_of_mass_are_markers_in_the_view_that_can_be_picked_and_measured() {
    let mut harness = Harness::new();
    let (body, _) = extruded_plate(&mut harness);
    let marker = Pickable::CentreOfMass(body);
    let marked = |harness: &mut Harness| {
        harness
            .built()
            .picks
            .pickables()
            .any(|pickable| pickable == marker)
    };

    assert!(!marked(&mut harness));

    run_from_palette(&mut harness, "show or hide centres of mass");
    harness.settle();

    assert!(marked(&mut harness));

    harness.key(Key::I, Modifiers::NONE);
    harness.select([marker]);
    harness.wait_until("the centre of mass is measured", |harness| {
        harness.shows("Centre of mass of Extrude 1") && harness.shows("Position")
    });

    run_from_palette(&mut harness, "show or hide centres of mass");
    harness.frame();
    harness.frame();

    assert!(!marked(&mut harness));
    assert!(harness.workspace.viewport.selection().is_empty());
}

#[test]
fn the_interference_panel_finds_bodies_that_overlap_or_touch_and_shows_where() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    add_peg(&mut harness);
    add_block(
        &mut harness,
        "Block",
        [Point2::new(-20.0, 0.0), Point2::new(0.0, 10.0)],
        "5 mm",
    );
    let block_side = pickable_described(&mut harness, "Block › Block end face");
    let revision = harness.model.revision();

    harness.select([]);
    harness.click(crate::toolbar::INTERFERENCE_LABEL);
    harness.wait_until("every pair is checked", |harness| {
        harness.shows("1 pair overlaps and 1 pair touches.")
    });

    assert!(harness.workspace.interference.open);
    assert!(harness.shows(crate::interference_panel::EVERYTHING));
    assert!(harness.shows("Extrude 1 and Peg"));
    assert!(harness.shows("1000.0 mm³"));
    assert!(harness.shows("35.000, 20.000, 2.500 mm"));
    assert!(harness.shows("Extrude 1 and Block"));
    assert!(harness.shows("0.000, 5.000, 2.500 mm"));
    assert!(harness.shows("Extrude 1 and Block touch"));
    assert!(!harness.shows("Peg and Block"));
    assert_eq!(harness.model.revision(), revision);

    harness.click(crate::interference_panel::SHOW_PLACE);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let target = harness.workspace.viewport.viewpoint().target;
    assert!(
        target.distance(caditor_geometry::Point3::new(35.0, 20.0, 2.5)) < 1e-6,
        "{target:?}"
    );
    assert!(harness.shows("Extrude 1 and Peg overlap"));

    harness.select([block_side]);
    harness.wait_until("the block is checked against the rest", |harness| {
        harness.shows("1 pair touches.")
    });
    assert!(harness.shows_containing("Block against every other body shown"));
    assert!(!harness.shows("Extrude 1 and Peg"));

    harness.select([top, block_side]);
    harness.wait_until("the two chosen bodies are checked", |harness| {
        harness.shows(crate::interference_panel::CHOSEN)
    });
    assert!(harness.shows("1 pair touches."));

    harness.click_button(crate::interference_panel::CLOSE);
    assert!(!harness.workspace.interference.open);
    assert!(!harness.shows("Extrude 1 and Peg overlap"));
}

#[test]
fn planes_axes_and_sketch_curves_are_measured() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(10.0, 10.0), 5.0);
    let circle_sketch = harness.add_sketch(sketch);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::I, Modifiers::NONE);
    harness.frame();

    harness.select([Pickable::Plane(PrincipalPlane::Xy), top]);
    harness.wait_until("the gap between the planes is measured", |harness| {
        harness.shows("Gap between the planes")
    });
    let normal_shown = harness.shows("(0, 0, 1)");
    let gap_shown = harness.count_shown("10.000 mm") >= 1;
    harness.select([Pickable::Axis(crate::selection::Axis::Z), top]);
    harness.wait_until("the axis is measured against the face", |harness| {
        harness.shows("Angle to the plane")
    });
    harness.select([Pickable::SketchEntity {
        feature: circle_sketch,
        entity: circle,
    }]);
    harness.wait_until("the circle is measured", |harness| {
        harness.shows("Radius") && harness.shows("Diameter")
    });
    let radius_shown = harness.shows("5.000 mm") && harness.shows("10.000 mm");

    assert!(normal_shown);
    assert!(gap_shown);
    assert!(radius_shown);
    assert!(!harness.shows(crate::measure::UNMEASURABLE));
}

#[test]
fn the_measure_panel_shows_the_mass_properties_of_a_body_and_the_area_of_a_face() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    let revision = harness.model.revision();

    harness.select([]);
    harness.click(crate::toolbar::MEASURE_LABEL);
    assert!(harness.workspace.measure.open);
    harness.wait_until("the mass properties are shown", |harness| {
        harness.shows("16000.0 mm³")
    });

    assert!(harness.shows(crate::measure_panel::MASS_TITLE));
    assert!(harness.shows("4800.00 mm²"));
    assert!(harness.shows("20.000, 20.000, 5.000 mm"));
    assert!(harness.shows("Extrude 1"));

    harness.select([top]);
    harness.wait_until("the face is measured", |harness| {
        harness.shows("1600.00 mm²")
    });
    assert!(harness.shows("Extrude 1 › Extrude 1 end face"));
    assert!(harness.shows("16000.0 mm³"));
    assert_eq!(harness.model.revision(), revision);

    harness.click_button(crate::measure_panel::CLOSE);
    assert!(!harness.workspace.measure.open);
}

#[test]
fn the_measure_panel_gives_exact_mass_properties_of_a_round_body() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_circle(Point2::ZERO, 10.0);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.click(crate::toolbar::MEASURE_LABEL);
    harness.wait_until("the mass properties are shown", |harness| {
        harness.shows("3141.6 mm³")
    });

    assert!(harness.shows("1256.64 mm²"));
    assert!(harness.shows("20.000 × 20.000 × 10.000 mm"));
    assert!(harness.shows("0.000, 0.000, 5.000 mm"));
    assert!(!harness.shows_containing("display mesh"));
    assert!(!harness.shows_containing("≈"));
}

#[test]
fn the_measure_panel_keeps_the_last_readout_until_the_next_one_arrives() {
    let mut harness = Harness::new();
    let (body, _) = extruded_plate(&mut harness);
    let corner = vertex_at(&harness, body, caditor_geometry::Point3::ZERO);
    let far = vertex_at(
        &harness,
        body,
        caditor_geometry::Point3::new(40.0, 40.0, 10.0),
    );
    harness.key(Key::I, Modifiers::NONE);
    harness.frame();
    harness.select([far]);
    harness.wait_until("the far corner is measured", |harness| {
        harness.shows("40.000, 40.000, 10.000 mm")
    });

    let mut blank_frames = 0;
    harness.select([corner]);
    let deadline = Instant::now() + FILE_TIMEOUT;
    while !harness.shows("0.000, 0.000, 0.000 mm") {
        assert!(Instant::now() < deadline, "the corner is never measured");
        if !harness.shows("40.000, 40.000, 10.000 mm") {
            blank_frames += 1;
        }
        harness.frame();
    }

    assert_eq!(blank_frames, 0);
    assert!(!harness.shows(crate::measure_panel::MEASURING));
}

#[test]
fn working_out_mass_properties_can_be_cancelled_and_asked_for_again() {
    use crate::{
        bodies::{Hold, Holds},
        measure_panel,
    };

    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let holds = Holds {
        measures: Hold::holding(),
        ..Holds::default()
    };
    harness.model.hold_body_work(&holds);
    harness.select([]);

    harness.click(crate::toolbar::MEASURE_LABEL);
    harness.wait_until("the mass properties are being worked out", |harness| {
        harness.shows(measure_panel::WORKING_OUT_MASSES)
    });
    let working = harness.shows("Working out this body's mass properties.");
    harness.click("Cancel");
    harness.wait_until("the mass properties stopped", |harness| {
        harness.shows(&measure_panel::stopped_masses_text(1)) && !harness.model.masses_pending()
    });

    assert!(working);
    assert!(harness.shows("Stopped before this body's mass properties were worked out."));
    assert!(!harness.shows(measure_panel::WORKING_OUT_MASSES));
    assert!(!harness.shows("16000.0 mm³"));

    holds.measures.release();
    harness.click(measure_panel::WORK_OUT_MASSES);
    harness.wait_until("the mass properties are shown", |harness| {
        harness.shows("16000.0 mm³")
    });

    assert!(!harness.shows(&measure_panel::stopped_masses_text(1)));
    assert!(!harness.shows(measure_panel::WORKING_OUT_MASSES));
}
