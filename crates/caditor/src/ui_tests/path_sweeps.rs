use std::f64::consts::{FRAC_PI_2, PI};

use caditor_document::{FeatureId, FeatureState, SolidFeature, Sweep, SweepPath};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::SweepOrientation;
use caditor_sketch::{EntityId, Sketch};
use egui::{Key, Modifiers};

use super::Harness;
use crate::{reference_picking::Slot, selection::Pickable, solid_panel};

const RADIUS: f64 = 5.0;
const STRAIGHT: f64 = 50.0;
const BEND: f64 = 25.0;

fn upright() -> Plane {
    Plane::from_frame(Point3::ZERO, Vector3::Y, Vector3::X).unwrap()
}

fn add_disc(harness: &mut Harness) -> (FeatureId, EntityId) {
    let mut sketch = Sketch::new(upright());
    let circle = sketch.add_circle(Point2::ZERO, RADIUS);
    (harness.add_sketch(sketch), circle)
}

fn add_bent_path(harness: &mut Harness) -> (FeatureId, [EntityId; 2]) {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(0.0, STRAIGHT));
    let arc = sketch.add_arc(
        Point2::new(BEND, STRAIGHT),
        Point2::new(BEND, STRAIGHT + BEND),
        Point2::new(0.0, STRAIGHT),
    );
    (harness.add_sketch(sketch), [line, arc])
}

fn sweep_of(harness: &Harness, feature: FeatureId) -> Sweep {
    match harness.solid(feature) {
        SolidFeature::Sweep(sweep) => sweep.clone(),
        SolidFeature::Extrude(_) | SolidFeature::Revolve(_) => panic!("expected a sweep"),
    }
}

fn tube_volume() -> f64 {
    PI * RADIUS * RADIUS * (STRAIGHT + BEND * FRAC_PI_2)
}

fn near(harness: &Harness, body: FeatureId, expected: f64) -> bool {
    (harness.body_volume(body) - expected).abs() < 1e-2 * expected
}

fn failure_reason(harness: &Harness, feature: FeatureId) -> Option<String> {
    match harness
        .model
        .evaluation()
        .feature(feature)
        .map(|status| status.state.clone())
    {
        Some(FeatureState::Failed(error)) => Some(error.reason),
        _ => None,
    }
}

#[test]
fn sweep_carries_a_profile_along_the_selected_curves_of_another_sketch() {
    let mut harness = Harness::new();
    let (profile, circle) = add_disc(&mut harness);
    let (path, [line, arc]) = add_bent_path(&mut harness);
    harness.select([
        Pickable::SketchEntity {
            feature: profile,
            entity: circle,
        },
        Pickable::SketchEntity {
            feature: path,
            entity: line,
        },
        Pickable::SketchEntity {
            feature: path,
            entity: arc,
        },
    ]);

    harness.key(Key::W, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();

    let sweep = harness
        .workspace
        .editing
        .solid()
        .expect("the sweep is open");
    assert_eq!(harness.model.undo_label(), Some("Create Sweep 1"));
    let made = sweep_of(&harness, sweep);
    assert_eq!(made.sketch, profile);
    assert_eq!(
        made.path,
        SweepPath::Curves {
            sketch: path,
            curves: vec![line, arc],
        }
    );
    assert_eq!(made.orientation, SweepOrientation::Square);
    assert!(near(&harness, sweep, tube_volume()));
    assert!(harness.shows(solid_panel::PATH));
    assert!(harness.shows("2 curves of Plate 1"));
    assert!(harness.shows(solid_panel::SQUARE_TO_PATH));

    super::choose(
        &mut harness,
        solid_panel::SQUARE_TO_PATH,
        solid_panel::PARALLEL,
    );
    assert_eq!(
        sweep_of(&harness, sweep).orientation,
        SweepOrientation::Parallel
    );
    let reason = failure_reason(&harness, sweep).expect("a parallel sweep cannot turn flat");
    assert!(reason.contains("slide along itself"), "{reason}");
    assert!(harness.shows(&reason));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    assert_eq!(
        sweep_of(&harness, sweep).orientation,
        SweepOrientation::Square
    );
    assert!(near(&harness, sweep, tube_volume()));
}

#[test]
fn sweep_with_no_path_selected_starts_choosing_it_in_the_view() {
    let mut harness = Harness::new();
    let (profile, _) = add_disc(&mut harness);
    let (path, [line, arc]) = add_bent_path(&mut harness);
    harness.select([]);

    super::run_from_palette(&mut harness, "sweep");
    harness.settle();

    let sweep = harness
        .workspace
        .editing
        .solid()
        .expect("the sweep is open");
    assert_eq!(sweep_of(&harness, sweep).sketch, profile);
    assert_eq!(sweep_of(&harness, sweep).path, SweepPath::Unchosen);
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
            .feature(sweep)
            .map(|status| status.state.clone()),
        Some(FeatureState::Waiting(caditor_document::MissingInput::Path))
    );
    assert!(harness.shows("Waiting for a path"));
    assert!(harness.shows(crate::feature_fields::KEEPS_CHOOSING));

    harness.click_pickable(
        Plane::XY,
        Point2::new(0.0, 0.5 * STRAIGHT),
        Pickable::SketchEntity {
            feature: path,
            entity: line,
        },
    );
    harness.settle();
    assert_eq!(
        sweep_of(&harness, sweep).path,
        SweepPath::Curves {
            sketch: path,
            curves: vec![line],
        }
    );
    assert!(near(&harness, sweep, PI * RADIUS * RADIUS * STRAIGHT));
    assert_eq!(
        harness
            .workspace
            .editing
            .picking()
            .map(|picking| picking.slot),
        Some(Slot::SweepPath)
    );

    let near_the_line = 160.0_f64.to_radians();
    let on_arc = Point2::new(
        BEND + BEND * near_the_line.cos(),
        STRAIGHT + BEND * near_the_line.sin(),
    );
    harness.click_pickable(
        Plane::XY,
        on_arc,
        Pickable::SketchEntity {
            feature: path,
            entity: arc,
        },
    );
    harness.settle();
    assert_eq!(sweep_of(&harness, sweep).path.len(), 2);
    assert!(near(&harness, sweep, tube_volume()));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.workspace.editing.picking(), None);
}

#[test]
fn a_profile_too_large_for_the_bend_fails_in_words_naming_the_curve() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(upright());
    let circle = sketch.add_circle(Point2::ZERO, 2.0 * BEND);
    let profile = harness.add_sketch(sketch);
    let (path, [line, arc]) = add_bent_path(&mut harness);
    harness.select([
        Pickable::SketchEntity {
            feature: profile,
            entity: circle,
        },
        Pickable::SketchEntity {
            feature: path,
            entity: line,
        },
        Pickable::SketchEntity {
            feature: path,
            entity: arc,
        },
    ]);

    super::run_from_palette(&mut harness, "sweep");
    harness.settle();

    let sweep = harness
        .workspace
        .editing
        .solid()
        .expect("the sweep is open");
    let reason = failure_reason(&harness, sweep).expect("the profile is too large");
    assert!(reason.contains("too large for the bend of"), "{reason}");
    assert!(harness.shows(&reason));
}

#[test]
fn typing_a_twist_and_an_end_scale_shapes_the_sweep_in_one_undoable_change_each() {
    let mut harness = Harness::new();
    let mut square = Sketch::new(upright());
    let corners = [
        Point2::new(-RADIUS, -RADIUS),
        Point2::new(RADIUS, -RADIUS),
        Point2::new(RADIUS, RADIUS),
        Point2::new(-RADIUS, RADIUS),
    ];
    let sides: Vec<EntityId> = (0..corners.len())
        .map(|index| square.add_line(corners[index], corners[(index + 1) % corners.len()]))
        .collect();
    let profile = harness.add_sketch(square);
    let mut straight = Sketch::new(Plane::XY);
    let line = straight.add_line(Point2::ZERO, Point2::new(0.0, STRAIGHT));
    let path = harness.add_sketch(straight);
    harness.select([
        Pickable::SketchEntity {
            feature: profile,
            entity: sides[0],
        },
        Pickable::SketchEntity {
            feature: path,
            entity: line,
        },
    ]);
    harness.key(Key::W, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    let sweep = harness
        .workspace
        .editing
        .solid()
        .expect("the sweep is open");
    let prism = 4.0 * RADIUS * RADIUS * STRAIGHT;
    let value = |harness: &Harness, expression: &caditor_expression::Expression| {
        harness
            .model
            .parameters()
            .evaluate_expression(expression)
            .unwrap()
            .value
    };

    harness.type_into_field(egui::Id::new(("solid-field", "twist", sweep)), "90");
    harness.settle();

    assert!(harness.shows(solid_panel::TWIST));
    assert!(harness.shows(solid_panel::END_SCALE));
    assert_eq!(harness.model.undo_label(), Some("Edit Sweep 1"));
    assert_eq!(value(&harness, &sweep_of(&harness, sweep).twist), 90.0);
    assert!(near(&harness, sweep, prism));

    harness.type_into_field(egui::Id::new(("solid-field", "scale", sweep)), "0.5");
    harness.settle();

    assert_eq!(value(&harness, &sweep_of(&harness, sweep).scale), 0.5);
    assert!(near(&harness, sweep, prism * 7.0 / 12.0));
    super::pipes::undone_and_redone(&mut harness, "the end scale", |harness| {
        value(harness, &sweep_of(harness, sweep).scale)
    });
    assert!(near(&harness, sweep, prism * 7.0 / 12.0));
}
