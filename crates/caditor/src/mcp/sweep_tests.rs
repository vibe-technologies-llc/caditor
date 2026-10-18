use std::f64::consts::PI;

use serde_json::{Value, json};
use tempfile::TempDir;

use super::{
    feature_tests::{added, body_count, box_of, close, number, started, total_volume},
    tests::Client,
};

const NEAR: f64 = 1e-3;

fn near(found: f64, wanted: f64) -> bool {
    (found - wanted).abs() <= NEAR * wanted.abs()
}

fn sketch_on(client: &mut Client, plane: Value, items: Value) -> (Value, Value) {
    let sketch = client.tool("add_sketch", plane)["sketch"]["id"].clone();
    let drawn = client.tool(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": items }),
    );
    assert_eq!(drawn["failing"], json!([]), "{drawn}");
    (sketch, drawn["drawn"].clone())
}

fn faces_of(client: &mut Client, body: &Value) -> Vec<Value> {
    client.tool("list_faces", json!({ "body": body }))["faces"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn pipes_sweeps_and_lofts_follow_paths_and_profiles() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let (path, drawn) = sketch_on(
        &mut client,
        json!({ "plane": "XY" }),
        json!([{ "kind": "line", "from": [0, 0], "to": [50, 0] }]),
    );
    let line = drawn[0]["id"].clone();

    let pipe = added(
        &mut client,
        "add_pipe",
        json!({ "path": { "sketch": path, "curves": [line] }, "size": 10 }),
    );
    let rod = PI * 25.0 * 50.0;
    assert!(near(total_volume(&mut client), rod));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": pipe, "size": 20, "wall": 2 }),
    );
    let tube = PI * (100.0 - 64.0) * 50.0;
    assert!(near(total_volume(&mut client), tube));

    let (profile, _) = sketch_on(
        &mut client,
        json!({ "plane": "YZ" }),
        json!([{ "kind": "circle", "center": [0, 40], "radius": 5 }]),
    );
    let (offset_path, offset_drawn) = sketch_on(
        &mut client,
        json!({ "plane": "XZ" }),
        json!([{ "kind": "line", "from": [0, 40], "to": [50, 40] }]),
    );
    let sweep = client.tool(
        "add_sweep",
        json!({
            "sketch": profile,
            "path": { "sketch": offset_path, "curves": [offset_drawn[0]["id"]] },
            "operation": "new_body"
        }),
    );
    assert_eq!(sweep["failing"], json!([]), "{sweep}");
    assert!(near(total_volume(&mut client), tube + rod));
    let shaped = client.tool(
        "edit_feature",
        json!({ "feature": sweep["feature"]["id"], "twist": "90 deg", "scale": 0.5 }),
    );
    assert_eq!(shaped["failing"], json!([]), "{shaped}");
    assert!(near(total_volume(&mut client), tube + rod * 7.0 / 12.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": sweep["feature"]["id"], "twist": 0, "scale": 1 }),
    );

    let (bottom, _) = sketch_on(
        &mut client,
        json!({ "plane": "XY" }),
        json!([{ "kind": "rectangle", "corner": [100, 0], "opposite": [110, 10] }]),
    );
    let datum = added(
        &mut client,
        "add_datum_plane",
        json!({ "base": "XY", "offset": 20 }),
    );
    let (top, _) = sketch_on(
        &mut client,
        json!({ "datum": datum }),
        json!([{ "kind": "rectangle", "corner": [100, 0], "opposite": [110, 10] }]),
    );
    let loft = added(
        &mut client,
        "add_loft",
        json!({ "profiles": [bottom, top], "skin": "ruled" }),
    );
    assert!(near(total_volume(&mut client), tube + rod + 2000.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": loft, "skin": "smooth", "start_weight": 1, "end_weight": 2 }),
    );
    assert!(near(total_volume(&mut client), tube + rod + 2000.0));
    let ruled_square = client.tool("edit_feature", json!({ "feature": loft, "skin": "ruled" }));
    assert_ne!(ruled_square["failing"], json!([]), "{ruled_square}");
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": loft, "start_weight": 0, "end_weight": 0 }),
    );

    let not_a_curve = client.tool_error(
        "add_pipe",
        json!({ "path": { "sketch": path, "curves": ["origin"] } }),
    );
    assert!(
        not_a_curve.contains("is not a line, arc or curve"),
        "{not_a_curve}"
    );
    let one_profile = client.tool_error("add_loft", json!({ "profiles": [bottom] }));
    assert!(
        one_profile.contains("at least two sketches"),
        "{one_profile}"
    );
    client.finish();
}

#[test]
fn coils_threads_and_scales_are_sized_and_edited() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let coil = added(
        &mut client,
        "add_coil",
        json!({ "diameter": 20, "section": 2, "turns": 3, "pitch": 5 }),
    );
    let three_turns = total_volume(&mut client);
    assert!(three_turns > 0.0);
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": coil, "turns": 6 }),
    );
    let ratio = total_volume(&mut client) / three_turns;
    assert!((1.9..2.1).contains(&ratio), "{ratio}");
    let three = client.tool_error(
        "edit_feature",
        json!({ "feature": coil, "turns": 2, "pitch": 3, "height": 9 }),
    );
    assert!(three.contains("not all three"), "{three}");
    client.tool("undo", json!({}));
    client.tool("undo", json!({}));

    let shaft = added(
        &mut client,
        "add_primitive",
        json!({ "kind": "cylinder", "diameter": 8, "height": 20, "plane": "XY", "at": [100, 0] }),
    );
    let round = faces_of(&mut client, &shaft)
        .into_iter()
        .find(|face| face["surface"] == "cylinder")
        .unwrap();
    let thread = client.tool(
        "add_thread",
        json!({ "body": shaft, "face": round["face"], "form": "cosmetic" }),
    );
    assert_eq!(thread["failing"], json!([]), "{thread}");
    assert!(
        thread["thread"].as_str().unwrap().contains("M8"),
        "{thread}"
    );
    assert!(close(total_volume(&mut client), PI * 16.0 * 20.0));
    let unknown = client.tool_error(
        "edit_feature",
        json!({ "feature": thread["feature"]["id"], "size": "M9" }),
    );
    assert!(unknown.contains("has no size"), "{unknown}");

    let cube = box_of(&mut client, [10.0, 10.0, 10.0], [200.0, 0.0], "Cube");
    let before = total_volume(&mut client);
    let scale = added(
        &mut client,
        "add_scale",
        json!({ "body": cube, "factor": 2, "center": [200, 0, 0] }),
    );
    assert!(close(total_volume(&mut client) - before, 7000.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": scale, "factor": 3 }),
    );
    assert!(close(total_volume(&mut client) - before, 26_000.0));
    client.finish();
}

#[test]
fn bodies_and_faces_are_split_and_faces_deleted() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [10.0, 10.0, 10.0], [-5.0, 0.0], "Block");
    let bodies = body_count(&mut client);
    added(
        &mut client,
        "add_split",
        json!({ "body": block, "along": "YZ" }),
    );
    assert_eq!(body_count(&mut client), bodies + 1);
    assert!(close(total_volume(&mut client), 1000.0));

    let slab = box_of(&mut client, [10.0, 10.0, 10.0], [-5.0, 100.0], "Slab");
    let top = faces_of(&mut client, &slab)
        .into_iter()
        .find(|face| face["normal"] == json!([0.0, 0.0, 1.0]))
        .unwrap();
    added(
        &mut client,
        "add_split_face",
        json!({ "body": slab, "faces": [top["face"]], "along": "YZ" }),
    );
    assert_eq!(faces_of(&mut client, &slab).len(), 7);

    let rounded = box_of(&mut client, [10.0, 10.0, 10.0], [100.0, 0.0], "Rounded");
    let edge = client.tool("list_edges", json!({ "body": rounded }))["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|edge| edge["kind"] == "line" && number(&edge["start"][2]) != number(&edge["end"][2]))
        .unwrap()["edge"]
        .clone();
    added(
        &mut client,
        "add_fillet",
        json!({ "body": rounded, "edges": [edge], "radius": 2 }),
    );
    let filleted = faces_of(&mut client, &rounded)
        .into_iter()
        .find(|face| face["surface"] == "cylinder")
        .unwrap();
    added(
        &mut client,
        "add_delete_face",
        json!({ "body": rounded, "faces": [filleted["face"]] }),
    );
    assert!(close(total_volume(&mut client), 3000.0));

    let refused = client.tool_error("add_delete_face", json!({ "body": rounded, "faces": [] }));
    assert!(refused.contains("faces to delete"), "{refused}");
    client.finish();
}

#[test]
fn faces_are_moved_along_their_normal_or_an_axis() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [10.0, 10.0, 10.0], [-5.0, 0.0], "Block");
    let top = faces_of(&mut client, &block)
        .into_iter()
        .find(|face| face["normal"] == json!([0.0, 0.0, 1.0]))
        .unwrap();

    let moved = added(
        &mut client,
        "add_move_face",
        json!({ "body": block, "faces": [top["face"]], "distance": 2 }),
    );
    assert!(close(total_volume(&mut client), 1200.0));

    added(
        &mut client,
        "edit_feature",
        json!({ "feature": moved, "distance": -3, "direction": "Z" }),
    );
    assert!(close(total_volume(&mut client), 700.0));

    let refused = client.tool_error("add_move_face", json!({ "body": block, "faces": [] }));
    assert!(refused.contains("faces to move"), "{refused}");
    client.finish();
}

#[test]
fn outlines_are_embossed_onto_a_face_and_changed_into_a_recess() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [10.0, 10.0, 10.0], [-5.0, 0.0], "Block");
    let top = faces_of(&mut client, &block)
        .into_iter()
        .find(|face| face["normal"] == json!([0.0, 0.0, 1.0]))
        .unwrap();
    let (label, _) = sketch_on(
        &mut client,
        json!({ "plane": "XY" }),
        json!([{ "kind": "rectangle", "corner": [-3, 2], "opposite": [3, 8] }]),
    );

    let emboss = added(
        &mut client,
        "add_emboss",
        json!({ "body": block, "faces": [top["face"]], "sketch": label, "depth": 1 }),
    );
    assert!(close(total_volume(&mut client), 1036.0));

    added(
        &mut client,
        "edit_feature",
        json!({ "feature": emboss, "form": "recess", "depth": 2 }),
    );
    assert!(close(total_volume(&mut client), 928.0));

    let refused = client.tool_error(
        "add_emboss",
        json!({ "body": block, "faces": [], "sketch": label, "depth": 1 }),
    );
    assert!(refused.contains("faces to emboss"), "{refused}");
    client.finish();
}

fn drafted_block(sides: [f64; 4]) -> f64 {
    let [east, west, north, south] = sides.map(|degrees: f64| degrees.to_radians().tan());
    let area = |z: f64| (10.0 - (east + west) * z) * (10.0 - (north + south) * z);
    10.0 / 6.0 * (area(0.0) + 4.0 * area(5.0) + area(10.0))
}

#[test]
fn walls_are_drafted_from_their_floor_and_one_takes_an_angle_of_its_own() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [10.0, 10.0, 10.0], [-5.0, 0.0], "Block");
    let faces = faces_of(&mut client, &block);
    let wall = |normal: [f64; 3]| {
        faces
            .iter()
            .find(|face| face["normal"] == json!(normal))
            .unwrap()["face"]
            .clone()
    };
    let east = wall([1.0, 0.0, 0.0]);
    let others = [
        wall([-1.0, 0.0, 0.0]),
        wall([0.0, 1.0, 0.0]),
        wall([0.0, -1.0, 0.0]),
    ];

    let drafted = added(
        &mut client,
        "add_draft",
        json!({ "body": block, "faces": [east, others[0], others[1], others[2]], "angle": 5 }),
    );
    assert!(close(total_volume(&mut client), drafted_block([5.0; 4])));

    added(
        &mut client,
        "edit_feature",
        json!({
            "feature": drafted,
            "faces": others,
            "own_angles": [{ "face": east, "angle": "10 deg" }]
        }),
    );
    assert!(close(
        total_volume(&mut client),
        drafted_block([10.0, 5.0, 5.0, 5.0])
    ));

    let refused = client.tool_error("add_draft", json!({ "body": block, "faces": [] }));
    assert!(refused.contains("faces to draft"), "{refused}");
    client.finish();
}

fn rim_sketch(client: &mut Client, body: &Value) -> ([f64; 3], [f64; 3], [f64; 3], Value) {
    let faces = faces_of(client, body);
    let rim = faces
        .iter()
        .find(|face| {
            face["surface"] == "plane"
                && face["normal"] == json!([0.0, 0.0, 1.0])
                && (number(&face["point"][2]) - 40.0).abs() < 1e-9
        })
        .unwrap_or_else(|| panic!("no rim: {faces:?}"))["face"]
        .clone();
    let added = client.tool("add_sketch", json!({ "body": body, "face": rim }));
    assert_eq!(added["failing"], json!([]), "{added}");
    let plane = &added["sketch"]["plane"];
    let vector = |value: &Value| [number(&value[0]), number(&value[1]), number(&value[2])];
    (
        vector(&plane["origin"]),
        vector(&plane["x_axis"]),
        vector(&plane["y_axis"]),
        added["sketch"]["id"].clone(),
    )
}

#[test]
fn a_web_takes_a_draft_and_free_ends_and_edits_them() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [100.0, 60.0, 40.0], [0.0, 0.0], "Box");
    let top = faces_of(&mut client, &block)
        .into_iter()
        .find(|face| face["surface"] == "plane" && face["normal"] == json!([0.0, 0.0, 1.0]))
        .unwrap()["face"]
        .clone();
    added(
        &mut client,
        "add_shell",
        json!({ "body": block, "open_faces": [top], "thickness": "5 mm" }),
    );
    let hollow = 100.0 * 60.0 * 40.0 - 90.0 * 50.0 * 35.0;
    let (origin, x_axis, y_axis, sketch) = rim_sketch(&mut client, &block);
    let local = |x: f64, y: f64| {
        let offset = [x - origin[0], y - origin[1], 40.0 - origin[2]];
        let along = |axis: [f64; 3]| offset.iter().zip(axis).map(|(a, b)| a * b).sum::<f64>();
        [along(x_axis), along(y_axis)]
    };
    let drawn = client.tool(
        "add_sketch_geometry",
        json!({
            "sketch": sketch,
            "items": [{ "kind": "line", "from": local(50.0, 15.0), "to": local(50.0, 45.0) }]
        }),
    );
    assert_eq!(drawn["failing"], json!([]), "{drawn}");
    let widening = 35.0 * 35.0 * 2.0_f64.to_radians().tan();

    let rib = added(
        &mut client,
        "add_rib",
        json!({
            "sketch": sketch,
            "body": block,
            "direction": "square",
            "draft": "2 deg",
            "ends": "round_arcs"
        }),
    );
    let drafted = total_volume(&mut client);
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": rib, "draft": "0 deg", "ends": "straight" }),
    );
    let plain = total_volume(&mut client);
    let steep = client.tool("edit_feature", json!({ "feature": rib, "draft": "30 deg" }));
    client.finish();

    assert!(
        near(drafted, hollow + 50.0 * (2.0 * 35.0 + widening)),
        "{drafted}"
    );
    assert!(near(plain, hollow + 2.0 * 35.0 * 50.0), "{plain}");
    assert_ne!(steep["failing"], json!([]), "{steep}");
}

#[test]
fn lofts_use_a_single_region_of_a_sketch_and_end_in_a_point() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let (bottom, _) = sketch_on(
        &mut client,
        json!({ "plane": "XY" }),
        json!([
            { "kind": "rectangle", "corner": [100, 0], "opposite": [110, 10] },
            { "kind": "rectangle", "corner": [200, 0], "opposite": [220, 10] }
        ]),
    );
    let datum = added(
        &mut client,
        "add_datum_plane",
        json!({ "base": "XY", "offset": 20 }),
    );
    let (top, _) = sketch_on(
        &mut client,
        json!({ "datum": datum }),
        json!([{ "kind": "rectangle", "corner": [100, 0], "opposite": [110, 10] }]),
    );
    let loft = added(
        &mut client,
        "add_loft",
        json!({
            "profiles": [{ "sketch": bottom, "inside": [105, 5] }, top],
            "skin": "ruled"
        }),
    );
    assert!(near(total_volume(&mut client), 2000.0));
    added(
        &mut client,
        "edit_feature",
        json!({
            "feature": loft,
            "profiles": [{ "sketch": bottom, "inside": [205, 5] }, top]
        }),
    );
    let shifted = total_volume(&mut client);
    assert!(near(shifted, 3000.0), "{shifted}");

    let missed = client.tool_error(
        "add_loft",
        json!({ "profiles": [{ "sketch": bottom, "inside": [150, 5] }, top] }),
    );
    assert!(missed.contains("no region around"), "{missed}");

    let apex = added(
        &mut client,
        "add_datum_point",
        json!({ "offset": [105, 5, 40] }),
    );
    let pyramid = added(
        &mut client,
        "add_loft",
        json!({
            "profiles": [{ "sketch": bottom, "inside": [105, 5] }],
            "end": { "datum": apex },
            "skin": "ruled"
        }),
    );
    let drawn = total_volume(&mut client) - shifted;
    assert!(near(drawn, 100.0 * 40.0 / 3.0), "{drawn}");
    added(
        &mut client,
        "edit_feature",
        json!({
            "feature": pyramid,
            "end": "off",
            "profiles": [{ "sketch": bottom, "inside": [105, 5] }, top]
        }),
    );
    assert!(near(total_volume(&mut client) - shifted, 2000.0));

    let closed = client.tool_error(
        "add_loft",
        json!({ "profiles": [bottom, top], "closed": true, "end": { "datum": apex } }),
    );
    assert!(closed.contains("one of closed or end"), "{closed}");
    client.finish();
}

fn circle_centre_x(client: &mut Client, body: &Value) -> f64 {
    let edges = client.tool("list_edges", json!({ "body": body }));
    let circle = edges["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|edge| edge["kind"] == "circle")
        .unwrap_or_else(|| panic!("no circle: {edges}"));
    number(&circle["center"][0])
}

#[test]
fn split_faces_carry_their_curves_along_an_axis_or_wrapped() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [40.0, 40.0, 20.0], [0.0, 0.0], "Block");
    let (circle, _) = sketch_on(
        &mut client,
        json!({ "plane": "XY" }),
        json!([{ "kind": "circle", "center": [15, 20], "radius": 5 }]),
    );
    let top = faces_of(&mut client, &block)
        .into_iter()
        .find(|face| face["normal"] == json!([0.0, 0.0, 1.0]))
        .unwrap();
    let lean = added(
        &mut client,
        "add_datum_point",
        json!({ "offset": [5, 0, 20] }),
    );
    let slant = added(
        &mut client,
        "add_datum_axis",
        json!({ "through_points": ["origin", { "datum": lean }] }),
    );
    let split = added(
        &mut client,
        "add_split_face",
        json!({ "body": block, "faces": [top["face"]], "along": { "sketch": circle }, "carry": "Z" }),
    );
    assert_eq!(faces_of(&mut client, &block).len(), 7);
    assert!(close(circle_centre_x(&mut client, &block), 15.0));

    added(
        &mut client,
        "edit_feature",
        json!({ "feature": split, "carry": { "datum": slant } }),
    );
    assert!(close(circle_centre_x(&mut client, &block), 20.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": split, "carry": "square" }),
    );
    assert!(close(circle_centre_x(&mut client, &block), 15.0));

    let drum = added(
        &mut client,
        "add_primitive",
        json!({
            "kind": "cylinder", "diameter": 20, "height": 30, "plane": "XY", "at": [200, 0],
            "anchor": "base_centre", "operation": "new_body", "name": "Drum"
        }),
    );
    let side = faces_of(&mut client, &drum)
        .into_iter()
        .find(|face| face["surface"] == "cylinder")
        .unwrap();
    let tangent = added(
        &mut client,
        "add_datum_plane",
        json!({ "base": "YZ", "offset": 210 }),
    );
    let (mark, _) = sketch_on(
        &mut client,
        json!({ "datum": tangent }),
        json!([{ "kind": "circle", "center": [0, 15], "radius": 3 }]),
    );
    let before = faces_of(&mut client, &drum).len();
    added(
        &mut client,
        "add_split_face",
        json!({ "body": drum, "faces": [side["face"]], "along": { "sketch": mark }, "carry": "wrapped" }),
    );
    assert!(faces_of(&mut client, &drum).len() > before);

    let any = faces_of(&mut client, &block)[0]["face"].clone();
    let no_sketch = client.tool_error(
        "add_split_face",
        json!({ "body": block, "faces": [any], "along": "YZ", "carry": "wrapped" }),
    );
    assert!(no_sketch.contains("curves of a sketch"), "{no_sketch}");
    client.finish();
}
