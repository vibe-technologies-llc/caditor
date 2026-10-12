use std::time::{Duration, Instant};

use caditor_geometry::{Plane, Point2};
use caditor_sketch::{EntityId, Sketch};

use super::{FILE_TIMEOUT, Harness, add_peg, extruded_plate, feature_named, top_edge_along_x};
use crate::{editing::EditingCommand, model::Action, selection::Pickable, units::LengthUnit};

#[test]
fn the_constraint_tools_work_out_their_candidates_when_the_selection_or_sketch_changes_not_every_frame()
 {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0));
    let second = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(10.0, 6.0));
    let id = harness.add_sketch(sketch);
    harness.settle();
    harness.perform(Action::Editing(EditingCommand::Enter(id)));
    let pick = |entity| Pickable::SketchEntity {
        feature: id,
        entity,
    };
    harness.select([pick(first), pick(second)]);
    harness.frame();
    let computed = harness.workspace.panels.constraint_offers.computations();
    assert!(computed > 0);

    for _ in 0..5 {
        harness.frame();
    }
    assert_eq!(
        harness.workspace.panels.constraint_offers.computations(),
        computed
    );

    harness.select([pick(first)]);
    harness.frame();
    assert_eq!(
        harness.workspace.panels.constraint_offers.computations(),
        computed + 1
    );
}

#[test]
fn what_the_selection_offers_is_worked_out_when_it_or_the_model_changes_not_every_frame() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    let measured = harness.model.masses_measured();
    harness.select([top]);
    harness.wait_until("the face's exact area arrives", |harness| {
        harness.model.masses_measured() > measured
    });
    let computed = harness.workspace.selection_offers.computations();

    for _ in 0..5 {
        harness.frame();
    }
    assert_eq!(harness.workspace.selection_offers.computations(), computed);
    assert!(harness.shows("Extrude 1 › Extrude 1 end face"));
    harness.hover("Shell");
    assert!(harness.shows_containing(&format!(
        "{} (1 face open)",
        crate::shell_tools::DESCRIPTION
    )));

    harness.select([]);
    harness.frame();
    assert_eq!(
        harness.workspace.selection_offers.computations(),
        computed + 1
    );
    assert!(harness.shows("Nothing selected"));

    let before = harness.workspace.selection_offers.computations();
    harness.perform(Action::Undo);
    harness.settle();
    assert!(harness.workspace.selection_offers.computations() > before);
}

#[test]
fn a_changed_angle_or_length_unit_works_the_selection_size_out_again() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_arc(
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 0.0),
        Point2::new(0.0, 10.0),
    );
    let feature = harness.add_sketch(sketch);
    harness.frame();
    harness.select([Pickable::SketchEntity {
        feature,
        entity: arc,
    }]);
    let size = |harness: &mut Harness| {
        harness
            .workspace
            .selection_offers
            .refresh(
                &harness.model,
                harness.workspace.viewport.selection(),
                &harness.workspace.editing,
                (&[], &[]),
            )
            .size
            .clone()
            .unwrap_or_default()
    };

    let degrees = size(&mut harness);
    harness
        .model
        .set_angle_unit(crate::units::AngleUnit::Radian);
    let radians = size(&mut harness);
    harness.model.set_length_unit(LengthUnit::Centimetre);
    let centimetres = size(&mut harness);

    assert!(degrees.contains("Sweep 90.0°"), "{degrees}");
    assert!(radians.contains("Sweep 1.571 rad"), "{radians}");
    assert!(radians.contains("R 10.000 mm"), "{radians}");
    assert!(centimetres.contains("R 1.0000 cm"), "{centimetres}");
    assert!(centimetres.contains("Sweep 1.571 rad"), "{centimetres}");
}

#[test]
fn a_large_selection_is_counted_whole_but_described_and_measured_only_in_part() {
    let mut harness = Harness::new();
    let mut lines = Sketch::new(Plane::XY);
    let entities: Vec<EntityId> = (0..20)
        .map(|index| {
            let y = f64::from(index) * 5.0;
            lines.add_line(Point2::new(0.0, y), Point2::new(10.0, y))
        })
        .collect();
    let feature = harness.add_sketch(lines);
    harness.frame();

    harness.select(entities.iter().map(|entity| Pickable::SketchEntity {
        feature,
        entity: *entity,
    }));
    harness.frame();
    let offers = harness
        .workspace
        .selection_offers
        .refresh(
            &harness.model,
            harness.workspace.viewport.selection(),
            &harness.workspace.editing,
            (&[], &[]),
        )
        .clone();

    assert_eq!(offers.selected, 20);
    assert_eq!(offers.described.len(), crate::offers::MAX_DESCRIBED);
    assert!(harness.shows("20 items selected"));
}

#[test]
fn a_use_selected_offer_is_worked_out_once_per_selection_and_model_change() {
    let harness = Harness::new();
    let ctx = egui::Context::default();
    let feature = feature_named(&harness, "Base sketch");
    let reference = (feature, crate::reference_picking::Slot::MirrorPlane);
    let mut computed = 0;
    let mut offer = |selection: &crate::selection::Selection| {
        crate::feature_fields::offered_change(&ctx, &harness.model, selection, reference, || {
            computed += 1;
            Err("nothing usable".to_owned())
        })
    };
    let mut selection = crate::selection::Selection::default();
    selection.replace_with(Pickable::Origin);

    let first = offer(&selection);
    let again = offer(&selection);
    selection.clear();
    let cleared = offer(&selection);

    assert_eq!(first, Err("nothing usable".to_owned()));
    assert_eq!(again, first);
    assert_eq!(cleared, first);
    assert_eq!(computed, 2);
}

#[test]
fn the_interference_report_is_rebuilt_only_when_its_inputs_or_findings_change() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    add_peg(&mut harness);
    let selection = crate::selection::Selection::default();
    let mut interference = crate::interference::Interference::default();

    let mut report = interference
        .refresh(&harness.model, &selection, &[])
        .expect("the first refresh reports");
    let deadline = Instant::now() + FILE_TIMEOUT;
    while report.is_checking() {
        assert!(Instant::now() < deadline, "the check never finished");
        std::thread::sleep(Duration::from_millis(2));
        if let Some(newer) = interference.refresh(&harness.model, &selection, &[]) {
            report = newer;
        }
    }

    assert_eq!(report.checked(), 1);
    assert!(
        interference
            .refresh(&harness.model, &selection, &[])
            .is_none()
    );
    assert!(
        interference
            .refresh(&harness.model, &selection, &[])
            .is_none()
    );
}

#[test]
fn the_status_bar_gives_the_size_of_the_selected_face_edge_or_body() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);

    harness.select([]);
    harness.settle();
    assert!(!harness.shows_containing("Area"));
    assert!(!harness.shows_containing("Length"));

    harness.select([top]);
    harness.settle();
    assert!(harness.shows_containing("Area 1600"));
    assert!(harness.shows_containing("40.000 mm × 40.000 mm"));

    let edge = top_edge_along_x(&harness, plate, 0.0);
    harness.select([Pickable::Edge { body: plate, edge }]);
    harness.settle();
    assert!(harness.shows_containing("Length 40"));
    assert!(!harness.shows_containing("Area"));

    harness.select([]);
    harness.workspace.panels.choose_only(plate);
    harness.settle();
    assert!(harness.shows_containing("Size 40.000 × 40.000 × 10.000"));

    let mut round = Sketch::new(Plane::XY);
    let circle = round.add_circle(Point2::new(80.0, 0.0), 5.0);
    let sketch = harness.add_sketch(round);
    harness.select([Pickable::SketchEntity {
        feature: sketch,
        entity: circle,
    }]);
    harness.settle();
    assert!(harness.shows_containing("Ø 10.000 mm"));
    assert!(harness.shows_containing("R 5.000 mm"));
    assert!(harness.shows_containing("Circumference 31.416 mm"));
}
