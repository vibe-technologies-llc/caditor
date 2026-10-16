use std::f64::consts::PI;

use caditor_document::{BodyOperation, ExtrudeExtent, FeatureId, SolidFeature};
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, choose, extruded_plate, extrusion_above_plate, feature_named,
    open_combo, open_solid, rectangle, run_from_palette, start_of_solid,
};
use crate::{editing::EditingCommand, model::Action, selection::Pickable};

#[test]
fn an_extrusion_takes_a_start_offset_from_its_panel_and_zero_clears_it() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.settle();
    let start_of = |harness: &Harness| harness.solid(extrude).clone();
    let start = |solid: SolidFeature| match solid {
        SolidFeature::Extrude(extrude) => extrude.start,
        SolidFeature::Revolve(_) | SolidFeature::Sweep(_) => panic!("expected an extrusion"),
    };
    let lowest = |harness: &Harness| {
        harness
            .model
            .evaluation()
            .body_result(extrude)
            .and_then(|result| result.solid())
            .and_then(|solid| solid.bounding_box())
            .map(|bounds| bounds.min().z)
            .unwrap()
    };
    assert!(harness.shows("Start offset"));
    assert_eq!(start(start_of(&harness)), None);
    assert!(lowest(&harness).abs() < 1e-9);

    harness.type_into_field(Id::unique(("solid-field", "start", extrude)), "6 mm");
    harness.settle();
    assert_eq!(
        start(start_of(&harness)).and_then(|start| start
            .distance()
            .map(|e| harness.document().expression_text(e))),
        Some("6 mm".to_owned())
    );
    assert!((lowest(&harness) - 6.0).abs() < 1e-9);

    harness.type_into_field(Id::unique(("solid-field", "start", extrude)), "0 mm");
    harness.settle();
    assert_eq!(start(start_of(&harness)), None);
    assert!(lowest(&harness).abs() < 1e-9);
}

fn extrude_extent(harness: &Harness, feature: FeatureId) -> &'static str {
    match harness.solid(feature) {
        SolidFeature::Extrude(extrude) => match extrude.extent {
            ExtrudeExtent::OneSide { .. } => "one side",
            ExtrudeExtent::Symmetric { .. } => "symmetric",
            ExtrudeExtent::TwoSides { .. } => "two sides",
        },
        SolidFeature::Revolve(_) => "a revolve",
        SolidFeature::Sweep(_) => "a sweep",
    }
}

#[test]
fn a_second_sketch_extrudes_as_a_new_body_unless_it_reaches_the_first() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let extruded_rectangle = |harness: &mut Harness, min: Point2, max: Point2| {
        let mut sketch = Sketch::new(Plane::XY);
        rectangle(&mut sketch, min, max);
        harness.add_sketch(sketch);
        harness.select([]);
        harness.click("Extrude");
        harness.settle();
        let feature = open_solid(harness);
        let reads_new_body = harness.shows("New body");
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        (feature, reads_new_body)
    };

    let (apart, apart_reads_new_body) = extruded_rectangle(
        &mut harness,
        Point2::new(60.0, 0.0),
        Point2::new(80.0, 20.0),
    );
    let (overlapping, _) = extruded_rectangle(
        &mut harness,
        Point2::new(30.0, 30.0),
        Point2::new(50.0, 50.0),
    );

    assert_eq!(harness.solid(apart).operation(), BodyOperation::NewBody);
    assert!(apart_reads_new_body);
    assert_eq!(
        harness.solid(overlapping).operation(),
        BodyOperation::Add(plate)
    );
    assert_eq!(harness.document().bodies_standing(), vec![plate, apart]);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn a_solid_feature_changes_its_extent_result_body_sketch_and_axis_from_its_panel() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let mut boss = Sketch::new(Plane::XY);
    rectangle(&mut boss, Point2::new(10.0, 10.0), Point2::new(20.0, 20.0));
    let boss = harness.add_sketch(boss);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    let second = open_solid(&harness);
    let starts_adding = harness.solid(second).operation() == BodyOperation::Add(plate);

    choose(&mut harness, "One side", "Symmetric");
    let symmetric = extrude_extent(&harness, second);
    choose(&mut harness, "Symmetric", "Two sides");
    let two_sides = extrude_extent(&harness, second);
    choose(&mut harness, "Add to body", "Remove from body");
    let removing = harness.solid(second).operation();
    choose(&mut harness, "Remove from body", "New body");
    let separate = harness.solid(second).operation();
    choose(&mut harness, "Plate 1", "Plate");
    let sketch = harness.solid(second).sketch();
    choose(&mut harness, "Plate", "Plate 1");

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let mut post = Sketch::new(Plane::XY);
    rectangle(&mut post, Point2::new(25.0, 25.0), Point2::new(30.0, 30.0));
    harness.add_sketch(post);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    let third = open_solid(&harness);
    let adds_to_the_plate_it_stands_on =
        harness.solid(third).operation() == BodyOperation::Add(plate);
    harness.hold_still();
    let third_card_in_view = harness.shows("Body");
    choose(&mut harness, "Extrude 1", "Extrude 2");
    let retargeted = harness.solid(third).operation();

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let mut section = Sketch::new(Plane::XZ);
    rectangle(
        &mut section,
        Point2::new(50.0, 0.0),
        Point2::new(60.0, 10.0),
    );
    harness.add_sketch(section);
    harness.select([]);
    harness.click("Revolve");
    harness.settle();
    let revolve = open_solid(&harness);
    choose(&mut harness, "Vertical axis", "Horizontal axis");
    let axis = harness.solid(revolve).axis().cloned();
    choose(&mut harness, "Full turn", "Symmetric");
    let turn = match harness.solid(revolve) {
        SolidFeature::Revolve(revolve) => revolve.extent.clone(),
        SolidFeature::Extrude(_) | SolidFeature::Sweep(_) => panic!("expected a revolve"),
    };

    assert!(starts_adding);
    assert_eq!(symmetric, "symmetric");
    assert_eq!(two_sides, "two sides");
    assert_eq!(removing, BodyOperation::Remove(plate));
    assert_eq!(separate, BodyOperation::NewBody);
    assert_eq!(sketch, feature_named(&harness, "Plate"));
    assert_eq!(harness.solid(second).sketch(), boss);
    assert!(adds_to_the_plate_it_stands_on);
    assert!(third_card_in_view);
    assert_eq!(retargeted, BodyOperation::Add(second));
    assert_eq!(
        axis,
        Some(caditor_document::RevolveAxis::Sketch(
            caditor_sketch::Reference::HorizontalAxis.id()
        ))
    );
    assert!(matches!(
        turn,
        caditor_document::RevolveExtent::Symmetric { .. }
    ));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn a_revolve_turns_up_to_a_plane_through_its_axis_chosen_in_its_panel() {
    let mut harness = Harness::new();
    let mut section = Sketch::new(Plane::XZ);
    rectangle(&mut section, Point2::new(2.0, 0.0), Point2::new(4.0, 3.0));
    harness.add_sketch(section);
    harness.select([]);
    harness.click("Revolve");
    harness.settle();
    let revolve = open_solid(&harness);

    choose(&mut harness, "Full turn", crate::solid_panel::TURN_UP_TO);
    let asks_for_a_plane =
        harness.shows("Click a flat face or plane through the axis to turn up to.");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.select([Pickable::Plane(caditor_document::PrincipalPlane::Yz)]);
    choose(&mut harness, "Full turn", crate::solid_panel::TURN_UP_TO);
    let turned = match harness.solid(revolve) {
        SolidFeature::Revolve(revolve) => revolve.extent.clone(),
        SolidFeature::Extrude(_) | SolidFeature::Sweep(_) => panic!("expected a revolve"),
    };
    let volume = harness.body_volume(revolve);

    assert!(asks_for_a_plane);
    assert_eq!(
        turned,
        caditor_document::RevolveExtent::up_to(
            caditor_document::PlaneReference::Principal(caditor_document::PrincipalPlane::Yz),
            false
        )
    );
    let quarter = std::f64::consts::PI * (16.0 - 4.0) * 3.0 / 4.0;
    assert!((volume - quarter).abs() < 0.01 * quarter, "{volume}");
    assert!(harness.shows("Up to"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn an_extrusion_runs_along_a_selected_sketch_line_chosen_in_its_panel() {
    let mut harness = Harness::new();
    let mut guide = Sketch::new(Plane::XZ);
    let slant = guide.add_line(Point2::new(0.0, 0.0), Point2::new(10.0, 10.0));
    let guide = harness.add_sketch(guide);
    let mut square = Sketch::new(Plane::XY);
    rectangle(&mut square, Point2::new(0.0, 0.0), Point2::new(10.0, 10.0));
    harness.add_sketch(square);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    let leaning = open_solid(&harness);

    harness.select([Pickable::SketchEntity {
        feature: guide,
        entity: slant,
    }]);
    choose(
        &mut harness,
        crate::solid_panel::SQUARE,
        crate::solid_panel::ALONG,
    );
    let direction = match harness.solid(leaning) {
        SolidFeature::Extrude(extrude) => extrude.direction.clone(),
        SolidFeature::Revolve(_) | SolidFeature::Sweep(_) => panic!("expected an extrusion"),
    };
    let volume = harness.body_volume(leaning);
    let named = harness.shows("Along");

    assert_eq!(
        direction.as_deref(),
        Some(&caditor_document::AxisReference::Sketch {
            sketch: guide,
            entity: slant
        })
    );
    let rise = 10.0 * std::f64::consts::FRAC_1_SQRT_2;
    assert!((volume - 100.0 * rise).abs() < 0.01 * volume, "{volume}");
    assert!(named);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

fn extent_of(harness: &Harness, feature: FeatureId) -> ExtrudeExtent {
    match harness.solid(feature) {
        SolidFeature::Extrude(extrude) => extrude.extent.clone(),
        SolidFeature::Revolve(_) | SolidFeature::Sweep(_) => panic!("expected an extrusion"),
    }
}

fn target_origin(extent: &ExtrudeExtent) -> Option<caditor_kernel::FaceOrigin> {
    let ExtrudeExtent::OneSide { end, .. } = extent else {
        return None;
    };
    let Some(caditor_document::PlaneReference::Face(attachment)) = end.target() else {
        return None;
    };
    attachment.face.origin()
}

#[test]
fn an_extrusion_cuts_through_all_or_up_to_the_next_face_chosen_in_its_panel() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let cut = extrusion_above_plate(&mut harness, 10.0);

    open_combo(&mut harness, "End");
    harness.hover("Through all");
    let explained = harness.shows(
        "Through all cuts into a body or intersects with it; choose Remove from body or Intersect \
         with body first",
    );
    open_combo(&mut harness, "End");
    harness.settle();
    let while_adding = extent_of(&harness, cut);
    choose(&mut harness, "Add to body", "Remove from body");
    harness.click_lowest(crate::feature_fields::REVERSE_DIRECTION);
    harness.settle();
    open_combo(&mut harness, "End");
    harness.click_lowest("Through all");
    harness.settle();
    let through = extent_of(&harness, cut);
    let through_volume = harness.body_volume(plate);
    choose(&mut harness, "Through all", "Up to next");
    let next = extent_of(&harness, cut);
    let next_volume = harness.body_volume(plate);
    harness.perform(Action::Undo);
    harness.settle();
    let undone = extent_of(&harness, cut);

    assert!(explained);
    assert!(matches!(
        while_adding,
        ExtrudeExtent::OneSide {
            end: caditor_document::ExtrudeEnd::Distance(_),
            reversed: false
        }
    ));
    assert_eq!(
        through,
        ExtrudeExtent::OneSide {
            end: caditor_document::ExtrudeEnd::ThroughAll,
            reversed: true
        }
    );
    assert!((through_volume - 15_000.0).abs() < 1.0);
    assert_eq!(
        next,
        ExtrudeExtent::OneSide {
            end: caditor_document::ExtrudeEnd::up_to_next(),
            reversed: true
        }
    );
    assert!((next_volume - 15_000.0).abs() < 1.0);
    assert_eq!(undone, through);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn an_end_up_to_the_next_face_stops_short_of_it_by_the_offset_typed_in_its_panel() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let cut = extrusion_above_plate(&mut harness, 10.0);
    choose(&mut harness, "Add to body", "Remove from body");
    harness.click_lowest(crate::feature_fields::REVERSE_DIRECTION);
    harness.settle();
    open_combo(&mut harness, "End");
    harness.click_lowest("Up to next");
    harness.settle();

    let offered = harness.shows(crate::solid_panel::END_OFFSET);
    let field = Id::unique(("solid-field", "distance-offset", cut));
    harness.type_into_field(field, "-4 mm");
    harness.settle();
    let short = extent_of(&harness, cut);
    let volume = harness.body_volume(plate);
    harness.type_into_field(field, "0 mm");
    harness.settle();
    let cleared = extent_of(&harness, cut);

    assert!(offered);
    assert_eq!(
        short,
        ExtrudeExtent::OneSide {
            end: caditor_document::ExtrudeEnd::up_to_next().with_offset(Some(
                caditor_expression::Expression::Negate(Box::new(
                    caditor_expression::Expression::Measure(
                        4.0,
                        caditor_expression::Unit::Millimetre
                    )
                ))
            )),
            reversed: true
        }
    );
    assert!((volume - 15_400.0).abs() < 1.0);
    assert_eq!(
        cleared,
        ExtrudeExtent::OneSide {
            end: caditor_document::ExtrudeEnd::up_to_next(),
            reversed: true
        }
    );
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn an_extrusion_runs_up_to_a_selected_curved_face() {
    let mut harness = Harness::new();
    let across = Plane::from_frame(
        caditor_geometry::Point3::new(-10.0, 0.0, 20.0),
        caditor_geometry::Vector3::X,
        caditor_geometry::Vector3::Y,
    )
    .unwrap();
    let mut rod = Sketch::new(across);
    rod.add_circle(Point2::new(0.0, 0.0), 5.0);
    harness.add_sketch(rod);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    let rod_body = harness.workspace.editing.solid().expect("the rod is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let mut strip = Sketch::new(Plane::XY);
    rectangle(&mut strip, Point2::new(-6.0, -1.0), Point2::new(-5.0, 1.0));
    harness.add_sketch(strip);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    let stand = open_solid(&harness);
    choose(&mut harness, "New body", "Add to body");
    harness.frame();

    let curved = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            matches!(pickable, Pickable::Face { .. })
                && pickable
                    .describe(harness.document(), harness.model.evaluation())
                    .starts_with("Extrude 1 › Extrude 1 side")
        })
        .expect("the side of the rod is pickable");
    harness.select([curved]);
    run_from_palette(&mut harness, "extrude up to selected");
    harness.settle();
    let reached = extent_of(&harness, stand);

    assert!(
        matches!(
            reached,
            ExtrudeExtent::OneSide {
                end: caditor_document::ExtrudeEnd::UpToSurface { .. },
                ..
            }
        ),
        "{reached:?}"
    );
    assert_eq!(
        harness.model.evaluation().failed_count(),
        0,
        "{:?}",
        harness
            .model
            .evaluation()
            .feature(stand)
            .map(|found| &found.state)
    );
    let half = 1.0_f64;
    let under = 2.0 * (half / 2.0 * (25.0 - half * half).sqrt() + 12.5 * (half / 5.0).asin());
    let expected = 250.0 * std::f64::consts::PI + 40.0 - under;
    let found = harness.body_volume(rod_body);
    assert!(
        (found - expected).abs() < 0.01 * expected,
        "{found} {expected}"
    );
}

#[test]
fn an_extrusion_runs_up_to_a_face_chosen_in_the_view_and_chosen_again() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let tower = extrusion_above_plate(&mut harness, 30.0);
    choose(&mut harness, "New body", "Add to body");
    harness.click_lowest(crate::feature_fields::REVERSE_DIRECTION);
    harness.settle();

    open_combo(&mut harness, "End");
    harness.click_lowest("Up to face");
    harness.settle();
    let asks_for_a_face = harness.shows("Click a face or plane to extrude up to.");
    let still_a_distance = extent_of(&harness, tower);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    harness.settle();
    let picking_ended = harness.workspace.editing.picking().is_none();
    let reached = extent_of(&harness, tower);
    let volume = harness.body_volume(plate);
    let named = harness.shows("Extrude 1 end face");

    let bottom = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            pickable.describe(harness.document(), harness.model.evaluation())
                == "Extrude 1 › Extrude 1 start face"
        })
        .expect("the bottom face is pickable");
    harness.select([bottom]);
    harness.click_button("Use selected");
    harness.settle();
    let chosen_again = extent_of(&harness, tower);
    harness.select([top]);
    run_from_palette(&mut harness, "extrude up to selected");
    harness.settle();
    let from_palette = extent_of(&harness, tower);

    let plate_feature = plate.raw();
    assert!(asks_for_a_face);
    assert!(matches!(
        still_a_distance,
        ExtrudeExtent::OneSide {
            end: caditor_document::ExtrudeEnd::Distance(_),
            ..
        }
    ));
    assert!(picking_ended);
    assert_eq!(
        target_origin(&reached),
        Some(caditor_kernel::FaceOrigin::EndCap {
            feature: plate_feature
        })
    );
    assert!(matches!(
        reached,
        ExtrudeExtent::OneSide { reversed: true, .. }
    ));
    assert!((volume - 18_000.0).abs() < 1.0);
    assert!(named);
    assert_eq!(
        target_origin(&chosen_again),
        Some(caditor_kernel::FaceOrigin::StartCap {
            feature: plate_feature
        })
    );
    assert_eq!(from_palette, reached);
    assert_eq!(harness.model.undo_label(), Some("Edit Extrude 2"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn an_extrusion_starts_at_a_face_chosen_in_the_view_and_goes_back_to_its_sketch_plane() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let tower = extrusion_above_plate(&mut harness, 30.0);
    choose(&mut harness, "New body", "Add to body");
    let highest = |harness: &Harness| {
        harness
            .model
            .evaluation()
            .body_result(plate)
            .and_then(|result| result.solid())
            .and_then(|solid| solid.bounding_box())
            .map(|bounds| bounds.max().z)
            .unwrap()
    };

    open_combo(&mut harness, "Start");
    harness.click_lowest("Face or plane");
    harness.settle();
    let asks_for_a_face =
        harness.shows("Click a flat face or plane parallel to the sketch to start from.");
    let still_at_the_sketch = start_of_solid(&harness, tower);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    harness.settle();
    let picking_ended = harness.workspace.editing.picking().is_none();
    let started = start_of_solid(&harness, tower);
    let from_the_face = highest(&harness);
    let named = harness.shows("Starts at");

    harness.click_button("Start at the sketch plane again");
    harness.settle();
    let cleared = start_of_solid(&harness, tower);
    let back_at_the_sketch = highest(&harness);

    assert!(asks_for_a_face);
    assert_eq!(still_at_the_sketch, None);
    assert!(picking_ended);
    assert!(named);
    assert!(matches!(
        started,
        Some(caditor_document::SolidStart::Plane(
            caditor_document::PlaneReference::Face(ref attachment)
        )) if attachment.body == plate
            && attachment.face.origin()
                == Some(caditor_kernel::FaceOrigin::EndCap { feature: plate.raw() })
    ));
    assert!((from_the_face - 20.0).abs() < 1e-9);
    assert_eq!(cleared, None);
    assert!((back_at_the_sketch - 40.0).abs() < 1e-9);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn a_revolve_turns_by_two_angles_set_in_its_panel() {
    let mut harness = Harness::new();
    let mut section = Sketch::new(Plane::XZ);
    rectangle(
        &mut section,
        Point2::new(10.0, 0.0),
        Point2::new(20.0, 10.0),
    );
    harness.add_sketch(section);
    harness.select([]);
    harness.click("Revolve");
    harness.settle();
    let revolve = open_solid(&harness);
    let full = PI * (400.0 - 100.0) * 10.0;

    choose(&mut harness, "Full turn", "Two angles");
    let both_ways = harness.body_volume(revolve);
    let field = Id::unique(("solid-field", "forward-angle", revolve));
    harness.type_into_field(field, "350 deg");
    let refused = harness.shows(crate::feature_fields::TURNS_TOGETHER);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.type_into_field(field, "90 deg");
    harness.settle();
    let turned = match harness.solid(revolve) {
        SolidFeature::Revolve(revolve) => revolve.extent.clone(),
        SolidFeature::Extrude(_) | SolidFeature::Sweep(_) => panic!("expected a revolve"),
    };

    assert!((both_ways - full * 210.0 / 360.0).abs() < 0.01 * full);
    assert!(refused);
    assert_eq!(
        turned,
        caditor_document::RevolveExtent::TwoSides {
            forward: Expression::Measure(90.0, Unit::Degree),
            backward: Expression::Measure(30.0, Unit::Degree),
        }
    );
    assert!((harness.body_volume(revolve) - full / 3.0).abs() < 0.01 * full);
}

#[test]
fn a_revolve_crossing_its_axis_keeps_the_larger_side_and_then_the_other() {
    let mut harness = Harness::new();
    let mut section = Sketch::new(Plane::XZ);
    rectangle(
        &mut section,
        Point2::new(-5.0, 0.0),
        Point2::new(20.0, 10.0),
    );
    harness.add_sketch(section);
    harness.select([]);
    harness.click("Revolve");
    harness.settle();
    let revolve = open_solid(&harness);
    let side = |harness: &Harness| match harness.solid(revolve) {
        SolidFeature::Revolve(revolve) => revolve.side,
        SolidFeature::Extrude(_) | SolidFeature::Sweep(_) => panic!("expected a revolve"),
    };
    let failed = matches!(
        harness
            .model
            .evaluation()
            .feature(revolve)
            .map(|status| status.state.clone()),
        Some(caditor_document::FeatureState::Failed(_))
    );

    harness.click("One side");
    harness.settle();
    let larger = side(&harness);
    let larger_volume = harness.body_volume(revolve);
    harness.click(crate::solid_panel::KEEP_OTHER_SIDE);
    harness.settle();
    let other = side(&harness);
    let other_volume = harness.body_volume(revolve);
    harness.click("Whole");
    harness.settle();

    assert!(failed);
    assert_eq!(larger, Some(caditor_document::AxisSide::Right));
    assert!((larger_volume - PI * 400.0 * 10.0).abs() < 0.01 * larger_volume);
    assert_eq!(other, Some(caditor_document::AxisSide::Left));
    assert!((other_volume - PI * 25.0 * 10.0).abs() < 0.01 * other_volume);
    assert_eq!(side(&harness), None);
}

#[test]
fn a_revolve_lists_the_model_axes_beside_its_sketch_lines_and_turns_about_one_chosen() {
    let mut harness = Harness::new();
    let mut section = Sketch::new(Plane::XZ);
    rectangle(&mut section, Point2::new(2.0, 0.0), Point2::new(4.0, 3.0));
    harness.add_sketch(section);
    harness.select([]);
    harness.click("Revolve");
    harness.settle();
    let revolve = open_solid(&harness);

    open_combo(&mut harness, crate::solid_panel::AXIS);
    let listed = harness.shows("The X axis") && harness.shows("Vertical axis");
    harness.click_lowest("The X axis");
    harness.settle();

    assert!(listed);
    assert_eq!(
        axis_of(harness.solid(revolve)),
        Some(caditor_document::RevolveAxis::Model(
            caditor_document::AxisReference::Principal(caditor_document::PrincipalAxis::X)
        ))
    );
}

fn axis_of(solid: &SolidFeature) -> Option<caditor_document::RevolveAxis> {
    match solid {
        SolidFeature::Revolve(revolve) => Some(revolve.axis.clone()),
        SolidFeature::Extrude(_) | SolidFeature::Sweep(_) => None,
    }
}
