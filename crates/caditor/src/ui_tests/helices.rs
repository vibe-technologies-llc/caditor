use std::f64::consts::TAU;

use caditor_document::{
    AxisReference, CoilLength, Edit, FeatureId, FeatureState, Helix, HelixKind, PointReference,
    PrincipalAxis, SweepPath, Transaction,
};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::{Entity, Sketch};
use egui::{Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, choose, drag_value_arrow, length_value, pipes::undone_and_redone,
    run_from_palette,
};
use crate::{
    coil_panel, coil_tools,
    editing::EditingCommand,
    helix_panel, helix_tools, icons,
    model::Action,
    pipe_tools,
    selection::{Axis, Pickable},
    solid_panel,
    value_gauges::Measured,
};

const RADIUS: f64 = 0.5 * coil_tools::DEFAULT_DIAMETER;

fn helix_of(harness: &Harness, feature: FeatureId) -> Helix {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.helix())
        .expect("a helix")
        .clone()
}

fn triangle_on_the_helix_start() -> Sketch {
    let plane = Plane::from_frame(Point3::ZERO, Vector3::Y, Vector3::X).unwrap();
    let mut sketch = Sketch::new(plane);
    let corners = [
        Point2::new(11.0, -1.0),
        Point2::new(11.0, 1.0),
        Point2::new(9.0, 0.0),
    ];
    for (from, to) in corners.iter().zip(corners.iter().cycle().skip(1)) {
        sketch.add_line(*from, *to);
    }
    sketch
}

#[test]
fn helix_winds_round_z_from_the_palette_and_a_triangle_swept_along_it_makes_a_thread() {
    let mut harness = Harness::new();
    harness.select([Pickable::Axis(Axis::Z)]);

    run_from_palette(&mut harness, "helix");
    harness.settle();

    let helix = harness
        .workspace
        .editing
        .solid()
        .expect("the helix is open");
    assert_eq!(harness.model.undo_label(), Some("Create Helix 1"));
    assert_eq!(
        helix_of(&harness, helix).axis,
        AxisReference::Principal(PrincipalAxis::Z)
    );
    assert!(harness.workspace.editing.picking().is_none());
    assert!(harness.shows(helix_panel::DESCRIPTION));
    assert!(harness.shows(coil_panel::SIZED_BY));
    assert!(!harness.document().feature(helix).unwrap().makes_body());

    harness.type_into_field(Id::unique(("helix-field", helix, "turns")), "2");
    harness.settle();
    assert_eq!(
        helix_of(&harness, helix).length.turns(),
        Some(&caditor_expression::Expression::number(2.0))
    );

    harness.perform(Action::Editing(EditingCommand::CloseSolid));
    let profile = harness.add_sketch(triangle_on_the_helix_start());
    harness.select([Pickable::Datum(helix)]);

    run_from_palette(&mut harness, "sweep");
    harness.settle();

    let sweep = harness
        .workspace
        .editing
        .solid()
        .expect("the sweep is open");
    let made = harness
        .document()
        .feature(sweep)
        .and_then(|feature| feature.kind.solid())
        .expect("a sweep")
        .clone();
    assert_eq!(made.sketch(), profile);
    assert_eq!(made.path(), Some(&SweepPath::Helix { helix }));
    assert!(harness.shows(solid_panel::PATH));
    assert!(harness.shows("Helix 1"));
    let expected = 2.0 * TAU * 2.0 * (31.0 / 3.0);
    let found = harness.body_volume(sweep);
    assert!(
        (found - expected).abs() < 1e-2 * expected,
        "{found} against {expected}"
    );

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    assert!(harness.document().feature(sweep).is_none());
}

fn click_removal_beside(harness: &mut Harness, label: &str) {
    let row = harness
        .texts
        .iter()
        .find(|(shown, _)| shown == label)
        .expect("the row is on screen")
        .1;
    let button = harness
        .texts
        .iter()
        .filter(|(shown, _)| shown == icons::REMOVE)
        .map(|(_, rect)| rect.center())
        .find(|center| (center.y - row.center().y).abs() < row.height() * 2.0)
        .expect("the row can be removed");
    harness.click_screen(button);
}

fn open_vertical_helix(harness: &mut Harness) -> FeatureId {
    harness.select([Pickable::Axis(Axis::Z)]);
    run_from_palette(harness, "helix");
    harness.settle();
    harness.select([]);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness
        .workspace
        .editing
        .solid()
        .expect("the helix is open")
}

fn start_of(harness: &Harness, helix: FeatureId) -> Point3 {
    helix_tools::points(harness.model.evaluation(), helix)
        .first()
        .copied()
        .expect("the helix is built")
}

#[test]
fn typing_a_start_angle_starts_the_helix_further_round_its_axis_in_one_undoable_change() {
    let mut harness = Harness::new();
    let helix = open_vertical_helix(&mut harness);
    let before = start_of(&harness, helix);

    harness.type_into_field(Id::unique(("helix-field", helix, "start")), "90");
    harness.settle();
    let after = start_of(&harness, helix);

    assert!(harness.shows(helix_panel::START_ANGLE));
    assert!(
        before.distance(Point3::new(RADIUS, 0.0, 0.0)) < 1e-9,
        "{before}"
    );
    assert!(
        after.distance(Point3::new(0.0, RADIUS, 0.0)) < 1e-9,
        "{after}"
    );
    assert_eq!(harness.model.undo_label(), Some("Edit Helix 1"));
    let angle = |harness: &Harness| {
        harness
            .model
            .parameters()
            .evaluate_expression(&helix_of(harness, helix).start)
            .unwrap()
            .value
    };
    undone_and_redone(&mut harness, "the start angle", angle);
    assert_eq!(angle(&harness), 90.0);
}

#[test]
fn dragging_a_helix_s_diameter_and_pitch_arrows_changes_them_in_one_undoable_change_each() {
    let mut harness = Harness::new();
    let helix = open_vertical_helix(&mut harness);
    let diameter = |harness: &Harness| length_value(harness, &helix_of(harness, helix).diameter);
    let pitch = |harness: &Harness| {
        let length = helix_of(harness, helix).length;
        length_value(harness, length.pitch().expect("a pitch"))
    };
    let before = diameter(&harness);

    let step = drag_value_arrow(&mut harness, Measured::CoilDiameter, 3.0);
    let after = diameter(&harness);

    assert_eq!(before, coil_tools::DEFAULT_DIAMETER);
    assert_eq!(harness.model.undo_label(), Some("Edit Helix 1"));
    assert!(
        (after - before - 6.0).abs() <= 2.0 * step,
        "{before} to {after} with steps of {step}"
    );
    undone_and_redone(&mut harness, "the diameter drag", diameter);

    let before = pitch(&harness);
    let step = drag_value_arrow(&mut harness, Measured::CoilPitch, 2.0);
    let after = pitch(&harness);

    assert_eq!(before, caditor_document::DEFAULT_PITCH);
    assert!(
        (after - before - 2.0).abs() <= step,
        "{before} to {after} with steps of {step}"
    );
    undone_and_redone(&mut harness, "the pitch drag", pitch);
}

#[test]
fn picking_a_sketch_point_starts_the_helix_at_its_angle_and_follows_it_until_removed() {
    let mut harness = Harness::new();
    let mut drawn = Sketch::new(Plane::XY);
    let mark = drawn.add_point(Point2::new(0.0, 7.0));
    let marks = harness.add_sketch(drawn);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let helix = open_vertical_helix(&mut harness);
    let typed = start_of(&harness, helix);

    harness.select([Pickable::SketchEntity {
        feature: marks,
        entity: mark,
    }]);
    harness.frame();
    harness.click_beside(icons::USE_SELECTED, helix_panel::STARTS_AT);
    harness.settle();
    let pointed = start_of(&harness, helix);

    assert!(harness.shows(helix_panel::STARTS_AT));
    assert!(!harness.shows(helix_panel::START_ANGLE));
    assert_eq!(harness.model.undo_label(), Some("Edit Helix 1"));
    assert_eq!(
        helix_of(&harness, helix).start_point,
        Some(PointReference::Sketch {
            sketch: marks,
            entity: mark
        })
    );
    assert!(
        typed.distance(Point3::new(RADIUS, 0.0, 0.0)) < 1e-9,
        "{typed}"
    );
    assert!(
        pointed.distance(Point3::new(0.0, RADIUS, 0.0)) < 1e-9,
        "{pointed}"
    );

    harness.perform(Action::Apply(Transaction::single(
        "Move the mark",
        Edit::SetSketchEntity {
            feature: marks,
            id: mark,
            entity: Entity::Point(Point2::new(-7.0, 0.0)),
        },
    )));
    harness.settle();
    let followed = start_of(&harness, helix);

    assert!(
        followed.distance(Point3::new(-RADIUS, 0.0, 0.0)) < 1e-9,
        "{followed}"
    );

    click_removal_beside(&mut harness, helix_panel::STARTS_AT);
    harness.settle();

    let returned = start_of(&harness, helix);

    assert_eq!(helix_of(&harness, helix).start_point, None);
    assert!(harness.shows(helix_panel::START_ANGLE));
    assert!(
        returned.distance(Point3::new(RADIUS, 0.0, 0.0)) < 1e-9,
        "{returned}"
    );
}

#[test]
fn a_point_on_the_helix_axis_fails_the_helix_in_words_naming_the_remedy() {
    let mut harness = Harness::new();
    let mut drawn = Sketch::new(Plane::XY);
    let mark = drawn.add_point(Point2::ZERO);
    let marks = harness.add_sketch(drawn);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let helix = open_vertical_helix(&mut harness);

    harness.select([Pickable::SketchEntity {
        feature: marks,
        entity: mark,
    }]);
    harness.frame();
    harness.click_beside(icons::USE_SELECTED, helix_panel::STARTS_AT);
    harness.settle();

    let reason = harness
        .model
        .evaluation()
        .feature(helix)
        .and_then(|status| match &status.state {
            caditor_document::FeatureState::Failed(error) => Some(error.reason()),
            _ => None,
        })
        .expect("the helix fails");
    assert_eq!(
        reason,
        "The start point of Helix 1 lies on its axis, so it has no angle round it."
    );
}

fn reason_of(harness: &Harness, feature: FeatureId) -> Option<String> {
    harness
        .model
        .evaluation()
        .feature(feature)
        .and_then(|status| match &status.state {
            FeatureState::Failed(error) => Some(error.reason()),
            _ => None,
        })
}

#[test]
fn a_flat_spiral_from_the_palette_sized_by_its_outer_diameter_carries_a_pipe_once_thin_enough() {
    let mut harness = Harness::new();
    harness.select([Pickable::Axis(Axis::Z)]);

    run_from_palette(&mut harness, "flat spiral");
    harness.settle();

    let spiral = harness
        .workspace
        .editing
        .solid()
        .expect("the spiral is open");
    assert_eq!(harness.model.undo_label(), Some("Create Spiral 1"));
    assert_eq!(helix_of(&harness, spiral).kind, HelixKind::FlatSpiral);
    assert!(harness.shows(helix_panel::SPIRAL_DESCRIPTION));
    assert!(harness.shows(helix_panel::INNER_DIAMETER));
    assert!(!harness.shows(coil_panel::TAPER));

    choose(&mut harness, "Turns and pitch", "Outer diameter and turns");
    let resized = helix_of(&harness, spiral).length;
    assert!(harness.shows(helix_panel::OUTER_DIAMETER));
    assert_eq!(
        resized.height().map(|outer| length_value(&harness, outer)),
        Some(coil_tools::DEFAULT_DIAMETER + 2.0 * caditor_document::DEFAULT_PITCH * 5.0)
    );

    harness.type_into_field(Id::unique(("helix-field", spiral, "height")), "50");
    harness.settle();
    let spec = helix_tools::points(harness.model.evaluation(), spiral);
    let end = spec.last().copied().expect("the spiral is built");
    assert!(matches!(
        helix_of(&harness, spiral).length,
        CoilLength::HeightAndTurns { .. }
    ));
    assert!((end.truncate().length() - 25.0).abs() < 1e-9, "{end}");
    assert!(spec.iter().all(|point| point.z.abs() < 1e-9));

    harness.perform(Action::Editing(EditingCommand::CloseSolid));
    harness.select([Pickable::Datum(spiral)]);
    run_from_palette(&mut harness, "pipe");
    harness.settle();

    let pipe = harness.workspace.editing.solid().expect("the pipe is open");
    assert_eq!(
        harness
            .document()
            .feature(pipe)
            .and_then(|feature| feature.kind.pipe())
            .map(|pipe| pipe.path.clone()),
        Some(SweepPath::Helix { helix: spiral })
    );
    assert_eq!(pipe_tools::DEFAULT_SIZE, 10.0);
    let refused = reason_of(&harness, pipe).expect("a 10 mm pipe touches itself");
    assert!(
        refused.starts_with("The turns of Pipe 1 would touch along Spiral 1"),
        "{refused}"
    );
    assert!(harness.shows_containing("would touch along Spiral 1"));

    harness.type_into_field(Id::unique(("pipe-field", pipe, "size")), "1.5");
    harness.settle();

    assert_eq!(reason_of(&harness, pipe), None);
    assert!(harness.body_volume(pipe) > 0.0);
}

#[test]
fn turning_a_helix_into_a_flat_spiral_keeps_its_turns_and_pitch_in_one_undoable_change() {
    let mut harness = Harness::new();
    let helix = open_vertical_helix(&mut harness);
    harness.type_into_field(Id::unique(("helix-field", helix, "turns")), "3");
    harness.settle();

    choose(&mut harness, "Helix", "Flat spiral");
    let turned = helix_of(&harness, helix);
    let points = helix_tools::points(harness.model.evaluation(), helix);

    assert_eq!(turned.kind, HelixKind::FlatSpiral);
    assert_eq!(turned.length.turns(), Some(&Expression::number(3.0)));
    assert_eq!(harness.model.undo_label(), Some("Edit Helix 1"));
    assert!(harness.shows(helix_panel::SPIRAL_DESCRIPTION));
    assert!(points.iter().all(|point| point.z.abs() < 1e-9));
    let end = points.last().copied().expect("the spiral is built");
    assert!(
        (end.truncate().length() - (RADIUS + 3.0 * caditor_document::DEFAULT_PITCH)).abs() < 1e-9,
        "{end}"
    );
    let kind = |harness: &Harness| f64::from(u8::from(helix_of(harness, helix).is_flat()));
    undone_and_redone(&mut harness, "the shape", kind);
}
