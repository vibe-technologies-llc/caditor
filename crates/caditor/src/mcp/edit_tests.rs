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

fn two_squares(client: &mut Client) -> Value {
    let sketch = client.tool("add_sketch", json!({ "plane": "XY" }))["sketch"]["id"].clone();
    let drawn = client.tool(
        "add_sketch_geometry",
        json!({
            "sketch": sketch,
            "items": [
                { "kind": "rectangle", "corner": [0, 0], "opposite": [10, 10] },
                { "kind": "rectangle", "corner": [20, 0], "opposite": [30, 10] }
            ]
        }),
    );
    assert_eq!(drawn["failing"], json!([]), "{drawn}");
    sketch
}

#[test]
fn extrusions_and_revolves_use_the_regions_chosen_by_a_point_inside() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = two_squares(&mut client);

    let described = client.tool("get_sketch", json!({ "sketch": sketch }));
    let listed = described["regions"].as_array().unwrap();
    assert_eq!(listed.len(), 2, "{described}");
    assert!(listed.iter().all(|region| region["inside"].is_array()));

    let post = added(
        &mut client,
        "add_extrusion",
        json!({ "sketch": sketch, "distance": 5, "regions": [[25, 5]], "operation": "new_body" }),
    );
    assert!(close(total_volume(&mut client), 500.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": post, "regions": [[5, 5], [25, 5]] }),
    );
    assert!(close(total_volume(&mut client), 1000.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": post, "regions": [[5, 5]] }),
    );
    assert!(close(total_volume(&mut client), 500.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": post, "regions": "all" }),
    );
    assert!(close(total_volume(&mut client), 1000.0));
    client.tool("undo", json!({}));
    assert!(close(total_volume(&mut client), 500.0));

    let missed = client.tool_error(
        "add_extrusion",
        json!({ "sketch": sketch, "distance": 5, "regions": [[15, 5]] }),
    );
    assert!(missed.contains("no region around"), "{missed}");
    let nothing = client.tool_error(
        "add_extrusion",
        json!({ "sketch": sketch, "distance": 5, "regions": [] }),
    );
    assert!(nothing.contains("a point inside each region"), "{nothing}");

    let profile = client.tool("add_sketch", json!({ "plane": "XZ" }))["sketch"]["id"].clone();
    client.tool(
        "add_sketch_geometry",
        json!({
            "sketch": profile,
            "items": [
                { "kind": "rectangle", "corner": [10, 0], "opposite": [20, 30] },
                { "kind": "rectangle", "corner": [40, 0], "opposite": [50, 30] }
            ]
        }),
    );
    let before = total_volume(&mut client);
    added(
        &mut client,
        "add_revolve",
        json!({ "sketch": profile, "axis": "Z", "regions": [[15, 15]], "operation": "new_body" }),
    );
    assert!(close(total_volume(&mut client) - before, PI * 300.0 * 30.0));
    client.finish();
}

#[test]
fn a_thin_wall_is_taken_off_again_by_extrusions_and_revolves() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = square_sketch(&mut client, [0.0, 0.0], 10.0);
    let tube = added(
        &mut client,
        "add_extrusion",
        json!({ "sketch": sketch, "distance": 10, "thin_wall": 1, "operation": "new_body" }),
    );
    assert!(close(total_volume(&mut client), 400.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": tube, "thin_wall": "off" }),
    );
    assert!(close(total_volume(&mut client), 1000.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": tube, "thin_wall": "1 mm", "wall_side": "inside" }),
    );
    assert!(close(total_volume(&mut client), 360.0));
    let side_without_wall = client.tool_error(
        "edit_feature",
        json!({ "feature": tube, "thin_wall": "off", "wall_side": "inside" }),
    );
    assert!(
        side_without_wall.contains("goes with thin_wall"),
        "{side_without_wall}"
    );

    let profile = client.tool("add_sketch", json!({ "plane": "XZ" }))["sketch"]["id"].clone();
    client.tool(
        "add_sketch_geometry",
        json!({
            "sketch": profile,
            "items": [{ "kind": "rectangle", "corner": [100, 0], "opposite": [120, 30] }]
        }),
    );
    let before = total_volume(&mut client);
    let ring = added(
        &mut client,
        "add_revolve",
        json!({
            "sketch": profile, "axis": "Z", "thin_wall": 1, "wall_side": "inside",
            "operation": "new_body"
        }),
    );
    let walled = total_volume(&mut client) - before;
    let solid = PI * (120.0 * 120.0 - 100.0 * 100.0) * 30.0;
    assert!(walled > 0.0 && walled < solid * 0.5, "{walled}");
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": ring, "thin_wall": "off" }),
    );
    assert!(close(total_volume(&mut client) - before, solid));
    client.finish();
}

#[test]
fn extrusions_end_up_to_a_curved_face() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let dome = added(
        &mut client,
        "add_primitive",
        json!({
            "kind": "sphere", "diameter": 20, "plane": "XY", "at": [0, 0],
            "anchor": "centre", "operation": "new_body", "name": "Dome"
        }),
    );
    let faces = client.tool("list_faces", json!({ "body": dome }));
    let round = faces["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|face| face["surface"] == "sphere")
        .unwrap()["face"]
        .clone();
    let low = added(
        &mut client,
        "add_datum_plane",
        json!({ "base": "XY", "offset": -30 }),
    );
    let sketch = client.tool("add_sketch", json!({ "datum": low }))["sketch"]["id"].clone();
    client.tool(
        "add_sketch_geometry",
        json!({
            "sketch": sketch,
            "items": [{ "kind": "rectangle", "corner": [-2, -2], "opposite": [2, 2] }]
        }),
    );
    let before = total_volume(&mut client);
    let column = added(
        &mut client,
        "add_extrusion",
        json!({
            "sketch": sketch,
            "up_to_surface": { "body": dome, "face": round },
            "operation": "new_body"
        }),
    );
    let reached = total_volume(&mut client) - before;
    assert!(reached > 315.0 && reached < 330.0, "{reached}");
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": column, "distance": 5 }),
    );
    assert!(close(total_volume(&mut client) - before, 80.0), "{reached}");

    let twice = client.tool_error(
        "add_extrusion",
        json!({
            "sketch": sketch, "distance": 5,
            "up_to_surface": { "body": dome, "face": round }
        }),
    );
    assert!(twice.contains("one end"), "{twice}");
    client.finish();
}

#[test]
fn the_second_side_of_an_extrusion_ends_at_a_plane_or_through_all() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [40.0, 30.0, 20.0], [0.0, 0.0], "Block");
    let middle = added(
        &mut client,
        "add_datum_plane",
        json!({ "base": "XY", "offset": 10 }),
    );
    let sketch = client.tool("add_sketch", json!({ "datum": middle }))["sketch"]["id"].clone();
    client.tool(
        "add_sketch_geometry",
        json!({
            "sketch": sketch,
            "items": [{ "kind": "rectangle", "corner": [5, 5], "opposite": [15, 15] }]
        }),
    );
    let pocket = added(
        &mut client,
        "add_extrusion",
        json!({
            "sketch": sketch, "distance": 5, "second_end": { "up_to": "XY" },
            "operation": "remove", "body": block
        }),
    );
    assert!(close(total_volume(&mut client), 24_000.0 - 1500.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": pocket, "second_end": { "through_all": true } }),
    );
    assert!(close(total_volume(&mut client), 24_000.0 - 1500.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": pocket, "second_end": { "distance": 2 } }),
    );
    assert!(close(total_volume(&mut client), 24_000.0 - 700.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": pocket, "second_distance": 3 }),
    );
    assert!(close(total_volume(&mut client), 24_000.0 - 800.0));

    let both = client.tool_error(
        "edit_feature",
        json!({ "feature": pocket, "second_distance": 3, "second_end": { "through_all": true } }),
    );
    assert!(both.contains("not both"), "{both}");
    let offset = client.tool_error(
        "edit_feature",
        json!({ "feature": pocket, "second_end": { "distance": 3, "end_offset": 1 } }),
    );
    assert!(offset.contains("nd_offset goes with"), "{offset}");
    client.finish();
}

#[test]
fn extrusions_run_along_an_axis_and_go_square_again() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [40.0, 30.0, 20.0], [0.0, 0.0], "Block");
    let slant = added(
        &mut client,
        "add_datum_axis",
        json!({
            "through_points": [
                { "body": block, "corner": [0.0, 0.0, 0.0] },
                { "body": block, "corner": [40.0, 0.0, 20.0] }
            ]
        }),
    );
    let sketch = square_sketch(&mut client, [100.0, 0.0], 10.0);
    let post = added(
        &mut client,
        "add_extrusion",
        json!({ "sketch": sketch, "distance": 10, "direction": { "datum": slant }, "operation": "new_body" }),
    );
    let slanted = total_volume(&mut client) - 24_000.0;
    assert!(slanted > 0.0 && !close(slanted, 1000.0), "{slanted}");
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": post, "direction": "square" }),
    );
    assert!(close(total_volume(&mut client) - 24_000.0, 1000.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": post, "direction": { "datum": slant } }),
    );
    assert!(close(total_volume(&mut client) - 24_000.0, slanted));
    client.finish();
}

#[test]
fn a_revolve_turns_up_to_a_plane() {
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
    let quarter = PI * 300.0 * 30.0 / 4.0;
    let ring = added(
        &mut client,
        "add_revolve",
        json!({ "sketch": sketch, "axis": "Z", "up_to": "YZ" }),
    );
    assert!(close(total_volume(&mut client), quarter));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": ring, "reversed": true }),
    );
    assert!(close(total_volume(&mut client), quarter));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": ring, "angle": "180 deg" }),
    );
    assert!(close(total_volume(&mut client), 2.0 * quarter));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": ring, "up_to": "YZ" }),
    );
    assert!(close(total_volume(&mut client), quarter));

    let both = client.tool_error(
        "add_revolve",
        json!({ "sketch": sketch, "axis": "Z", "up_to": "YZ", "angle": "90 deg" }),
    );
    assert!(both.contains("not both"), "{both}");
    client.finish();
}

fn hole_sketch(client: &mut Client, items: Value) -> Value {
    let datum = added(
        client,
        "add_datum_plane",
        json!({ "base": "XY", "offset": 20 }),
    );
    let sketch = client.tool("add_sketch", json!({ "datum": datum }))["sketch"]["id"].clone();
    let drawn = client.tool(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": items }),
    );
    assert_eq!(drawn["failing"], json!([]), "{drawn}");
    sketch
}

#[test]
fn holes_take_steps_slots_drill_points_and_sizes_from_circles() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [40.0, 30.0, 20.0], [0.0, 0.0], "Block");
    let sketch = hole_sketch(&mut client, json!([{ "kind": "point", "at": [20, 15] }]));

    let hole = added(
        &mut client,
        "add_hole",
        json!({ "sketch": sketch, "diameter": 4, "depth": 10, "body": block }),
    );
    let plain = total_volume(&mut client);
    assert!(close(plain, 24_000.0 - PI * 4.0 * 10.0));

    added(
        &mut client,
        "edit_feature",
        json!({ "feature": hole, "steps": [{ "diameter": 8, "depth": 3 }] }),
    );
    let stepped = total_volume(&mut client);
    assert!(close(stepped, 24_000.0 - PI * (16.0 * 3.0 + 4.0 * 7.0)));
    added(
        &mut client,
        "edit_feature",
        json!({
            "feature": hole,
            "steps": [{ "diameter": 8, "depth": 3 }, { "diameter": 6, "depth": 2 }]
        }),
    );
    assert!(close(
        total_volume(&mut client),
        24_000.0 - PI * (16.0 * 3.0 + 9.0 * 2.0 + 4.0 * 5.0)
    ));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": hole, "style": "plain" }),
    );
    assert!(close(total_volume(&mut client), plain));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": hole, "style": "stepped" }),
    );
    assert!(total_volume(&mut client) < plain);

    let clash = client.tool_error(
        "edit_feature",
        json!({ "feature": hole, "style": "countersink", "steps": [{ "diameter": 8, "depth": 3 }] }),
    );
    assert!(clash.contains("stepped"), "{clash}");

    added(
        &mut client,
        "edit_feature",
        json!({ "feature": hole, "style": "plain", "slot_length": 10 }),
    );
    let slotted = total_volume(&mut client);
    assert!(
        close(slotted, 24_000.0 - (PI * 4.0 + 4.0 * 10.0) * 10.0),
        "{slotted}"
    );
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": hole, "slot_length": 14, "slot_angle": "90 deg" }),
    );
    assert!(close(
        total_volume(&mut client),
        24_000.0 - (PI * 4.0 + 4.0 * 14.0) * 10.0
    ));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": hole, "shape": "round" }),
    );
    assert!(close(total_volume(&mut client), plain));
    let round_slot = client.tool_error(
        "edit_feature",
        json!({ "feature": hole, "shape": "round", "slot_length": 5 }),
    );
    assert!(round_slot.contains("leave out shape round"), "{round_slot}");

    added(
        &mut client,
        "edit_feature",
        json!({ "feature": hole, "bottom": "drill_point" }),
    );
    let pointed = total_volume(&mut client);
    assert!(pointed < plain, "{pointed} {plain}");
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": hole, "point_angle": "90 deg" }),
    );
    assert!(total_volume(&mut client) < pointed);
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": hole, "bottom": "flat" }),
    );
    assert!(close(total_volume(&mut client), plain));
    let flat_point = client.tool_error(
        "edit_feature",
        json!({ "feature": hole, "bottom": "flat", "point_angle": "90 deg" }),
    );
    assert!(flat_point.contains("leave out bottom flat"), "{flat_point}");

    let circled = hole_sketch(
        &mut client,
        json!([{ "kind": "circle", "center": [10, 15], "radius": 3 }]),
    );
    let sized = added(
        &mut client,
        "add_hole",
        json!({ "sketch": circled, "diameter": 4, "depth": 5, "sizing": "circles", "body": block }),
    );
    assert!(close(total_volume(&mut client), plain - PI * 9.0 * 5.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": sized, "sizing": "typed" }),
    );
    assert!(close(total_volume(&mut client), plain - PI * 4.0 * 5.0));
    client.finish();
}
