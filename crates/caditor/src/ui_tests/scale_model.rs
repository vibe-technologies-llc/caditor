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
