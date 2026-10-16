use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, extruded_plate, offer, open_combo, pattern_of, run_from_palette,
    volume_about,
};
use crate::{
    commands::Command,
    model::Action,
    selection::{Axis, Pickable},
};

#[test]
fn a_linear_pattern_repeats_the_body_and_takes_its_count_and_directions_from_the_panel() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);

    harness.select([]);
    harness.hover("Linear pattern");
    assert!(harness.shows_containing(
        "Repeat the body of Extrude 1 along the X axis, or along an edge or axis you select first"
    ));
    harness.click("Linear pattern");
    harness.settle();
    let pattern = harness
        .workspace
        .editing
        .solid()
        .expect("the pattern is open");
    assert_eq!(harness.model.undo_label(), Some("Create Linear pattern 1"));
    assert_eq!(pattern_of(&harness, pattern).body, plate);
    assert!(volume_about(&harness, plate, 3.0 * 16000.0));
    assert!(harness.shows("Direction"));
    assert!(harness.shows("Spacing"));

    harness.type_into_field(Id::unique(("pattern-field", "count", pattern)), "4");
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Edit Linear pattern 1"));
    assert!(volume_about(&harness, plate, 4.0 * 16000.0));

    assert_eq!(
        offer(&harness, Command::PatternSecondUseSelected).availability,
        Err("Select an axis, straight edge or round face made before this pattern".into())
    );
    harness.select([Pickable::Axis(Axis::Y)]);
    run_from_palette(&mut harness, "pattern also along selected");
    harness.settle();
    let caditor_document::PatternKind::Linear { second, .. } = &pattern_of(&harness, pattern).kind
    else {
        panic!("the pattern stays linear");
    };
    assert!(second.is_some());
    assert!(volume_about(&harness, plate, 8.0 * 16000.0));

    harness.perform(Action::Undo);
    harness.settle();
    assert!(volume_about(&harness, plate, 4.0 * 16000.0));
    harness.perform(Action::Redo);
    harness.settle();
    assert!(volume_about(&harness, plate, 8.0 * 16000.0));

    harness.type_into_field(Id::unique(("pattern-field", "count", pattern)), "2.5");
    assert!(harness.shows("Enter a whole number of at least 1"));
    assert!(volume_about(&harness, plate, 8.0 * 16000.0));
}

#[test]
fn a_hole_chosen_in_the_tree_is_repeated_by_a_pattern_instead_of_its_body() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let top = Plane::from_frame(
        caditor_geometry::Point3::new(0.0, 0.0, 10.0),
        caditor_geometry::Vector3::Z,
        caditor_geometry::Vector3::X,
    )
    .unwrap();
    let mut sketch = Sketch::new(top);
    sketch.add_point(Point2::new(8.0, 20.0));
    harness.add_sketch(sketch);
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
    harness.hover("Linear pattern");
    let described = harness.shows_containing("Repeat Hole 1 along the X axis");
    harness.click("Linear pattern");
    harness.settle();
    let pattern = harness
        .workspace
        .editing
        .solid()
        .expect("the pattern is open");

    assert!(described);
    let created = pattern_of(&harness, pattern).clone();
    assert_eq!(created.body, plate);
    assert_eq!(created.repeated, vec![hole]);
    assert!(volume_about(&harness, plate, 16000.0 - 3.0 * drilled));
    assert!(harness.shows(crate::pattern_panel::REPEATS));
    assert!(harness.shows("Hole 1"));

    harness.click_button("Stop repeating Hole 1");
    harness.settle();
    assert!(pattern_of(&harness, pattern).repeated.is_empty());
    assert!(harness.shows(crate::pattern_panel::WHOLE_BODY));
    assert!(harness.body_volume(plate) > 16000.0);
}

#[test]
fn a_chamfer_chosen_with_its_hole_is_repeated_on_every_copy_but_alone_repeats_the_body() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let top = Plane::from_frame(
        caditor_geometry::Point3::new(0.0, 0.0, 10.0),
        caditor_geometry::Vector3::Z,
        caditor_geometry::Vector3::X,
    )
    .unwrap();
    let mut sketch = Sketch::new(top);
    sketch.add_point(Point2::new(8.0, 20.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let wall = {
        let solid = harness.model.evaluation().body(plate).expect("the plate");
        let face = solid
            .faces()
            .find(|(_, face)| {
                face.origin()
                    .is_some_and(|origin| origin.feature() == hole.raw())
            })
            .map(|(id, _)| id)
            .expect("the hole's wall");
        caditor_kernel::FaceReference::capture(solid, face).expect("the wall's reference")
    };
    let mut transaction = harness.document().transaction("Chamfer");
    let chamfer = transaction.add_feature(
        "Rim",
        caditor_document::FeatureKind::Blend(caditor_document::Blend {
            kind: caditor_document::BlendKind::Chamfer,
            body: plate,
            edges: Vec::new(),
            groups: vec![caditor_document::EdgeGroup::Face(wall)],
            size: caditor_expression::Expression::parse_stored("1 mm").unwrap(),
            form: caditor_document::ChamferForm::Equal,
            flipped: false,
        }),
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    let one = 16000.0 - harness.body_volume(plate);

    harness.workspace.panels.choose_all(&[hole, chamfer]);
    harness.frame();
    harness.frame();
    harness.hover("Linear pattern");
    let described = harness.shows_containing("Repeat Hole 1 and Rim along the X axis");
    harness.click("Linear pattern");
    harness.settle();
    let pattern = harness
        .workspace
        .editing
        .solid()
        .expect("the pattern is open");

    assert!(one > std::f64::consts::PI * 9.0 * 10.0 + 10.0);
    assert!(described);
    assert_eq!(pattern_of(&harness, pattern).repeated, vec![hole, chamfer]);
    assert!(harness.shows("Rim"));
    assert!(volume_about(&harness, plate, 16000.0 - 3.0 * one));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    harness.workspace.panels.choose_only(chamfer);
    harness.frame();
    harness.frame();
    harness.hover("Linear pattern");
    assert!(harness.shows_containing("Repeat the body of Extrude 1 along the X axis"));
}

#[test]
fn a_pattern_takes_its_directions_from_lists_in_its_panel() {
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
    let directions = |harness: &Harness| match &pattern_of(harness, pattern).kind {
        caditor_document::PatternKind::Linear { first, second } => (
            first.axis.clone(),
            second.as_ref().map(|second| second.axis.clone()),
        ),
        caditor_document::PatternKind::Circular(_)
        | caditor_document::PatternKind::Curve(_)
        | caditor_document::PatternKind::Points(_) => panic!("the pattern stays linear"),
    };

    open_combo(&mut harness, "Direction");
    harness.click_lowest("The Y axis");
    harness.settle();
    open_combo(&mut harness, "Second direction");
    harness.click_lowest("The X axis");
    harness.settle();

    assert_eq!(
        directions(&harness),
        (
            caditor_document::AxisReference::Principal(caditor_document::PrincipalAxis::Y),
            Some(caditor_document::AxisReference::Principal(
                caditor_document::PrincipalAxis::X
            ))
        )
    );

    open_combo(&mut harness, "Second direction");
    harness.click_lowest(crate::pattern_panel::NO_SECOND_DIRECTION);
    harness.settle();

    assert_eq!(
        directions(&harness),
        (
            caditor_document::AxisReference::Principal(caditor_document::PrincipalAxis::Y),
            None
        )
    );
}

#[test]
fn a_pattern_measures_first_to_last_and_leaves_out_the_copies_clicked_in_its_panel() {
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
    let spacing = |harness: &Harness| {
        let caditor_document::PatternKind::Linear { first, .. } =
            &pattern_of(harness, pattern).kind
        else {
            panic!("the pattern stays linear");
        };
        (first.measured, first.spacing.to_stored_text())
    };

    assert!(harness.shows(crate::pattern_panel::INSTANCES));
    assert!(harness.shows("Spacing"));
    assert_eq!(
        spacing(&harness),
        (
            caditor_document::LinearSpacing::BetweenCopies,
            "48 mm".to_owned()
        )
    );

    harness.click(crate::pattern_panel::MEASURED_OVERALL);
    harness.settle();

    assert!(harness.shows("Total length"));
    assert_eq!(
        spacing(&harness),
        (caditor_document::LinearSpacing::Total, "96 mm".to_owned())
    );
    assert!(volume_about(&harness, plate, 3.0 * 16000.0));

    harness.click_button("Copy 2");
    harness.settle();

    assert!(pattern_of(&harness, pattern).is_skipped([2, 0]));
    assert!(harness.shows("1 copy is left out."));
    assert!(volume_about(&harness, plate, 2.0 * 16000.0));
    assert_eq!(harness.model.undo_label(), Some("Edit Linear pattern 1"));

    harness.perform(Action::Undo);
    harness.settle();

    assert!(pattern_of(&harness, pattern).skipped.is_empty());
    assert!(volume_about(&harness, plate, 3.0 * 16000.0));
}

#[test]
fn a_copy_clicked_in_the_view_while_its_pattern_is_open_is_left_out() {
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
    let copy_face = {
        let body = harness.model.evaluation().body(plate).unwrap();
        let (_, face) = body
            .faces()
            .find(|(_, face)| {
                face.origin()
                    .and_then(|origin| origin.copy())
                    .is_some_and(|copy| copy.index == [1, 0])
            })
            .expect("the first copy has faces");
        Pickable::Face {
            body: plate,
            face: crate::bodies::FaceKey {
                name: face.name(),
                occurrence: 0,
            },
        }
    };
    let top = Plane::from_frame(
        caditor_geometry::Point3::new(0.0, 0.0, 10.0),
        caditor_geometry::Vector3::Z,
        caditor_geometry::Vector3::X,
    )
    .unwrap();

    harness.key(Key::F, Modifiers::NONE);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.hover_pickable(top, Point2::new(68.0, 20.0), copy_face);
    harness.frame();
    let offered = harness.shows_containing("Click to leave copy 1 out of Linear pattern 1");
    harness.click_pickable(top, Point2::new(68.0, 20.0), copy_face);
    harness.settle();

    assert!(offered);
    assert!(pattern_of(&harness, pattern).is_skipped([1, 0]));
    assert!(volume_about(&harness, plate, 2.0 * 16000.0));
    assert_eq!(harness.workspace.editing.solid(), Some(pattern));
}
