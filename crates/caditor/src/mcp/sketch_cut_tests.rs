use caditor_document::{Document, FeatureKind, PrincipalPlane};
use caditor_geometry::Point2;
use caditor_sketch::{FitSpacing, Sketch, SplineKind};
use serde_json::json;
use tempfile::TempDir;

use super::{
    feature_tests::started,
    sketch_edit_tests::{
        count_of, described, draw, ends, entity, modify, near, new_sketch, of_kind, refused,
    },
    tests::{Client, opening},
};

#[test]
fn curves_switch_to_construction_and_back() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);
    let drawn = draw(
        &mut client,
        &sketch,
        json!([
            { "kind": "line", "from": [0, 0], "to": [20, 0] },
            { "kind": "line", "from": [0, 5], "to": [20, 5] },
            { "kind": "point", "at": [5, 5] }
        ]),
    );
    let (first, second, lone) = (
        drawn[0]["id"].clone(),
        drawn[1]["id"].clone(),
        drawn[2]["id"].clone(),
    );

    let made = modify(
        &mut client,
        &sketch,
        json!({ "operation": "construction", "items": [first, second] }),
    );
    let guides = described(&mut client, &sketch);
    let back = modify(
        &mut client,
        &sketch,
        json!({ "operation": "construction", "items": [second] }),
    );
    let one_left = described(&mut client, &sketch);
    let forced = modify(
        &mut client,
        &sketch,
        json!({ "operation": "construction", "items": [first, second], "construction": true }),
    );
    let ordinary = modify(
        &mut client,
        &sketch,
        json!({ "operation": "construction", "items": [first, second], "construction": false }),
    );
    let after_ordinary = described(&mut client, &sketch);
    let nothing = refused(
        &mut client,
        &sketch,
        json!({ "operation": "construction", "items": [first], "construction": false }),
    );
    let points = refused(
        &mut client,
        &sketch,
        json!({ "operation": "construction", "items": [lone] }),
    );
    client.tool("undo", json!({}));
    let undone = described(&mut client, &sketch);
    client.finish();

    assert_eq!(made["construction"], true, "{made}");
    assert_eq!(made["changed"], json!([first, second]));
    assert!(made["change"].as_str().unwrap().contains("construction"));
    assert_eq!(
        of_kind(&guides, "line")
            .iter()
            .filter(|line| line["construction"] == true)
            .count(),
        2
    );
    assert_eq!(back["construction"], false);
    assert_eq!(entity(&one_left, &second).get("construction"), None);
    assert_eq!(entity(&one_left, &first)["construction"], true);
    assert_eq!(forced["changed"], json!([second]));
    assert_eq!(ordinary["changed"], json!([first, second]));
    assert_eq!(entity(&after_ordinary, &first).get("construction"), None);
    assert!(nothing.contains("to change"), "{nothing}");
    assert!(points.contains("to change"), "{points}");
    assert_eq!(entity(&undone, &first)["construction"], true);
}

#[test]
fn a_curve_is_split_at_a_point_and_crossing_curves_are_broken() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);
    let drawn = draw(
        &mut client,
        &sketch,
        json!([
            { "kind": "line", "from": [0, 0], "to": [20, 0] },
            { "kind": "line", "from": [10, -10], "to": [10, 10] },
            { "kind": "point", "at": [5, 0] },
            { "kind": "point", "at": [5, 4] }
        ]),
    );
    let (across, upright) = (drawn[0]["id"].clone(), drawn[1]["id"].clone());
    let (on_it, off_it) = (drawn[2]["id"].clone(), drawn[3]["id"].clone());

    let off = refused(
        &mut client,
        &sketch,
        json!({ "operation": "split", "curve": across, "point": off_it }),
    );
    let not_a_point = refused(
        &mut client,
        &sketch,
        json!({ "operation": "split", "curve": across, "point": upright }),
    );
    let split = modify(
        &mut client,
        &sketch,
        json!({ "operation": "split", "curve": across, "point": on_it }),
    );
    let halves = described(&mut client, &sketch);
    let broken = modify(
        &mut client,
        &sketch,
        json!({ "operation": "break", "curves": [upright] }),
    );
    let pieces = described(&mut client, &sketch);
    let nothing = refused(
        &mut client,
        &sketch,
        json!({ "operation": "break", "curves": [upright] }),
    );
    client.tool("undo", json!({}));
    let unbroken = described(&mut client, &sketch);
    client.finish();

    assert!(off.contains("does not lie on"), "{off}");
    assert!(not_a_point.contains("not a point"), "{not_a_point}");
    assert_eq!(split["change"], "Split curve");
    assert_eq!(count_of(&split, "added_entities"), 1, "{split}");
    let spans: Vec<([f64; 2], [f64; 2])> = of_kind(&halves, "line")
        .iter()
        .map(|line| ends(&halves, &line["id"]))
        .collect();
    assert_eq!(spans.len(), 3, "{spans:?}");
    assert!(
        spans.iter().any(
            |(from, to)| near(*from, [5.0, 0.0]) && near(*to, [20.0, 0.0])
                || near(*to, [5.0, 0.0]) && near(*from, [20.0, 0.0])
        ),
        "{spans:?}"
    );
    assert_eq!(broken["change"], "Break curves");
    assert_eq!(of_kind(&pieces, "line").len(), 4, "{pieces}");
    assert!(nothing.contains("break"), "{nothing}");
    assert_eq!(of_kind(&unbroken, "line").len(), 3, "{unbroken}");
}

fn model_with_an_evenly_spaced_spline(folder: &TempDir) -> std::path::PathBuf {
    let mut sketch = Sketch::new(PrincipalPlane::Xy.plane());
    sketch.add_spline_of(
        &[
            Point2::new(0.0, 0.0),
            Point2::new(10.0, 8.0),
            Point2::new(11.0, -3.0),
            Point2::new(30.0, 0.0),
        ],
        SplineKind::Fit {
            closed: false,
            spacing: FitSpacing::Even,
        },
    );
    let mut document = Document::default();
    let mut transaction = document.transaction("Old spline");
    transaction.add_feature("Old spline", FeatureKind::from(sketch));
    document.apply(transaction.finish()).unwrap();
    let path = folder.path().join("old.caditor");
    caditor_file::save(&document, &path, false).unwrap();
    path
}

#[test]
fn old_evenly_spaced_splines_are_respaced() {
    let folder = TempDir::new().unwrap();
    let mut client = Client::start(opening(model_with_an_evenly_spaced_spline(&folder)));
    client.initialize("2025-06-18");

    let before = described(&mut client, &json!("Old spline"));
    let spline = of_kind(&before, "spline")[0]["id"].clone();
    let respaced = modify(
        &mut client,
        &json!("Old spline"),
        json!({ "operation": "respace", "splines": [spline] }),
    );
    let after = described(&mut client, &json!("Old spline"));
    let again = refused(
        &mut client,
        &json!("Old spline"),
        json!({ "operation": "respace", "splines": [spline] }),
    );
    client.finish();

    assert_eq!(
        before["entities"].as_array().unwrap().len(),
        entity_count(&after)
    );
    assert_eq!(respaced["change"], "Respace fit-point splines");
    assert_eq!(respaced["respaced"], json!([spline]));
    assert_ne!(
        entity(&before, &spline)["spacing"],
        entity(&after, &spline)["spacing"]
    );
    assert!(again.contains("evenly spaced"), "{again}");
}

fn entity_count(sketch: &serde_json::Value) -> usize {
    sketch["entities"].as_array().unwrap().len()
}

fn model_with_a_control_spline(folder: &TempDir) -> std::path::PathBuf {
    let mut sketch = Sketch::new(PrincipalPlane::Xy.plane());
    sketch.add_spline(&[
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 8.0),
        Point2::new(20.0, -6.0),
        Point2::new(30.0, 7.0),
        Point2::new(40.0, 0.0),
    ]);
    sketch.add_spline_of(
        &[
            Point2::new(0.0, 20.0),
            Point2::new(10.0, 28.0),
            Point2::new(20.0, 14.0),
        ],
        SplineKind::fit(false),
    );
    let mut document = Document::default();
    let mut transaction = document.transaction("Splines");
    transaction.add_feature("Splines", FeatureKind::from(sketch));
    document.apply(transaction.finish()).unwrap();
    let path = folder.path().join("splines.caditor");
    caditor_file::save(&document, &path, false).unwrap();
    path
}

#[test]
fn a_control_point_is_added_to_a_control_spline_and_refused_for_a_fit_spline() {
    let folder = TempDir::new().unwrap();
    let mut client = Client::start(opening(model_with_a_control_spline(&folder)));
    client.initialize("2025-06-18");

    let before = described(&mut client, &json!("Splines"));
    let splines = of_kind(&before, "spline");
    let control = splines
        .iter()
        .find(|spline| spline["form"] == "control")
        .unwrap()["id"]
        .clone();
    let fit = splines
        .iter()
        .find(|spline| spline["form"] == "fit")
        .unwrap()["id"]
        .clone();
    let added = modify(
        &mut client,
        &json!("Splines"),
        json!({ "operation": "insert_control_point", "splines": [control] }),
    );
    let after = described(&mut client, &json!("Splines"));
    let refusal = refused(
        &mut client,
        &json!("Splines"),
        json!({ "operation": "insert_control_point", "splines": [fit] }),
    );
    client.finish();

    assert_eq!(added["change"], "Add control point");
    assert_eq!(added["refined"], json!([control]));
    assert_eq!(entity_count(&after), entity_count(&before) + 1);
    assert_eq!(
        entity(&after, &control)["points"].as_array().unwrap().len(),
        6
    );
    assert!(refusal.contains("passes through its points"), "{refusal}");
}
