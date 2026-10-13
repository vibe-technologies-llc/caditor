use std::f64::consts::PI;

use caditor_document::{CancelToken, ModelEvaluator, Recompute};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::tests::{Client, opening};

const LENGTH: f64 = 80.0;
const WIDTH: f64 = 50.0;
const THICKNESS: f64 = 10.0;
const POCKET: [f64; 4] = [20.0, 15.0, 60.0, 35.0];
const POCKET_DEPTH: f64 = 4.0;
const HOLE_DIAMETER: f64 = 6.0;
const HOLES: [[f64; 2]; 2] = [[10.0, 10.0], [70.0, 40.0]];

fn number(value: &Value) -> f64 {
    value
        .as_f64()
        .unwrap_or_else(|| panic!("{value} is not a number"))
}

fn vector(value: &Value) -> [f64; 3] {
    [number(&value[0]), number(&value[1]), number(&value[2])]
}

fn dot(first: [f64; 3], second: [f64; 3]) -> f64 {
    first.iter().zip(second).map(|(a, b)| a * b).sum()
}

struct SketchFrame {
    origin: [f64; 3],
    x_axis: [f64; 3],
    y_axis: [f64; 3],
}

impl SketchFrame {
    fn of(sketch: &Value) -> Self {
        let plane = &sketch["plane"];
        Self {
            origin: vector(&plane["origin"]),
            x_axis: vector(&plane["x_axis"]),
            y_axis: vector(&plane["y_axis"]),
        }
    }

    fn local(&self, x: f64, y: f64, z: f64) -> [f64; 2] {
        let [ox, oy, oz] = self.origin;
        let offset = [x - ox, y - oy, z - oz];
        [dot(offset, self.x_axis), dot(offset, self.y_axis)]
    }
}

fn volume_of(client: &mut Client) -> f64 {
    let bodies = client.tool("list_bodies", json!({}));
    let listed = bodies["bodies"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "{bodies}");
    assert_eq!(listed[0]["exact"], true);
    number(&listed[0]["volume"])
}

fn top_face(client: &mut Client, body: &Value, level: f64) -> String {
    let faces = client.tool("list_faces", json!({ "body": body }));
    let found = faces["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|face| {
            face["surface"] == "plane"
                && face["normal"] == json!([0.0, 0.0, 1.0])
                && (number(&face["point"][2]) - level).abs() < 1e-9
        })
        .unwrap_or_else(|| panic!("no face faces up at {level}: {faces}"));
    found["face"].as_str().unwrap().to_owned()
}

fn sketch_on_top(client: &mut Client, body: &Value, name: &str) -> (Value, SketchFrame) {
    let face = top_face(client, body, THICKNESS);
    let added = client.tool(
        "add_sketch",
        json!({ "body": body, "face": face, "name": name }),
    );
    assert_eq!(added["failing"], json!([]), "{added}");
    let sketch = added["sketch"].clone();
    let frame = SketchFrame::of(&sketch);
    (sketch["id"].clone(), frame)
}

#[test]
fn tool_calls_alone_build_a_plate_with_a_pocket_and_two_holes() {
    let folder = TempDir::new().unwrap();
    let model = folder.path().join("bracket plate.caditor");
    let mut client = Client::start(opening(model.clone()));
    client.initialize("2025-06-18");

    for (name, expression) in [
        ("length", "80 mm"),
        ("width", "50 mm"),
        ("thickness", "10 mm"),
    ] {
        let set = client.tool(
            "set_parameter",
            json!({ "name": name, "expression": expression }),
        );
        assert_eq!(set["change"], format!("Add {name}"));
    }

    let outline = client.tool("add_sketch", json!({ "plane": "XY" }));
    let outline_id = outline["sketch"]["id"].clone();
    let drawn = client.tool(
        "add_sketch_geometry",
        json!({
            "sketch": outline_id,
            "items": [{ "kind": "rectangle", "corner": [0, 0], "opposite": [70, 40] }]
        }),
    );
    let corners = drawn["drawn"][0]["corners"].clone();
    let constrained = client.tool(
        "add_sketch_constraints",
        json!({
            "sketch": outline_id,
            "constraints": [
                { "kind": "coincident", "a": corners[0], "b": "origin" },
                { "kind": "distance", "from": corners[0], "to": corners[1], "value": "length" },
                { "kind": "distance", "from": corners[0], "to": corners[3], "value": "width" }
            ]
        }),
    );
    assert_eq!(constrained["sketch_status"], "Fully constrained");

    let plate = client.tool(
        "add_extrusion",
        json!({ "sketch": outline_id, "distance": "thickness", "name": "Plate" }),
    );
    assert_eq!(plate["operation"], "makes a new body", "{plate}");
    assert_eq!(plate["failing"], json!([]));
    assert!((volume_of(&mut client) - LENGTH * WIDTH * THICKNESS).abs() < 1e-6);
    let body = json!("Plate");

    let (pocket_sketch, frame) = sketch_on_top(&mut client, &body, "Pocket sketch");
    let [left, bottom, right, top] = POCKET;
    let pocket_drawn = client.tool(
        "add_sketch_geometry",
        json!({
            "sketch": pocket_sketch,
            "items": [{
                "kind": "rectangle",
                "corner": frame.local(left, bottom, THICKNESS),
                "opposite": frame.local(right, top, THICKNESS)
            }]
        }),
    );
    assert_eq!(pocket_drawn["failing"], json!([]));
    let pocket = client.tool(
        "add_extrusion",
        json!({ "sketch": pocket_sketch, "distance": "4 mm", "name": "Pocket" }),
    );
    assert_eq!(pocket["operation"], "removes from Plate", "{pocket}");
    assert_eq!(pocket["failing"], json!([]));

    let (hole_sketch, frame) = sketch_on_top(&mut client, &body, "Hole sketch");
    let points: Vec<Value> = HOLES
        .iter()
        .map(|[x, y]| json!({ "kind": "point", "at": frame.local(*x, *y, THICKNESS) }))
        .collect();
    client.tool(
        "add_sketch_geometry",
        json!({ "sketch": hole_sketch, "items": points }),
    );
    let holes = client.tool(
        "add_hole",
        json!({ "sketch": hole_sketch, "diameter": "6 mm", "depth": "through all" }),
    );
    assert_eq!(holes["holes"], 2);
    assert_eq!(holes["body"], "Plate");
    assert_eq!(holes["failing"], json!([]), "{holes}");

    let pocketed = LENGTH * WIDTH * THICKNESS - (right - left) * (top - bottom) * POCKET_DEPTH;
    let radius = HOLE_DIAMETER / 2.0;
    let drilled = pocketed - 2.0 * PI * radius * radius * THICKNESS;
    let built = volume_of(&mut client);
    assert!(
        (built - drilled).abs() < 1e-6 * drilled,
        "{built} != {drilled}"
    );

    let undone = client.tool("undo", json!({}));
    assert_eq!(undone["undone"], "Create Hole 1");
    assert!((volume_of(&mut client) - pocketed).abs() < 1e-6 * pocketed);
    client.tool("redo", json!({}));
    assert!((volume_of(&mut client) - drilled).abs() < 1e-6 * drilled);

    let thinner = client.tool(
        "set_parameter",
        json!({ "name": "thickness", "expression": "12 mm" }),
    );
    assert_eq!(thinner["failing"], json!([]), "{thinner}");
    let thicker = drilled + LENGTH * WIDTH * 2.0 - 2.0 * PI * radius * radius * 2.0;
    assert!((volume_of(&mut client) - thicker).abs() < 1e-6 * thicker);

    client.tool("save", json!({}));
    let exported = client.tool("export", json!({ "file": "plate.step" }));
    assert_eq!(exported["bodies"], 1);
    let outside = client.tool_error("export", json!({ "file": "../plate.step" }));
    assert!(outside.contains("lies outside the folders"), "{outside}");
    let model_summary = client.tool("get_model", json!({}));
    assert_eq!(model_summary["unsaved_changes"], false);
    client.finish();

    assert!(
        std::fs::read(folder.path().join("plate.step"))
            .unwrap()
            .starts_with(b"ISO-10303-21;")
    );
    let saved = caditor_file::load(&model).unwrap().document;
    let evaluation =
        Recompute::default().run(&saved, &ModelEvaluator, &CancelToken::never(), &|_, _| {});
    assert_eq!(evaluation.failed_count(), 0);
    let names: Vec<&str> = saved
        .features()
        .map(|feature| feature.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "Sketch 1",
            "Plate",
            "Pocket sketch",
            "Pocket",
            "Hole sketch",
            "Hole 1"
        ]
    );
}
