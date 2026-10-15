use std::f64::consts::PI;

use serde_json::json;
use tempfile::TempDir;

use super::{
    feature_tests::{close, number, started},
    sketch_edit_tests::{
        assert_area, described, descriptions, draw, entity, extruded_area, modify, near,
        new_sketch, of_kind, position, refused,
    },
};

#[test]
fn circles_touch_two_or_three_curves() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);
    let drawn = draw(
        &mut client,
        &sketch,
        json!([
            { "kind": "line", "from": [0, 0], "to": [30, 0] },
            { "kind": "line", "from": [0, 0], "to": [0, 30] },
            { "kind": "line", "from": [30, 0], "to": [0, 30] }
        ]),
    );
    let lines: Vec<_> = drawn.iter().map(|line| line["id"].clone()).collect();

    let inscribed = modify(
        &mut client,
        &sketch,
        json!({ "operation": "tangent_circle", "curves": lines }),
    );
    let sized = modify(
        &mut client,
        &sketch,
        json!({
            "operation": "tangent_circle",
            "curves": [lines[0], lines[1]],
            "radius": "5 mm",
            "near": [8, 8]
        }),
    );
    let shown = described(&mut client, &sketch);
    let one = refused(
        &mut client,
        &sketch,
        json!({ "operation": "tangent_circle", "curves": [lines[0]] }),
    );
    let open = refused(
        &mut client,
        &sketch,
        json!({ "operation": "tangent_circle", "curves": [lines[0], lines[1]] }),
    );
    client.finish();

    let circles = of_kind(&shown, "circle");
    assert_eq!(circles.len(), 2, "{shown}");
    let wide = (60.0 - 30.0 * 2.0_f64.sqrt()) / 2.0;
    assert!(
        circles
            .iter()
            .any(|circle| close(number(&circle["radius"]), wide)),
        "{shown}"
    );
    let small = circles
        .iter()
        .find(|circle| close(number(&circle["radius"]), 5.0))
        .unwrap();
    assert!(
        near(position(&shown, &small["center"]), [5.0, 5.0]) || small["at"] == json!([5.0, 5.0])
    );
    assert_eq!(inscribed["change"], "Draw tangent circle");
    assert_eq!(count_added(&inscribed), 2, "{inscribed}");
    assert!(
        descriptions(&shown)
            .iter()
            .filter(|description| description.contains("tangent"))
            .count()
            >= 5
    );
    assert!(sized["sketch_status"].is_string());
    assert!(one.contains("two curves at a given radius"), "{one}");
    assert!(open.contains("give its radius"), "{open}");
}

fn count_added(answer: &serde_json::Value) -> usize {
    answer["added_entities"].as_array().unwrap().len()
}

#[test]
fn two_curve_ends_are_blended_by_a_spline() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);
    let drawn = draw(
        &mut client,
        &sketch,
        json!([
            { "kind": "line", "from": [0, 0], "to": [10, 0] },
            { "kind": "line", "from": [30, 10], "to": [40, 10] }
        ]),
    );
    let (left, right) = (&drawn[0], &drawn[1]);

    let tangent = modify(
        &mut client,
        &sketch,
        json!({
            "operation": "blend",
            "first": { "curve": left["id"], "end": left["end"] },
            "second": { "curve": right["id"], "end": right["start"] }
        }),
    );
    let shown = described(&mut client, &sketch);
    client.tool("undo", json!({}));
    let curved = modify(
        &mut client,
        &sketch,
        json!({
            "operation": "blend",
            "first": { "curve": left["id"], "end": left["end"] },
            "second": { "curve": right["id"], "end": right["start"] },
            "continuity": "curvature"
        }),
    );
    let curvature = described(&mut client, &sketch);
    let same = refused(
        &mut client,
        &sketch,
        json!({
            "operation": "blend",
            "first": { "curve": left["id"], "end": left["end"] },
            "second": { "curve": left["id"], "end": left["start"] }
        }),
    );
    let wrong_end = refused(
        &mut client,
        &sketch,
        json!({
            "operation": "blend",
            "first": { "curve": left["id"], "end": right["start"] },
            "second": { "curve": right["id"], "end": right["start"] }
        }),
    );
    client.finish();

    assert_eq!(tangent["change"], "Draw blend curve");
    let splines = of_kind(&shown, "spline");
    assert_eq!(splines.len(), 1, "{shown}");
    assert!(
        descriptions(&shown)
            .iter()
            .filter(|description| description.contains("tangent"))
            .count()
            >= 2
    );
    assert!(count_added(&curved) > count_added(&tangent), "{curved}");
    assert!(
        descriptions(&curvature)
            .iter()
            .any(|description| description.contains("curvature")),
        "{curvature}"
    );
    assert!(same.contains("two different curves"), "{same}");
    assert!(wrong_end.contains("not an end"), "{wrong_end}");
}

#[test]
fn gears_pairs_and_sprockets_are_drawn_where_asked() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);

    let solid = modify(
        &mut client,
        &sketch,
        json!({ "operation": "gear", "teeth": 20, "module": "2 mm" }),
    );
    let solid_area = extruded_area(&mut client, &sketch);
    client.tool("undo", json!({}));
    let bored = modify(
        &mut client,
        &sketch,
        json!({ "operation": "gear", "teeth": 20, "module": "2 mm", "bore": "10 mm" }),
    );
    let bored_area = extruded_area(&mut client, &sketch);
    client.tool("undo", json!({}));
    let pair = modify(
        &mut client,
        &sketch,
        json!({ "operation": "gear", "shape": "pair", "teeth": 20, "meshing_teeth": 30, "at": [5, 5] }),
    );
    let paired = described(&mut client, &sketch);
    client.tool("undo", json!({}));
    let sprocket = modify(
        &mut client,
        &sketch,
        json!({ "operation": "gear", "shape": "sprocket", "teeth": 15, "chain": "10a" }),
    );
    let chain_area = extruded_area(&mut client, &sketch);
    let unknown = refused(
        &mut client,
        &sketch,
        json!({ "operation": "gear", "shape": "sprocket", "chain": "99Z" }),
    );
    let both = refused(
        &mut client,
        &sketch,
        json!({ "operation": "gear", "center": "origin", "at": [1, 1] }),
    );
    let few = refused(
        &mut client,
        &sketch,
        json!({ "operation": "gear", "teeth": 2 }),
    );
    let module = refused(
        &mut client,
        &sketch,
        json!({ "operation": "gear", "module": "0 mm" }),
    );
    client.finish();

    assert_eq!(solid["change"], "Draw spur gear");
    let pitch = PI * 20.0 * 20.0;
    assert!(
        solid_area > 0.8 * pitch && solid_area < 1.2 * pitch,
        "{solid_area}"
    );
    assert_eq!(bored["change"], "Draw spur gear");
    assert_area(solid_area - bored_area, PI * 25.0);
    assert_eq!(pair["change"], "Draw gear pair");
    let centres: Vec<[f64; 2]> = of_kind(&paired, "circle")
        .iter()
        .filter_map(|circle| circle["at"].as_array())
        .map(|at| [number(&at[0]), number(&at[1])])
        .collect();
    assert!(
        centres.iter().any(|at| near(*at, [5.0, 5.0])),
        "{centres:?}"
    );
    assert!(
        centres.iter().any(|at| at[0] > 5.0 && close(at[1], 5.0)),
        "{centres:?}"
    );
    assert_eq!(sprocket["change"], "Draw sprocket");
    assert!(chain_area > 0.0);
    assert!(unknown.contains("roller chain"), "{unknown}");
    assert!(both.contains("not both"), "{both}");
    assert!(few.contains("teeth"), "{few}");
    assert!(module.contains("module"), "{module}");
}

#[test]
fn a_tangent_arc_continues_a_curve_drawn_in_the_same_call() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);

    let drawn = draw(
        &mut client,
        &sketch,
        json!([
            { "kind": "line", "from": [0, 0], "to": [10, 0] },
            { "kind": "tangent_arc", "after_item": 0, "to": [20, 10] },
            { "kind": "tangent_arc", "after_item": 1, "to": [30, 20] }
        ]),
    );
    let shown = described(&mut client, &sketch);
    let polyline = draw(
        &mut client,
        &sketch,
        json!([
            { "kind": "polyline", "points": [[0, -20], [10, -20]] },
            { "kind": "tangent_arc", "after_item": 0, "to": [20, -10] }
        ]),
    );
    let late = client.tool_error(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": [{ "kind": "tangent_arc", "after_item": 3, "to": [1, 1] }] }),
    );
    let neither = client.tool_error(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": [{ "kind": "tangent_arc", "to": [1, 1] }] }),
    );
    let circle = client.tool_error(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": [
            { "kind": "circle", "center": [0, 50], "radius": 2 },
            { "kind": "tangent_arc", "after_item": 0, "to": [5, 55] }
        ] }),
    );
    client.finish();

    assert_eq!(drawn[1]["tangent_to"], drawn[0]["id"]);
    assert_eq!(drawn[2]["tangent_to"], drawn[1]["id"]);
    assert!(near(position(&shown, &drawn[1]["start"]), [10.0, 0.0]));
    assert!(near(position(&shown, &drawn[1]["center"]), [10.0, 10.0]));
    let tangents = descriptions(&shown)
        .into_iter()
        .filter(|description| description.contains("tangent"))
        .count();
    assert_eq!(tangents, 2, "{shown}");
    assert_eq!(polyline[1]["tangent_to"], polyline[0]["lines"][0]);
    assert!(late.contains("not an earlier item"), "{late}");
    assert!(neither.contains("either from_point"), "{neither}");
    assert!(circle.contains("no end"), "{circle}");
    assert_eq!(entity(&shown, &drawn[1]["id"])["kind"], "arc");
}

#[test]
fn a_conics_rho_is_held_by_a_parameter() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);
    client.tool(
        "set_parameter",
        json!({ "name": "fullness", "expression": "0.3" }),
    );

    let drawn = draw(
        &mut client,
        &sketch,
        json!([{ "kind": "conic", "start": [0, 0], "end": [20, 0], "apex": [10, 10], "rho": "fullness" }]),
    );
    let before = described(&mut client, &sketch);
    client.tool(
        "set_parameter",
        json!({ "name": "fullness", "expression": "0.6" }),
    );
    let after = described(&mut client, &sketch);
    let plain = draw(
        &mut client,
        &sketch,
        json!([{ "kind": "conic", "start": [0, 20], "end": [20, 20], "apex": [10, 30], "rho": "0.4" }]),
    );
    let shown = described(&mut client, &sketch);
    let out_of_range = client.tool_error(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": [{ "kind": "conic", "start": [0, 0], "end": [10, 0], "apex": [5, 5], "rho": "fullness * 5" }] }),
    );
    client.finish();

    let id = &drawn[0]["id"];
    assert!(close(number(&entity(&before, id)["rho"]), 0.3));
    assert!(close(number(&entity(&after, id)["rho"]), 0.6));
    assert!(
        descriptions(&after)
            .iter()
            .any(|description| description.contains("rho")),
        "{after}"
    );
    assert!(close(number(&entity(&shown, &plain[0]["id"])["rho"]), 0.4));
    assert_eq!(
        descriptions(&shown)
            .iter()
            .filter(|description| description.contains("rho"))
            .count(),
        1
    );
    assert!(out_of_range.contains("rho"), "{out_of_range}");
}
