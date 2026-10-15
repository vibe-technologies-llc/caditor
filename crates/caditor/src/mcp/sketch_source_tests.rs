use serde_json::{Value, json};
use tempfile::TempDir;

use super::{
    feature_tests::{added, box_of, face_where, started},
    sketch_edit_tests::{
        assert_area, count_of, described, draw, extruded_area, modify, near, new_sketch, of_kind,
        position, refused,
    },
};

fn corner(body: &Value, at: [f64; 3]) -> Value {
    json!({ "body": body, "corner": at })
}

fn projected(sketch: &Value) -> Vec<&Value> {
    sketch["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entity| entity["projected"] == true)
        .collect()
}

#[test]
fn corners_axes_datums_the_origin_and_other_sketches_are_projected() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let early = new_sketch(&mut client);
    let lines = draw(
        &mut client,
        &early,
        json!([
            { "kind": "line", "from": [0, 0], "to": [10, 0] },
            { "kind": "line", "from": [0, 5], "to": [10, 5] }
        ]),
    );
    let block = box_of(&mut client, [40.0, 30.0, 20.0], [0.0, 0.0], "Block");
    let datum_axis = added(
        &mut client,
        "add_datum_axis",
        json!({ "through_points": [corner(&block, [40.0, 0.0, 0.0]), corner(&block, [40.0, 0.0, 20.0])] }),
    );
    let datum_point = added(
        &mut client,
        "add_datum_point",
        json!({ "offset": [5, 5, 20] }),
    );
    let datum_plane = added(
        &mut client,
        "add_datum_plane",
        json!({ "base": "XY", "offset": 5 }),
    );
    let top = face_where(&mut client, &block, [0.0, 0.0, 1.0]);
    let sketch =
        client.tool("add_sketch", json!({ "body": block, "face": top }))["sketch"]["id"].clone();

    let corners = modify(
        &mut client,
        &sketch,
        json!({ "operation": "project", "body": block, "corners": [[0, 0, 20], [40, 30, 20]] }),
    );
    let axes = modify(
        &mut client,
        &sketch,
        json!({ "operation": "project", "axes": ["X", "Z"] }),
    );
    let origin = modify(
        &mut client,
        &sketch,
        json!({ "operation": "project", "origin": true }),
    );
    let datums = modify(
        &mut client,
        &sketch,
        json!({ "operation": "project", "datums": [datum_axis, datum_point] }),
    );
    let one_curve = modify(
        &mut client,
        &sketch,
        json!({ "operation": "project", "source_sketch": early, "source_curves": [lines[0]["id"]] }),
    );
    let rest = modify(
        &mut client,
        &sketch,
        json!({ "operation": "project", "source_sketch": early }),
    );
    let shown = described(&mut client, &sketch);
    client.tool("edit_feature", json!({ "feature": "Block", "length": 50 }));
    let followed = described(&mut client, &sketch);

    let again = refused(
        &mut client,
        &sketch,
        json!({ "operation": "project", "origin": true }),
    );
    let nothing = refused(&mut client, &sketch, json!({ "operation": "project" }));
    let no_body = refused(
        &mut client,
        &sketch,
        json!({ "operation": "project", "corners": [[0, 0, 20]] }),
    );
    let no_corner = refused(
        &mut client,
        &sketch,
        json!({ "operation": "project", "body": block, "corners": [[1, 2, 3]] }),
    );
    let no_sketch = refused(
        &mut client,
        &sketch,
        json!({ "operation": "project", "source_curves": [lines[0]["id"]] }),
    );
    let plane = refused(
        &mut client,
        &sketch,
        json!({ "operation": "project", "datums": [datum_plane] }),
    );
    let later = refused(
        &mut client,
        &early,
        json!({ "operation": "project", "body": block, "corners": [[0, 0, 20]] }),
    );
    client.finish();

    assert_eq!(count_of(&corners, "projected"), 2, "{corners}");
    assert_eq!(count_of(&axes, "projected"), 2, "{axes}");
    assert_eq!(count_of(&origin, "projected"), 1, "{origin}");
    assert_eq!(count_of(&datums, "projected"), 2, "{datums}");
    assert_eq!(count_of(&one_curve, "projected"), 1, "{one_curve}");
    assert_eq!(count_of(&rest, "projected"), 1, "{rest}");
    assert_eq!(corners["change"], "Project 2 curves and points");
    assert_eq!(projected(&shown).len(), 15, "{shown}");
    assert_eq!(of_kind(&shown, "line").len(), 3, "{shown}");
    let at = |sketch: &Value, id: &Value| position(sketch, id);
    let first = &corners["projected"][1];
    assert_ne!(
        at(&shown, first),
        at(&followed, first),
        "{shown} {followed}"
    );
    assert!(again.contains("already projected"), "{again}");
    assert!(nothing.contains("something to project"), "{nothing}");
    assert!(no_body.contains("give body"), "{no_body}");
    assert!(no_corner.contains("no corner"), "{no_corner}");
    assert!(no_sketch.contains("source_sketch"), "{no_sketch}");
    assert!(plane.contains("not a datum axis or point"), "{plane}");
    assert!(later.contains("made after"), "{later}");
}

#[test]
fn a_body_a_face_and_planes_are_intersected_with_the_sketch() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let block = box_of(&mut client, [40.0, 30.0, 20.0], [0.0, 0.0], "Block");
    let middle = added(
        &mut client,
        "add_datum_plane",
        json!({ "base": "XY", "offset": 10 }),
    );
    let upright = added(
        &mut client,
        "add_datum_plane",
        json!({ "base": "YZ", "offset": 20 }),
    );
    let side = face_where(&mut client, &block, [1.0, 0.0, 0.0]);
    let top = face_where(&mut client, &block, [0.0, 0.0, 1.0]);
    let sketch = client.tool("add_sketch", json!({ "datum": middle }))["sketch"]["id"].clone();

    let one_face = modify(
        &mut client,
        &sketch,
        json!({ "operation": "intersect", "body": block, "face": side }),
    );
    let rest = modify(
        &mut client,
        &sketch,
        json!({ "operation": "intersect", "body": block }),
    );
    let by_plane = modify(
        &mut client,
        &sketch,
        json!({ "operation": "intersect", "plane": "XZ" }),
    );
    let by_datum = modify(
        &mut client,
        &sketch,
        json!({ "operation": "intersect", "datum": upright }),
    );
    let shown = described(&mut client, &sketch);

    let again = refused(
        &mut client,
        &sketch,
        json!({ "operation": "intersect", "body": block }),
    );
    let misses = refused(
        &mut client,
        &sketch,
        json!({ "operation": "intersect", "body": block, "face": top }),
    );
    let parallel = refused(
        &mut client,
        &sketch,
        json!({ "operation": "intersect", "plane": "XY" }),
    );
    let nothing = refused(&mut client, &sketch, json!({ "operation": "intersect" }));
    let two_ways = refused(
        &mut client,
        &sketch,
        json!({ "operation": "intersect", "body": block, "plane": "XZ" }),
    );
    let face_alone = refused(
        &mut client,
        &sketch,
        json!({ "operation": "intersect", "face": side }),
    );
    client.tool("undo", json!({}));
    let undone = described(&mut client, &sketch);
    client.finish();

    assert!(
        one_face["change"]
            .as_str()
            .unwrap()
            .starts_with("Intersect ")
    );
    assert_eq!(
        of_kind(&shown, "line")
            .iter()
            .filter(|line| line["projected"] == true)
            .count(),
        4 + 2,
        "{shown}"
    );
    assert_eq!(
        of_kind(&shown, "line")
            .iter()
            .filter(|line| line["construction"] == true)
            .count(),
        2,
        "{shown}"
    );
    assert!(count_of(&one_face, "added_entities") >= 1);
    assert!(count_of(&rest, "added_entities") >= 3);
    assert_eq!(by_plane["change"], "Intersect the XZ plane");
    assert!(
        by_datum["change"]
            .as_str()
            .unwrap()
            .starts_with("Intersect ")
    );
    assert!(again.contains("already drawn"), "{again}");
    assert!(misses.contains("does not cut that face"), "{misses}");
    assert!(parallel.contains("parallel"), "{parallel}");
    assert!(nothing.contains("with one of"), "{nothing}");
    assert!(two_ways.contains("with one of"), "{two_ways}");
    assert!(face_alone.contains("with one of"), "{face_alone}");
    assert_eq!(
        of_kind(&undone, "line").len() + 1,
        of_kind(&shown, "line").len()
    );
}

#[test]
fn geometry_is_copied_within_a_sketch_and_into_another() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);
    let drawn = draw(
        &mut client,
        &sketch,
        json!([{ "kind": "rectangle", "corner": [0, 0], "opposite": [10, 6] }]),
    );
    let sides = drawn[0]["sides"].clone();

    let shifted = modify(
        &mut client,
        &sketch,
        json!({ "operation": "copy", "items": sides }),
    );
    let after_shift = described(&mut client, &sketch);
    client.tool("undo", json!({}));
    let by = modify(
        &mut client,
        &sketch,
        json!({ "operation": "copy", "items": sides, "by": [20, 0] }),
    );
    let two = extruded_area(&mut client, &sketch);
    client.tool("undo", json!({}));
    let to = modify(
        &mut client,
        &sketch,
        json!({ "operation": "copy", "items": sides, "to": [50, 50] }),
    );
    let placed = described(&mut client, &sketch);
    client.tool("undo", json!({}));

    let other = new_sketch(&mut client);
    let into = modify(
        &mut client,
        &other,
        json!({ "operation": "copy", "from_sketch": sketch, "items": sides }),
    );
    let in_place = extruded_area(&mut client, &other);
    let both = refused(
        &mut client,
        &sketch,
        json!({ "operation": "copy", "items": sides, "by": [1, 1], "to": [2, 2] }),
    );
    let reference = refused(
        &mut client,
        &sketch,
        json!({ "operation": "copy", "items": ["origin"] }),
    );
    let stranger = refused(
        &mut client,
        &sketch,
        json!({ "operation": "copy", "from_sketch": "No such sketch", "items": sides }),
    );
    client.finish();

    assert_eq!(shifted["offset"], json!([2.5, 2.5]), "{shifted}");
    assert_eq!(of_kind(&after_shift, "line").len(), 8);
    assert!(
        shifted["change"]
            .as_str()
            .unwrap()
            .starts_with("Paste 4 curves")
    );
    assert_eq!(count_of(&by, "pasted"), 4);
    assert_area(two, 120.0);
    assert_eq!(count_of(&to, "pasted"), 4);
    let pasted_points: Vec<[f64; 2]> = to["pasted"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|id| {
            placed["entities"]
                .as_array()?
                .iter()
                .find(|entity| &entity["id"] == id)
        })
        .filter(|entity| entity["kind"] == "line")
        .map(|line| {
            let start = position(&placed, &line["start"]);
            let end = position(&placed, &line["end"]);
            [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0]
        })
        .collect();
    assert!(
        pasted_points
            .iter()
            .any(|middle| near(*middle, [50.0, 47.0])),
        "{pasted_points:?}"
    );
    assert_eq!(into["offset"], json!([0.0, 0.0]));
    assert_area(in_place, 60.0);
    assert!(both.contains("not both"), "{both}");
    assert!(reference.contains("cannot be copied"), "{reference}");
    assert!(stranger.contains("No such sketch"), "{stranger}");
}
