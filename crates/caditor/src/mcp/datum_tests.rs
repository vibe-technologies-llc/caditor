use serde_json::{Value, json};
use tempfile::TempDir;

use super::{
    feature_tests::{added, box_of, close, face_where, number, started},
    tests::Client,
};

struct Bounds {
    min: [f64; 3],
    max: [f64; 3],
}

impl Bounds {
    fn touches(&self, axis: usize, at: f64) -> bool {
        close(self.min[axis], at) || close(self.max[axis], at)
    }
}

fn triple(value: &Value) -> [f64; 3] {
    [number(&value[0]), number(&value[1]), number(&value[2])]
}

fn bounds_of(client: &mut Client, body: &Value) -> Bounds {
    let bodies = client.tool("list_bodies", json!({}));
    let found = bodies["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|listed| listed["id"] == *body)
        .unwrap_or_else(|| panic!("no body {body}: {bodies}"))
        .clone();
    Bounds {
        min: triple(&found["bounding_box"]["min"]),
        max: triple(&found["bounding_box"]["max"]),
    }
}

fn probe_plane(client: &mut Client, plane: Value) -> Bounds {
    let probe = added(
        client,
        "add_primitive",
        json!({
            "kind": "box", "length": 1, "width": 1, "height": 1, "plane": plane,
            "at": [0, 0], "anchor": "corner", "operation": "new_body", "name": "Probe"
        }),
    );
    let bounds = bounds_of(client, &probe);
    client.tool("undo", json!({}));
    bounds
}

fn probe_point(client: &mut Client, point: Value) -> [f64; 3] {
    let probe = box_of(client, [1.0, 1.0, 1.0], [300.0, 300.0], "Probe");
    added(
        client,
        "add_mate",
        json!({
            "body": probe,
            "point": { "body": probe, "corner": [300, 300, 0] },
            "to_point": point
        }),
    );
    let bounds = bounds_of(client, &probe);
    client.tool("undo", json!({}));
    client.tool("undo", json!({}));
    bounds.min
}

fn probe_axis(client: &mut Client, axis: Value) -> Bounds {
    let probe = box_of(client, [1.0, 1.0, 1.0], [300.0, 300.0], "Probe");
    added(
        client,
        "add_move",
        json!({ "body": probe, "turn_axis": axis, "turn_angle": 180 }),
    );
    let bounds = bounds_of(client, &probe);
    client.tool("undo", json!({}));
    client.tool("undo", json!({}));
    bounds
}

fn edge_between(client: &mut Client, body: &Value, from: [f64; 3], to: [f64; 3]) -> String {
    let edges = client.tool("list_edges", json!({ "body": body }));
    let found = edges["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|edge| {
            let ends = (triple(&edge["start"]), triple(&edge["end"]));
            edge["kind"] == "line" && (ends == (from, to) || ends == (to, from))
        })
        .unwrap_or_else(|| panic!("no edge from {from:?} to {to:?}: {edges}"));
    found["edge"].as_str().unwrap().to_owned()
}

fn corner(body: &Value, at: [f64; 3]) -> Value {
    json!({ "body": body, "corner": at })
}

#[test]
fn datum_planes_are_placed_every_way_and_edited() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [40.0, 30.0, 20.0], [0.0, 0.0], "Block");
    let top = face_where(&mut client, &block, [0.0, 0.0, 1.0]);
    let along_x = edge_between(&mut client, &block, [0.0, 0.0, 0.0], [40.0, 0.0, 0.0]);
    let along_y = edge_between(&mut client, &block, [0.0, 0.0, 0.0], [0.0, 30.0, 0.0]);
    let upright = edge_between(&mut client, &block, [40.0, 0.0, 0.0], [40.0, 0.0, 20.0]);

    let three = added(
        &mut client,
        "add_datum_plane",
        json!({ "through_points": [
            corner(&block, [0.0, 0.0, 20.0]),
            corner(&block, [40.0, 0.0, 20.0]),
            corner(&block, [0.0, 30.0, 20.0])
        ] }),
    );
    assert!(probe_plane(&mut client, json!({ "datum": three })).touches(2, 20.0));
    let midway = added(
        &mut client,
        "add_datum_plane",
        json!({ "midway": ["XY", { "body": block, "face": top }] }),
    );
    assert!(probe_plane(&mut client, json!({ "datum": midway })).touches(2, 10.0));
    let holding = added(
        &mut client,
        "add_datum_plane",
        json!({ "through_axis": { "body": block, "edge": upright }, "point": corner(&block, [0.0, 0.0, 0.0]) }),
    );
    assert!(probe_plane(&mut client, json!({ "datum": holding })).touches(1, 0.0));
    let square = added(
        &mut client,
        "add_datum_plane",
        json!({ "square_to_axis": "Z", "point": corner(&block, [0.0, 0.0, 20.0]) }),
    );
    assert!(probe_plane(&mut client, json!({ "datum": square })).touches(2, 20.0));
    let lines = added(
        &mut client,
        "add_datum_plane",
        json!({ "lines": [{ "body": block, "edge": along_x }, { "body": block, "edge": along_y }] }),
    );
    assert!(probe_plane(&mut client, json!({ "datum": lines })).touches(2, 0.0));
    let station = added(
        &mut client,
        "add_datum_plane",
        json!({ "square_to_edge": { "body": block, "edge": along_x }, "distance": 15 }),
    );
    let stationed = probe_plane(&mut client, json!({ "datum": station }));
    assert!(stationed.touches(0, 15.0) || stationed.touches(0, 25.0));

    let cylinder = added(
        &mut client,
        "add_primitive",
        json!({ "kind": "cylinder", "diameter": 10, "height": 10, "plane": "XY", "at": [100, 0], "anchor": "base_centre", "operation": "new_body", "name": "Roller" }),
    );
    let faces = client.tool("list_faces", json!({ "body": cylinder }));
    let round = faces["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|face| face["surface"] == "cylinder")
        .unwrap()["face"]
        .clone();
    let beside = added(
        &mut client,
        "add_datum_point",
        json!({ "offset": [120, 0, 5] }),
    );
    let tangent = added(
        &mut client,
        "add_datum_plane",
        json!({ "tangent_to": { "body": cylinder, "face": round }, "point": { "datum": beside } }),
    );
    assert!(probe_plane(&mut client, json!({ "datum": tangent })).touches(0, 105.0));

    let edited = client.tool(
        "edit_feature",
        json!({ "feature": three, "base": "XY", "offset": 5 }),
    );
    assert_eq!(edited["failing"], json!([]), "{edited}");
    assert!(probe_plane(&mut client, json!({ "datum": three })).touches(2, 5.0));
    client.tool("undo", json!({}));
    assert!(probe_plane(&mut client, json!({ "datum": three })).touches(2, 20.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": square, "point": corner(&block, [0.0, 0.0, 0.0]) }),
    );
    assert!(probe_plane(&mut client, json!({ "datum": square })).touches(2, 0.0));

    let two_ways = client.tool_error(
        "add_datum_plane",
        json!({ "base": "XY", "midway": ["XY", "XZ"] }),
    );
    assert!(two_ways.contains("one way"), "{two_ways}");
    let stray = client.tool_error(
        "add_datum_plane",
        json!({ "base": "XY", "point": "origin" }),
    );
    assert!(stray.contains("goes with"), "{stray}");
    let flat = client.tool_error(
        "add_datum_plane",
        json!({ "tangent_to": { "body": block, "face": top }, "point": "origin" }),
    );
    assert!(flat.contains("is flat"), "{flat}");
    let no_corner = client.tool_error(
        "add_datum_plane",
        json!({ "through_axis": "Z", "point": corner(&block, [1.0, 2.0, 3.0]) }),
    );
    assert!(no_corner.contains("has no corner"), "{no_corner}");
    client.finish();
}

#[test]
fn datum_axes_points_and_coordinate_systems_are_placed_and_edited() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [40.0, 30.0, 20.0], [0.0, 0.0], "Block");
    let top = face_where(&mut client, &block, [0.0, 0.0, 1.0]);
    let along_x = edge_between(&mut client, &block, [0.0, 0.0, 0.0], [40.0, 0.0, 0.0]);
    let along_y = edge_between(&mut client, &block, [0.0, 0.0, 0.0], [0.0, 30.0, 0.0]);

    let offset = added(
        &mut client,
        "add_datum_point",
        json!({ "at": "origin", "offset": [50, 5, 0] }),
    );
    assert_eq!(
        probe_point(&mut client, json!({ "datum": offset })),
        [50.0, 5.0, 0.0]
    );
    let crossing = added(
        &mut client,
        "add_datum_point",
        json!({ "lines": [{ "body": block, "edge": along_x }, { "body": block, "edge": along_y }] }),
    );
    assert_eq!(
        probe_point(&mut client, json!({ "datum": crossing })),
        [0.0; 3]
    );
    let piercing = added(
        &mut client,
        "add_datum_point",
        json!({ "axis": "Z", "plane": { "body": block, "face": top } }),
    );
    assert_eq!(
        probe_point(&mut client, json!({ "datum": piercing })),
        [0.0, 0.0, 20.0]
    );
    let meeting = added(
        &mut client,
        "add_datum_point",
        json!({ "planes": ["YZ", "XZ", { "body": block, "face": top }] }),
    );
    assert_eq!(
        probe_point(&mut client, json!({ "datum": meeting })),
        [0.0, 0.0, 20.0]
    );
    let middle = added(
        &mut client,
        "add_datum_point",
        json!({ "middle_of": { "body": block, "edge": along_x } }),
    );
    assert_eq!(
        probe_point(&mut client, json!({ "datum": middle })),
        [20.0, 0.0, 0.0]
    );
    let centre = added(
        &mut client,
        "add_datum_point",
        json!({ "centre_of": { "body": block, "face": top } }),
    );
    assert_eq!(
        probe_point(&mut client, json!({ "datum": centre })),
        [20.0, 15.0, 20.0]
    );
    let along = added(
        &mut client,
        "add_datum_point",
        json!({ "along_edge": { "body": block, "edge": along_x }, "distance": 10 }),
    );
    let stationed = probe_point(&mut client, json!({ "datum": along }));
    assert!(
        stationed == [10.0, 0.0, 0.0] || stationed == [30.0, 0.0, 0.0],
        "{stationed:?}"
    );
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": offset, "offset": [60, 5, 0] }),
    );
    assert_eq!(
        probe_point(&mut client, json!({ "datum": offset })),
        [60.0, 5.0, 0.0]
    );

    let through = added(
        &mut client,
        "add_datum_axis",
        json!({ "through_points": [corner(&block, [40.0, 0.0, 0.0]), corner(&block, [40.0, 0.0, 20.0])] }),
    );
    let turned = probe_axis(&mut client, json!({ "datum": through }));
    assert!(close(turned.min[0], -221.0) && close(turned.max[0], -220.0));
    let square = added(
        &mut client,
        "add_datum_axis",
        json!({ "square_to_plane": "XY", "point": { "datum": offset } }),
    );
    let turned = probe_axis(&mut client, json!({ "datum": square }));
    assert!(close(turned.min[0], -181.0) && close(turned.min[1], -291.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": square, "point": "origin" }),
    );
    let turned = probe_axis(&mut client, json!({ "datum": square }));
    assert!(close(turned.min[0], -301.0) && close(turned.min[1], -301.0));
    let refused = client.tool_error(
        "add_datum_axis",
        json!({ "along": "Z", "planes": ["XY", "XZ"] }),
    );
    assert!(refused.contains("one way"), "{refused}");

    let frame = added(
        &mut client,
        "add_coordinate_system",
        json!({ "origin": { "datum": offset }, "plane": "XY" }),
    );
    let on_frame = probe_plane(&mut client, json!({ "frame": frame, "plane": "XY" }));
    assert_eq!(on_frame.min, [60.0, 5.0, 0.0]);
    assert_eq!(
        probe_point(&mut client, json!({ "datum": frame })),
        [60.0, 5.0, 0.0]
    );
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": frame, "origin": { "datum": centre } }),
    );
    let on_frame = probe_plane(&mut client, json!({ "frame": frame, "plane": "XY" }));
    assert_eq!(on_frame.min, [20.0, 15.0, 20.0]);
    let turned = probe_axis(&mut client, json!({ "frame": frame, "axis": "Z" }));
    assert!(close(turned.min[0], -261.0) && close(turned.min[1], -271.0));
    client.finish();
}

#[test]
fn mates_move_bodies_onto_faces_axes_and_points_and_keep_their_kind() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let base = box_of(&mut client, [40.0, 30.0, 20.0], [0.0, 0.0], "Base");
    let peg = box_of(&mut client, [10.0, 10.0, 10.0], [100.0, 100.0], "Peg");
    let top = face_where(&mut client, &base, [0.0, 0.0, 1.0]);
    let bottom = face_where(&mut client, &peg, [0.0, 0.0, -1.0]);

    let flush = client.tool(
        "add_mate",
        json!({ "body": peg, "face": bottom, "to_plane": { "body": base, "face": top } }),
    );
    assert_eq!(flush["failing"], json!([]), "{flush}");
    assert_eq!(flush["mate"], "flush", "{flush}");
    let mate = flush["feature"]["id"].clone();
    let rested = bounds_of(&mut client, &peg);
    assert!(close(rested.min[2], 20.0) && close(rested.max[2], 30.0));
    added(
        &mut client,
        "edit_feature",
        json!({ "feature": mate, "distance": 5 }),
    );
    let lifted = bounds_of(&mut client, &peg);
    assert!(close(lifted.min[2], 25.0), "{:?}", lifted.min);
    client.tool("undo", json!({}));
    assert!(close(bounds_of(&mut client, &peg).min[2], 20.0));
    let kind = client.tool_error(
        "edit_feature",
        json!({ "feature": mate, "axis": "Z", "to_axis": "Z" }),
    );
    assert!(
        kind.contains("must lie on") || kind.contains("keeps its kind"),
        "{kind}"
    );

    let pin = added(
        &mut client,
        "add_primitive",
        json!({ "kind": "cylinder", "diameter": 6, "height": 10, "plane": "XY", "at": [200, 50], "anchor": "base_centre", "operation": "new_body", "name": "Pin" }),
    );
    let faces = client.tool("list_faces", json!({ "body": pin }));
    let round = faces["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|face| face["surface"] == "cylinder")
        .unwrap()["face"]
        .clone();
    let concentric = client.tool(
        "add_mate",
        json!({ "body": pin, "axis": { "body": pin, "face": round }, "to_axis": "Z" }),
    );
    assert_eq!(concentric["mate"], "concentric", "{concentric}");
    let centred = bounds_of(&mut client, &pin);
    assert!(close(centred.min[0], -3.0) && close(centred.max[1], 3.0));
    let kept = client.tool_error(
        "edit_feature",
        json!({ "feature": concentric["feature"]["id"], "angle": 30 }),
    );
    assert!(kept.contains("keeps its kind"), "{kept}");

    let itself = client.tool_error(
        "add_mate",
        json!({ "body": peg, "face": bottom, "to_plane": { "body": peg, "face": bottom } }),
    );
    assert!(itself.contains("not onto itself"), "{itself}");
    let elsewhere = client.tool_error(
        "add_mate",
        json!({ "body": pin, "axis": "Z", "to_axis": "X" }),
    );
    assert!(elsewhere.contains("must lie on"), "{elsewhere}");
    let nothing = client.tool_error("add_mate", json!({ "body": pin }));
    assert!(nothing.contains("what to mate"), "{nothing}");
    client.finish();
}
