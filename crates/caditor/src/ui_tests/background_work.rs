use caditor_geometry::Point2;

use super::{Harness, add_peg, block_into, extruded_plate, offer, painted_faces, run_from_palette};
use crate::{
    analysis::Band,
    analysis_panel,
    bodies::{Hold, Holds},
    commands::Command,
    interference_panel, isocurve_panel, measure_panel,
    model::Action,
    scene_palette::{Canvas, Contrast},
    status_bar, toolbar,
};

#[test]
fn face_analysis_is_cancelled_and_asked_for_again_from_the_palette() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let hold = Hold::holding();
    harness
        .workspace
        .viewport
        .analyses_mut()
        .hold_on_a_thread(hold.clone());
    let palette = Contrast::Standard.palette(Canvas::Dark);
    let revision = harness.model.revision();

    run_from_palette(&mut harness, "analyse draft");
    harness.wait_until("the faces are being analysed", |harness| {
        harness.shows(analysis_panel::WORKING)
    });
    let cancelling = offer(&harness, Command::CancelAnalysis).availability;
    let analysing_again = offer(&harness, Command::AnalyseAgain).availability;

    run_from_palette(&mut harness, "cancel the face analysis");
    harness.wait_until("the analysis stopped", |harness| {
        harness.shows(analysis_panel::ANALYSE_AGAIN)
    });
    let resuming = offer(&harness, Command::AnalyseAgain).availability;
    let cancelled = offer(&harness, Command::CancelAnalysis).availability;

    assert!(cancelling.is_ok());
    assert!(analysing_again.is_err());
    assert!(resuming.is_ok());
    assert!(cancelled.is_err());
    assert!(!harness.shows(analysis_panel::WORKING));
    assert_eq!(
        painted_faces(&mut harness, Band::Drafted.colour(palette)),
        0
    );

    hold.release();
    run_from_palette(&mut harness, "analyse faces again");
    harness.wait_until("the faces are analysed", |harness| {
        !harness.shows(analysis_panel::WORKING) && !harness.shows(analysis_panel::ANALYSE_AGAIN)
    });

    assert_eq!(
        painted_faces(&mut harness, Band::Drafted.colour(palette)),
        1
    );
    assert_eq!(harness.model.revision(), revision);
}

#[test]
fn mass_properties_are_cancelled_and_worked_out_again_from_the_palette() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let holds = Holds {
        measures: Hold::holding(),
        ..Holds::default()
    };
    harness.model.hold_body_work(&holds);
    harness.select([]);

    harness.click(toolbar::MEASURE_LABEL);
    harness.wait_until("the mass properties are being worked out", |harness| {
        harness.shows(measure_panel::WORKING_OUT_MASSES)
    });
    let cancelling = offer(&harness, Command::CancelMassProperties).availability;
    let working_out = offer(&harness, Command::WorkOutMassProperties).availability;

    run_from_palette(&mut harness, "cancel the mass properties");
    harness.wait_until("the mass properties stopped", |harness| {
        harness.shows(&measure_panel::stopped_masses_text(1)) && !harness.model.masses_pending()
    });
    let resuming = offer(&harness, Command::WorkOutMassProperties).availability;

    assert!(cancelling.is_ok());
    assert!(working_out.is_err());
    assert!(resuming.is_ok());
    assert!(!harness.shows(measure_panel::WORKING_OUT_MASSES));

    holds.measures.release();
    run_from_palette(&mut harness, "work out the stopped mass properties");
    harness.wait_until("the mass properties are shown", |harness| {
        harness.shows("16000.0 mm³")
    });

    assert!(!harness.shows(&measure_panel::stopped_masses_text(1)));
}

#[test]
fn preparing_bodies_is_cancelled_and_resumed_from_the_palette() {
    let mut harness = Harness::new();
    let holds = Holds {
        converts: Hold::holding(),
        ..Holds::default()
    };
    harness.model.hold_body_work(&holds);
    let mut transaction = harness.document().transaction("Add two blocks");
    block_into(
        &mut transaction,
        "Left",
        [Point2::new(0.0, 0.0), Point2::new(10.0, 10.0)],
        "5 mm",
    );
    block_into(
        &mut transaction,
        "Right",
        [Point2::new(20.0, 0.0), Point2::new(30.0, 10.0)],
        "5 mm",
    );
    harness.perform(Action::Apply(transaction.finish()));
    let preparing = format!("{}…", status_bar::showing_bodies_text(0, 2));
    harness.wait_until("the bodies are being prepared", |harness| {
        harness.shows(&preparing)
    });
    let cancelling = offer(&harness, Command::CancelPreparingBodies).availability;
    let showing_the_rest = offer(&harness, Command::ShowRestOfBodies).availability;

    run_from_palette(&mut harness, "cancel preparing bodies for display");
    harness.frame();
    harness.frame();
    let resuming = offer(&harness, Command::ShowRestOfBodies).availability;

    assert!(cancelling.is_ok());
    assert!(showing_the_rest.is_err());
    assert!(resuming.is_ok());
    assert!(!harness.model.bodies_pending());
    assert_eq!(harness.model.bodies_stopped(), Some((0, 2)));
    assert_eq!(harness.workspace.viewport.bodies().iter().count(), 0);

    holds.converts.release();
    run_from_palette(&mut harness, "show the rest of the bodies");
    harness.settle();

    assert_eq!(harness.model.bodies_stopped(), None);
    assert!(!harness.shows(status_bar::SHOW_THE_REST));
    assert_eq!(harness.workspace.viewport.bodies().iter().count(), 2);
}

#[test]
fn isocurves_are_cancelled_from_the_palette() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    let hold = Hold::holding();
    harness.workspace.isocurves.hold_on_a_thread(hold.clone());

    run_from_palette(&mut harness, "show or hide isocurves");
    harness.frame();
    harness.select([top]);
    harness.wait_until("the isocurves are being worked out", |harness| {
        harness.shows(isocurve_panel::WORKING)
    });
    let cancelling = offer(&harness, Command::CancelIsocurves).availability;

    run_from_palette(&mut harness, "cancel the isocurves");
    harness.wait_until("the isocurves stopped", |harness| {
        harness.shows(isocurve_panel::STOPPED)
    });
    let cancelled = offer(&harness, Command::CancelIsocurves).availability;

    assert!(cancelling.is_ok());
    assert!(cancelled.is_err());
    assert!(!harness.shows(isocurve_panel::WORKING));
    assert!(!harness.workspace.isocurves.is_working());
    assert!(harness.workspace.isocurves.is_stopped());
}

#[test]
fn the_interference_check_is_cancelled_and_made_again_from_the_palette() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    add_peg(&mut harness);
    let hold = Hold::holding();
    harness
        .workspace
        .interference
        .interference
        .hold_on_a_thread(hold.clone());
    harness.select([]);

    harness.click(toolbar::INTERFERENCE_LABEL);
    harness.wait_until("the pair is being checked", |harness| {
        harness.shows(interference_panel::CHECKING)
    });
    let cancelling = offer(&harness, Command::CancelInterference).availability;
    let checking_again = offer(&harness, Command::CheckInterferenceAgain).availability;

    run_from_palette(&mut harness, "cancel the interference check");
    harness.wait_until("the check stopped", |harness| {
        harness.shows(interference_panel::CHECK_AGAIN)
    });
    let resuming = offer(&harness, Command::CheckInterferenceAgain).availability;

    assert!(cancelling.is_ok());
    assert!(checking_again.is_err());
    assert!(resuming.is_ok());
    assert!(!harness.shows(interference_panel::CHECKING));

    hold.release();
    run_from_palette(&mut harness, "check interference again");
    harness.wait_until("the pair is checked", |harness| {
        harness.shows("1 pair overlaps.")
    });

    assert!(!harness.shows(interference_panel::CHECK_AGAIN));
}
