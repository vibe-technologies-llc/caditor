use std::f64::consts::{FRAC_PI_2, PI};

use caditor_document::{FeatureId, FeatureState, MissingInput, Pipe, SweepPath};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::PipeShape;
use caditor_sketch::{EntityId, Sketch};
use egui::{Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, choose, drag_value_arrow, extruded_plate, length_value,
    run_from_palette, top_edge_along_x,
};
use crate::{
    move_manipulator::Handle, pipe_panel, pipe_tools, reference_picking::Slot, selection::Pickable,
    solid_panel, value_gauges::Measured,
};

const FIRST: f64 = 40.0;
const SECOND: f64 = 30.0;

fn add_elbow(harness: &mut Harness) -> (FeatureId, [EntityId; 2]) {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::ZERO, Point2::new(FIRST, 0.0));
    let second = sketch.add_line(Point2::new(FIRST, 0.0), Point2::new(FIRST, SECOND));
    (harness.add_sketch(sketch), [first, second])
}

fn pipe_of(harness: &Harness, feature: FeatureId) -> Pipe {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.pipe())
        .expect("a pipe")
        .clone()
}

fn near(found: f64, expected: f64) -> bool {
    (found - expected).abs() < 1e-2 * expected
}

fn bent_length(bend: f64) -> f64 {
    FIRST + SECOND - 2.0 * bend + bend * FRAC_PI_2
}

#[test]
fn pipe_runs_along_the_selected_lines_bending_their_corner_and_sizes_from_its_panel() {
    let mut harness = Harness::new();
    let (path, [first, second]) = add_elbow(&mut harness);
    harness.select([
        Pickable::SketchEntity {
            feature: path,
            entity: first,
        },
        Pickable::SketchEntity {
            feature: path,
            entity: second,
        },
    ]);

    harness.key(Key::I, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();

    let pipe = harness.workspace.editing.solid().expect("the pipe is open");
    assert_eq!(harness.model.undo_label(), Some("Create Pipe 1"));
    let made = pipe_of(&harness, pipe);
    assert_eq!(
        made.path,
        SweepPath::Curves {
            sketch: path,
            curves: vec![first, second],
        }
    );
    assert_eq!(made.shape, PipeShape::Round);
    assert!(near(
        harness.body_volume(pipe),
        PI * 25.0 * bent_length(10.0)
    ));
    assert!(harness.shows(solid_panel::PATH));
    assert!(harness.shows("2 curves of Plate"));
    assert!(harness.shows("Diameter"));
    assert!(harness.shows(pipe_panel::BEND_RADIUS));

    harness.type_into_field(Id::new(("pipe-field", pipe, "bend")), "0 mm");
    harness.settle();
    assert!(near(
        harness.body_volume(pipe),
        PI * 25.0 * (FIRST + SECOND)
    ));

    choose(&mut harness, "Round", "Square");
    assert_eq!(pipe_of(&harness, pipe).shape, PipeShape::Square);
    assert!(harness.shows("Width"));
    assert!(near(harness.body_volume(pipe), 100.0 * (FIRST + SECOND)));

    choose(&mut harness, pipe_panel::SOLID, pipe_panel::HOLLOW);
    assert!(harness.shows(pipe_panel::WALL_THICKNESS));
    assert!(near(
        harness.body_volume(pipe),
        (100.0 - 64.0) * (FIRST + SECOND)
    ));

    harness.type_into_field(Id::new(("pipe-field", pipe, "wall")), "5 mm");
    harness.settle();
    let reason = match harness
        .model
        .evaluation()
        .feature(pipe)
        .map(|status| status.state.clone())
    {
        Some(FeatureState::Failed(error)) => error.reason,
        other => panic!("a wall as thick as the pipe fails, found {other:?}"),
    };
    assert!(reason.contains("half its width"), "{reason}");
    assert!(harness.shows(&reason));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    assert!(near(
        harness.body_volume(pipe),
        (100.0 - 64.0) * (FIRST + SECOND)
    ));
}

#[test]
fn pipe_with_nothing_selected_starts_choosing_its_path_in_the_view() {
    let mut harness = Harness::new();
    let (path, [first, _]) = add_elbow(&mut harness);
    harness.select([]);

    run_from_palette(&mut harness, "pipe");
    harness.settle();

    let pipe = harness.workspace.editing.solid().expect("the pipe is open");
    assert_eq!(pipe_of(&harness, pipe).path, SweepPath::Unchosen);
    assert_eq!(
        harness
            .workspace
            .editing
            .picking()
            .map(|picking| picking.slot),
        Some(Slot::SweepPath)
    );
    assert_eq!(
        harness
            .model
            .evaluation()
            .feature(pipe)
            .map(|status| status.state.clone()),
        Some(FeatureState::Waiting(MissingInput::Path))
    );
    assert!(harness.shows("Waiting for a path"));

    harness.click_pickable(
        Plane::XY,
        Point2::new(0.5 * FIRST, 0.0),
        Pickable::SketchEntity {
            feature: path,
            entity: first,
        },
    );
    harness.settle();

    assert_eq!(
        pipe_of(&harness, pipe).path,
        SweepPath::Curves {
            sketch: path,
            curves: vec![first],
        }
    );
    assert!(near(harness.body_volume(pipe), PI * 25.0 * FIRST));
    assert_eq!(
        harness
            .workspace
            .editing
            .picking()
            .map(|picking| picking.slot),
        Some(Slot::SweepPath)
    );
}

fn open_elbow_pipe(harness: &mut Harness) -> FeatureId {
    let (path, [first, second]) = add_elbow(harness);
    harness.select([
        Pickable::SketchEntity {
            feature: path,
            entity: first,
        },
        Pickable::SketchEntity {
            feature: path,
            entity: second,
        },
    ]);
    harness.key(Key::I, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    harness.select([]);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.workspace.editing.solid().expect("the pipe is open")
}

pub(super) fn undone_and_redone(
    harness: &mut Harness,
    label: &str,
    value: impl Fn(&Harness) -> f64,
) {
    let changed = value(harness);

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    let restored = value(harness);

    harness.key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.settle();

    assert_eq!(value(harness), changed, "redoing {label}");
    assert_ne!(restored, changed, "undoing {label}");
}

#[test]
fn dragging_a_pipe_s_size_arrow_changes_its_diameter_in_one_undoable_change() {
    let mut harness = Harness::new();
    let pipe = open_elbow_pipe(&mut harness);
    let size = |harness: &Harness| length_value(harness, &pipe_of(harness, pipe).size);
    let before = size(&harness);

    let step = drag_value_arrow(&mut harness, Measured::PipeSize, 3.0);
    let after = size(&harness);

    assert_eq!(before, pipe_tools::DEFAULT_SIZE);
    assert_eq!(harness.model.undo_label(), Some("Edit Pipe 1"));
    assert!(
        (after - before - 6.0).abs() <= 2.0 * step,
        "{before} to {after} with steps of {step}"
    );

    undone_and_redone(&mut harness, "the size drag", size);
}

#[test]
fn dragging_a_hollow_pipe_s_wall_arrow_changes_its_wall_thickness() {
    let mut harness = Harness::new();
    let pipe = open_elbow_pipe(&mut harness);
    choose(&mut harness, pipe_panel::SOLID, pipe_panel::HOLLOW);
    let wall = |harness: &Harness| {
        length_value(
            harness,
            pipe_of(harness, pipe).wall.as_ref().expect("a hollow pipe"),
        )
    };
    let before = wall(&harness);

    let step = drag_value_arrow(&mut harness, Measured::PipeWall, 1.0);
    let after = wall(&harness);

    assert_eq!(harness.model.undo_label(), Some("Edit Pipe 1"));
    assert!(
        (after - before - 1.0).abs() <= step,
        "{before} to {after} with steps of {step}"
    );

    undone_and_redone(&mut harness, "the wall drag", wall);
}

#[test]
fn a_solid_pipe_has_no_wall_arrow() {
    let mut harness = Harness::new();
    open_elbow_pipe(&mut harness);

    assert!(
        harness
            .workspace
            .viewport
            .handle_position(Handle::Value(Measured::PipeWall), 0.0)
            .is_none()
    );
    assert!(
        harness
            .workspace
            .viewport
            .handle_position(Handle::Value(Measured::PipeSize), 0.0)
            .is_some()
    );
}

#[test]
fn dragging_a_pipe_s_bend_arrow_changes_the_radius_of_the_first_corner() {
    let mut harness = Harness::new();
    let pipe = open_elbow_pipe(&mut harness);
    let bend = |harness: &Harness| length_value(harness, &pipe_of(harness, pipe).bend);
    let before = bend(&harness);
    let per_radius = std::f64::consts::SQRT_2 - 1.0;

    let step = drag_value_arrow(&mut harness, Measured::PipeBend, 1.0);
    let after = bend(&harness);

    assert_eq!(before, pipe_tools::DEFAULT_BEND);
    assert_eq!(harness.model.undo_label(), Some("Edit Pipe 1"));
    assert!(
        (after - before - 1.0 / per_radius).abs() <= step / per_radius,
        "{before} to {after} with steps of {step}"
    );

    undone_and_redone(&mut harness, "the bend drag", bend);
}

#[test]
fn a_straight_pipe_has_no_bend_arrow() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(FIRST, 0.0));
    let path = harness.add_sketch(sketch);
    harness.select([Pickable::SketchEntity {
        feature: path,
        entity: line,
    }]);
    harness.key(Key::I, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    harness.select([]);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();

    assert!(
        harness
            .workspace
            .viewport
            .handle_position(Handle::Value(Measured::PipeBend), 0.0)
            .is_none()
    );
    assert!(
        harness
            .workspace
            .viewport
            .handle_position(Handle::Value(Measured::PipeSize), 0.0)
            .is_some()
    );
}

fn foot_of(harness: &Harness, measured: Measured) -> Point3 {
    harness
        .workspace
        .viewport
        .handle_foot(Handle::Value(measured))
        .expect("the arrow has a foot")
}

#[test]
fn a_pipe_s_size_and_wall_arrows_stand_on_the_rims_of_its_start_section_in_every_shape() {
    let mut harness = Harness::new();
    let pipe = open_elbow_pipe(&mut harness);
    choose(&mut harness, pipe_panel::SOLID, pipe_panel::HOLLOW);
    let size = pipe_tools::DEFAULT_SIZE;
    let wall = pipe_tools::DEFAULT_WALL;
    let triangle_wall = wall * 2.0 / 3.0_f64.sqrt();
    let shapes = [
        ("Round", "Round", size / 2.0, wall),
        ("Round", "Square", size / 2.0, wall),
        ("Square", "Triangle", size / 3.0, triangle_wall),
    ];

    for (from, to, outer, thickness) in shapes {
        choose(&mut harness, from, to);
        harness.workspace.viewport.advance(CAMERA_SETTLE);
        harness.frame();

        let outer_foot = foot_of(&harness, Measured::PipeSize);
        let inner_foot = foot_of(&harness, Measured::PipeWall);

        assert!(
            outer_foot.distance(Point3::new(0.0, outer, 0.0)) < 1e-6,
            "{to} size foot {outer_foot:?}"
        );
        assert!(
            inner_foot.distance(Point3::new(0.0, outer - thickness, 0.0)) < 1e-6,
            "{to} wall foot {inner_foot:?}"
        );
    }
    assert_eq!(pipe_of(&harness, pipe).shape, PipeShape::Triangle);
}

#[test]
fn a_pipe_s_bend_arrow_stands_at_the_middle_of_the_first_rounded_corner() {
    let mut harness = Harness::new();
    open_elbow_pipe(&mut harness);
    let centre = Point3::new(FIRST - 10.0, 10.0, 0.0);
    let towards_corner = Vector3::new(1.0, -1.0, 0.0).normalize();

    let foot = foot_of(&harness, Measured::PipeBend);

    assert!(
        foot.distance(centre + towards_corner * 10.0) < 1e-6,
        "{foot:?}"
    );
}

#[test]
fn a_pipe_along_a_body_edge_has_its_size_arrow_at_the_start_of_the_edge() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let edge = top_edge_along_x(&harness, plate, 0.0);
    harness.select([Pickable::Edge { body: plate, edge }]);
    harness.key(Key::I, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    harness.select([]);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();

    let foot = foot_of(&harness, Measured::PipeSize);

    assert!(
        (foot.x.abs() < 1e-6 || (foot.x - FIRST).abs() < 1e-6)
            && (foot.y.abs() - pipe_tools::DEFAULT_SIZE / 2.0).abs() < 1e-6
            && (foot.z - 10.0).abs() < 1e-6,
        "{foot:?}"
    );
}

const ARC_RADIUS: f64 = 20.0;
const ARC_CENTRE: Point2 = Point2::new(FIRST + ARC_RADIUS, 0.0);

fn open_line_into_arc_pipe(harness: &mut Harness) -> FeatureId {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(FIRST, 0.0));
    let arc = sketch.add_arc(
        ARC_CENTRE,
        ARC_CENTRE + Point2::new(0.0, ARC_RADIUS),
        Point2::new(FIRST, 0.0),
    );
    let path = harness.add_sketch(sketch);
    harness.select([line, arc].map(|entity| Pickable::SketchEntity {
        feature: path,
        entity,
    }));
    harness.key(Key::I, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    harness.select([]);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.workspace.editing.solid().expect("the pipe is open")
}

fn bend_centre(bend: f64) -> Point3 {
    let reach = ((ARC_RADIUS + bend).powi(2) - bend * bend).sqrt();
    Point3::new(ARC_CENTRE.x - reach, bend, 0.0)
}

#[test]
fn a_pipe_s_bend_arrow_stands_on_the_bend_where_a_line_meets_an_arc_sharply() {
    let mut harness = Harness::new();
    let pipe = open_line_into_arc_pipe(&mut harness);
    let bend = length_value(&harness, &pipe_of(&harness, pipe).bend);

    let foot = foot_of(&harness, Measured::PipeBend);

    assert_eq!(bend, pipe_tools::DEFAULT_BEND);
    assert!(
        (foot.distance(bend_centre(bend)) - bend).abs() < 1e-6,
        "{foot:?}"
    );
    assert!(foot.y > 0.0 && foot.y < bend, "{foot:?}");
}

#[test]
fn dragging_a_pipe_s_bend_arrow_where_a_line_meets_an_arc_changes_its_bend_radius() {
    let mut harness = Harness::new();
    let pipe = open_line_into_arc_pipe(&mut harness);
    let bend = |harness: &Harness| length_value(harness, &pipe_of(harness, pipe).bend);
    let before = bend(&harness);

    let step = drag_value_arrow(&mut harness, Measured::PipeBend, 4.0);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let after = bend(&harness);
    let foot = foot_of(&harness, Measured::PipeBend);

    assert_eq!(harness.model.undo_label(), Some("Edit Pipe 1"));
    assert!(
        (after - before - 4.0).abs() <= 2.0 * step,
        "{before} to {after} with steps of {step}"
    );
    assert!(
        (foot.distance(bend_centre(after)) - after).abs() < 1e-6,
        "{foot:?}"
    );

    undone_and_redone(&mut harness, "the bend drag", bend);
}
