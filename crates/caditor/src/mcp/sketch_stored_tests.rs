use serde_json::{Value, json};
use tempfile::TempDir;

use super::{
    feature_tests::{close, number, started},
    sketch_edit_tests::{described, extruded_area, modify, new_sketch, refused},
};

fn only(sketch: &Value, kind: &str) -> Value {
    let listed = sketch[kind]
        .as_array()
        .unwrap_or_else(|| panic!("{sketch}"));
    assert_eq!(listed.len(), 1, "{sketch}");
    listed[0].clone()
}

#[test]
fn a_drawn_gear_is_read_back_and_edited() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);

    let before = described(&mut client, &sketch);
    let drawn = modify(
        &mut client,
        &sketch,
        json!({ "operation": "gear", "teeth": 20, "module": "2 mm", "at": [5, 5] }),
    );
    let read = described(&mut client, &sketch);
    let gear = only(&read, "gears");
    let id = gear["id"].clone();
    let small_area = extruded_area(&mut client, &sketch);

    let edited = modify(
        &mut client,
        &sketch,
        json!({ "operation": "edit_gear", "gear": id, "teeth": 30, "module": "3 mm", "turn": "10 deg" }),
    );
    let after = only(&described(&mut client, &sketch), "gears");
    let large_area = extruded_area(&mut client, &sketch);

    client.tool("undo", json!({}));
    let undone = only(&described(&mut client, &sketch), "gears");

    client.tool(
        "set_parameter",
        json!({ "name": "pitch", "expression": "2.5 mm" }),
    );
    modify(
        &mut client,
        &sketch,
        json!({ "operation": "edit_gear", "gear": id, "module": "pitch", "teeth": "12 + 8" }),
    );
    let held = only(&described(&mut client, &sketch), "gears");
    client.tool(
        "set_parameter",
        json!({ "name": "pitch", "expression": "4 mm" }),
    );
    let followed = only(&described(&mut client, &sketch), "gears");

    let nothing = refused(
        &mut client,
        &sketch,
        json!({ "operation": "edit_gear", "gear": id }),
    );
    let few = refused(
        &mut client,
        &sketch,
        json!({ "operation": "edit_gear", "gear": id, "teeth": 2 }),
    );
    let fraction = refused(
        &mut client,
        &sketch,
        json!({ "operation": "edit_gear", "gear": id, "teeth": 20.5 }),
    );
    let unknown = refused(
        &mut client,
        &sketch,
        json!({ "operation": "edit_gear", "gear": 9999, "teeth": 20 }),
    );
    let removed = client.tool(
        "remove_sketch_items",
        json!({ "sketch": sketch, "entities": [id] }),
    );
    let gone = described(&mut client, &sketch);
    client.finish();

    assert!(before.get("gears").is_none(), "{before}");
    assert_eq!(drawn["added_gears"], json!([id]), "{drawn}");
    assert_eq!(gear["module"], "2 mm");
    assert_eq!(gear["teeth"], "20");
    assert!(gear["center"].is_u64(), "{gear}");
    assert_eq!(gear["at"], json!([5.0, 5.0]));
    assert_eq!(gear["values"]["teeth"], 20);
    assert!(close(number(&gear["values"]["module"]), 2.0), "{gear}");
    assert_eq!(edited["gear"], id);
    assert_eq!(edited["change"], "Edit spur gear");
    assert_eq!(after["module"], "3 mm");
    assert_eq!(after["teeth"], "30");
    assert_eq!(after["values"]["teeth"], 30);
    assert!(close(number(&after["values"]["turn"]), 10.0), "{after}");
    assert!(large_area > 2.0 * small_area, "{large_area} {small_area}");
    assert_eq!(undone["module"], "2 mm");
    assert_eq!(held["module"], "pitch");
    assert!(close(number(&held["values"]["module"]), 2.5), "{held}");
    assert_eq!(held["values"]["teeth"], 20);
    assert!(
        close(number(&followed["values"]["module"]), 4.0),
        "{followed}"
    );
    assert!(nothing.contains("at least one value"), "{nothing}");
    assert!(few.contains("teeth"), "{few}");
    assert!(fraction.contains("whole"), "{fraction}");
    assert!(unknown.contains("no gear"), "{unknown}");
    assert_eq!(removed["removed_gears"], json!([id]), "{removed}");
    assert!(gone.get("gears").is_none(), "{gone}");
}

#[test]
fn drawn_text_is_read_back_and_edited() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);

    let drawn = modify(
        &mut client,
        &sketch,
        json!({
            "operation": "text",
            "content": "Hello",
            "height": "8 mm",
            "angle": "15 deg",
            "bold": true,
            "align": "centre",
            "at": [10, 20]
        }),
    );
    let text = only(&described(&mut client, &sketch), "texts");
    let id = text["id"].clone();
    let area = extruded_area(&mut client, &sketch);

    let edited = modify(
        &mut client,
        &sketch,
        json!({ "operation": "edit_text", "text": id, "content": "Hello\nworld", "height": "4 mm", "bold": false, "italic": true, "align": "right" }),
    );
    let after = only(&described(&mut client, &sketch), "texts");
    let smaller_area = extruded_area(&mut client, &sketch);
    client.tool("undo", json!({}));
    let undone = only(&described(&mut client, &sketch), "texts");

    let origin = modify(
        &mut client,
        &sketch,
        json!({ "operation": "text", "content": "Origin" }),
    );
    let blank = refused(
        &mut client,
        &sketch,
        json!({ "operation": "text", "content": "   " }),
    );
    let both = refused(
        &mut client,
        &sketch,
        json!({ "operation": "text", "content": "x", "anchor": "origin", "at": [1, 1] }),
    );
    let flat = refused(
        &mut client,
        &sketch,
        json!({ "operation": "text", "content": "x", "height": "0 mm" }),
    );
    let nothing = refused(
        &mut client,
        &sketch,
        json!({ "operation": "edit_text", "text": id }),
    );
    let unknown = refused(
        &mut client,
        &sketch,
        json!({ "operation": "edit_text", "text": 9999, "bold": true }),
    );
    let removed = client.tool(
        "remove_sketch_items",
        json!({ "sketch": sketch, "entities": [id] }),
    );
    let remaining = described(&mut client, &sketch);
    client.finish();

    assert_eq!(drawn["change"], "Draw text");
    assert_eq!(drawn["added_texts"], json!([id]), "{drawn}");
    assert_eq!(text["content"], "Hello");
    assert_eq!(text["height"], "8 mm");
    assert_eq!(text["bold"], true);
    assert_eq!(text["italic"], false);
    assert_eq!(text["align"], "centre");
    assert_eq!(text["at"], json!([10.0, 20.0]));
    assert!(close(number(&text["values"]["height"]), 8.0), "{text}");
    assert!(close(number(&text["values"]["angle"]), 15.0), "{text}");
    assert!(area > 0.0);
    assert_eq!(edited["text"], id);
    assert_eq!(edited["change"], "Edit text");
    assert_eq!(after["content"], "Hello\nworld");
    assert_eq!(after["height"], "4 mm");
    assert_eq!(after["bold"], false);
    assert_eq!(after["italic"], true);
    assert_eq!(after["align"], "right");
    assert!(smaller_area > 0.0);
    assert_eq!(undone["content"], "Hello");
    assert_eq!(origin["added_texts"].as_array().unwrap().len(), 1);
    assert!(blank.contains("letter"), "{blank}");
    assert!(both.contains("not both"), "{both}");
    assert!(flat.contains("height"), "{flat}");
    assert!(nothing.contains("at least one value"), "{nothing}");
    assert!(unknown.contains("no text"), "{unknown}");
    assert_eq!(removed["removed_texts"], json!([id]), "{removed}");
    assert_eq!(
        remaining["texts"].as_array().unwrap().len(),
        1,
        "{remaining}"
    );
}
