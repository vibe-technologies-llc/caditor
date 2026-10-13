use std::f64::consts::{FRAC_PI_2, PI};

use caditor_document::{FeatureId, FeatureState, MissingInput, Pipe, SweepPath};
use caditor_geometry::{Plane, Point2};
use caditor_kernel::PipeShape;
use caditor_sketch::{EntityId, Sketch};
use egui::{Id, Key, Modifiers};

use super::{Harness, choose, run_from_palette};
use crate::{pipe_panel, reference_picking::Slot, selection::Pickable, solid_panel};

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
