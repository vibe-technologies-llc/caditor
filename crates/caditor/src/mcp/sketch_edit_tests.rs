use std::f64::consts::PI;

use serde_json::{Value, json};
use tempfile::TempDir;

use super::{
    feature_tests::{box_of, close, face_where, number, started},
    tests::Client,
};

const HEIGHT: f64 = 5.0;

pub(super) fn new_sketch(client: &mut Client) -> Value {
    client.tool("add_sketch", json!({ "plane": "XY" }))["sketch"]["id"].clone()
}

pub(super) fn draw(client: &mut Client, sketch: &Value, items: Value) -> Vec<Value> {
    let drawn = client.tool(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": items }),
    );
    assert_eq!(drawn["failing"], json!([]), "{drawn}");
    drawn["drawn"].as_array().unwrap().clone()
}

pub(super) fn modify(client: &mut Client, sketch: &Value, operation: Value) -> Value {
    let mut arguments = operation;
    arguments["sketch"] = sketch.clone();
    let answer = client.tool("modify_sketch", arguments);
    assert_eq!(answer["failing"], json!([]), "{answer}");
    answer
}

pub(super) fn refused(client: &mut Client, sketch: &Value, operation: Value) -> String {
    let mut arguments = operation;
    arguments["sketch"] = sketch.clone();
    client.tool_error("modify_sketch", arguments)
}

pub(super) fn described(client: &mut Client, sketch: &Value) -> Value {
    client.tool("get_sketch", json!({ "sketch": sketch }))
}

pub(super) fn entity<'a>(sketch: &'a Value, id: &Value) -> &'a Value {
    sketch["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entity| &entity["id"] == id)
        .unwrap_or_else(|| panic!("no entity {id} in {sketch}"))
}

pub(super) fn position(sketch: &Value, id: &Value) -> [f64; 2] {
    let at = &entity(sketch, id)["at"];
    [number(&at[0]), number(&at[1])]
}

pub(super) fn near(found: [f64; 2], wanted: [f64; 2]) -> bool {
    close(found[0], wanted[0]) && close(found[1], wanted[1])
}

pub(super) fn ends(sketch: &Value, line: &Value) -> ([f64; 2], [f64; 2]) {
    let found = entity(sketch, line);
    let point = |key: &str| [number(&found[key][0]), number(&found[key][1])];
    (point("from"), point("to"))
}

pub(super) fn of_kind<'a>(sketch: &'a Value, kind: &str) -> Vec<&'a Value> {
    sketch["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entity| entity["kind"] == kind)
        .collect()
}

pub(super) fn count_of(answer: &Value, key: &str) -> usize {
    answer[key].as_array().unwrap().len()
}

pub(super) fn extruded_area(client: &mut Client, sketch: &Value) -> f64 {
    let extruded = client.tool(
        "add_extrusion",
        json!({ "sketch": sketch, "distance": format!("{HEIGHT} mm"), "operation": "new_body" }),
    );
    assert_eq!(extruded["failing"], json!([]), "{extruded}");
    let bodies = client.tool("list_bodies", json!({}));
    let volume: f64 = bodies["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|body| number(&body["volume"]))
        .sum();
    client.tool("undo", json!({}));
    volume / HEIGHT
}

pub(super) fn assert_area(found: f64, wanted: f64) {
    assert!(
        (found - wanted).abs() < 1e-3 * wanted,
        "{found} != {wanted}"
    );
}

pub(super) fn descriptions(sketch: &Value) -> Vec<String> {
    sketch["constraints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|constraint| constraint["description"].as_str().unwrap().to_lowercase())
        .collect()
}

#[test]
fn elliptical_arcs_conics_tangent_arcs_and_polygons_on_a_side_are_drawn() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);

    let drawn = draw(
        &mut client,
        &sketch,
        json!([
            { "kind": "elliptical_arc", "center": [0, 0], "major_end": [20, 0], "minor_radius": 10, "start": [1, 0], "end": [0, 3] },
            { "kind": "conic", "start": [30, 0], "end": [50, 0], "apex": [40, 10], "rho": 0.3 },
            { "kind": "line", "from": [0, -20], "to": [10, -20] }
        ]),
    );
    let [arc, conic, line] = [&drawn[0], &drawn[1], &drawn[2]];
    let tangent = draw(
        &mut client,
        &sketch,
        json!([{ "kind": "tangent_arc", "from_point": line["end"], "to": [10, 0] }]),
    );
    let shown = described(&mut client, &sketch);

    let elliptical = entity(&shown, &arc["id"]);
    assert_eq!(elliptical["kind"], "elliptical arc");
    assert!(close(number(&elliptical["minor_radius"]), 10.0));
    assert!(near(position(&shown, &arc["start"]), [20.0, 0.0]));
    assert!(near(position(&shown, &arc["end"]), [0.0, 10.0]));
    let drawn_conic = entity(&shown, &conic["id"]);
    assert_eq!(drawn_conic["form"], "conic");
    assert!(close(number(&drawn_conic["rho"]), 0.3));
    assert_eq!(tangent[0]["tangent_to"], line["id"]);
    assert!(near(position(&shown, &tangent[0]["center"]), [10.0, -10.0]));
    let constraints = descriptions(&shown);
    assert!(
        constraints
            .iter()
            .any(|description| description.contains("tangent")),
        "{constraints:?}"
    );

    let square = new_sketch(&mut client);
    let polygon = draw(
        &mut client,
        &square,
        json!([{ "kind": "polygon", "from": [0, 0], "to": [10, 0], "sides": 4 }]),
    );
    assert!(near(
        position(&described(&mut client, &square), &polygon[0]["center"]),
        [5.0, 5.0]
    ));
    assert_area(extruded_area(&mut client, &square), 100.0);

    let mut rejected = |item: Value| {
        client.tool_error(
            "add_sketch_geometry",
            json!({ "sketch": sketch, "items": [item] }),
        )
    };
    let flat_conic =
        rejected(json!({ "kind": "conic", "start": [0, 0], "end": [10, 0], "apex": [5, 0] }));
    let full_conic = rejected(
        json!({ "kind": "conic", "start": [0, 0], "end": [10, 0], "apex": [5, 5], "rho": 1.5 }),
    );
    let no_direction =
        rejected(json!({ "kind": "tangent_arc", "from_point": arc["center"], "to": [5, 5] }));
    let both = rejected(
        json!({ "kind": "polygon", "center": [0, 0], "from": [0, 0], "to": [10, 0], "sides": 4 }),
    );
    let same_ends = rejected(json!({
        "kind": "elliptical_arc", "center": [0, 0], "major_end": [20, 0], "minor_radius": 10,
        "start": [1, 0], "end": [2, 0]
    }));
    client.finish();

    assert!(flat_conic.contains("apex must lie off"), "{flat_conic}");
    assert!(full_conic.contains("rho"), "{full_conic}");
    assert!(
        no_direction.contains("does not end a line"),
        "{no_direction}"
    );
    assert!(both.contains("from and to"), "{both}");
    assert!(same_ends.contains("different directions"), "{same_ends}");
}

#[test]
fn trimming_cuts_pieces_back_to_their_crossings_in_one_undo_step() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);
    let drawn = draw(
        &mut client,
        &sketch,
        json!([
            { "kind": "line", "from": [-10, 5], "to": [10, 5] },
            { "kind": "line", "from": [3, -10], "to": [3, 10] }
        ]),
    );
    let (across, upright) = (drawn[0]["id"].clone(), drawn[1]["id"].clone());

    let trimmed = modify(
        &mut client,
        &sketch,
        json!({ "operation": "trim", "pieces": [{ "curve": across, "near": [8, 5] }] }),
    );
    let after_one = described(&mut client, &sketch);
    let both = modify(
        &mut client,
        &sketch,
        json!({ "operation": "trim", "pieces": [
            { "curve": across, "near": [-8, 5] },
            { "curve": upright, "near": [3, -8] }
        ] }),
    );
    let after_both = described(&mut client, &sketch);
    client.tool("undo", json!({}));
    let undone = described(&mut client, &sketch);
    let not_a_curve = refused(
        &mut client,
        &sketch,
        json!({ "operation": "trim", "pieces": [{ "curve": drawn[0]["start"], "near": [-10, 5] }] }),
    );
    let stray = refused(
        &mut client,
        &sketch,
        json!({ "operation": "trim", "pieces": [], "radius": "2 mm" }),
    );
    client.finish();

    assert!(
        trimmed["change"].as_str().unwrap().starts_with("Trim "),
        "{trimmed}"
    );
    let (from, to) = ends(&after_one, &across);
    assert!(
        near(from, [-10.0, 5.0]) && near(to, [3.0, 5.0]),
        "{after_one}"
    );
    assert_eq!(both["change"], "Trim 2 pieces");
    let lines = of_kind(&after_both, "line");
    let spans: Vec<([f64; 2], [f64; 2])> = lines
        .iter()
        .map(|line| ends(&after_both, &line["id"]))
        .collect();
    assert!(
        spans
            .iter()
            .any(|(from, to)| near(*from, [0.0, 5.0]) && near(*to, [3.0, 5.0])),
        "{spans:?}"
    );
    assert!(
        spans
            .iter()
            .any(|(from, to)| near(*from, [3.0, 0.0]) && near(*to, [3.0, 10.0])),
        "{spans:?}"
    );
    let (from, to) = ends(&undone, &across);
    assert!(near(from, [-10.0, 5.0]) && near(to, [3.0, 5.0]), "{undone}");
    assert!(not_a_curve.contains("not a curve"), "{not_a_curve}");
    assert!(stray.contains("do not fit"), "{stray}");
}

#[test]
fn extending_reaches_the_next_curve_from_the_named_end() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);
    let drawn = draw(
        &mut client,
        &sketch,
        json!([
            { "kind": "line", "from": [5, 5], "to": [10, 5] },
            { "kind": "line", "from": [20, 0], "to": [20, 10] },
            { "kind": "circle", "center": [40, 40], "radius": 3 }
        ]),
    );
    let line = drawn[0]["id"].clone();

    let extended = modify(
        &mut client,
        &sketch,
        json!({ "operation": "extend", "curve": line, "end": drawn[0]["end"] }),
    );
    let shown = described(&mut client, &sketch);
    let neither = refused(
        &mut client,
        &sketch,
        json!({ "operation": "extend", "curve": line }),
    );
    let closed = refused(
        &mut client,
        &sketch,
        json!({ "operation": "extend", "curve": drawn[2]["id"], "near": [43, 40] }),
    );
    client.finish();

    assert_eq!(
        extended["change"],
        format!(
            "Extend {}",
            entity(&shown, &line)["label"].as_str().unwrap()
        )
    );
    assert!(extended["extended_to"].is_string(), "{extended}");
    let (from, to) = ends(&shown, &line);
    assert!(near(from, [5.0, 5.0]) && near(to, [20.0, 5.0]), "{shown}");
    assert!(neither.contains("not both and not neither"), "{neither}");
    assert!(closed.contains("cannot be extended"), "{closed}");
}

#[test]
fn offsetting_runs_a_chain_inward_or_round_both_sides_of_a_line() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);
    let rectangle = draw(
        &mut client,
        &sketch,
        json!([{ "kind": "rectangle", "corner": [10, 10], "opposite": [30, 20] }]),
    );

    let inward = modify(
        &mut client,
        &sketch,
        json!({
            "operation": "offset", "curves": [rectangle[0]["sides"][0]], "whole_chain": true,
            "distance": "2 mm", "toward": [20, 15]
        }),
    );
    let shown = described(&mut client, &sketch);
    let not_a_curve = refused(
        &mut client,
        &sketch,
        json!({ "operation": "offset", "curves": [rectangle[0]["corners"][0]], "distance": "1 mm" }),
    );
    client.tool("undo", json!({}));
    let undone = described(&mut client, &sketch);

    let slot = new_sketch(&mut client);
    let line = draw(
        &mut client,
        &slot,
        json!([{ "kind": "line", "from": [0, 0], "to": [20, 0] }]),
    );
    let around = modify(
        &mut client,
        &slot,
        json!({
            "operation": "offset", "curves": [line[0]["id"]], "distance": "1 mm",
            "both_sides": "round"
        }),
    );
    let slot_area = extruded_area(&mut client, &slot);
    client.finish();

    assert_eq!(inward["change"], "Offset curves");
    assert_eq!(count_of(&inward, "offset_curves"), 4);
    let new_lines: Vec<&Value> = of_kind(&shown, "line")
        .into_iter()
        .filter(|line| {
            inward["added_entities"]
                .as_array()
                .unwrap()
                .contains(&line["id"])
        })
        .collect();
    assert_eq!(new_lines.len(), 4, "{inward}");
    for line in new_lines {
        let (from, to) = ends(&shown, &line["id"]);
        for [x, y] in [from, to] {
            assert!(close(x, 12.0) || close(x, 28.0), "{x}");
            assert!(close(y, 12.0) || close(y, 18.0), "{y}");
        }
    }
    assert!(not_a_curve.contains("cannot be offset"), "{not_a_curve}");
    assert_eq!(of_kind(&undone, "line").len(), 4);
    assert_eq!(around["change"], "Offset curves to both sides");
    assert_area(slot_area, 20.0 * 2.0 + PI);
}

#[test]
fn mirroring_and_patterns_copy_the_selection_with_expressions() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    client.tool(
        "set_parameter",
        json!({ "name": "pitch", "expression": "7 mm" }),
    );
    let sketch = new_sketch(&mut client);
    let circle = draw(
        &mut client,
        &sketch,
        json!([{ "kind": "circle", "center": [10, 10], "radius": 2 }]),
    );
    let circle = circle[0]["id"].clone();

    let mirrored = modify(
        &mut client,
        &sketch,
        json!({ "operation": "mirror", "items": [circle], "about": "vertical_axis" }),
    );
    let after_mirror = described(&mut client, &sketch);
    let grid = modify(
        &mut client,
        &sketch,
        json!({
            "operation": "rectangular_pattern", "items": [circle], "count": 3,
            "spacing": "pitch", "rows": { "count": 2, "spacing": "10 mm" }
        }),
    );
    let after_grid = described(&mut client, &sketch);
    let not_a_line = refused(
        &mut client,
        &sketch,
        json!({ "operation": "mirror", "items": [circle], "about": circle }),
    );
    let one = refused(
        &mut client,
        &sketch,
        json!({ "operation": "rectangular_pattern", "items": [circle], "count": 1, "spacing": "5 mm" }),
    );

    let wheel = new_sketch(&mut client);
    let spoke = draw(
        &mut client,
        &wheel,
        json!([{ "kind": "circle", "center": [10, 0], "radius": 1 }]),
    );
    let around = modify(
        &mut client,
        &wheel,
        json!({ "operation": "circular_pattern", "items": [spoke[0]["id"]], "center": "origin", "count": 4 }),
    );
    let wheel_area = extruded_area(&mut client, &wheel);
    let wheel_circles = of_kind(&described(&mut client, &wheel), "circle").len();
    client.finish();

    assert_eq!(mirrored["change"], "Mirror geometry");
    let circles = of_kind(&after_mirror, "circle");
    assert_eq!(circles.len(), 2);
    let centres: Vec<[f64; 2]> = circles
        .iter()
        .map(|circle| [number(&circle["at"][0]), number(&circle["at"][1])])
        .collect();
    assert!(
        centres.iter().any(|at| near(*at, [-10.0, 10.0])),
        "{centres:?}"
    );
    assert_eq!(grid["change"], "Pattern geometry");
    let circles = of_kind(&after_grid, "circle");
    assert_eq!(circles.len(), 7);
    assert!(
        circles.iter().any(|circle| near(
            [number(&circle["at"][0]), number(&circle["at"][1])],
            [24.0, 20.0]
        )),
        "{after_grid}"
    );
    assert!(not_a_line.contains("not a line or an axis"), "{not_a_line}");
    assert!(one.contains("at least 2"), "{one}");
    assert_eq!(around["change"], "Pattern geometry");
    assert_eq!(wheel_circles, 4);
    assert_area(wheel_area, 4.0 * PI);
}

#[test]
fn filleting_and_chamfering_round_and_cut_corners() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);
    let rectangle = draw(
        &mut client,
        &sketch,
        json!([
            { "kind": "rectangle", "corner": [10, 10], "opposite": [30, 20] }
        ]),
    );
    let corners = rectangle[0]["corners"].clone();
    let sides = rectangle[0]["sides"].clone();

    let rounded = modify(
        &mut client,
        &sketch,
        json!({ "operation": "fillet", "corners": [corners[0]], "radius": "2 mm" }),
    );
    let rounded_area = extruded_area(&mut client, &sketch);
    let cut = modify(
        &mut client,
        &sketch,
        json!({ "operation": "chamfer", "corners": [[sides[0], sides[1]]], "distance": "3 mm" }),
    );
    let cut_area = extruded_area(&mut client, &sketch);
    client.tool("undo", json!({}));
    let undone_area = extruded_area(&mut client, &sketch);
    let two_ways = refused(
        &mut client,
        &sketch,
        json!({
            "operation": "chamfer", "corners": [corners[2]], "distance": "1 mm",
            "second_distance": "2 mm", "angle": "30 deg"
        }),
    );
    let lone = draw(
        &mut client,
        &sketch,
        json!([{ "kind": "point", "at": [50, 50] }]),
    );
    let no_corner = refused(
        &mut client,
        &sketch,
        json!({ "operation": "fillet", "corners": [lone[0]["id"]], "radius": "1 mm" }),
    );
    client.finish();

    assert_eq!(rounded["change"], "Fillet corner");
    assert_area(rounded_area, 200.0 - (4.0 - PI));
    assert_eq!(cut["change"], "Chamfer corner");
    assert_area(cut_area, 200.0 - (4.0 - PI) - 4.5);
    assert_area(undone_area, 200.0 - (4.0 - PI));
    assert!(two_ways.contains("not both"), "{two_ways}");
    assert!(no_corner.contains("cannot be rounded"), "{no_corner}");
}

#[test]
fn moving_rotating_and_scaling_solve_like_a_drag_and_keep_constraints() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);
    let drawn = draw(
        &mut client,
        &sketch,
        json!([
            { "kind": "circle", "center": [10, 10], "radius": 2 },
            { "kind": "line", "from": [10, -5], "to": [20, -5] },
            { "kind": "point", "at": [30, 30] }
        ]),
    );
    let (circle, line, lone) = (
        drawn[0]["id"].clone(),
        drawn[1]["id"].clone(),
        drawn[2]["id"].clone(),
    );
    let line_start = drawn[1]["start"].clone();

    let moved = modify(
        &mut client,
        &sketch,
        json!({ "operation": "move", "items": [circle], "by": [5, 0] }),
    );
    let scaled = modify(
        &mut client,
        &sketch,
        json!({ "operation": "scale", "items": [circle], "factor": 2 }),
    );
    let after_scale = described(&mut client, &sketch);
    let turned = modify(
        &mut client,
        &sketch,
        json!({ "operation": "rotate", "items": [line], "angle": "90", "about": "origin" }),
    );
    let after_turn = described(&mut client, &sketch);
    let placed = modify(
        &mut client,
        &sketch,
        json!({ "operation": "move", "items": [lone], "to": [-30, 30] }),
    );
    let after_place = described(&mut client, &sketch);
    client.tool("undo", json!({}));
    let undone = described(&mut client, &sketch);
    client.tool(
        "add_sketch_constraints",
        json!({ "sketch": sketch, "constraints": [
            { "kind": "fix", "point": line_start },
            { "kind": "fix", "point": lone }
        ] }),
    );
    let pulled = modify(
        &mut client,
        &sketch,
        json!({ "operation": "move", "items": [line], "by": [0, 5] }),
    );
    let after_pull = described(&mut client, &sketch);
    let held = refused(
        &mut client,
        &sketch,
        json!({ "operation": "move", "items": [lone], "by": [1, 1] }),
    );
    let both = refused(
        &mut client,
        &sketch,
        json!({ "operation": "move", "items": [circle], "by": [1, 1], "to": [0, 0] }),
    );
    let about_itself = refused(
        &mut client,
        &sketch,
        json!({ "operation": "rotate", "items": [line], "angle": "10 deg", "about": line_start }),
    );
    let shrunk = refused(
        &mut client,
        &sketch,
        json!({ "operation": "scale", "items": [circle], "factor": -1 }),
    );
    client.finish();

    assert_eq!(moved["reached"], true, "{moved}");
    assert_eq!(scaled["reached"], true, "{scaled}");
    let scaled_circle = entity(&after_scale, &circle);
    assert!(
        close(number(&scaled_circle["radius"]), 4.0),
        "{scaled_circle}"
    );
    assert!(near(
        [
            number(&scaled_circle["at"][0]),
            number(&scaled_circle["at"][1])
        ],
        [15.0, 10.0]
    ));
    assert_eq!(turned["reached"], true, "{turned}");
    let (from, to) = ends(&after_turn, &line);
    assert!(
        near(from, [5.0, 10.0]) && near(to, [5.0, 20.0]),
        "{from:?} {to:?}"
    );
    assert!(near(position(&after_place, &lone), [-30.0, 30.0]));
    assert!(
        placed["change"].as_str().unwrap().starts_with("Move "),
        "{placed}"
    );
    assert!(near(position(&undone, &lone), [30.0, 30.0]));
    assert_eq!(pulled["reached"], false, "{pulled}");
    assert!(pulled["note"].is_string(), "{pulled}");
    assert!(near(position(&after_pull, &line_start), [5.0, 10.0]));
    assert!(held.starts_with("Nothing moved"), "{held}");
    assert!(both.contains("not both and not neither"), "{both}");
    assert!(
        about_itself.contains("belongs to the geometry"),
        "{about_itself}"
    );
    assert!(shrunk.contains("above zero"), "{shrunk}");
}

#[test]
fn projected_edges_and_faces_follow_the_body() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let early = new_sketch(&mut client);
    let block = box_of(&mut client, [40.0, 30.0, 20.0], [0.0, 0.0], "Block");
    let top = face_where(&mut client, &block, [0.0, 0.0, 1.0]);
    let sketch =
        client.tool("add_sketch", json!({ "body": block, "face": top }))["sketch"]["id"].clone();
    let edges = client.tool("list_edges", json!({ "body": block }));
    let top_edge = edges["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|edge| {
            edge["kind"] == "line"
                && close(number(&edge["start"][2]), 20.0)
                && close(number(&edge["end"][2]), 20.0)
        })
        .unwrap()["edge"]
        .clone();

    let one = modify(
        &mut client,
        &sketch,
        json!({ "operation": "project", "body": block, "edges": [top_edge] }),
    );
    let face = modify(
        &mut client,
        &sketch,
        json!({ "operation": "project", "body": block, "faces": [top] }),
    );
    let again = refused(
        &mut client,
        &sketch,
        json!({ "operation": "project", "body": block, "faces": [top] }),
    );
    let shown = described(&mut client, &sketch);
    client.tool("edit_feature", json!({ "feature": "Block", "length": 50 }));
    let followed = described(&mut client, &sketch);
    let later = refused(
        &mut client,
        &early,
        json!({ "operation": "project", "body": block, "edges": [top_edge] }),
    );
    let unknown = refused(
        &mut client,
        &sketch,
        json!({ "operation": "project", "body": block, "edges": ["abc"] }),
    );
    client.finish();

    assert_eq!(count_of(&one, "projected"), 1, "{one}");
    assert_eq!(count_of(&face, "projected"), 3, "{face}");
    assert_eq!(face["change"], "Project 3 edges of Block");
    assert!(again.contains("already projected"), "{again}");
    let projected_lines = |sketch: &Value| -> Vec<([f64; 2], [f64; 2])> {
        of_kind(sketch, "line")
            .into_iter()
            .filter(|line| line["projected"] == true)
            .map(|line| ends(sketch, &line["id"]))
            .collect()
    };
    assert_eq!(projected_lines(&shown).len(), 4, "{shown}");
    let reach = |lines: &[([f64; 2], [f64; 2])]| {
        lines
            .iter()
            .flat_map(|(from, to)| [from[0].abs(), to[0].abs(), from[1].abs(), to[1].abs()])
            .fold(0.0, f64::max)
    };
    let before = reach(&projected_lines(&shown));
    let after = reach(&projected_lines(&followed));
    assert!(after > before, "{before} {after}");
    assert!(later.contains("made after"), "{later}");
    assert!(unknown.contains("no edge"), "{unknown}");
}
