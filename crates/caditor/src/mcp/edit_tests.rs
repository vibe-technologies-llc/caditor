use std::f64::consts::PI;

use serde_json::{Value, json};
use tempfile::TempDir;

use super::{
    feature_tests::{added, box_of, close, face_where, started, total_volume},
    tests::Client,
};

fn square_sketch(client: &mut Client, corner: [f64; 2], side: f64) -> Value {
    let sketch = client.tool("add_sketch", json!({ "plane": "XY" }))["sketch"]["id"].clone();
    let [x, y] = corner;
    let drawn = client.tool(
        "add_sketch_geometry",
        json!({
            "sketch": sketch,
            "items": [{ "kind": "rectangle", "corner": [x, y], "opposite": [x + side, y + side] }]
        }),
    );
    assert_eq!(drawn["failing"], json!([]), "{drawn}");
    sketch
}

#[test]
fn features_are_edited_in_place_as_one_undo_step() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [40.0, 30.0, 20.0], [0.0, 0.0], "Block");

    let edited = client.tool(
        "edit_feature",
        json!({ "feature": "Block", "length": 60, "name": "Long block" }),
    );
    assert_eq!(edited["change"], "Edit Block", "{edited}");
    assert_eq!(edited["feature"]["name"], "Long block");
    assert!(close(total_volume(&mut client), 36_000.0));
    client.tool("undo", json!({}));
    assert!(close(total_volume(&mut client), 24_000.0));
    let features = client.tool("list_features", json!({}));
    assert_eq!(features["features"][0]["name"], "Block", "{features}");

    let sketch = square_sketch(&mut client, [100.0, 0.0], 10.0);
    added(
        &mut client,
        "add_extrusion",
        json!({ "sketch": sketch, "distance": 10, "operation": "new_body", "name": "Post" }),
    );
    assert!(close(total_volume(&mut client), 25_000.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": "Post", "distance": "20 mm" }),
    );
    assert!(close(total_volume(&mut client), 26_000.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": "Post", "second_distance": 5 }),
    );
    assert!(close(total_volume(&mut client), 26_500.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": "Post", "symmetric": true }),
    );
    assert!(close(total_volume(&mut client), 26_000.0));

    let pattern = added(
        &mut client,
        "add_linear_pattern",
        json!({ "body": block, "axis": "Y", "count": 3, "spacing": 50 }),
    );
    assert!(close(total_volume(&mut client), 74_000.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": pattern, "count": 4, "skipped": [2] }),
    );
    assert!(close(total_volume(&mut client), 74_000.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": pattern, "skipped": [] }),
    );
    assert!(close(total_volume(&mut client), 98_000.0));

    let sketch_refused = client.tool_error("edit_feature", json!({ "feature": sketch }));
    assert!(sketch_refused.contains("is a sketch"), "{sketch_refused}");
    let unfit = client.tool_error("edit_feature", json!({ "feature": "Post", "radius": 2 }));
    assert!(unfit.contains("do not fit add_extrusion"), "{unfit}");
    let empty = client.tool_error("edit_feature", json!({ "feature": "Post" }));
    assert!(
        empty.contains("with the arguments add_extrusion takes"),
        "{empty}"
    );
    client.finish();
}

#[test]
fn revolves_and_fillets_keep_what_an_edit_leaves_out() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = client.tool("add_sketch", json!({ "plane": "XZ" }))["sketch"]["id"].clone();
    client.tool(
        "add_sketch_geometry",
        json!({
            "sketch": sketch,
            "items": [{ "kind": "rectangle", "corner": [10, 0], "opposite": [20, 30] }]
        }),
    );
    let ring = added(
        &mut client,
        "add_revolve",
        json!({ "sketch": sketch, "axis": "Z", "angle": "90 deg" }),
    );
    let quarter = PI * 300.0 * 30.0 / 4.0;
    assert!(close(total_volume(&mut client), quarter));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": ring, "second_angle": "90 deg" }),
    );
    assert!(close(total_volume(&mut client), 2.0 * quarter));
    let symmetric_refused = client.tool_error(
        "edit_feature",
        json!({ "feature": ring, "symmetric": true, "second_angle": 10 }),
    );
    assert!(
        symmetric_refused.contains("takes no second_angle"),
        "{symmetric_refused}"
    );

    let block = box_of(&mut client, [20.0, 20.0, 20.0], [100.0, 0.0], "Block");
    let top = face_where(&mut client, &block, [0.0, 0.0, 1.0]);
    let fillet = added(
        &mut client,
        "add_fillet",
        json!({ "body": block, "faces": [top], "radius": 2 }),
    );
    let small = total_volume(&mut client);
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": fillet, "radius": "4 mm" }),
    );
    let large = total_volume(&mut client);
    assert!(large < small, "{large} {small}");
    client.finish();
}

#[test]
fn extrusions_end_up_to_a_face_and_take_walls_tapers_and_offsets() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [40.0, 30.0, 20.0], [0.0, 0.0], "Block");
    let top = face_where(&mut client, &block, [0.0, 0.0, 1.0]);

    let pocket = square_sketch(&mut client, [5.0, 5.0], 10.0);
    added(
        &mut client,
        "add_extrusion",
        json!({ "sketch": pocket, "up_to": { "body": block, "face": top }, "operation": "remove" }),
    );
    assert!(close(total_volume(&mut client), 22_000.0));

    let tube = square_sketch(&mut client, [100.0, 0.0], 10.0);
    added(
        &mut client,
        "add_extrusion",
        json!({ "sketch": tube, "distance": 10, "thin_wall": 1, "operation": "new_body" }),
    );
    assert!(close(total_volume(&mut client), 22_400.0));

    let lifted = square_sketch(&mut client, [200.0, 0.0], 10.0);
    added(
        &mut client,
        "add_extrusion",
        json!({ "sketch": lifted, "distance": 10, "start_offset": 5, "operation": "new_body" }),
    );
    assert!(close(total_volume(&mut client), 23_400.0));

    let tapered = square_sketch(&mut client, [300.0, 0.0], 10.0);
    added(
        &mut client,
        "add_extrusion",
        json!({ "sketch": tapered, "distance": 10, "taper": "5 deg", "operation": "new_body" }),
    );
    let with_taper = total_volume(&mut client) - 23_400.0;
    assert!(
        with_taper > 0.0 && !close(with_taper, 1000.0),
        "{with_taper}"
    );

    let refused = client.tool_error(
        "add_extrusion",
        json!({ "sketch": tapered, "distance": 10, "up_to": "next" }),
    );
    assert!(refused.contains("one end: a distance"), "{refused}");
    let no_wall = client.tool_error(
        "add_extrusion",
        json!({ "sketch": tapered, "distance": 10, "wall_side": "inside" }),
    );
    assert!(no_wall.contains("goes with thin_wall"), "{no_wall}");
    client.finish();
}

#[test]
fn holes_take_standard_sizes_heads_and_threads() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [40.0, 30.0, 20.0], [0.0, 0.0], "Block");
    let datum = added(
        &mut client,
        "add_datum_plane",
        json!({ "base": "XY", "offset": 20 }),
    );
    let sketch = client.tool("add_sketch", json!({ "datum": datum }))["sketch"]["id"].clone();
    client.tool(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": [{ "kind": "point", "at": [20, 15] }] }),
    );

    let hole = added(
        &mut client,
        "add_hole",
        json!({ "sketch": sketch, "standard": "M6", "depth": "through all", "body": block }),
    );
    let plain = total_volume(&mut client);
    assert!(
        plain < 24_000.0 - PI * 9.0 * 20.0 && plain > 24_000.0 - PI * 16.0 * 20.0,
        "{plain}"
    );
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": hole, "style": "counterbore", "head_depth": 3 }),
    );
    assert!(total_volume(&mut client) < plain);
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": hole, "style": "plain", "fit": "tapped", "thread_form": "cosmetic" }),
    );
    assert!(total_volume(&mut client) > plain);

    let both = client.tool_error(
        "edit_feature",
        json!({ "feature": hole, "diameter": 5, "standard": "M5" }),
    );
    assert!(both.contains("not both"), "{both}");
    let untapped = client.tool_error(
        "add_hole",
        json!({ "sketch": sketch, "diameter": 5, "depth": 5, "thread_hand": "left" }),
    );
    assert!(untapped.contains("tapped hole"), "{untapped}");
    let unknown = client.tool_error(
        "add_hole",
        json!({ "sketch": sketch, "standard": "M7", "depth": 5 }),
    );
    assert!(unknown.contains("M6"), "{unknown}");
    client.finish();
}
