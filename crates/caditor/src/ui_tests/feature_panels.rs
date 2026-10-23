use caditor_document::{
    AxisReference, Datum, DatumAxis, DatumResult, Edit, FeatureId, PatternKind, PlaneThrough,
    PointBy, PrincipalAxis, Transaction,
};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, blend_of, choose, datum_of, datum_plane_of, extruded_plate, offer,
    pattern_of, rectangle, run_from_palette, top_edge_along_x, volume_about,
};
use crate::{
    bodies,
    commands::Command,
    datum_tools::DatumFromSelectionError,
    feature_fields::{
        self, ABOVE_ZERO_OR_REVERSE, CHOOSE_IN_VIEW, MISSING_BODY, REVERSE_DIRECTION, TURN,
    },
    mirror_panel,
    model::Action,
    reference_picking::Slot,
    selection::{Axis, Pickable, PrincipalPlane},
};

fn click_in_view(harness: &mut Harness, pickable: Pickable) {
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(0.0, 0.0), pickable);
    harness.settle();
}

fn picking_slot(harness: &Harness) -> Option<Slot> {
    harness
        .workspace
        .editing
        .picking()
        .map(|picking| picking.slot)
}

#[test]
fn a_circular_pattern_waits_for_an_axis_clicked_in_the_view_instead_of_choosing_one() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.select([]);
    harness.click("Circular pattern");
    harness.settle();
    let pattern = harness
        .workspace
        .editing
        .solid()
        .expect("the pattern is open");
    harness.hold_still();

    let asked = picking_slot(&harness);
    let prompted = harness.shows("Click an axis, straight edge or round face to turn about");
    let PatternKind::Circular(waiting) = &pattern_of(&harness, pattern).kind else {
        panic!("the pattern is circular");
    };
    let waiting = waiting.axis.clone();
    let state = harness
        .model
        .evaluation()
        .feature(pattern)
        .map(|status| status.state.clone());
    let told = harness.shows("Waiting for an axis");
    click_in_view(&mut harness, Pickable::Axis(Axis::Y));
    let PatternKind::Circular(chosen) = &pattern_of(&harness, pattern).kind else {
        panic!("the pattern stays circular");
    };
    let chosen = chosen.axis.clone();

    assert_eq!(asked, Some(Slot::PatternDirection));
    assert!(prompted);
    assert_eq!(waiting, None);
    assert_eq!(
        state,
        Some(caditor_document::FeatureState::Waiting(
            caditor_document::MissingInput::Axis
        )),
        "the pattern waits for an axis rather than failing"
    );
    assert!(told);
    assert_eq!(chosen, Some(AxisReference::Principal(PrincipalAxis::Y)));
    assert_eq!(picking_slot(&harness), None);
}

#[test]
fn a_pattern_takes_its_second_direction_clicked_in_the_view_after_choose_in_the_view() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    harness.select([]);
    harness.click("Linear pattern");
    harness.settle();
    let pattern = harness
        .workspace
        .editing
        .solid()
        .expect("the pattern is open");
    harness.hold_still();

    let offered = harness.shows(CHOOSE_IN_VIEW);
    harness.click_beside(CHOOSE_IN_VIEW, "Second direction");
    harness.settle();
    let asked = picking_slot(&harness);
    let prompted =
        harness.shows("Click an axis, straight edge or round face to also repeat along.");
    click_in_view(&mut harness, Pickable::Axis(Axis::Y));
    let PatternKind::Linear { second, .. } = &pattern_of(&harness, pattern).kind else {
        panic!("the pattern stays linear");
    };
    let second = second.clone();

    assert!(offered);
    assert_eq!(asked, Some(Slot::PatternSecond));
    assert!(prompted);
    assert!(second.is_some());
    assert_eq!(picking_slot(&harness), None);
    assert_eq!(harness.workspace.editing.solid(), Some(pattern));
    assert_eq!(pattern_of(&harness, pattern).body, plate);
    assert_eq!(harness.model.undo_label(), Some("Edit Linear pattern 1"));
}

#[test]
fn a_datum_takes_its_references_clicked_in_the_view_and_escape_stops_choosing() {
    let mut harness = Harness::new();
    harness.select([]);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the plane is open");
    harness.hold_still();

    harness.click_beside(CHOOSE_IN_VIEW, "Turned about");
    harness.settle();
    let asked = picking_slot(&harness);
    click_in_view(&mut harness, Pickable::Axis(Axis::Z));
    let turned = datum_plane_of(&harness, plane).rotation.is_some();

    harness.click_beside(CHOOSE_IN_VIEW, "Starts from");
    harness.settle();
    let asked_again = picking_slot(&harness);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let stopped = picking_slot(&harness);
    let still_open = harness.workspace.editing.solid();

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.select([Pickable::Axis(Axis::X)]);
    harness.click("Axis");
    harness.settle();
    let axis = harness.workspace.editing.solid().expect("the axis is open");
    harness.hold_still();
    harness.select([]);
    harness.frame();
    harness.click_beside(CHOOSE_IN_VIEW, "Runs along");
    harness.settle();
    click_in_view(&mut harness, Pickable::Plane(PrincipalPlane::Xy));
    let waits_for_a_second =
        harness.shows_containing("Click a second plane or flat face that crosses the first");
    click_in_view(&mut harness, Pickable::Plane(PrincipalPlane::Xz));
    let meeting = matches!(
        datum_of(&harness, axis),
        Datum::Axis(DatumAxis::Intersection(..))
    );

    assert_eq!(asked, Some(Slot::DatumRotation));
    assert!(turned);
    assert_eq!(asked_again, Some(Slot::DatumBase));
    assert_eq!(stopped, None);
    assert_eq!(still_open, Some(plane));
    assert!(waits_for_a_second);
    assert!(meeting);
    assert_eq!(picking_slot(&harness), None);
    assert!(harness.shows("Defined by"));
}

#[test]
fn every_panel_refuses_a_value_with_the_shared_wording() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.select([]);
    harness.click("Linear pattern");
    harness.settle();
    let pattern = harness
        .workspace
        .editing
        .solid()
        .expect("the pattern is open");
    harness.hold_still();

    harness.type_into_field(Id::unique(("pattern-field", "spacing", pattern)), "0 mm");
    let spacing = harness.shows(ABOVE_ZERO_OR_REVERSE);
    choose(&mut harness, "Linear", "Circular");
    harness.type_into_field(Id::unique(("pattern-field", "angle", pattern)), "400 deg");
    let angle = harness.shows(TURN);

    assert!(spacing);
    assert!(angle);
    assert!(harness.shows(REVERSE_DIRECTION));
    assert!(!harness.shows("Reversed"));
    assert!(harness.shows(crate::pattern_panel::CIRCULAR_HINT));
    assert_eq!(
        crate::feature_fields::Rule::AboveZero.check(-1.0),
        Err(crate::feature_fields::RuleError::AboveZero)
    );
}

#[test]
fn a_missing_body_is_marked_and_a_refused_change_names_the_feature() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    harness.select([]);
    harness.click("Linear pattern");
    harness.settle();
    harness.hold_still();
    harness.perform(Action::Apply(Transaction::single(
        "Delete Extrude 1",
        Edit::RemoveFeature { id: plate },
    )));
    harness.settle();
    harness.hold_still();

    let refused = feature_fields::applied("Linear pattern 1", Err("It is in use".to_owned()));

    assert!(harness.shows(MISSING_BODY));
    assert!(!harness.shows("a missing body"));
    assert!(matches!(
        refused,
        Action::Inform(notice) if notice.text == "Linear pattern 1 was not changed: It is in use"
    ));
}

fn result_of(harness: &Harness, feature: caditor_document::FeatureId) -> DatumResult {
    crate::datum_tools::result(harness.model.evaluation(), feature).expect("the datum computed")
}

#[test]
fn datums_are_placed_through_selected_corners() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let corner = |harness: &Harness, x: f64, y: f64, z: f64| {
        super::vertex_at(harness, plate, Point3::new(x, y, z))
    };
    let top = [
        corner(&harness, 0.0, 0.0, 10.0),
        corner(&harness, 40.0, 0.0, 10.0),
        corner(&harness, 40.0, 40.0, 10.0),
    ];
    let diagonal = [
        corner(&harness, 0.0, 0.0, 0.0),
        corner(&harness, 40.0, 40.0, 10.0),
    ];

    harness.select(top);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the plane is open");
    let through_points = matches!(
        datum_of(&harness, plane),
        Datum::PlaneThrough(PlaneThrough::Points(_))
    );
    let shows_corners = harness.shows("Through three corners of Extrude 1");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.select(diagonal);
    harness.click("Axis");
    harness.settle();
    let axis = harness.workspace.editing.solid().expect("the axis is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.select([diagonal[0]]);
    harness.click("Point");
    harness.settle();
    let point = harness
        .workspace
        .editing
        .solid()
        .expect("the point is open");
    harness.type_into_field(Id::unique(("datum-field", "offset-z", point)), "-5 mm");
    harness.settle();

    let plane_result = result_of(&harness, plane).plane().unwrap();
    let axis_result = result_of(&harness, axis).axis().unwrap();
    assert!(through_points);
    assert!(shows_corners);
    assert!(plane_result.normal().cross(Vector3::Z).length() < 1e-9);
    assert!(
        plane_result
            .signed_distance(Point3::new(7.0, 3.0, 10.0))
            .abs()
            < 1e-9
    );
    assert!(
        axis_result
            .direction()
            .cross(Vector3::new(40.0, 40.0, 10.0))
            .length()
            < 1e-6
    );
    assert_eq!(
        result_of(&harness, point),
        DatumResult::Point(Point3::new(0.0, 0.0, -5.0))
    );
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn plane_and_point_refuse_a_selection_holding_nothing_they_can_use() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let mut square = Sketch::new(Plane::XY);
    rectangle(&mut square, Point2::new(60.0, 0.0), Point2::new(70.0, 10.0));
    let sketch = harness.add_sketch(square);
    let region = crate::selection::sketch_regions(harness.model.evaluation(), sketch)
        .and_then(|regions| regions.first())
        .map(|region| region.region.key())
        .expect("the sketch has a region");
    let features = harness.document().features().count();

    for (selected, tool, choices) in [
        (
            Pickable::SketchRegion {
                feature: sketch,
                region,
            },
            "datum plane",
            DatumFromSelectionError::PlaneChoices,
        ),
        (
            Pickable::CentreOfMass(plate),
            "datum plane",
            DatumFromSelectionError::PlaneChoices,
        ),
        (
            Pickable::SketchRegion {
                feature: sketch,
                region,
            },
            "datum point",
            DatumFromSelectionError::PointChoices,
        ),
        (
            Pickable::CentreOfMass(plate),
            "datum point",
            DatumFromSelectionError::PointChoices,
        ),
    ] {
        harness.select([selected]);
        run_from_palette(&mut harness, tool);
        harness.settle();

        assert_eq!(harness.document().features().count(), features);
        assert!(harness.workspace.editing.solid().is_none());
        assert!(
            harness
                .model
                .notice()
                .is_some_and(|notice| notice.text.contains(&choices.to_string())),
            "{tool} with {selected:?}"
        );
    }
}

#[test]
fn plane_and_point_with_nothing_selected_start_on_xy_and_at_the_origin() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);

    harness.select([]);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the plane is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.select([]);
    harness.click("Point");
    harness.settle();
    let point = harness
        .workspace
        .editing
        .solid()
        .expect("the point is open");

    assert!(
        result_of(&harness, plane)
            .plane()
            .unwrap()
            .normal()
            .cross(Vector3::Z)
            .length()
            < 1e-9
    );
    assert_eq!(
        result_of(&harness, point),
        DatumResult::Point(Point3::new(0.0, 0.0, 0.0))
    );
}

#[test]
fn a_plane_through_points_takes_three_clicks_in_the_view() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let corner = |harness: &Harness, x: f64, y: f64| {
        super::vertex_at(harness, plate, Point3::new(x, y, 0.0))
    };
    let bottom = [
        corner(&harness, 0.0, 0.0),
        corner(&harness, 40.0, 0.0),
        corner(&harness, 0.0, 40.0),
    ];
    let side = [
        super::vertex_at(&harness, plate, Point3::new(0.0, 0.0, 10.0)),
        bottom[0],
        bottom[1],
    ];
    harness.select(side);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the plane is open");
    harness.select([]);
    harness.frame();

    harness.click_beside(CHOOSE_IN_VIEW, "Defined by");
    harness.settle();
    let asked = harness.shows_containing("Click three points");
    click_in_view(&mut harness, bottom[0]);
    let waiting = harness.shows_containing("Click the next point, plane or axis");
    click_in_view(&mut harness, bottom[1]);
    click_in_view(&mut harness, bottom[2]);

    let result = result_of(&harness, plane).plane().unwrap();
    assert!(asked);
    assert!(waiting);
    assert_eq!(picking_slot(&harness), None);
    assert!(result.normal().cross(Vector3::Z).length() < 1e-9);
    assert!(result.signed_distance(Point3::new(5.0, 5.0, 0.0)).abs() < 1e-9);
}

#[test]
fn a_pattern_and_a_datum_axis_run_along_a_selected_sketch_line() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let mut guide = caditor_sketch::Sketch::new(Plane::XY);
    let line = guide.add_line(Point2::new(0.0, 60.0), Point2::new(30.0, 90.0));
    let guide = harness.add_sketch(guide);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let along = Pickable::SketchEntity {
        feature: guide,
        entity: line,
    };

    harness.select([along]);
    harness.click("Axis");
    harness.settle();
    let axis = harness.workspace.editing.solid().expect("the axis is open");
    let axis_along_line = matches!(
        datum_of(&harness, axis),
        Datum::Axis(DatumAxis::Along(
            caditor_document::AxisReference::Sketch { .. }
        ))
    );
    let guide_feature = harness.document().feature(guide).unwrap();
    let label = format!(
        "{} of {}",
        guide_feature.kind.sketch().unwrap().entity_label(line),
        guide_feature.name
    );
    let named = harness.shows(&label);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.select([along, top]);
    harness.click("Linear pattern");
    harness.settle();
    let pattern = harness
        .workspace
        .editing
        .solid()
        .expect("the pattern is open");

    let PatternKind::Linear { first, .. } = &pattern_of(&harness, pattern).kind else {
        panic!("a linear pattern was made");
    };
    assert!(axis_along_line);
    assert!(named);
    assert!(matches!(
        first.axis,
        caditor_document::AxisReference::Sketch { sketch, .. } if sketch == guide
    ));
    assert_eq!(pattern_of(&harness, pattern).body, plate);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

const CONTAINS: &str = "Contains it";

fn plate_edge(harness: &Harness, plate: FeatureId, middle: Point3) -> Pickable {
    let edge = harness
        .model
        .evaluation()
        .body(plate)
        .unwrap()
        .edges()
        .find(|(_, edge)| {
            let halfway = edge.curve().point(edge.interval().middle());
            (halfway - middle).length() < 1e-6
        })
        .map(|(_, edge)| edge.name())
        .expect("the plate has that edge");
    Pickable::Edge { body: plate, edge }
}

#[test]
fn two_selected_edges_give_a_plane_through_them_and_a_point_where_they_cross() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let along_x = plate_edge(&harness, plate, Point3::new(20.0, 0.0, 0.0));
    let along_y = plate_edge(&harness, plate, Point3::new(0.0, 20.0, 0.0));
    let raised_x = plate_edge(&harness, plate, Point3::new(20.0, 0.0, 10.0));

    harness.select([along_x, along_y]);
    harness.click("Point");
    harness.settle();
    let corner = harness
        .workspace
        .editing
        .solid()
        .expect("the point is open");
    let crossing = matches!(
        datum_of(&harness, corner),
        Datum::PointBy(PointBy::LinesCross(..))
    );
    let shows_where = harness.shows_containing("Where an edge of Extrude 1 crosses");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.select([along_x, raised_x]);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the plane is open");
    let through_lines = matches!(
        datum_of(&harness, plane),
        Datum::PlaneThrough(PlaneThrough::Lines(..))
    );

    assert!(crossing);
    assert!(shows_where);
    assert!(through_lines);
    assert_eq!(
        result_of(&harness, corner),
        DatumResult::Point(Point3::ZERO)
    );
    assert!(
        result_of(&harness, plane)
            .plane()
            .unwrap()
            .normal()
            .cross(Vector3::Y)
            .length()
            < 1e-9
    );
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn a_point_along_a_selected_edge_takes_a_typed_distance() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let along_x = plate_edge(&harness, plate, Point3::new(20.0, 0.0, 0.0));

    harness.select([along_x]);
    harness.click("Point");
    harness.settle();
    let point = harness
        .workspace
        .editing
        .solid()
        .expect("the point is open");
    let along = matches!(datum_of(&harness, point), Datum::PointBy(PointBy::Along(_)));
    harness.type_into_field(Id::unique(("datum-field", "distance", point)), "15 mm");
    harness.settle();

    let found = result_of(&harness, point).point().unwrap();
    assert!(along);
    assert!(found.y.abs() < 1e-9 && found.z.abs() < 1e-9);
    assert!((found.x - 15.0).abs() < 1e-9 || (found.x - 25.0).abs() < 1e-9);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn a_round_face_and_a_point_can_switch_from_holding_the_axis_to_touching_the_face() {
    let mut harness = Harness::new();
    let mut disc = Sketch::new(Plane::XY);
    disc.add_circle(Point2::new(30.0, 0.0), 10.0);
    harness.add_sketch(disc);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let side = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            pickable
                .describe(harness.document(), harness.model.evaluation())
                .contains("side")
        })
        .expect("the round side is pickable");

    harness.select([side, Pickable::Origin]);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the plane is open");
    let holding = matches!(
        datum_of(&harness, plane),
        Datum::PlaneThrough(PlaneThrough::AxisAndPoint(..))
    );
    harness.select([]);
    harness.frame();
    if !harness.shows("Tangent to it") {
        harness.click(CONTAINS);
        harness.frame();
    }
    harness.click("Tangent to it");
    harness.settle();
    let touching = matches!(
        datum_of(&harness, plane),
        Datum::PlaneThrough(PlaneThrough::Tangent(_))
    );

    let found = result_of(&harness, plane).plane().unwrap();
    assert!(holding);
    assert!(touching);
    assert!(found.normal().cross(Vector3::X).length() < 1e-9);
    assert!(found.signed_distance(Point3::new(20.0, 0.0, 3.0)).abs() < 1e-9);
    assert!(harness.shows_containing("Tangent to"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn a_round_face_and_a_point_give_an_axis_square_to_the_face_nearest_the_point() {
    let mut harness = Harness::new();
    let mut disc = Sketch::new(Plane::XY);
    disc.add_circle(Point2::new(30.0, 0.0), 10.0);
    harness.add_sketch(disc);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let side = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            pickable
                .describe(harness.document(), harness.model.evaluation())
                .contains("side")
        })
        .expect("the round side is pickable");

    harness.select([side, Pickable::Origin]);
    harness.click("Axis");
    harness.settle();
    let axis = harness.workspace.editing.solid().expect("the axis is open");

    assert!(matches!(
        datum_of(&harness, axis),
        Datum::Axis(DatumAxis::SquareToFace(_))
    ));
    let found = result_of(&harness, axis).axis().unwrap();
    assert!(found.origin().distance(Point3::new(20.0, 0.0, 0.0)) < 1e-6);
    assert!((found.direction() + Vector3::X).length() < 1e-6);
    assert!(harness.shows_containing("Square to"));
}

#[test]
fn a_sphere_and_a_point_give_a_plane_touching_the_sphere_nearest_the_point() {
    use caditor_document::{DatumPoint, PointReference};

    let mut harness = Harness::new();
    harness.select([]);
    super::run_from_palette(&mut harness, "Sphere");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let unit = harness.model.length_unit();
    let (above, point) = crate::datum_tools::create(
        harness.document(),
        Datum::Point(DatumPoint {
            base: PointReference::Origin,
            offset: [0.0, 0.0, 30.0].map(|value| unit.default_length(value)),
        }),
    );
    harness.perform(Action::Apply(above));
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let surface = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            matches!(pickable, Pickable::Face { .. })
                && pickable
                    .describe(harness.document(), harness.model.evaluation())
                    .contains("surface")
        })
        .expect("the sphere is pickable");

    harness.select([surface, Pickable::Datum(point)]);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the plane is open");

    assert!(matches!(
        datum_of(&harness, plane),
        Datum::PlaneThrough(PlaneThrough::TangentAt(_))
    ));
    let found = result_of(&harness, plane).plane().unwrap();
    assert!((found.normal() - Vector3::Z).length() < 1e-6);
    assert!(found.signed_distance(Point3::new(0.0, 0.0, 10.0)).abs() < 1e-6);
    assert!(harness.shows_containing("Tangent to"));
}

#[test]
fn a_point_sits_at_a_selected_faces_centre_or_an_edges_middle() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let along_x = plate_edge(&harness, plate, Point3::new(20.0, 0.0, 0.0));

    harness.select([top]);
    harness.click("Point");
    harness.settle();
    let centre = harness
        .workspace
        .editing
        .solid()
        .expect("the point is open");

    assert!(matches!(
        datum_of(&harness, centre),
        Datum::PointBy(PointBy::FaceCentre { .. })
    ));
    let found = result_of(&harness, centre).point().unwrap();
    assert!((found.x - 20.0).abs() < 1e-6 && (found.y - 20.0).abs() < 1e-6);

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.select([along_x]);
    harness.click("Point");
    harness.settle();
    let middle = harness
        .workspace
        .editing
        .solid()
        .expect("the point is open");
    harness.select([]);
    harness.frame();
    if !harness.shows(crate::datum_panel::MIDDLE_OF_EDGE) {
        harness.click("Along it");
        harness.frame();
    }
    harness.click(crate::datum_panel::MIDDLE_OF_EDGE);
    harness.settle();

    assert!(matches!(
        datum_of(&harness, middle),
        Datum::PointBy(PointBy::EdgeMiddle { .. })
    ));
    let found = result_of(&harness, middle).point().unwrap();
    assert!(found.distance(Point3::new(20.0, 0.0, 0.0)) < 1e-9);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

fn mirrored_features(harness: &Harness, feature: FeatureId) -> Vec<FeatureId> {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.mirror())
        .map(|mirror| mirror.mirrored.clone())
        .unwrap()
}

#[test]
fn a_hole_chosen_in_the_tree_is_mirrored_onto_its_body_instead_of_the_whole_body() {
    let mut harness = Harness::new();
    let mut outline = Sketch::new(Plane::XY);
    rectangle(
        &mut outline,
        Point2::new(-20.0, 0.0),
        Point2::new(20.0, 40.0),
    );
    harness.add_sketch(outline);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    let plate = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let top = Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap();
    let mut points = Sketch::new(top);
    points.add_point(Point2::new(8.0, 20.0));
    harness.add_sketch(points);
    harness.select([]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let drilled = std::f64::consts::PI * 9.0 * 10.0;

    harness.workspace.panels.selected = Some(hole);
    harness.frame();
    harness.frame();
    harness.hover("Mirror body");
    let described = harness.shows_containing("Mirror Hole 1 across the YZ plane");
    harness.click("Mirror body");
    harness.settle();
    let mirror = harness
        .workspace
        .editing
        .solid()
        .expect("the mirror is open");

    assert!(described);
    assert_eq!(harness.model.undo_label(), Some("Create Mirror features 1"));
    assert_eq!(mirrored_features(&harness, mirror), vec![hole]);
    assert!(volume_about(&harness, plate, 16000.0 - 2.0 * drilled));
    assert!(harness.shows(mirror_panel::MIRRORS));
    assert!(!harness.shows(mirror_panel::RESULT));

    harness.click_button("Stop mirroring Hole 1");
    harness.settle();
    assert!(mirrored_features(&harness, mirror).is_empty());
    assert!(harness.shows(mirror_panel::WHOLE_BODY));
    assert!(harness.shows(mirror_panel::RESULT));

    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(mirrored_features(&harness, mirror), vec![hole]);
    assert!(volume_about(&harness, plate, 16000.0 - 2.0 * drilled));
}

fn drilled_plate(harness: &mut Harness) -> (FeatureId, FeatureId) {
    let mut outline = Sketch::new(Plane::XY);
    rectangle(
        &mut outline,
        Point2::new(-20.0, 0.0),
        Point2::new(20.0, 40.0),
    );
    harness.add_sketch(outline);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    let plate = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let top = Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap();
    let mut points = Sketch::new(top);
    points.add_point(Point2::new(8.0, 20.0));
    harness.add_sketch(points);
    harness.select([]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    (plate, hole)
}

fn faces_made_by(harness: &Harness, body: FeatureId, made_by: FeatureId) -> Vec<Pickable> {
    let shown = bodies::shown(harness.model.evaluation(), body).unwrap();
    bodies::face_keys(&shown.solid)
        .into_iter()
        .filter(|(id, _)| {
            shown
                .solid
                .face(*id)
                .and_then(|face| face.origin())
                .is_some_and(|origin| origin.feature() == made_by.raw())
        })
        .map(|(_, face)| Pickable::Face { body, face })
        .collect()
}

fn mirrored_face_count(harness: &Harness, feature: FeatureId) -> usize {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.mirror())
        .map(|mirror| mirror.faces.len())
        .unwrap()
}

#[test]
fn a_hole_s_faces_are_mirrored_from_the_palette_and_chosen_again_in_the_panel() {
    let mut harness = Harness::new();
    let (plate, hole) = drilled_plate(&mut harness);
    let drilled = std::f64::consts::PI * 9.0 * 10.0;
    let walls = faces_made_by(&harness, plate, hole);

    harness.select(walls.clone());
    run_from_palette(&mut harness, "mirror faces");
    harness.settle();
    let mirror = harness
        .workspace
        .editing
        .solid()
        .expect("the mirror is open");

    assert_eq!(harness.model.undo_label(), Some("Create Mirror faces 1"));
    assert_eq!(mirrored_face_count(&harness, mirror), walls.len());
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(volume_about(&harness, plate, 16000.0 - 2.0 * drilled));
    assert!(harness.shows(mirror_panel::FACES_DESCRIPTION));
    assert!(!harness.shows(mirror_panel::RESULT));

    let removals = walls.len();
    for _ in 0..removals {
        harness.click_button("Stop mirroring this face");
        harness.settle();
    }
    assert_eq!(mirrored_face_count(&harness, mirror), 0);
    assert!(harness.shows(mirror_panel::WHOLE_BODY));
    assert!(harness.shows(mirror_panel::RESULT));

    choose(
        &mut harness,
        mirror_panel::ADD_TO_BODY,
        mirror_panel::REPLACE_BODY,
    );
    harness.select(walls.clone());
    harness.frame();
    harness.click(mirror_panel::MIRROR_SELECTED_FACES);
    harness.settle();
    assert_eq!(mirrored_face_count(&harness, mirror), walls.len());
    assert!(volume_about(&harness, plate, 16000.0 - 2.0 * drilled));
}

#[test]
fn a_tapped_holes_thread_takes_its_hand_and_depth_from_the_hole_panel() {
    use caditor_document::{HoleFit, HoleStandard, MetricSize, ThreadHand, placed_threads};

    use crate::hole_panel::{TO_A_DEPTH, WHOLE_BORE};

    let mut harness = Harness::new();
    harness.perform(Action::ShowThreadsAsCosmetic(true));
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    let tapped = crate::hole_tools::with_standard(
        &super::open_hole(&harness, hole),
        HoleStandard {
            size: MetricSize::M6,
            fit: HoleFit::Tapped,
        },
        crate::units::LengthUnit::Millimetre,
    );
    let change = crate::hole_tools::edit(harness.document(), hole, tapped).unwrap();
    harness.perform(Action::Apply(change));
    harness.settle();

    assert!(harness.shows(crate::hole_panel::THREAD_CLASS));
    assert!(harness.shows("6H"));

    super::choose(&mut harness, "Right", "Left");
    super::choose(&mut harness, WHOLE_BORE, TO_A_DEPTH);
    harness.type_into_field(Id::unique(("hole-field", "thread-depth", hole)), "4 mm");
    harness.settle();

    let thread = super::open_hole(&harness, hole).thread;
    assert_eq!(thread.hand, ThreadHand::Left);
    assert_eq!(
        thread.depth.map(|depth| depth.to_stored_text()).as_deref(),
        Some("4 mm")
    );
    let placed = placed_threads(harness.document(), harness.model.evaluation());
    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].designation, "M6-6H-LH");
    assert!((placed[0].placement.length - 4.0).abs() < 1e-6);
}

#[test]
fn a_circular_pattern_waiting_for_its_axis_reads_none_chosen_and_its_callout_offers_to_choose_it() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.select([]);
    harness.click("Circular pattern");
    harness.settle();
    let pattern = harness
        .workspace
        .editing
        .solid()
        .expect("the pattern is open");
    harness.hold_still();

    let none_chosen = harness.shows(feature_fields::NONE_CHOSEN);
    let keeps_choosing = harness.shows(feature_fields::KEEPS_CHOOSING);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let closed = harness.workspace.editing.solid();
    let failed = harness.model.evaluation().failed_count();
    let told = harness.shows("Waiting for an axis");
    let asked =
        harness.shows("Choose an axis, straight edge or round face for the copies to turn about.");
    harness.hold_still();
    harness.click(CHOOSE_IN_VIEW);
    harness.settle();

    assert!(none_chosen, "an unset axis reads None chosen");
    assert!(!keeps_choosing, "one click chooses the axis, so no pill");
    assert_eq!(closed, None);
    assert_eq!(failed, 0, "a pattern waiting for its axis has not failed");
    assert!(told && asked);
    assert_eq!(harness.workspace.editing.solid(), Some(pattern));
    assert_eq!(picking_slot(&harness), Some(Slot::PatternDirection));
}

#[test]
fn a_feature_waiting_for_input_shows_a_waiting_pill_that_selects_it_instead_of_up_to_date() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.select([]);
    harness.click("Circular pattern");
    harness.settle();
    let pattern = harness
        .workspace
        .editing
        .solid()
        .expect("the pattern is open");
    harness.hold_still();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    harness.select([]);
    harness.frame();

    let waiting = harness.shows("1 waiting");
    let up_to_date = harness.shows(crate::status_bar::UP_TO_DATE);
    let failed = harness.model.evaluation().failed_count();
    harness.click("1 waiting");
    harness.frame();
    harness.frame();

    assert!(waiting, "a waiting feature is counted beside the failures");
    assert!(
        !up_to_date,
        "a model with a waiting feature is not up to date"
    );
    assert_eq!(failed, 0);
    assert_eq!(harness.workspace.panels.selected, Some(pattern));
}

fn leave_a_circular_pattern_waiting(harness: &mut Harness) -> FeatureId {
    harness.select([]);
    harness.click("Circular pattern");
    harness.settle();
    let pattern = harness
        .workspace
        .editing
        .solid()
        .expect("the pattern is open");
    harness.hold_still();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    harness.select([]);
    harness.frame();
    pattern
}

#[test]
fn f9_and_shift_f9_step_through_the_waiting_features_and_wrap() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);

    let no_waiting = offer(&harness, Command::ShowNextWaiting).availability;
    let first = leave_a_circular_pattern_waiting(&mut harness);
    let second = leave_a_circular_pattern_waiting(&mut harness);
    harness.workspace.panels.selected = None;
    let step = |harness: &mut Harness, modifiers: Modifiers| {
        harness.key(Key::F9, modifiers);
        harness.frame();
        harness.frame();
        harness.workspace.panels.selected
    };

    let reached = step(&mut harness, Modifiers::NONE);
    let next = step(&mut harness, Modifiers::NONE);
    let wrapped = step(&mut harness, Modifiers::NONE);
    let back = step(&mut harness, Modifiers::SHIFT);

    assert!(no_waiting.is_err());
    assert_eq!(harness.model.evaluation().waiting_count(), 2);
    assert_eq!(reached, Some(first));
    assert_eq!(next, Some(second));
    assert_eq!(wrapped, Some(first));
    assert_eq!(back, Some(second));
    assert!(
        offer(&harness, Command::ShowPreviousWaiting)
            .availability
            .is_ok()
    );
}

#[test]
fn an_edge_list_choosing_in_the_view_says_it_keeps_choosing_and_stop_choosing_closes_the_fillet() {
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
    harness.hold_still();

    let prompted = harness.shows("Click edges in the view to add them or leave them out.");
    let keeps_choosing = harness.shows(feature_fields::KEEPS_CHOOSING);
    harness.click(feature_fields::STOP_CHOOSING);
    harness.settle();

    assert!(prompted);
    assert!(keeps_choosing);
    assert_eq!(harness.workspace.editing.solid(), None);
    assert_eq!(blend_of(&harness, fillet).edges.len(), 1);
}
