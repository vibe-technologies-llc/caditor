use std::{f64::consts::PI, fs};

use caditor_document::Helix;
use caditor_expression::Expression;
use serde_json::{Value, json};
use tempfile::TempDir;

use super::{
    feature_tests::{added, body_count, box_of, close, started, total_volume},
    tests::Client,
};

const NEAR: f64 = 1e-2;

fn near(found: f64, wanted: f64) -> bool {
    (found - wanted).abs() <= NEAR * wanted.abs()
}

fn sketch_with(client: &mut Client, items: Value) -> Value {
    let sketch = client.tool("add_sketch", json!({ "plane": "XY" }))["sketch"]["id"].clone();
    let drawn = client.tool(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": items }),
    );
    assert_eq!(drawn["failing"], json!([]), "{drawn}");
    sketch
}

fn helix_length(turns: f64, diameter: f64, pitch: f64) -> f64 {
    turns * (PI * diameter).hypot(pitch)
}

fn saved_helix(client: &mut Client, folder: &TempDir) -> Helix {
    client.tool("save", json!({}));
    let loaded = caditor_file::load(&folder.path().join("features.caditor")).unwrap();
    loaded
        .document
        .features()
        .find_map(|feature| feature.kind.helix())
        .unwrap()
        .clone()
}

#[test]
fn helices_are_wound_edited_and_followed() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);

    let helix = added(
        &mut client,
        "add_helix",
        json!({ "axis": "Z", "diameter": 20, "turns": 1, "pitch": 4 }),
    );
    let edited = client.tool("edit_feature", json!({ "feature": helix, "turns": 0.5 }));
    let halved = saved_helix(&mut client, &folder);

    assert_eq!(edited["change"], "Edit Helix 1", "{edited}");
    assert_eq!(halved.length.turns(), Some(&Expression::number(0.5)));
    assert!(halved.length.pitch().is_some());

    added(
        &mut client,
        "edit_feature",
        json!({ "feature": helix, "height": 3 }),
    );
    let raised = saved_helix(&mut client, &folder);

    assert_eq!(raised.length.turns(), Some(&Expression::number(0.5)));
    assert!(raised.length.height().is_some() && raised.length.pitch().is_none());

    client.tool("undo", json!({}));
    added(
        &mut client,
        "add_pipe",
        json!({ "path": { "helix": helix }, "size": 2 }),
    );

    assert!(near(
        total_volume(&mut client),
        PI * helix_length(0.5, 20.0, 4.0)
    ));

    let three = client.tool_error("add_helix", json!({ "turns": 2, "pitch": 3, "height": 6 }));

    assert!(three.contains("two of"), "{three}");
    client.finish();
}

#[test]
fn curve_and_point_patterns_follow_their_sketches() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [10.0, 10.0, 10.0], [0.0, 0.0], "Block");
    let path = sketch_with(
        &mut client,
        json!([{ "kind": "line", "from": [0, -20], "to": [100, -20] }]),
    );

    let pattern = added(
        &mut client,
        "add_curve_pattern",
        json!({ "body": block, "sketch": path, "count": 3 }),
    );
    assert!(close(total_volume(&mut client), 3000.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": pattern, "count": 4 }),
    );
    assert!(close(total_volume(&mut client), 4000.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": pattern, "spacing": 5 }),
    );
    assert!(close(total_volume(&mut client), 2500.0));
    client.tool("undo", json!({}));
    assert!(close(total_volume(&mut client), 4000.0));

    let cube = box_of(&mut client, [5.0, 5.0, 5.0], [200.0, 0.0], "Cube");
    let points = sketch_with(
        &mut client,
        json!([{ "kind": "point", "at": [0, 50] }, { "kind": "point", "at": [30, 50] }]),
    );
    let before = body_count(&mut client);
    let placed = added(
        &mut client,
        "add_point_pattern",
        json!({ "body": cube, "sketch": points }),
    );
    let placed_volume = total_volume(&mut client) - 4000.0;
    assert!(
        close(placed_volume, 375.0) || close(placed_volume, 250.0),
        "{placed_volume}"
    );
    assert_eq!(body_count(&mut client), before);
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": placed, "skipped": [1] }),
    );
    assert!(close(
        total_volume(&mut client) - 4000.0,
        placed_volume - 125.0
    ));

    let not_a_path = client.tool_error(
        "add_curve_pattern",
        json!({ "body": cube, "sketch": points }),
    );
    assert!(not_a_path.contains("join end to end"), "{not_a_path}");
    let no_points = client.tool_error("add_point_pattern", json!({ "body": cube, "sketch": path }));
    assert!(no_points.contains("no lone points"), "{no_points}");
    let features = client.tool("list_features", json!({}));
    let kinds: Vec<Value> = features["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|feature| feature["name"].clone())
        .collect();
    assert!(kinds.contains(&json!("Curve pattern 1")), "{features}");
    assert!(kinds.contains(&json!("Point pattern 1")), "{features}");
    client.finish();
}

#[test]
fn files_are_imported_and_the_model_saved_under_another_name_within_the_folders() {
    let folder = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let mut client = started(&folder);
    box_of(&mut client, [10.0, 20.0, 30.0], [0.0, 0.0], "Block");
    client.tool("export", json!({ "file": "part.step" }));

    let imported = client.tool("import", json!({ "file": "part.step" }));
    assert_eq!(imported["failing"], json!([]), "{imported}");
    assert_eq!(imported["bodies"], 1, "{imported}");
    assert_eq!(body_count(&mut client), 2);
    assert!(close(total_volume(&mut client), 12_000.0));
    client.tool("undo", json!({}));
    assert_eq!(body_count(&mut client), 1);

    fs::write(
        folder.path().join("outline.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><rect x="0" y="0" width="20" height="10"/></svg>"#,
    )
    .unwrap();
    let drawing = client.tool("import", json!({ "file": "outline.svg", "plane": "XZ" }));
    assert_eq!(drawing["curves"], 4, "{drawing}");
    assert_eq!(drawing["feature"]["name"], "outline", "{drawing}");
    added(
        &mut client,
        "add_extrusion",
        json!({ "sketch": "outline", "distance": 5, "operation": "new_body" }),
    );
    assert!(close(total_volume(&mut client), 7000.0));

    let step_on_plane = client.tool_error("import", json!({ "file": "part.step", "plane": "XY" }));
    assert!(step_on_plane.contains("drawing"), "{step_on_plane}");
    let missing = client.tool_error("import", json!({ "file": "gone.step" }));
    assert!(missing.contains("does not exist"), "{missing}");
    fs::write(folder.path().join("notes.txt"), "hello").unwrap();
    let unknown = client.tool_error("import", json!({ "file": "notes.txt" }));
    assert!(unknown.contains("not a file caditor imports"), "{unknown}");
    let far = outside.path().join("part.step");
    fs::copy(folder.path().join("part.step"), &far).unwrap();
    let refused = client.tool_error("import", json!({ "file": far.display().to_string() }));
    assert!(refused.contains("lies outside"), "{refused}");

    client.tool("save", json!({}));
    let saved = client.tool("save_as", json!({ "file": "copy.caditor" }));
    assert!(
        saved["saved"].as_str().unwrap().ends_with("copy.caditor"),
        "{saved}"
    );
    assert!(folder.path().join("copy.caditor").is_file());
    let summary = client.tool("get_model", json!({}));
    assert_eq!(summary["unsaved_changes"], false, "{summary}");
    let taken = client.tool_error("save_as", json!({ "file": "features.caditor" }));
    assert!(taken.contains("already exists"), "{taken}");
    let not_a_model = client.tool_error("save_as", json!({ "file": "copy.step" }));
    assert!(
        not_a_model.contains("must end in .caditor"),
        "{not_a_model}"
    );
    box_of(&mut client, [1.0, 1.0, 1.0], [50.0, 0.0], "Speck");
    client.tool("save", json!({}));
    let reloaded = caditor_file::load(&folder.path().join("copy.caditor")).unwrap();
    assert!(
        reloaded
            .document
            .features()
            .any(|feature| feature.name == "Speck")
    );
    let original = caditor_file::load(&folder.path().join("features.caditor")).unwrap();
    assert!(
        original
            .document
            .features()
            .all(|feature| feature.name != "Speck")
    );
    client.finish();
}
