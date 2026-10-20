use std::f64::consts::PI;

use serde_json::{Value, json};
use tempfile::TempDir;

use super::tests::{Client, opening};

const TOLERANCE: f64 = 1e-6;

pub(super) fn number(value: &Value) -> f64 {
    value
        .as_f64()
        .unwrap_or_else(|| panic!("{value} is not a number"))
}

pub(super) fn close(found: f64, wanted: f64) -> bool {
    (found - wanted).abs() <= TOLERANCE * wanted.abs().max(1.0)
}

pub(super) fn started(folder: &TempDir) -> Client {
    let mut client = Client::start(opening(folder.path().join("features.caditor")));
    client.initialize("2025-06-18");
    client
}

pub(super) fn added(client: &mut Client, tool: &str, arguments: Value) -> Value {
    let answer = client.tool(tool, arguments);
    assert_eq!(answer["failing"], json!([]), "{tool}: {answer}");
    answer["feature"]["id"].clone()
}

pub(super) fn total_volume(client: &mut Client) -> f64 {
    let bodies = client.tool("list_bodies", json!({}));
    bodies["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|body| number(&body["volume"]))
        .sum()
}

pub(super) fn body_count(client: &mut Client) -> usize {
    let bodies = client.tool("list_bodies", json!({}));
    bodies["bodies"].as_array().unwrap().len()
}

pub(super) fn box_of(client: &mut Client, size: [f64; 3], at: [f64; 2], name: &str) -> Value {
    let [length, width, height] = size;
    added(
        client,
        "add_primitive",
        json!({
            "kind": "box",
            "length": length,
            "width": width,
            "height": format!("{height} mm"),
            "plane": "XY",
            "at": at,
            "anchor": "corner",
            "operation": "new_body",
            "name": name
        }),
    )
}

pub(super) fn face_where(client: &mut Client, body: &Value, normal: [f64; 3]) -> String {
    let faces = client.tool("list_faces", json!({ "body": body }));
    let found = faces["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|face| face["surface"] == "plane" && face["normal"] == json!(normal))
        .unwrap_or_else(|| panic!("no face with normal {normal:?}: {faces}"));
    found["face"].as_str().unwrap().to_owned()
}

fn vertical_edges(client: &mut Client, body: &Value) -> Vec<Value> {
    let edges = client.tool("list_edges", json!({ "body": body }));
    edges["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|edge| {
            edge["kind"] == "line" && number(&edge["start"][0]) == number(&edge["end"][0]) && {
                number(&edge["start"][1]) == number(&edge["end"][1])
            }
        })
        .map(|edge| edge["edge"].clone())
        .collect()
}

#[test]
fn edges_are_listed_with_keys_kinds_and_sizes_and_measured_between() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [40.0, 30.0, 20.0], [0.0, 0.0], "Block");

    let edges = client.tool("list_edges", json!({ "body": block }));
    let listed = edges["edges"].as_array().unwrap();
    assert_eq!(listed.len(), 12, "{edges}");
    let mut lengths: Vec<f64> = listed.iter().map(|edge| number(&edge["length"])).collect();
    lengths.sort_by(f64::total_cmp);
    assert_eq!(
        lengths,
        [
            20.0, 20.0, 20.0, 20.0, 30.0, 30.0, 30.0, 30.0, 40.0, 40.0, 40.0, 40.0
        ]
    );
    assert!(listed.iter().all(|edge| edge["kind"] == "line"));
    let described = listed[0]["description"].as_str().unwrap();
    assert!(described.starts_with("Edge between Block"), "{described}");

    let bottom = face_where(&mut client, &block, [0.0, 0.0, -1.0]);
    let top = face_where(&mut client, &block, [0.0, 0.0, 1.0]);
    let measured = client.tool(
        "measure",
        json!({ "from": { "body": block, "face": bottom }, "to": { "body": block, "face": top } }),
    );
    assert!(close(number(&measured["distance"]), 20.0), "{measured}");
    assert!(close(number(&measured["angle"]), 0.0), "{measured}");
    assert_eq!(measured["exact"], true);

    let edge = vertical_edges(&mut client, &block)[0].clone();
    let from_point = client.tool(
        "measure",
        json!({ "from": [100, 0, 10], "to": { "body": block, "edge": edge } }),
    );
    assert!(number(&from_point["distance"]) > 0.0, "{from_point}");

    let unknown = client.tool_error(
        "measure",
        json!({ "from": [0, 0, 0], "to": { "body": block, "edge": "abc" } }),
    );
    assert!(unknown.contains("has no edge abc"), "{unknown}");
    let unreadable = client.tool_error("list_edges", json!({ "body": "Nothing" }));
    assert!(unreadable.contains("There is no body"), "{unreadable}");
    client.finish();
}

#[test]
fn fillets_and_chamfers_take_edges_faces_or_the_whole_body() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [40.0, 30.0, 20.0], [0.0, 0.0], "Block");
    let whole = 40.0 * 30.0 * 20.0;

    let vertical = vertical_edges(&mut client, &block);
    assert_eq!(vertical.len(), 4);
    let fillet = client.tool(
        "add_fillet",
        json!({ "body": block, "edges": vertical, "radius": "2 mm" }),
    );
    assert_eq!(fillet["change"], "Create Fillet 1", "{fillet}");
    assert_eq!(fillet["failing"], json!([]));
    let rounded = whole - 4.0 * (4.0 - PI) * 20.0;
    assert!(close(total_volume(&mut client), rounded));

    let top = face_where(&mut client, &block, [0.0, 0.0, 1.0]);
    added(
        &mut client,
        "add_chamfer",
        json!({ "body": block, "faces": [top], "distance": 1, "name": "Top chamfer" }),
    );
    assert!(total_volume(&mut client) < rounded);

    let refused = client.tool_error("add_fillet", json!({ "body": block, "radius": 1 }));
    assert!(refused.contains("Say which edges to fillet"), "{refused}");
    let both = client.tool_error(
        "add_chamfer",
        json!({ "body": block, "all_edges": true, "faces": [top], "distance": 1 }),
    );
    assert!(
        both.contains("which takes every edge of the body"),
        "{both}"
    );
    let negative = client.tool_error(
        "add_fillet",
        json!({ "body": block, "all_edges": true, "radius": -1 }),
    );
    assert!(negative.contains("cannot be negative"), "{negative}");

    let other = box_of(&mut client, [10.0, 10.0, 10.0], [100.0, 0.0], "Cube");
    added(
        &mut client,
        "add_chamfer",
        json!({ "body": other, "all_edges": true, "distance": "1 mm", "angle": "45 deg" }),
    );
    client.finish();
}

#[test]
fn a_shell_hollows_a_body_and_an_offset_face_moves_one() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [40.0, 30.0, 20.0], [0.0, 0.0], "Block");

    let top = face_where(&mut client, &block, [0.0, 0.0, 1.0]);
    added(
        &mut client,
        "add_offset_face",
        json!({ "body": block, "faces": [top], "distance": "5 mm" }),
    );
    assert!(close(total_volume(&mut client), 40.0 * 30.0 * 25.0));

    let top = face_where(&mut client, &block, [0.0, 0.0, 1.0]);
    let shell = client.tool(
        "add_shell",
        json!({ "body": block, "open_faces": [top], "thickness": "2 mm" }),
    );
    assert_eq!(shell["failing"], json!([]), "{shell}");
    let hollow = 40.0 * 30.0 * 25.0 - 36.0 * 26.0 * 23.0;
    assert!(close(total_volume(&mut client), hollow));
    client.finish();
}

#[test]
fn a_sketch_revolves_about_its_axis_or_a_model_axis() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = client.tool("add_sketch", json!({ "plane": "XZ", "name": "Profile" }));
    let sketch = sketch["sketch"]["id"].clone();
    client.tool(
        "add_sketch_geometry",
        json!({
            "sketch": sketch,
            "items": [{ "kind": "rectangle", "corner": [10, 0], "opposite": [20, 30] }]
        }),
    );

    let ring = client.tool(
        "add_revolve",
        json!({ "sketch": sketch, "axis": "Z", "name": "Ring" }),
    );
    assert_eq!(ring["operation"], "makes a new body", "{ring}");
    assert_eq!(ring["failing"], json!([]));
    assert!(close(
        total_volume(&mut client),
        PI * (400.0 - 100.0) * 30.0
    ));

    client.tool("undo", json!({}));
    added(
        &mut client,
        "add_revolve",
        json!({ "sketch": sketch, "axis": "vertical_axis", "angle": "90 deg" }),
    );
    assert!(close(total_volume(&mut client), PI * 300.0 * 30.0 / 4.0));

    let refused = client.tool_error("add_revolve", json!({ "sketch": sketch, "axis": 999 }));
    assert!(refused.contains("has no entity 999"), "{refused}");
    client.finish();
}

#[test]
fn primitives_take_their_own_sizes_on_planes_faces_and_datum_planes() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);

    let cylinder = client.tool(
        "add_primitive",
        json!({ "kind": "cylinder", "diameter": 20, "height": "10 mm", "name": "Disc" }),
    );
    assert_eq!(cylinder["operation"], "makes a new body", "{cylinder}");
    let disc = cylinder["feature"]["id"].clone();
    assert!(close(total_volume(&mut client), PI * 100.0 * 10.0));

    let top = face_where(&mut client, &disc, [0.0, 0.0, 1.0]);
    let boss = client.tool(
        "add_primitive",
        json!({ "kind": "box", "length": 4, "width": 4, "height": 5, "plane": { "body": disc, "face": top } }),
    );
    assert_eq!(boss["operation"], "adds to Disc", "{boss}");
    assert!(close(total_volume(&mut client), PI * 1000.0 + 80.0));

    let refused = client.tool_error("add_primitive", json!({ "kind": "sphere", "height": 3 }));
    assert!(
        refused.contains("Sphere takes diameter, not height"),
        "{refused}"
    );

    let datum = added(
        &mut client,
        "add_datum_plane",
        json!({ "base": "XY", "offset": "50 mm", "name": "Shelf" }),
    );
    let lifted = client.tool(
        "add_primitive",
        json!({ "kind": "sphere", "diameter": 10, "plane": { "datum": datum } }),
    );
    assert_eq!(lifted["failing"], json!([]), "{lifted}");
    let bodies = client.tool("list_bodies", json!({}));
    let sphere = bodies["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|body| close(number(&body["volume"]), PI * 1000.0 / 6.0))
        .unwrap_or_else(|| panic!("no sphere: {bodies}"));
    assert!(
        close(number(&sphere["centre_of_mass"][2]), 50.0),
        "{sphere}"
    );
    client.finish();
}

#[test]
fn mirrors_and_patterns_repeat_a_body() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let cube = box_of(&mut client, [10.0, 10.0, 10.0], [10.0, 0.0], "Cube");

    added(
        &mut client,
        "add_mirror",
        json!({ "body": cube, "plane": "YZ" }),
    );
    assert!(close(total_volume(&mut client), 2000.0));
    client.tool("undo", json!({}));

    added(
        &mut client,
        "add_linear_pattern",
        json!({
            "body": cube, "axis": "X", "count": 3, "spacing": "20 mm",
            "second": { "axis": "Y", "count": 2, "spacing": 20 }
        }),
    );
    assert!(close(total_volume(&mut client), 6000.0));
    client.tool("undo", json!({}));

    let axis = added(
        &mut client,
        "add_datum_axis",
        json!({ "along": "Z", "name": "Spindle" }),
    );
    added(
        &mut client,
        "add_circular_pattern",
        json!({ "body": cube, "axis": { "datum": axis }, "count": 4 }),
    );
    assert!(close(total_volume(&mut client), 4000.0));

    let refused = client.tool_error("add_circular_pattern", json!({ "axis": "Z", "count": 2 }));
    assert!(refused.contains("Give the body to repeat"), "{refused}");
    let not_repeatable =
        client.tool_error("add_mirror", json!({ "features": [axis], "plane": "XY" }));
    assert!(
        not_repeatable.contains("cannot be repeated"),
        "{not_repeatable}"
    );
    client.finish();
}

#[test]
fn bodies_are_combined_and_moved() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [20.0, 20.0, 20.0], [0.0, 0.0], "Block");
    let cutter = box_of(&mut client, [10.0, 10.0, 30.0], [5.0, 5.0], "Cutter");
    assert_eq!(body_count(&mut client), 2);

    let combined = client.tool(
        "add_combine",
        json!({ "body": block, "tools": [cutter], "operation": "cut" }),
    );
    assert_eq!(combined["failing"], json!([]), "{combined}");
    assert_eq!(body_count(&mut client), 1);
    assert!(close(total_volume(&mut client), 8000.0 - 2000.0));

    let itself = client.tool_error(
        "add_combine",
        json!({ "body": block, "tools": [block], "operation": "join" }),
    );
    assert!(
        itself.contains("cannot be combined with itself"),
        "{itself}"
    );

    added(
        &mut client,
        "add_move",
        json!({ "body": block, "offset": [5, 0, "-2 mm"], "copy": true }),
    );
    assert_eq!(body_count(&mut client), 2);
    let bodies = client.tool("list_bodies", json!({}));
    let lowest = bodies["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|body| number(&body["bounding_box"]["min"][2]))
        .fold(f64::INFINITY, f64::min);
    assert!(close(lowest, -2.0), "{bodies}");

    let half = client.tool_error("add_move", json!({ "body": block, "turn_axis": "Z" }));
    assert!(
        half.contains("takes both turn_axis and turn_angle"),
        "{half}"
    );
    client.finish();
}

#[test]
fn features_are_renamed_suppressed_reordered_and_deleted() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [20.0, 20.0, 20.0], [0.0, 0.0], "Block");
    let fillet = added(
        &mut client,
        "add_fillet",
        json!({ "body": block, "all_edges": true, "radius": 1 }),
    );
    let cube = box_of(&mut client, [5.0, 5.0, 5.0], [50.0, 0.0], "Cube");

    let renamed = client.tool(
        "rename_feature",
        json!({ "feature": fillet, "name": "Soft edges" }),
    );
    assert_eq!(renamed["change"], "Rename Fillet 1", "{renamed}");
    let features = client.tool("list_features", json!({}));
    assert_eq!(features["features"][1]["name"], "Soft edges", "{features}");

    let suppressed = client.tool("suppress_features", json!({ "features": ["Soft edges"] }));
    assert_eq!(
        suppressed["change"], "Suppress “Soft edges”",
        "{suppressed}"
    );
    assert!(close(total_volume(&mut client), 8000.0 + 125.0));
    client.tool(
        "suppress_features",
        json!({ "features": ["Soft edges"], "suppressed": false }),
    );
    assert!(total_volume(&mut client) < 8000.0 + 125.0);

    let refused = client.tool_error(
        "move_feature",
        json!({ "feature": fillet, "before": block }),
    );
    assert!(refused.contains("Soft edges cannot go there"), "{refused}");
    let moved = client.tool("move_feature", json!({ "feature": cube, "before": fillet }));
    assert_eq!(moved["tree"][1]["name"], "Cube", "{moved}");

    let dependents = client.tool_error("delete_features", json!({ "features": [block] }));
    assert!(
        dependents.contains("“Soft edges” depend on “Block”"),
        "{dependents}"
    );
    let deleted = client.tool(
        "delete_features",
        json!({ "features": [block], "dependents": "delete" }),
    );
    assert_eq!(
        deleted["change"], "Delete “Block” and its dependents",
        "{deleted}"
    );
    assert_eq!(body_count(&mut client), 1);
    client.tool("undo", json!({}));
    assert_eq!(body_count(&mut client), 2);

    let kept = client.tool(
        "delete_features",
        json!({ "features": [block], "dependents": "keep" }),
    );
    assert_eq!(kept["failing"][0]["name"], "Soft edges", "{kept}");
    assert_eq!(kept["failing"][0]["new"], true);
    client.finish();
}

#[test]
fn a_shell_gives_chosen_walls_a_thickness_of_their_own() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [40.0, 30.0, 20.0], [0.0, 0.0], "Block");
    let top = face_where(&mut client, &block, [0.0, 0.0, 1.0]);
    let bottom = face_where(&mut client, &block, [0.0, 0.0, -1.0]);

    let shell = added(
        &mut client,
        "add_shell",
        json!({
            "body": block, "open_faces": [top], "thickness": 2,
            "face_thicknesses": [{ "face": bottom, "thickness": "5 mm" }]
        }),
    );
    assert!(close(
        total_volume(&mut client),
        24_000.0 - 36.0 * 26.0 * 15.0
    ));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": shell, "face_thicknesses": [] }),
    );
    assert!(close(
        total_volume(&mut client),
        24_000.0 - 36.0 * 26.0 * 18.0
    ));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": shell, "face_thicknesses": [{ "face": bottom, "thickness": 4 }] }),
    );
    assert!(close(
        total_volume(&mut client),
        24_000.0 - 36.0 * 26.0 * 16.0
    ));
    client.tool("undo", json!({}));
    client.tool("undo", json!({}));
    assert!(close(
        total_volume(&mut client),
        24_000.0 - 36.0 * 26.0 * 15.0
    ));
    client.finish();
}

#[test]
fn a_mirror_reflects_the_chosen_faces_of_a_body() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [40.0, 30.0, 20.0], [-20.0, 0.0], "Block");
    let datum = added(
        &mut client,
        "add_datum_plane",
        json!({ "base": "XY", "offset": 20 }),
    );
    let sketch = client.tool("add_sketch", json!({ "datum": datum }))["sketch"]["id"].clone();
    client.tool(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": [{ "kind": "point", "at": [10, 15] }] }),
    );
    added(
        &mut client,
        "add_hole",
        json!({ "sketch": sketch, "diameter": 4, "depth": 10, "body": block }),
    );
    let drilled = 24_000.0 - PI * 4.0 * 10.0;
    assert!(close(total_volume(&mut client), drilled));

    let faces = client.tool("list_faces", json!({ "body": block }));
    let hole_faces: Vec<Value> = faces["faces"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|face| {
            face["surface"] == "cylinder"
                || (face["surface"] == "plane" && number(&face["area"]) < 20.0)
        })
        .map(|face| face["face"].clone())
        .collect();
    assert_eq!(hole_faces.len(), 2, "{faces}");

    let mirror = added(
        &mut client,
        "add_mirror",
        json!({ "body": block, "faces": hole_faces, "plane": "YZ" }),
    );
    assert!(close(total_volume(&mut client), drilled - PI * 4.0 * 10.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": mirror, "faces": [] }),
    );
    assert!(close(total_volume(&mut client), 24_000.0));

    let both = client.tool_error(
        "add_mirror",
        json!({ "body": block, "faces": hole_faces, "features": [mirror], "plane": "YZ" }),
    );
    assert!(both.contains("not both"), "{both}");
    client.finish();
}

#[test]
fn a_fillet_takes_the_convex_edges_or_the_edges_a_feature_made_by_rule() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [40.0, 30.0, 20.0], [0.0, 0.0], "Block");
    let whole = 40.0 * 30.0 * 20.0;

    let all = added(
        &mut client,
        "add_fillet",
        json!({ "body": block, "all_edges": true, "radius": 1 }),
    );
    let by_all = total_volume(&mut client);
    client.tool("delete_features", json!({ "features": [all] }));
    assert!(close(total_volume(&mut client), whole));

    added(
        &mut client,
        "add_fillet",
        json!({ "body": block, "convex_edges": true, "radius": 1 }),
    );
    assert!(close(total_volume(&mut client), by_all));

    let both = client.tool_error(
        "add_chamfer",
        json!({ "body": block, "all_edges": true, "convex_edges": true, "distance": 1 }),
    );
    assert!(
        both.contains("which takes every edge of the body"),
        "{both}"
    );
    let unknown = client.tool_error(
        "add_chamfer",
        json!({ "body": block, "edges_made_by": ["Nothing"], "distance": 1 }),
    );
    assert!(unknown.contains("There is no feature"), "{unknown}");
    client.finish();
}
