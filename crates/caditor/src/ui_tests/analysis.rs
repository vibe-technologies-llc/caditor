use std::time::Instant;

use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Key, Modifiers};

use super::{Harness, RECOMPUTE_TIMEOUT, extruded_plate, painted_faces, run_from_palette};
use crate::{commands::Command, selection::Pickable};

#[test]
fn draft_analysis_bands_the_faces_of_a_plate_and_leaves_the_model_alone() {
    use crate::{analysis::Band, scene_palette::Contrast};

    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let revision = harness.model.revision();
    let palette = Contrast::Standard.palette(crate::scene_palette::Canvas::Dark);
    let on_screen =
        |harness: &mut Harness, band: Band| painted_faces(harness, band.colour(palette));

    run_from_palette(&mut harness, "analyse draft");
    harness.frame();
    harness.frame();

    assert!(harness.shows(crate::analysis_panel::TITLE));
    assert!(harness.shows_containing("Too little draft"));
    assert_eq!(on_screen(&mut harness, Band::Drafted), 1);
    assert_eq!(on_screen(&mut harness, Band::Undercut), 1);
    assert_eq!(on_screen(&mut harness, Band::TooLittleDraft), 4);
    assert!(harness.workspace.analysis.open);
    assert_eq!(harness.model.revision(), revision);

    run_from_palette(&mut harness, "analyse minimum radius");
    harness.frame();
    harness.frame();

    assert!(harness.shows_containing("Too tight to reach"));
    assert_eq!(on_screen(&mut harness, Band::Drafted), 0);
    assert_eq!(on_screen(&mut harness, Band::TooTight), 0);

    run_from_palette(&mut harness, "analyse minimum radius");
    harness.frame();
    harness.frame();

    assert!(!harness.workspace.analysis.open);
    assert_eq!(on_screen(&mut harness, Band::TooLittleDraft), 0);
    assert_eq!(harness.model.revision(), revision);
}

#[test]
fn reach_analysis_colours_what_a_tool_from_above_gets_to_and_reverses_to_the_other_side() {
    use crate::{analysis::Band, scene_palette::Contrast};

    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let revision = harness.model.revision();
    let palette = Contrast::Standard.palette(crate::scene_palette::Canvas::Dark);
    let on_screen =
        |harness: &mut Harness, band: Band| painted_faces(harness, band.colour(palette));

    run_from_palette(&mut harness, "analyse tool reach");
    harness.frame();
    harness.frame();

    assert!(harness.shows(crate::analysis_panel::TITLE));
    assert!(harness.shows("Reach from"));
    assert!(harness.shows_containing("Hidden behind other faces"));
    assert_eq!(on_screen(&mut harness, Band::Reachable), 5);
    assert_eq!(on_screen(&mut harness, Band::FacesAway), 1);
    assert_eq!(on_screen(&mut harness, Band::Blocked), 0);

    run_from_palette(&mut harness, "reverse the pull or reach direction");
    harness.frame();
    harness.frame();

    assert!(harness.workspace.analysis.reversed);
    assert_eq!(on_screen(&mut harness, Band::Reachable), 5);
    assert_eq!(on_screen(&mut harness, Band::FacesAway), 1);
    assert_eq!(harness.model.revision(), revision);
}

#[test]
fn curvature_zebra_and_chrome_show_on_the_bodies_and_leave_the_model_alone() {
    use caditor_render::Reflection;

    use crate::{analysis::Band, scene_palette::Contrast};

    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let revision = harness.model.revision();
    let palette = Contrast::Standard.palette(crate::scene_palette::Canvas::Dark);

    run_from_palette(&mut harness, "analyse curvature");
    harness.frame();
    harness.frame();

    assert!(harness.shows("Reference radius"));
    assert!(harness.shows_containing("Flat or bent one way"));
    assert_eq!(
        painted_faces(&mut harness, Band::Developable.colour(palette)),
        6
    );

    run_from_palette(&mut harness, "show zebra stripes");
    harness.frame();
    harness.frame();
    let striped = harness.built().scene;

    assert!(harness.shows(crate::analysis_panel::STRIPES));
    assert_eq!(striped.reflective_meshes.len(), 1);
    assert!(striped.meshes.is_empty());
    assert!(matches!(
        striped.reflection,
        Reflection::Zebra {
            stripes: crate::analysis::DEFAULT_STRIPES,
            ..
        }
    ));

    run_from_palette(&mut harness, "show a chrome reflection");
    harness.frame();
    harness.frame();
    let chrome = harness.built().scene;

    assert_eq!(chrome.reflective_meshes.len(), 1);
    assert_eq!(chrome.reflection, Reflection::Chrome);

    run_from_palette(&mut harness, "show a chrome reflection");
    harness.frame();
    harness.frame();
    let plain = harness.built().scene;

    assert!(!harness.workspace.analysis.open);
    assert!(plain.reflective_meshes.is_empty());
    assert_eq!(plain.meshes.len(), 1);
    assert_eq!(harness.model.revision(), revision);
}

#[test]
fn the_pull_direction_follows_the_selected_face_and_reverses_from_the_palette() {
    use crate::{
        analysis::{AnalysisCommand, Pull, Refusal},
        commands::Offer,
    };

    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([]);
    let use_selected = Command::Analysis(AnalysisCommand::UseSelected);
    let offered = |harness: &Harness| -> Option<Offer> {
        harness
            .workspace
            .last_offers
            .iter()
            .find(|offer| offer.command == use_selected)
            .cloned()
    };

    run_from_palette(&mut harness, "analyse draft");
    harness.frame();
    harness.frame();

    assert_eq!(
        offered(&harness).map(|offer| offer.availability),
        Some(Err(Refusal::NothingGivesADirection.to_string().into()))
    );

    harness.select([top]);
    harness.frame();
    run_from_palette(&mut harness, "pull or reach along the selected");
    harness.frame();
    harness.frame();
    run_from_palette(&mut harness, "reverse the pull or reach direction");
    harness.frame();
    harness.frame();

    assert_eq!(harness.workspace.analysis.pull, Pull::Picked(top));
    assert!(harness.workspace.analysis.reversed);
    assert!(harness.shows_containing(", reversed"));
}

fn comb_lines(harness: &mut Harness, colour: caditor_render::Color) -> usize {
    harness
        .built()
        .scene
        .lines()
        .filter(|line| line.color == colour)
        .count()
}

#[test]
fn the_curvature_comb_combs_the_chosen_sketch_curves_and_names_the_jump_where_they_meet() {
    use crate::{comb::DEFAULT_TEETH, comb_panel, scene_palette::Contrast};

    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0));
    let arc = sketch.add_arc(
        Point2::new(10.0, 5.0),
        Point2::new(10.0, 0.0),
        Point2::new(15.0, 5.0),
    );
    let feature = harness.add_sketch(sketch);
    let revision = harness.model.revision();
    let look = Contrast::Standard
        .palette(crate::scene_palette::Canvas::Dark)
        .comb;

    harness.select([line, arc].map(|entity| Pickable::SketchEntity { feature, entity }));
    run_from_palette(&mut harness, "curvature comb");
    harness.frame();
    harness.frame();

    assert!(harness.workspace.comb.open);
    assert!(harness.shows(comb_panel::TITLE));
    assert!(harness.shows_containing("Tangent, but the curvature jumps (G1)"));
    assert!(harness.shows_containing("smallest radius ≈ 5"));
    assert_eq!(comb_lines(&mut harness, look.teeth), DEFAULT_TEETH + 1);
    assert_eq!(
        comb_lines(&mut harness, look.envelope),
        2 * DEFAULT_TEETH + 1
    );

    harness.select([]);
    harness.frame();

    assert_eq!(comb_lines(&mut harness, look.teeth), DEFAULT_TEETH + 1);

    run_from_palette(&mut harness, "curvature comb");
    harness.frame();
    harness.frame();

    assert!(!harness.workspace.comb.open);
    assert_eq!(comb_lines(&mut harness, look.teeth), 0);
    assert_eq!(comb_lines(&mut harness, look.envelope), 0);
    assert_eq!(harness.model.revision(), revision);
}

#[test]
fn the_curvature_comb_follows_a_selected_body_edge_and_reads_it_as_straight() {
    use crate::{comb_panel, scene_palette::Contrast};

    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let edge = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| matches!(pickable, Pickable::Edge { .. }))
        .expect("an edge is pickable");
    let look = Contrast::Standard
        .palette(crate::scene_palette::Canvas::Dark)
        .comb;

    run_from_palette(&mut harness, "curvature comb");
    harness.frame();

    assert!(harness.shows_containing(comb_panel::NOTHING_CHOSEN));

    harness.select([edge]);
    harness.frame();

    assert_eq!(harness.workspace.comb.curves(), [edge]);
    assert!(harness.shows_containing(": straight"));
    assert_eq!(comb_lines(&mut harness, look.teeth), 0);
    assert!(comb_lines(&mut harness, look.envelope) > 0);
}

fn worked_out_isocurves(harness: &mut Harness) {
    let deadline = Instant::now() + RECOMPUTE_TIMEOUT;
    harness.frame();
    while harness.workspace.isocurves.is_working() {
        assert!(
            Instant::now() < deadline,
            "the isocurves were not worked out"
        );
        std::thread::yield_now();
        harness.frame();
    }
    harness.frame();
}

#[test]
fn isocurves_draw_the_parameter_lines_of_the_chosen_faces_with_their_combs() {
    use crate::{comb::DEFAULT_SCALE, isocurve_panel, isocurves, scene_palette::Contrast};

    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_circle(Point2::new(0.0, 0.0), 10.0);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let faces: Vec<Pickable> = harness
        .built()
        .picks
        .pickables()
        .filter(|pickable| matches!(pickable, Pickable::Face { .. }))
        .collect();
    let revision = harness.model.revision();
    let look = Contrast::Standard
        .palette(crate::scene_palette::Canvas::Dark)
        .comb;

    run_from_palette(&mut harness, "isocurves");
    harness.frame();
    let nothing_chosen = harness.shows_containing(isocurve_panel::NOTHING_CHOSEN);
    harness.select(faces.clone());
    worked_out_isocurves(&mut harness);
    let lines = comb_lines(&mut harness, look.isocurve);
    let teeth = comb_lines(&mut harness, look.teeth);
    let circles_combed = harness.shows_containing("U lines bend to a smallest radius of ≈ 10");
    let rulings_straight = harness.shows_containing("V lines straight");

    harness.workspace.isocurves.directions = isocurves::Directions::V;
    worked_out_isocurves(&mut harness);
    let only_rulings_teeth = comb_lines(&mut harness, look.teeth);
    let only_rulings_lines = comb_lines(&mut harness, look.isocurve);

    harness.workspace.isocurves.scale = DEFAULT_SCALE * 2.0;
    harness.select([]);
    harness.frame();
    let kept = comb_lines(&mut harness, look.isocurve);

    run_from_palette(&mut harness, "isocurves");
    harness.frame();
    harness.frame();

    assert!(nothing_chosen);
    assert_eq!(harness.workspace.isocurves.faces().len(), 0);
    assert_eq!(faces.len(), 3);
    assert!(lines > 0);
    assert_eq!(
        teeth,
        isocurves::DEFAULT_LINES * (isocurves::DEFAULT_TEETH + 1)
    );
    assert!(circles_combed);
    assert!(rulings_straight);
    assert_eq!(only_rulings_teeth, 0);
    assert!(only_rulings_lines > 0 && only_rulings_lines < lines);
    assert_eq!(kept, only_rulings_lines);
    assert!(!harness.workspace.isocurves.open);
    assert_eq!(comb_lines(&mut harness, look.isocurve), 0);
    assert_eq!(harness.model.revision(), revision);
}

#[test]
fn analysing_a_large_body_can_be_cancelled_leaving_it_plain_until_analysed_again() {
    use crate::{analysis::Band, analysis_panel, bodies::Hold, scene_palette::Contrast};

    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let hold = Hold::holding();
    harness
        .workspace
        .viewport
        .analyses_mut()
        .hold_on_a_thread(hold.clone());
    let palette = Contrast::Standard.palette(crate::scene_palette::Canvas::Dark);
    let revision = harness.model.revision();

    run_from_palette(&mut harness, "analyse draft");
    harness.wait_until("the faces are being analysed", |harness| {
        harness.shows(analysis_panel::WORKING)
    });
    harness.click("Cancel");
    harness.wait_until("the analysis stopped", |harness| {
        harness.shows(analysis_panel::ANALYSE_AGAIN)
    });

    assert!(harness.shows_containing("Stopped before every body was analysed."));
    assert!(!harness.shows(analysis_panel::WORKING));
    assert!(harness.workspace.analysis.open);
    assert_eq!(
        painted_faces(&mut harness, Band::Drafted.colour(palette)),
        0
    );

    hold.release();
    harness.click(analysis_panel::ANALYSE_AGAIN);
    harness.wait_until("the faces are analysed", |harness| {
        !harness.shows(analysis_panel::WORKING) && !harness.shows(analysis_panel::ANALYSE_AGAIN)
    });

    assert_eq!(
        painted_faces(&mut harness, Band::Drafted.colour(palette)),
        1
    );
    assert_eq!(
        painted_faces(&mut harness, Band::TooLittleDraft.colour(palette)),
        4
    );
    assert_eq!(harness.model.revision(), revision);
}
