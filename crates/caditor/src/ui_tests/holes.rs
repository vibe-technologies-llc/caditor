use caditor_document::{
    Datum, DatumAxis, DatumPoint, Edit, FeatureKind, PlaneReference, PointReference,
    PrincipalPlane, Transaction,
};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::{Entity, Sketch};
use egui::{Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, attached_body, choose, drag_screen, extruded_plate, is_face,
    open_combo, open_extrude, open_hole, open_hole_on, placed_holes, plate_edge_at, removed_about,
    rows_named, run_from_palette, selected_of,
};
use crate::{
    editing::EditingCommand,
    model::Action,
    move_manipulator::Handle,
    panels::Focus,
    selection::{Axis, Pickable},
};

#[test]
fn a_hole_is_drilled_at_the_points_of_a_sketch_and_its_panel_changes_the_style_and_sizes() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let top = Plane::from_frame(
        caditor_geometry::Point3::new(0.0, 0.0, 10.0),
        caditor_geometry::Vector3::Z,
        caditor_geometry::Vector3::X,
    )
    .unwrap();
    let mut sketch = Sketch::new(top);
    sketch.add_point(Point2::new(20.0, 20.0));
    let sketch = harness.add_sketch(sketch);
    harness.select([]);

    harness.hover("Hole");
    assert!(harness.shows_containing(crate::hole_tools::DESCRIPTION));
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    assert_eq!(harness.model.undo_label(), Some("Create Hole 1"));
    let definition = harness
        .document()
        .feature(hole)
        .unwrap()
        .kind
        .hole()
        .unwrap()
        .clone();
    assert_eq!((definition.sketch, definition.body), (sketch, plate));
    assert!(harness.document().feature(sketch).unwrap().hidden);
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 9.0 * 10.0
    ));
    assert!(harness.shows("Diameter"));
    assert!(harness.shows("Depth"));

    harness.type_into_field(Id::new(("hole-field", "diameter", hole)), "10 mm");
    harness.settle();
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 25.0 * 10.0
    ));

    harness.type_into_field(Id::new(("hole-field", "depth", hole)), "0 mm");
    assert!(harness.shows(crate::feature_fields::ABOVE_ZERO));
    assert_eq!(harness.workspace.editing.solid(), Some(hole));

    harness.click("Plain");
    harness.settle();
    harness.click("Countersink");
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert!(harness.shows("Countersink angle"));
    assert_eq!(harness.model.evaluation().failed_count(), 1);
    harness.type_into_field(
        Id::new(("hole-field", "countersink-diameter", hole)),
        "14 mm",
    );
    open_combo(&mut harness, "Depth");
    harness.click_lowest("Through all");
    harness.settle();
    assert_eq!(rows_named(&harness, "Depth").len(), 1);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    let label = harness.model.undo_label().map(str::to_owned);
    harness.type_into_field(
        Id::new(("hole-field", "countersink-angle", hole)),
        "180 deg",
    );
    assert!(harness.shows("Enter an angle above 0° and up to 179°"));
    assert_eq!(harness.model.undo_label().map(str::to_owned), label);

    harness.perform(Action::Undo);
    harness.perform(Action::Undo);
    harness.perform(Action::Undo);
    harness.settle();
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 25.0 * 10.0
    ));
}

#[test]
fn a_hole_is_drilled_up_to_the_next_face_or_a_chosen_face_short_of_it_by_its_offset() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let top = Plane::from_frame(
        caditor_geometry::Point3::new(0.0, 0.0, 10.0),
        caditor_geometry::Vector3::Z,
        caditor_geometry::Vector3::X,
    )
    .unwrap();
    let mut sketch = Sketch::new(top);
    sketch.add_point(Point2::new(20.0, 20.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    let bore = std::f64::consts::PI * 3.0 * 3.0;

    open_combo(&mut harness, "Depth");
    harness.click_lowest(crate::hole_panel::UP_TO_NEXT);
    harness.settle();
    let to_next = open_hole(&harness, hole).depth;
    let through = removed_about(&harness, plate, bore * 10.0);
    harness.type_into_field(Id::new(("hole-field", "end-offset", hole)), "-4 mm");
    harness.settle();
    let short = removed_about(&harness, plate, bore * 6.0);

    open_combo(&mut harness, "Depth");
    harness.click_lowest(crate::hole_panel::UP_TO_FACE);
    harness.settle();
    let asks_for_a_face = harness.shows("Click a flat face or plane to drill up to.");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
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
    open_combo(&mut harness, "Depth");
    harness.click_lowest(crate::hole_panel::UP_TO_FACE);
    harness.settle();
    let to_face = open_hole(&harness, hole).depth;
    let still_short = removed_about(&harness, plate, bore * 6.0);

    assert_eq!(to_next, caditor_document::HoleDepth::up_to_next());
    assert!(through);
    assert!(short);
    assert!(asks_for_a_face);
    assert!(to_face.target().is_some());
    assert!(to_face.offset().is_some());
    assert!(still_short);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn a_blind_hole_can_end_in_a_drill_point_whose_angle_is_set_in_its_panel() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let top = Plane::from_frame(
        caditor_geometry::Point3::new(0.0, 0.0, 10.0),
        caditor_geometry::Vector3::Z,
        caditor_geometry::Vector3::X,
    )
    .unwrap();
    let mut sketch = Sketch::new(top);
    sketch.add_point(Point2::new(20.0, 20.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    harness.type_into_field(Id::new(("hole-field", "depth", hole)), "2 mm");
    harness.settle();
    let flat = std::f64::consts::PI * 3.0 * 3.0 * 2.0;
    let cone = std::f64::consts::PI * 3.0 * 3.0 * (3.0 / 59.0_f64.to_radians().tan()) / 3.0;

    assert!(harness.shows(crate::hole_panel::DRILL_POINT));
    assert!(!harness.shows(crate::hole_panel::DRILL_POINT_ANGLE));
    assert_eq!(
        open_hole(&harness, hole).bottom,
        caditor_document::HoleBottom::Flat
    );
    assert!(removed_about(&harness, plate, flat));

    harness.click_lowest(crate::hole_panel::DRILL_POINT);
    harness.settle();

    assert_eq!(
        open_hole(&harness, hole).bottom,
        caditor_document::HoleBottom::DrillPoint(Expression::parse_stored("118 deg").unwrap())
    );
    assert!(harness.shows(crate::hole_panel::DRILL_POINT_ANGLE));
    assert!(removed_about(&harness, plate, flat + cone));

    harness.type_into_field(Id::new(("hole-field", "drill-point-angle", hole)), "90 deg");
    harness.settle();
    assert_eq!(
        open_hole(&harness, hole).bottom,
        caditor_document::HoleBottom::DrillPoint(Expression::parse_stored("90 deg").unwrap())
    );
    assert_eq!(harness.model.evaluation().failed_count(), 0);

    harness.type_into_field(
        Id::new(("hole-field", "drill-point-angle", hole)),
        "180 deg",
    );
    assert!(harness.shows("Enter an angle above 0° and up to 179°"));

    open_combo(&mut harness, "Depth");
    harness.click_lowest("Through all");
    harness.settle();
    assert!(!harness.shows(crate::hole_panel::DRILL_POINT));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn a_stepped_hole_takes_steps_added_sized_and_removed_in_its_panel() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let top = Plane::from_frame(
        caditor_geometry::Point3::new(0.0, 0.0, 10.0),
        caditor_geometry::Vector3::Z,
        caditor_geometry::Vector3::X,
    )
    .unwrap();
    let mut sketch = Sketch::new(top);
    sketch.add_point(Point2::new(20.0, 20.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    let steps = |harness: &Harness| match &harness.document().feature(hole).unwrap().kind {
        FeatureKind::Hole(definition) => match &definition.style {
            caditor_document::HoleStyle::Stepped(steps) => steps.len(),
            _ => 0,
        },
        other => panic!("{other:?}"),
    };

    choose(&mut harness, "Plain", "Stepped");
    let two = steps(&harness);
    let shown = harness.shows("Step 2 diameter") && harness.shows("Step 2 depth");
    let removed = std::f64::consts::PI * (25.0 * 3.0 + 16.0 * 2.0 + 9.0 * 5.0);
    let stepped = removed_about(&harness, plate, removed);
    harness.click(crate::hole_panel::ADD_STEP);
    harness.settle();
    let three = steps(&harness);
    let failed_deeper = harness.model.evaluation().failed_count();
    harness.type_into_field(Id::new(("hole-step", 2usize, false, hole)), "1 mm");
    harness.settle();
    let after_sizing = harness.model.evaluation().failed_count();
    harness.click(crate::hole_panel::REMOVE_STEP);
    harness.settle();

    assert_eq!(two, 2);
    assert!(shown);
    assert!(stepped);
    assert_eq!(three, 3);
    assert_eq!(failed_deeper, 0);
    assert_eq!(after_sizing, 0);
    assert_eq!(steps(&harness), 2);
    assert!(removed_about(&harness, plate, removed));
}

#[test]
fn a_hole_at_a_circle_can_take_the_circle_s_diameter_from_its_panel() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let top = Plane::from_frame(
        caditor_geometry::Point3::new(0.0, 0.0, 10.0),
        caditor_geometry::Vector3::Z,
        caditor_geometry::Vector3::X,
    )
    .unwrap();
    let mut sketch = Sketch::new(top);
    sketch.add_circle(Point2::new(20.0, 20.0), 4.0);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");

    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 3.0 * 3.0 * 10.0
    ));
    assert!(harness.shows("Sized by"));

    harness.click(crate::hole_panel::SIZED_BY_CIRCLES);
    harness.settle();

    assert_eq!(
        open_hole(&harness, hole).sizing,
        caditor_document::HoleSizing::CirclesAndHeads
    );
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 4.0 * 4.0 * 10.0
    ));
    assert!(!harness.shows(crate::hole_panel::SCALE_HEADS));

    harness.click("Plain");
    harness.click("Counterbore");
    harness.settle();
    harness.click(crate::hole_panel::SCALE_HEADS);
    harness.settle();

    assert_eq!(
        open_hole(&harness, hole).sizing,
        caditor_document::HoleSizing::Circles
    );
}

#[test]
fn a_hole_takes_the_selected_sketch_point_over_a_selected_face() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let mut points = Sketch::new(Plane::XY);
    let point = points.add_point(Point2::new(5.0, 5.0));
    let sketch = harness.add_sketch(points);

    harness.select([
        top,
        Pickable::SketchEntity {
            feature: sketch,
            entity: point,
        },
    ]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");

    assert_eq!(open_hole(&harness, hole).sketch, sketch);
    assert_eq!(open_hole(&harness, hole).body, plate);
}

#[test]
fn a_hole_drilled_on_a_face_moves_by_its_position_fields_and_a_click_on_the_face() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);

    harness.select([top]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    let sketch = open_hole(&harness, hole).sketch;
    let point_of = |harness: &Harness| {
        harness
            .sketch(sketch)
            .entities()
            .find_map(|(_, entity)| match entity {
                Entity::Point(at) => Some(*at),
                _ => None,
            })
            .expect("the hole's sketch holds its point")
    };

    assert!(harness.shows(crate::hole_panel::PLACED_ON));
    assert!(harness.shows(crate::feature_fields::POSITION_CAPTIONS[0]));
    assert_eq!(point_of(&harness), Point2::new(20.0, 20.0));

    let point = harness
        .sketch(sketch)
        .entities()
        .find_map(|(id, entity)| matches!(entity, Entity::Point(_)).then_some(id))
        .expect("the hole's sketch holds its point");
    harness.type_into_field(Id::new(("hole-position", hole, point, 0_usize)), "8 mm");
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Move Hole 1"));
    assert_eq!(point_of(&harness), Point2::new(8.0, 20.0));
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 9.0 * 10.0
    ));

    harness.select([]);
    harness.click("Choose in the view");
    harness.settle();

    assert_eq!(
        harness
            .workspace
            .editing
            .picking()
            .map(|picking| picking.slot),
        Some(crate::reference_picking::Slot::HolePlace)
    );

    let plane = crate::scene::sketch_plane(harness.document(), harness.model.evaluation(), sketch)
        .expect("the sketch has a plane");
    harness.click_pickable(plane, Point2::new(30.0, 12.0), top);
    harness.settle();

    assert_eq!(harness.workspace.editing.picking(), None);
    assert_eq!(harness.model.undo_label(), Some("Move Hole 1"));
    assert!(point_of(&harness).distance(Point2::new(30.0, 12.0)) < 0.01);
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 9.0 * 10.0
    ));

    harness.click(crate::hole_panel::EDIT_SKETCH);
    harness.settle();

    assert_eq!(harness.workspace.editing.feature(), Some(sketch));
}

#[test]
fn a_hole_s_position_typed_as_an_expression_follows_its_parameters() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    let mut transaction = harness.document().transaction("Add spot");
    let spot = transaction.parse("12 mm").unwrap();
    transaction.add_parameter("spot", spot);
    harness.perform(Action::Apply(transaction.finish()));
    harness.select([top]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    let sketch = open_hole(&harness, hole).sketch;
    let point = harness
        .sketch(sketch)
        .entities()
        .find_map(|(id, entity)| matches!(entity, Entity::Point(_)).then_some(id))
        .expect("the hole's sketch holds its point");
    let field = |axis: usize| Id::new(("hole-position", hole, point, axis));
    let solved = |harness: &Harness| placed_holes(harness, hole)[0].1;

    harness.type_into_field(field(0), "spot");
    harness.settle();
    harness.type_into_field(field(1), "spot + 10 mm");
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Move Hole 1"));
    assert!(harness.shows("spot + 10 mm"));
    assert!(solved(&harness).distance(Point3::new(12.0, 22.0, 10.0)) < 1e-6);

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.type_into(Focus::ParameterValue(harness.parameter("spot")), "15 mm");
    harness.settle();

    assert!(solved(&harness).distance(Point3::new(15.0, 25.0, 10.0)) < 1e-6);
    assert_eq!(harness.model.evaluation().failed_count(), 0);

    harness.perform(Action::Editing(EditingCommand::OpenSolid(hole)));
    harness.settle();
    harness.type_into_field(field(0), "8 mm");
    harness.settle();
    let held = crate::hole_placement::read(harness.document(), &open_hole(&harness, hole))
        .expect("the hole stays placed by its position");

    assert!(held.holes[0].along[0].is_none());
    assert!(held.holes[0].along[1].is_some());
    assert!(solved(&harness).distance(Point3::new(8.0, 25.0, 10.0)) < 1e-6);
}

#[test]
fn a_hole_measured_from_two_edges_keeps_its_distances_when_the_plate_grows() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let hole = open_hole_on(&mut harness, top);
    let front = plate_edge_at(&harness, plate, Point3::new(20.0, 0.0, 10.0));
    let left = plate_edge_at(&harness, plate, Point3::new(0.0, 20.0, 10.0));

    harness.select([Pickable::Edge {
        body: plate,
        edge: front,
    }]);
    harness.click(crate::hole_panel::MEASURE_SELECTED);
    harness.settle();
    let holes = placed_holes(&harness, hole);
    let crate::hole_placement::Anchor::Edges(edges) = &holes[0].0.anchor else {
        panic!("the hole is measured from an edge");
    };
    let from_front = edges[0].held.constraint;

    assert_eq!(
        harness.model.undo_label(),
        Some("Place Hole 1 from an edge")
    );
    assert_eq!(edges.len(), 1);
    assert_eq!(harness.document().expression_text(&edges[0].value), "20 mm");
    assert!(harness.shows(crate::hole_panel::MEASURED_FROM));
    assert!(!harness.shows(crate::feature_fields::POSITION_CAPTIONS[0]));

    harness.type_into_field(Id::new(("hole-edge-distance", hole, from_front)), "8 mm");
    harness.settle();
    harness.select([Pickable::Edge {
        body: plate,
        edge: left,
    }]);
    harness.click(crate::hole_panel::MEASURE_SELECTED);
    harness.settle();
    let from_left = match &placed_holes(&harness, hole)[0].0.anchor {
        crate::hole_placement::Anchor::Edges(edges) => edges[1].held.constraint,
        anchor => panic!("the hole is measured from two edges, not {anchor:?}"),
    };
    harness.type_into_field(Id::new(("hole-edge-distance", hole, from_left)), "12 mm");
    harness.settle();

    assert!(
        placed_holes(&harness, hole)[0]
            .1
            .distance(Point3::new(12.0, 8.0, 10.0))
            < 1e-6
    );
    assert!(!harness.shows(crate::hole_panel::MEASURE_SELECTED));
    assert!(!harness.shows(crate::hole_panel::MEASURE_CHOOSE));
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 9.0 * 10.0
    ));

    harness.select([Pickable::Edge {
        body: plate,
        edge: front,
    }]);
    let refused = crate::hole_placement::edge_change(
        &harness.model,
        harness.workspace.viewport.selection(),
        hole,
        &open_hole(&harness, hole),
        placed_holes(&harness, hole)[0].0.point,
    );

    assert!(refused.unwrap_err().contains("two edges"));

    let base = open_extrude(&harness, plate).sketch;
    let widened: Vec<Edit> = harness
        .sketch(base)
        .entities()
        .filter_map(|(id, entity)| match entity {
            Entity::Point(at) if at.x.abs() < 1e-9 => Some(Edit::SetSketchEntity {
                feature: base,
                id,
                entity: Entity::Point(Point2::new(-10.0, at.y)),
            }),
            _ => None,
        })
        .collect();
    harness.perform(Action::Apply(Transaction::new("Widen the plate", widened)));
    harness.settle();

    assert!(
        placed_holes(&harness, hole)[0]
            .1
            .distance(Point3::new(2.0, 8.0, 10.0))
            < 1e-6
    );

    harness.click_button("Stop measuring hole 1 from this edge");
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Free Hole 1 from an edge"));
    assert!(matches!(
        &placed_holes(&harness, hole)[0].0.anchor,
        crate::hole_placement::Anchor::Edges(edges) if edges.len() == 1
    ));
}

#[test]
fn a_hole_centred_on_a_round_edge_follows_it_and_takes_further_holes_clicked_on_the_face() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let first = open_hole_on(&mut harness, top);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();
    let top = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            pickable.describe(harness.document(), harness.model.evaluation())
                == "Extrude 1 › Extrude 1 end face"
        })
        .expect("the top face is still pickable");
    let second = open_hole_on(&mut harness, top);
    let mouth = {
        let solid = harness.model.evaluation().body(plate).unwrap();
        solid
            .edges()
            .find(|(_, edge)| {
                let middle = edge.curve().point(edge.interval().middle());
                (middle.z - 10.0).abs() < 1e-6
                    && ((middle - Point3::new(20.0, 20.0, 10.0)).length() - 3.0).abs() < 1e-6
            })
            .map(|(_, edge)| edge.name())
            .expect("the first hole's mouth is an edge")
    };
    harness.type_into_field(Id::new(("hole-field", "diameter", second)), "10 mm");
    harness.settle();

    assert!(
        placed_holes(&harness, second)[0]
            .1
            .distance(Point3::new(20.0, 20.0, 10.0))
            > 5.0
    );

    harness.select([Pickable::Edge {
        body: plate,
        edge: mouth,
    }]);
    harness.click(crate::hole_panel::CENTRE_SELECTED);
    harness.settle();

    assert_eq!(
        harness.model.undo_label(),
        Some("Make Hole 2 concentric with an edge")
    );
    assert!(harness.shows(crate::hole_panel::CONCENTRIC_WITH));
    assert!(
        placed_holes(&harness, second)[0]
            .1
            .distance(Point3::new(20.0, 20.0, 10.0))
            < 1e-6
    );

    let first_hole = open_hole(&harness, first);
    let first_point = placed_holes(&harness, first)[0].0.point;
    let x = harness.document().parse("12 mm").unwrap();
    let y = harness.document().parse("25 mm").unwrap();
    let moved = crate::hole_placement::positioned(
        &harness.model,
        first,
        &first_hole,
        first_point,
        [Some(&x), Some(&y)],
    )
    .unwrap();
    harness.perform(Action::Apply(moved));
    harness.settle();

    assert!(
        placed_holes(&harness, second)[0]
            .1
            .distance(Point3::new(12.0, 25.0, 10.0))
            < 1e-6
    );

    harness.select([]);
    harness.click(crate::hole_panel::ADD_HOLE);
    harness.settle();

    assert_eq!(
        harness
            .workspace
            .editing
            .picking()
            .map(|picking| picking.slot),
        Some(crate::reference_picking::Slot::HoleAdd)
    );

    let plane = crate::scene::sketch_plane(
        harness.document(),
        harness.model.evaluation(),
        open_hole(&harness, second).sketch,
    )
    .expect("the sketch has a plane");
    harness.click_pickable(plane, Point2::new(32.0, 8.0), top);
    harness.settle();
    harness.click_pickable(plane, Point2::new(32.0, 32.0), top);
    harness.settle();
    let holes = placed_holes(&harness, second);

    assert_eq!(harness.model.undo_label(), Some("Add a hole to Hole 2"));
    assert_eq!(
        harness
            .workspace
            .editing
            .picking()
            .map(|picking| picking.slot),
        Some(crate::reference_picking::Slot::HoleAdd)
    );
    assert_eq!(holes.len(), 3);
    assert!(holes[1].1.distance(Point3::new(32.0, 8.0, 10.0)) < 0.01);
    assert!(holes[2].1.distance(Point3::new(32.0, 32.0, 10.0)) < 0.01);
    assert!(harness.shows("Hole 3"));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.click_button("Remove hole 2");
    harness.settle();
    let holes = placed_holes(&harness, second);

    assert_eq!(
        harness.model.undo_label(),
        Some("Remove a hole from Hole 2")
    );
    assert_eq!(holes.len(), 2);
    assert!(holes[1].1.distance(Point3::new(32.0, 32.0, 10.0)) < 0.01);

    let mut chosen = crate::selection::Selection::default();
    chosen.toggle(top);
    let added = crate::hole_placement::add_change(
        &harness.model,
        &chosen,
        second,
        &open_hole(&harness, second),
    )
    .unwrap();
    harness.perform(Action::Apply(added));
    harness.settle();
    let holes = placed_holes(&harness, second);

    assert_eq!(holes.len(), 3);
    assert!(
        holes
            .iter()
            .take(2)
            .all(|(_, at)| at.distance(holes[2].1) > 5.0)
    );
}

fn datum_spot(harness: &Harness, [x, y, z]: [&str; 3]) -> FeatureKind {
    FeatureKind::Datum(Datum::Point(DatumPoint {
        base: PointReference::Origin,
        offset: [x, y, z].map(|text| harness.document().parse(text).unwrap()),
    }))
}

fn the_hole_at(harness: &Harness, hole: caditor_document::FeatureId, at: Point3) -> bool {
    placed_holes(harness, hole)[0].1.distance(at) < 1e-6
}

#[test]
fn a_hole_is_placed_by_axes_a_datum_point_the_origin_and_another_sketch_s_circle() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let lid = Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap();
    let mut ring = Sketch::new(lid);
    let circle = ring.add_circle(Point2::new(10.0, 30.0), 3.0);
    let mut transaction = harness.document().transaction("References");
    let spot = transaction.add_feature("Spot", datum_spot(&harness, ["30 mm", "15 mm", "10 mm"]));
    let rail = transaction.add_feature(
        "Rail",
        FeatureKind::Datum(Datum::Axis(DatumAxis::NormalTo(
            PlaneReference::Principal(PrincipalPlane::Yz),
            PointReference::Datum(spot),
        ))),
    );
    let ring = transaction.add_feature("Ring", FeatureKind::from(ring));
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    let hole = open_hole_on(&mut harness, top);

    harness.select([Pickable::Axis(Axis::Y)]);
    harness.click(crate::hole_panel::MEASURE_SELECTED);
    harness.settle();

    assert_eq!(
        harness.model.undo_label(),
        Some("Place Hole 1 from an edge")
    );
    assert!(harness.shows("The Y axis"));

    harness.select([Pickable::Datum(rail)]);
    harness.click(crate::hole_panel::MEASURE_SELECTED);
    harness.settle();
    let values: Vec<String> = match &placed_holes(&harness, hole)[0].0.anchor {
        crate::hole_placement::Anchor::Edges(edges) => edges
            .iter()
            .map(|distance| harness.document().expression_text(&distance.value))
            .collect(),
        anchor => panic!("the hole is measured from two axes, not {anchor:?}"),
    };

    assert_eq!(values, ["20 mm", "5 mm"]);
    assert!(harness.shows("Rail"));
    assert!(harness.document().dependents_of(&[rail]).contains(&hole));

    harness.perform(Action::Apply(Transaction::single(
        "Move the spot",
        Edit::SetFeatureKind {
            id: spot,
            kind: datum_spot(&harness, ["30 mm", "12 mm", "10 mm"]),
        },
    )));
    harness.settle();

    assert!(the_hole_at(&harness, hole, Point3::new(20.0, 17.0, 10.0)));
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 9.0 * 10.0
    ));

    harness.click_button("Stop measuring hole 1 from this axis");
    harness.settle();
    harness.click_button("Stop measuring hole 1 from this axis");
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Free Hole 1 from an axis"));
    assert_eq!(
        placed_holes(&harness, hole)[0].0.anchor,
        crate::hole_placement::Anchor::Free
    );

    harness.select([Pickable::Datum(spot)]);
    harness.click(crate::hole_panel::CENTRE_SELECTED);
    harness.settle();

    assert_eq!(
        harness.model.undo_label(),
        Some("Make Hole 1 centred on a point")
    );
    assert!(harness.shows(crate::hole_panel::CONCENTRIC_WITH));
    assert!(harness.shows("Spot"));
    assert!(the_hole_at(&harness, hole, Point3::new(30.0, 12.0, 10.0)));
    assert!(matches!(
        placed_holes(&harness, hole)[0].0.anchor,
        crate::hole_placement::Anchor::At(_)
    ));

    harness.perform(Action::Apply(Transaction::single(
        "Move the spot",
        Edit::SetFeatureKind {
            id: spot,
            kind: datum_spot(&harness, ["25 mm", "12 mm", "10 mm"]),
        },
    )));
    harness.settle();

    assert!(the_hole_at(&harness, hole, Point3::new(25.0, 12.0, 10.0)));
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 9.0 * 10.0
    ));

    harness.select([Pickable::SketchEntity {
        feature: ring,
        entity: circle,
    }]);
    harness.click(crate::hole_panel::CENTRE_SELECTED);
    harness.settle();

    assert_eq!(
        harness.model.undo_label(),
        Some("Make Hole 1 concentric with an edge")
    );
    assert!(harness.shows(&format!("Circle {circle} of Ring")));
    assert!(the_hole_at(&harness, hole, Point3::new(10.0, 30.0, 10.0)));

    let Some(Entity::Circle { center, .. }) = harness.sketch(ring).entity(circle).cloned() else {
        panic!("the ring is a circle");
    };
    harness.perform(Action::Apply(Transaction::single(
        "Move the ring",
        Edit::SetSketchEntity {
            feature: ring,
            id: center,
            entity: Entity::Point(Point2::new(12.0, 28.0)),
        },
    )));
    harness.settle();

    assert!(the_hole_at(&harness, hole, Point3::new(12.0, 28.0, 10.0)));

    let mut origin = crate::selection::Selection::default();
    origin.toggle(Pickable::Origin);
    let centred = crate::hole_placement::concentric_change(
        &harness.model,
        &origin,
        hole,
        &open_hole(&harness, hole),
        placed_holes(&harness, hole)[0].0.point,
    )
    .unwrap();
    harness.perform(Action::Apply(centred));
    harness.settle();

    assert!(the_hole_at(&harness, hole, Point3::new(0.0, 0.0, 10.0)));
    assert!(harness.shows("The origin"));
}

#[test]
fn a_hole_on_a_round_face_drills_square_into_it_where_it_is_clicked() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_circle(Point2::new(0.0, 0.0), 20.0);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let puck = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let full = harness.body_volume(puck);
    let wall = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            matches!(pickable, Pickable::Face { .. })
                && harness
                    .model
                    .evaluation()
                    .body(puck)
                    .and_then(|solid| {
                        let Pickable::Face { face, .. } = pickable else {
                            return None;
                        };
                        let shown = crate::bodies::shown(harness.model.evaluation(), puck)?;
                        let id = crate::bodies::find_face(shown, *face)?;
                        solid.face(id).map(|face| {
                            matches!(face.surface(), caditor_kernel::Surface::Cylinder(_))
                        })
                    })
                    .unwrap_or(false)
        })
        .expect("the round wall is pickable");

    harness.select([wall]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");

    assert_eq!(harness.model.undo_label(), Some("Create Hole 1"));
    assert_eq!(
        harness
            .workspace
            .editing
            .picking()
            .map(|picking| picking.slot),
        Some(crate::reference_picking::Slot::HolePlace)
    );
    assert!(harness.shows_containing("drilled square into the curved face"));
    assert!(full - harness.body_volume(puck) > 0.5 * std::f64::consts::PI * 9.0 * 10.0);

    let spot = Point3::new(0.0, -20.0, 4.0);
    let tangent = Plane::from_frame(spot, Vector3::NEG_Y, Vector3::X).unwrap();
    harness.click_pickable(tangent, Point2::ZERO, wall);
    harness.settle();
    let bore = harness
        .model
        .evaluation()
        .body(puck)
        .unwrap()
        .faces()
        .find_map(|(_, face)| match face.surface() {
            caditor_kernel::Surface::Cylinder(cylinder) if cylinder.radius() < 4.0 => {
                Some(*cylinder.frame())
            }
            _ => None,
        })
        .expect("the hole has a bore");

    assert_eq!(harness.workspace.editing.picking(), None);
    assert_eq!(harness.model.undo_label(), Some("Move Hole 1"));
    assert!(crate::hole_on_curve::mount(harness.document(), &open_hole(&harness, hole)).is_some());
    assert!(bore.normal().y.abs() > 1.0 - 1e-9, "{bore:?}");
    assert!(
        bore.origin().x.abs() < 1e-3 && (bore.origin().z - 4.0).abs() < 1e-3,
        "{bore:?}"
    );
    assert!(full - harness.body_volume(puck) > 0.5 * std::f64::consts::PI * 9.0 * 10.0);
    assert!(!harness.model.evaluation().body(puck).unwrap().faces().any(
        |(_, face)| matches!(face.surface(), caditor_kernel::Surface::Cylinder(cylinder)
                if cylinder.radius() < 4.0 && cylinder.frame().normal().y.abs() < 0.99)
    ));

    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let on_face = Handle::Place(crate::move_manipulator::PlaceGrip::OnPlane);
    let from = harness
        .workspace
        .viewport
        .handle_position(on_face, 0.0)
        .expect("the hole on the round wall has a square to drag");
    let to = harness
        .workspace
        .viewport
        .handle_position(on_face, 6.0)
        .unwrap();
    drag_screen(&mut harness, from, to);
    harness.frame();
    harness.settle();
    let dragged = harness
        .model
        .evaluation()
        .body(puck)
        .unwrap()
        .faces()
        .find_map(|(_, face)| match face.surface() {
            caditor_kernel::Surface::Cylinder(cylinder) if cylinder.radius() < 4.0 => {
                Some(*cylinder.frame())
            }
            _ => None,
        })
        .expect("the dragged hole has a bore");
    let radial = Vector3::new(dragged.origin().x, dragged.origin().y, 0.0).normalize();

    assert_eq!(harness.model.undo_label(), Some("Move Hole 1"));
    assert!(
        dragged.origin().distance(bore.origin()) > 3.0,
        "{dragged:?}"
    );
    assert!((dragged.origin().z - 4.0).abs() < 1e-3, "{dragged:?}");
    assert!(
        dragged.normal().dot(radial).abs() > 1.0 - 1e-6,
        "{dragged:?}"
    );
}

#[test]
fn a_hole_is_drilled_on_a_selected_face_and_takes_a_metric_size_fit_and_slot_from_its_panel() {
    let mut harness = Harness::new();
    harness.perform(Action::ShowThreadsAsCosmetic(true));
    let (plate, top) = extruded_plate(&mut harness);

    harness.select([top]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    let definition = open_hole(&harness, hole);
    let sketch = harness.sketch(definition.sketch).clone();

    assert_eq!(harness.model.undo_label(), Some("Create Hole 1"));
    assert_eq!(definition.body, plate);
    assert_eq!(
        sketch
            .entities()
            .map(|(_, entity)| entity.clone())
            .collect::<Vec<_>>(),
        vec![Entity::Point(Point2::new(20.0, 20.0))]
    );
    assert!(attached_body(&harness, definition.sketch) == Some(plate));
    assert!(harness.shows_containing("Hole 1 is drilled in the middle of the face"));
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 9.0 * 10.0
    ));

    choose(&mut harness, crate::hole_panel::CUSTOM_SIZE, "M3");

    assert_eq!(
        open_hole(&harness, hole).diameter.to_stored_text(),
        "3.4 mm"
    );
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 1.7 * 1.7 * 10.0
    ));
    assert!(harness.shows("Normal"));

    harness.click("Normal");
    harness.click("Tapped");
    harness.settle();

    assert_eq!(
        open_hole(&harness, hole).diameter.to_stored_text(),
        "2.5 mm"
    );
    assert!(harness.shows_containing("Thread M3 × 0.5"));

    harness.click("Tapped");
    harness.click("Fine");
    harness.settle();

    assert_eq!(
        open_hole(&harness, hole).diameter.to_stored_text(),
        "2.65 mm"
    );
    assert!(harness.shows_containing("Thread M3 × 0.35"));

    harness.click("Plain");
    harness.click("Counterbore");
    harness.settle();

    assert!(matches!(
        open_hole(&harness, hole).style,
        caditor_document::HoleStyle::Counterbore { ref diameter, .. } if diameter.to_stored_text() == "6.5 mm"
    ));

    assert!(!harness.shows(crate::hole_panel::PITCH));

    harness.click("Fine");
    harness.click("Insert");
    harness.settle();

    assert_eq!(open_hole(&harness, hole).diameter.to_stored_text(), "4 mm");
    assert!(harness.shows_containing("standard M3 heat-set insert 5.7 mm long"));

    let m8_fine = crate::hole_tools::with_standard(
        &open_hole(&harness, hole),
        caditor_document::HoleStandard {
            size: caditor_document::MetricSize::M8,
            fit: caditor_document::HoleFit::TappedFine(caditor_document::FinePitch::First),
        },
        crate::units::LengthUnit::Millimetre,
    );
    let resized = crate::hole_tools::edit(harness.document(), hole, m8_fine).unwrap();
    harness.perform(Action::Apply(resized));
    harness.settle();

    assert_eq!(open_hole(&harness, hole).diameter.to_stored_text(), "7 mm");
    assert!(harness.shows(crate::hole_panel::PITCH));

    harness.click("0.75");
    harness.settle();

    assert_eq!(
        open_hole(&harness, hole).diameter.to_stored_text(),
        "7.25 mm"
    );
    assert!(harness.shows_containing("Thread M8 × 0.75"));

    harness.type_into_field(Id::new(("hole-field", "diameter", hole)), "2.6 mm");
    harness.settle();

    assert_eq!(open_hole(&harness, hole).standard, None);
    assert!(!harness.shows("Tapped"));

    harness.click("Round");
    harness.click("Slot");
    harness.settle();

    assert!(matches!(
        open_hole(&harness, hole).shape,
        caditor_document::HoleShape::Slot { .. }
    ));
    assert!(harness.shows("Slot length"));
    assert!(harness.shows("Slot angle"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn the_hole_of_a_selected_wall_joins_the_selection_and_a_flat_face_is_no_hole() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        Point2::new(-20.0, -20.0),
        Point2::new(20.0, -20.0),
        Point2::new(20.0, 20.0),
        Point2::new(-20.0, 20.0),
    ];
    for index in 0..4 {
        sketch.add_line(corners[index], corners[(index + 1) % 4]);
    }
    let circle = sketch.add_circle(Point2::ZERO, 5.0);
    let circle_label = sketch.entity_label(circle);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let faces: Vec<Pickable> = harness.built().picks.pickables().filter(is_face).collect();
    let describe = |face: &Pickable| face.describe(harness.document(), harness.model.evaluation());
    let walls: Vec<Pickable> = faces
        .iter()
        .copied()
        .filter(|face| describe(face).contains(&circle_label))
        .collect();
    let flat = faces
        .iter()
        .copied()
        .find(|face| describe(face).contains("end face"))
        .unwrap();

    harness.select([walls[0]]);
    run_from_palette(&mut harness, "Select the whole hole");
    harness.frame();
    let hole = selected_of(&harness, is_face);
    harness.select([flat]);
    run_from_palette(&mut harness, "Select the whole hole");
    harness.frame();

    assert!(!walls.is_empty());
    assert_eq!(hole, walls.len());
    assert_eq!(selected_of(&harness, is_face), 1);
    assert!(harness.shows_containing(crate::body_selection::NO_HOLE));
}

fn extruded(harness: &mut Harness, rectangles: &[(Point2, Point2)]) -> caditor_document::FeatureId {
    let mut sketch = Sketch::new(Plane::XY);
    for (min, max) in rectangles {
        super::rectangle(&mut sketch, *min, *max);
    }
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
    harness.settle();
    let extrusion = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    extrusion
}

fn refusals_on_flat_faces(harness: &mut Harness) -> Vec<String> {
    let bodies: Vec<_> = harness
        .model
        .evaluation()
        .bodies()
        .map(|(body, _)| body)
        .collect();
    let mut refusals = Vec::new();
    for body in bodies {
        let faces: Vec<Pickable> = crate::bodies::shown(harness.model.evaluation(), body)
            .map(|shown| crate::bodies::face_keys(&shown.solid))
            .unwrap_or_default()
            .into_iter()
            .map(|(_, face)| Pickable::Face { body, face })
            .collect();
        for face in faces {
            harness.select([face]);
            let Ok(start) = crate::hole_tools::start(
                &harness.model,
                harness.workspace.viewport.selection(),
                &harness.workspace.editing,
            ) else {
                continue;
            };
            if !matches!(start, crate::hole_tools::HoleStart::Face(_)) {
                continue;
            }
            refusals.extend(
                crate::hole_tools::create_actions(&harness.model, start)
                    .into_iter()
                    .filter_map(|action| match action {
                        Action::Inform(notice) => Some(format!("{body:?}: {notice:?}")),
                        _ => None,
                    })
                    .filter(|notice| notice.contains("Warning")),
            );
        }
    }
    refusals
}

fn drill_end_face(harness: &mut Harness, body: caditor_document::FeatureId) -> bool {
    let end = harness.built().picks.pickables().find(|pickable| {
        matches!(pickable, Pickable::Face { body: owner, .. } if *owner == body)
            && pickable
                .describe(harness.document(), harness.model.evaluation())
                .ends_with("end face")
    });
    let Some(end) = end else {
        return false;
    };
    harness.select([end]);
    let Ok(start) = crate::hole_tools::start(
        &harness.model,
        harness.workspace.viewport.selection(),
        &harness.workspace.editing,
    ) else {
        return false;
    };
    let volume = harness.body_volume(body);
    for action in crate::hole_tools::create_actions(&harness.model, start) {
        harness.perform(action);
    }
    harness.settle();
    harness.model.evaluation().failed_count() == 0 && harness.body_volume(body) < volume
}

#[test]
fn a_hole_starts_on_any_flat_face_of_a_separate_piece() {
    let mut harness = Harness::new();
    let extrusion = extruded(
        &mut harness,
        &[
            (Point2::new(0.0, 0.0), Point2::new(20.0, 20.0)),
            (Point2::new(40.0, 0.0), Point2::new(60.0, 20.0)),
        ],
    );
    harness.perform(Action::Apply(Transaction::single(
        "A body each",
        Edit::SetFeaturePieces {
            id: extrusion,
            pieces: caditor_document::Pieces::BodyEach,
        },
    )));
    harness.settle();

    let pieces = harness.model.evaluation().pieces_made_by(extrusion);

    assert_eq!(pieces.len(), 2);
    assert_eq!(refusals_on_flat_faces(&mut harness), Vec::<String>::new());
    assert!(drill_end_face(&mut harness, pieces[1]));
}

#[test]
fn a_hole_starts_on_any_flat_face_of_both_halves_of_a_split_body() {
    let mut harness = Harness::new();
    let extrusion = extruded(
        &mut harness,
        &[(Point2::new(-20.0, -10.0), Point2::new(20.0, 10.0))],
    );
    let mut transaction = harness.document().transaction("Split");
    transaction.add_feature(
        "Split 1",
        FeatureKind::Split(caditor_document::Split {
            body: extrusion,
            along: caditor_document::SplitAlong::Plane(PlaneReference::Principal(
                PrincipalPlane::Yz,
            )),
            flipped: false,
        }),
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();

    let bodies: Vec<_> = harness
        .model
        .evaluation()
        .bodies()
        .map(|(body, _)| body)
        .collect();
    let half = *bodies
        .iter()
        .find(|body| **body != extrusion)
        .expect("the split makes a second body");

    assert_eq!(bodies.len(), 2);
    assert_eq!(refusals_on_flat_faces(&mut harness), Vec::<String>::new());
    assert!(drill_end_face(&mut harness, half));
}
