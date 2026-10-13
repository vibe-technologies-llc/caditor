use caditor_document::{
    BodyOperation, Extrude, ExtrudeExtent, FeatureId, FeatureKind, FeatureState, RegionChoice, Rib,
    RibDirection, RibSide, Shell, ShellSide, SolidFeature,
};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{FaceReference, Surface};
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{Harness, choose, rectangle, run_from_palette};
use crate::{model::Action, rib_panel, selection::Pickable};

const BOX_VOLUME: f64 = 100.0 * 60.0 * 40.0 - 90.0 * 50.0 * 35.0;
const WEB_AREA: f64 = 50.0 * 35.0;

fn length(text: &str) -> Expression {
    Expression::parse_stored(text).unwrap()
}

fn open_box(harness: &mut Harness) -> FeatureId {
    let mut outline = Sketch::new(Plane::XY);
    rectangle(
        &mut outline,
        Point2::new(0.0, 0.0),
        Point2::new(100.0, 60.0),
    );
    let outline = harness.add_sketch(outline);
    let mut transaction = harness.document().transaction("Box");
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(length("40 mm"), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    let solid = harness
        .model
        .evaluation()
        .body(base)
        .expect("the box is built");
    let (top, _) = solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => plane.frame().normal() * face.sense().sign() == Vector3::Z,
            _ => false,
        })
        .expect("the box has a top");
    let open = vec![FaceReference::capture(solid, top).unwrap()];
    let mut transaction = harness.document().transaction("Hollow");
    transaction.add_feature(
        "Shell 1",
        FeatureKind::Shell(Shell {
            body: base,
            open,
            thickness: length("5 mm"),
            side: ShellSide::Inside,
            faces: Vec::new(),
        }),
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    base
}

fn rib_of(harness: &Harness, feature: FeatureId) -> Rib {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.rib())
        .expect("a rib")
        .clone()
}

fn near(found: f64, expected: f64) -> bool {
    (found - expected).abs() < 1e-3 * expected
}

#[test]
fn rib_from_the_palette_follows_the_selected_line_and_turns_into_a_web_from_its_panel() {
    let mut harness = Harness::new();
    let body = open_box(&mut harness);
    let plane = Plane::new(Point3::new(0.0, 0.0, 40.0), Vector3::Z).unwrap();
    let mut drawn = Sketch::new(plane);
    let line = drawn.add_line(Point2::new(50.0, 15.0), Point2::new(50.0, 45.0));
    let sketch = harness.add_sketch(drawn);
    harness.select([Pickable::SketchEntity {
        feature: sketch,
        entity: line,
    }]);

    run_from_palette(&mut harness, "rib");
    harness.settle();

    let rib = harness.workspace.editing.solid().expect("the rib is open");
    assert_eq!(harness.model.undo_label(), Some("Create Rib 1"));
    let made = rib_of(&harness, rib);
    assert_eq!((made.sketch, made.body), (sketch, body));
    assert_eq!(made.direction, RibDirection::Parallel);
    assert_eq!(made.side, RibSide::Symmetric);
    assert!(harness.shows(rib_panel::THICKNESS));
    let reason = match harness
        .model
        .evaluation()
        .feature(rib)
        .map(|status| status.state.clone())
    {
        Some(FeatureState::Failed(error)) => error.reason,
        other => panic!("a rib lying in the open top runs past the box, found {other:?}"),
    };
    assert!(reason.contains("not closed off"), "{reason}");
    assert!(harness.shows(&reason));

    choose(&mut harness, "Parallel to sketch", "Square to sketch");

    let web = rib_of(&harness, rib);
    assert_eq!(web.direction, RibDirection::Square);
    assert!(web.reversed, "the web runs down into the box");
    assert!(harness.shows(rib_panel::REVERSE_DIRECTION));
    assert!(near(harness.body_volume(body), BOX_VOLUME + 2.0 * WEB_AREA));

    harness.type_into_field(Id::new(("rib-field", rib, "thickness")), "4 mm");
    harness.settle();
    assert!(near(harness.body_volume(body), BOX_VOLUME + 4.0 * WEB_AREA));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    assert_eq!(rib_of(&harness, rib).thickness, length("2 mm"));
    assert!(near(harness.body_volume(body), BOX_VOLUME + 2.0 * WEB_AREA));
}
