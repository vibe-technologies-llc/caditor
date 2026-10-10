use caditor_document::{
    Datum, DatumPoint, FeatureKind, Mirror, PlaneReference, PointReference, PrincipalPlane,
};
use caditor_expression::{Expression, Unit};
use egui::{Key, Modifiers};

use super::{Harness, extruded_plate, run_from_palette};
use crate::{model::Action, scale_model};

const CLOSE: f64 = 1e-6;

fn top_corner(harness: &Harness, body: caditor_document::FeatureId) -> caditor_geometry::Point3 {
    harness
        .model
        .evaluation()
        .body(body)
        .expect("the plate stands")
        .bounding_box()
        .expect("the plate has a size")
        .max()
}

#[test]
fn scale_model_resizes_the_whole_model_from_the_palette_as_one_undoable_change() {
    let mut harness = Harness::new();
    let (body, _) = extruded_plate(&mut harness);
    let before = top_corner(&harness, body);

    run_from_palette(&mut harness, "Scale model");
    assert!(harness.workspace.scale_model.is_some());
    assert!(harness.shows(scale_model::PLAIN_VALUES));
    harness.type_text("0");
    harness.click_lowest(scale_model::SCALE_LABEL);
    harness.frame();
    assert!(harness.shows_containing("The factor must be more than zero"));

    harness.type_into_field(scale_model::factor_field_id(), "5 mm");
    harness.frame();
    assert!(harness.shows_containing("a plain number is needed"));
    assert!(harness.workspace.scale_model.is_some());

    harness.type_into_field(scale_model::factor_field_id(), "2");
    harness.settle();
    let after = top_corner(&harness, body);

    assert!(harness.workspace.scale_model.is_none());
    assert!(
        (after - before * 2.0).length() < CLOSE,
        "{after:?} from {before:?}"
    );
    assert!(harness.shows_containing("Scaled the model by 2"));

    harness.perform(Action::Undo);
    harness.settle();
    assert!((top_corner(&harness, body) - before).length() < CLOSE);

    run_from_palette(&mut harness, "Scale model");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert!(harness.workspace.scale_model.is_none());
}

#[test]
fn scaling_about_a_point_off_a_mirror_plane_moves_the_mirror_onto_new_datums() {
    let mut harness = Harness::new();
    let (body, _) = extruded_plate(&mut harness);
    let mut transaction = harness.document().transaction("Point and mirror");
    let point = transaction.add_feature(
        "Point 1",
        FeatureKind::Datum(Datum::Point(DatumPoint {
            base: PointReference::Origin,
            offset: [
                Expression::measure(10.0, Unit::Millimetre),
                Expression::measure(0.0, Unit::Millimetre),
                Expression::measure(0.0, Unit::Millimetre),
            ],
        })),
    );
    let mirror = transaction.add_feature(
        "Mirror 1",
        FeatureKind::Mirror(Mirror {
            body,
            plane: PlaneReference::Principal(PrincipalPlane::Yz),
            keep_original: true,
            mirrored: Vec::new(),
        }),
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    let before = harness.document().features().count();

    run_from_palette(&mut harness, "Scale model");
    if let Some(draft) = harness.workspace.scale_model.as_mut() {
        draft.centre = scale_model::Centre::Datum(point);
    }
    harness.frame();

    assert!(harness.shows_containing("moves Mirror 1 (the YZ plane) onto new datums"));

    harness.type_into_field(scale_model::factor_field_id(), "2");
    harness.settle();
    let plane = harness
        .document()
        .feature(mirror)
        .and_then(|feature| match &feature.kind {
            FeatureKind::Mirror(mirror) => Some(mirror.plane.clone()),
            _ => None,
        });

    assert!(harness.workspace.scale_model.is_none());
    assert!(
        harness.shows_containing("Mirror 1 now uses Origin, scaled and Axes and planes, scaled")
    );
    assert_eq!(harness.document().features().count(), before + 2);
    assert!(matches!(
        plane,
        Some(PlaneReference::Frame {
            plane: PrincipalPlane::Yz,
            ..
        })
    ));
    assert_eq!(harness.model.evaluation().failed_count(), 0);

    harness.perform(Action::Undo);
    harness.settle();

    assert_eq!(harness.document().features().count(), before);
}
