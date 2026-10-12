use caditor_document::FeatureId;
use caditor_geometry::Point3;
use egui::{Event, Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, drag_screen, extruded_plate, length_value, open_hole, open_hole_on,
    placed_holes, plate_edge_at,
};
use crate::{
    hole_handles::HoleGrip,
    hole_panel,
    hole_placement::{self, Anchor},
    model::Action,
    move_manipulator::Handle,
    selection::{Pickable, Selection},
};

fn open_two_holes(harness: &mut Harness) -> (FeatureId, FeatureId, Pickable) {
    let (plate, top) = extruded_plate(harness);
    let hole = open_hole_on(harness, top);
    let mut chosen = Selection::default();
    chosen.toggle(top);
    let added =
        hole_placement::add_change(&harness.model, &chosen, hole, &open_hole(harness, hole))
            .unwrap();
    harness.perform(Action::Apply(added));
    harness.settle();
    harness.select([]);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    (plate, hole, top)
}

fn open_measured_hole(harness: &mut Harness, edges: &[[f64; 3]]) -> (FeatureId, FeatureId) {
    let (plate, top) = extruded_plate(harness);
    let hole = open_hole_on(harness, top);
    for [x, y, z] in edges {
        let edge = plate_edge_at(harness, plate, Point3::new(*x, *y, *z));
        harness.select([Pickable::Edge { body: plate, edge }]);
        harness.click(hole_panel::MEASURE_SELECTED);
        harness.settle();
    }
    harness.select([]);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    (plate, hole)
}

fn constraints_of(harness: &Harness, hole: FeatureId) -> Vec<caditor_sketch::ConstraintId> {
    match &placed_holes(harness, hole)[0].0.anchor {
        Anchor::Edges(edges) => edges.iter().map(|edge| edge.held.constraint).collect(),
        anchor => panic!("the hole is measured from edges, not {anchor:?}"),
    }
}

fn grip(harness: &Harness, handle: Handle, along: f64) -> egui::Pos2 {
    harness
        .workspace
        .viewport
        .handle_position(handle, along)
        .unwrap_or_else(|| panic!("{handle:?} is shown"))
}

fn hover(harness: &mut Harness, handle: Handle) {
    let at = grip(harness, handle, 0.0);
    harness.events.push(Event::PointerMoved(at));
    harness.frame();
    harness.frame();
}

fn drag_handle(harness: &mut Harness, handle: Handle, along: f64) -> f64 {
    let from = grip(harness, handle, 0.0);
    let to = grip(harness, handle, along);
    let step = harness.workspace.viewport.manipulator_step().unwrap();
    drag_screen(harness, from, to);
    harness.frame();
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    step
}

fn distance_text(harness: &Harness, hole: FeatureId, index: usize) -> String {
    match &placed_holes(harness, hole)[0].0.anchor {
        Anchor::Edges(edges) => harness.document().expression_text(&edges[index].value),
        anchor => panic!("the hole is measured from edges, not {anchor:?}"),
    }
}

const FRONT: [f64; 3] = [20.0, 0.0, 10.0];
const LEFT: [f64; 3] = [0.0, 20.0, 10.0];

#[test]
fn each_free_point_of_a_hole_of_several_points_has_a_square_that_moves_only_that_point() {
    let mut harness = Harness::new();
    let (_, hole, _) = open_two_holes(&mut harness);
    let points: Vec<_> = placed_holes(&harness, hole)
        .into_iter()
        .map(|(placed, at)| (placed.point, at))
        .collect();
    let (first, second) = (points[0], points[1]);
    let square = |point| Handle::Hole(HoleGrip::Square(point));

    hover(&mut harness, square(second.0));

    assert!(harness.shows("Drag to move hole 2 on its face"));

    let step = drag_handle(&mut harness, square(second.0), 8.0);
    let after = placed_holes(&harness, hole);

    assert_eq!(harness.model.undo_label(), Some("Move Hole 1"));
    assert!(
        (after[1].1.x - second.1.x - 8.0).abs() <= step,
        "{:?} to {:?} with steps of {step}",
        second.1,
        after[1].1
    );
    assert!((after[1].1.y - second.1.y).abs() <= step);
    assert!(after[0].1.distance(first.1) < 1e-9);

    harness.perform(Action::Undo);
    harness.settle();

    assert!(placed_holes(&harness, hole)[1].1.distance(second.1) < 1e-9);
}

#[test]
fn a_square_on_a_point_held_by_a_parameter_does_not_drag_and_says_why() {
    let mut harness = Harness::new();
    let (_, hole, _) = open_two_holes(&mut harness);
    let mut transaction = harness.document().transaction("Add spot");
    let spot = transaction.parse("12 mm").unwrap();
    transaction.add_parameter("spot", spot);
    harness.perform(Action::Apply(transaction.finish()));
    let (placed, _) = placed_holes(&harness, hole).remove(1);
    harness.type_into_field(Id::new(("hole-position", hole, placed.point, 0)), "spot");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let before = placed_holes(&harness, hole)[1].1;
    let square = Handle::Hole(HoleGrip::Square(placed.point));

    hover(&mut harness, square);

    assert!(harness.shows("Hole 2's position follows spot; change spot in Parameters"));

    drag_handle(&mut harness, square, 6.0);

    assert_eq!(placed_holes(&harness, hole)[1].1, before);
}

#[test]
fn a_hole_measured_from_edges_has_an_arrow_per_distance_and_dragging_one_changes_only_it() {
    let mut harness = Harness::new();
    let (_, hole) = open_measured_hole(&mut harness, &[FRONT, LEFT]);
    let [from_front, from_left] = constraints_of(&harness, hole)[..] else {
        panic!("the hole is measured from two edges");
    };
    let at = placed_holes(&harness, hole)[0].1;
    let front = Handle::Hole(HoleGrip::Distance(from_front));

    hover(&mut harness, front);

    assert!(harness.shows("Drag to change the hole's distance from its edge"));
    assert!(
        harness
            .workspace
            .viewport
            .handle_position(Handle::Hole(HoleGrip::Distance(from_left)), 0.0)
            .is_some()
    );

    let step = drag_handle(&mut harness, front, 5.0);
    let moved = placed_holes(&harness, hole)[0].1;

    assert_eq!(harness.model.undo_label(), Some("Move Hole 1"));
    assert!(
        (moved.y - at.y - 5.0).abs() <= step,
        "{at:?} to {moved:?} with steps of {step}"
    );
    assert!((moved.x - at.x).abs() < 1e-9, "{at:?} to {moved:?}");
    assert_eq!(distance_text(&harness, hole, 1), "20 mm");

    let step = drag_handle(
        &mut harness,
        Handle::Hole(HoleGrip::Distance(from_left)),
        -4.0,
    );
    let again = placed_holes(&harness, hole)[0].1;

    assert!(
        (again.x - moved.x + 4.0).abs() <= step,
        "{moved:?} to {again:?} with steps of {step}"
    );
    assert!((again.y - moved.y).abs() < 1e-9);

    harness.perform(Action::Undo);
    harness.settle();
    harness.perform(Action::Undo);
    harness.settle();

    assert!(placed_holes(&harness, hole)[0].1.distance(at) < 1e-9);
}

#[test]
fn an_edge_arrow_dragged_toward_its_edge_stops_one_step_short_of_it() {
    let mut harness = Harness::new();
    let (_, hole) = open_measured_hole(&mut harness, &[FRONT]);
    let [from_front] = constraints_of(&harness, hole)[..] else {
        panic!("the hole is measured from one edge");
    };
    let arrow = Handle::Hole(HoleGrip::Distance(from_front));

    let step = drag_handle(&mut harness, arrow, -60.0);
    let y = placed_holes(&harness, hole)[0].1.y;

    assert!(
        y >= step - 1e-9 && y <= 2.0 * step,
        "{y} with steps of {step}"
    );
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn an_edge_arrow_keeps_a_named_distance_and_leaves_a_driven_one_alone() {
    let mut harness = Harness::new();
    let (_, hole) = open_measured_hole(&mut harness, &[FRONT]);
    let [from_front] = constraints_of(&harness, hole)[..] else {
        panic!("the hole is measured from one edge");
    };
    let arrow = Handle::Hole(HoleGrip::Distance(from_front));
    let field = Id::new(("hole-edge-distance", hole, from_front));

    harness.type_into_field(field, "gap = 12 mm");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let step = drag_handle(&mut harness, arrow, 5.0);
    let gap = harness
        .model
        .parameters()
        .value(harness.parameter("gap"))
        .unwrap()
        .value;

    assert_eq!(distance_text(&harness, hole, 0), "gap");
    assert!((gap - 17.0).abs() <= step, "{gap} with steps of {step}");
    assert_eq!(harness.model.undo_label(), Some("Move Hole 1"));

    harness.type_into_field(field, "gap * 2");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let before = placed_holes(&harness, hole)[0].1;
    hover(&mut harness, arrow);

    assert!(harness.shows_containing("Distance follows gap"));

    drag_handle(&mut harness, arrow, 5.0);

    assert_eq!(placed_holes(&harness, hole)[0].1, before);
}

#[test]
fn a_value_typed_over_an_edge_arrow_sets_the_distance_in_one_change() {
    let mut harness = Harness::new();
    let (_, hole) = open_measured_hole(&mut harness, &[FRONT]);
    let [from_front] = constraints_of(&harness, hole)[..] else {
        panic!("the hole is measured from one edge");
    };

    hover(&mut harness, Handle::Hole(HoleGrip::Distance(from_front)));
    harness.type_text("30");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Move Hole 1"));
    assert!((placed_holes(&harness, hole)[0].1.y - 30.0).abs() < 1e-6);
    assert_eq!(
        length_value(
            &harness,
            match &placed_holes(&harness, hole)[0].0.anchor {
                Anchor::Edges(edges) => &edges[0].value,
                anchor => panic!("the hole is measured from edges, not {anchor:?}"),
            }
        ),
        30.0
    );
}
