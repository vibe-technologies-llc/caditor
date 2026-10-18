use serde_json::{Map, Value, json};

use crate::mcp::{
    feature_schemas::{amount, axis, keys, name, operation, plane, point},
    tools::{object, reference},
};

pub(super) fn flag(description: &str) -> Value {
    json!({ "type": "boolean", "description": description })
}

pub(super) fn choice(values: &[&str], description: &str) -> Value {
    json!({ "type": "string", "enum": values, "description": description })
}

fn up_to() -> Value {
    json!({
        "description": "Where it ends instead of a distance: \"next\" for the next face it meets, or a plane as XY, XZ, YZ, a flat face {body, face} or a datum plane {datum}",
        "anyOf": [{ "type": "string", "enum": ["next"] }, plane()]
    })
}

pub(super) fn regions() -> Value {
    json!({
        "description": "Which closed regions to use instead of all of them: a point [x, y] inside each region wanted, in the sketch's own millimetre coordinates (get_sketch lists a point inside each region), or \"all\"",
        "anyOf": [
            { "type": "string", "enum": ["all"] },
            {
                "type": "array",
                "items": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
                "minItems": 1
            }
        ]
    })
}

pub(super) fn thin_wall() -> Value {
    json!({
        "description": "Use a wall of this thickness along the sketch's curves instead of its regions, such as \"1 mm\"; \"off\" takes a thin wall off again",
        "anyOf": [{ "type": "string", "enum": ["off"] }, amount("A thickness")]
    })
}

pub(super) fn wall_side() -> Value {
    choice(
        &["inside", "outside", "centred"],
        "With thin_wall: which side of the curves the wall lies",
    )
}

fn surface() -> Value {
    json!({
        "type": "object",
        "description": "A curved or flat face {body, face} that stops it, found as it stands before the new feature",
        "properties": {
            "body": reference("The body, by id or name"),
            "face": { "type": "string", "description": "A face key from list_faces" }
        },
        "required": ["body", "face"],
        "additionalProperties": false
    })
}

fn second_end() -> Value {
    json!({
        "type": "object",
        "description": "The other side of the sketch plane ends in something other than a distance: give one of distance, through_all, up_to or up_to_surface, and for up_to an end_offset",
        "properties": {
            "distance": amount("How far it goes on the other side"),
            "through_all": flag("Cut through the whole body on the other side"),
            "up_to": up_to(),
            "up_to_surface": surface(),
            "end_offset": amount("With up_to: stop this far short of it (negative: beyond)")
        },
        "additionalProperties": false
    })
}

fn direction() -> Value {
    json!({
        "description": "Extrude along an axis instead of square to the sketch: X, Y, Z, an edge {body, edge}, a round face {body, face}, a sketch line {sketch, line}, a datum axis {datum}; \"square\" goes back to square",
        "anyOf": [{ "type": "string", "enum": ["square"] }, axis()]
    })
}

fn path() -> Value {
    json!({
        "description": "What it follows: {sketch, curves} with the entity ids of lines, arcs or curves, {body, edges} with edge keys from list_edges, or {helix}",
        "anyOf": [
            {
                "type": "object",
                "properties": {
                    "sketch": reference("The sketch, by feature id or name"),
                    "curves": { "type": "array", "items": { "type": ["integer", "string"] }, "minItems": 1 }
                },
                "required": ["sketch", "curves"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "body": reference("The body, by id or name"),
                    "edges": keys("Edge keys from list_edges, joined end to end")
                },
                "required": ["body", "edges"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": { "helix": reference("A helix, by feature id or name") },
                "required": ["helix"],
                "additionalProperties": false
            }
        ]
    })
}

fn along() -> Value {
    json!({
        "description": "What it splits along: a plane (XY, XZ, YZ, a flat face {body, face}, a datum plane {datum}), a sketch's curves {sketch}, or another body {tool}",
        "anyOf": [
            plane(),
            {
                "type": "object",
                "properties": { "sketch": reference("The sketch, by feature id or name") },
                "required": ["sketch"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": { "tool": reference("The other body, by id or name") },
                "required": ["tool"],
                "additionalProperties": false
            }
        ]
    })
}

fn with_target(mut properties: Map<String, Value>) -> Map<String, Value> {
    properties.insert("operation".to_owned(), operation());
    properties.insert(
        "body".to_owned(),
        reference("The body it changes, with add, remove or intersect"),
    );
    properties.insert("name".to_owned(), name());
    properties
}

fn properties(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

pub fn extrusion() -> Value {
    let properties = with_target(properties(json!({
        "sketch": reference("The sketch whose regions are extruded"),
        "distance": amount("How far, as an expression such as \"20 mm\""),
        "through_all": flag("Cut through the whole body"),
        "up_to": up_to(),
        "up_to_surface": surface(),
        "end_offset": amount("With up_to: stop this far short of it (negative: beyond)"),
        "symmetric": flag("Half the distance each way from the sketch plane"),
        "second_distance": amount("How far it also goes on the other side of the sketch plane"),
        "second_end": second_end(),
        "start_offset": amount("Start this far from the sketch plane instead of on it"),
        "taper": amount("Draft angle of the sides, such as \"3 deg\"; negative narrows the other way"),
        "direction": direction(),
        "regions": regions(),
        "thin_wall": thin_wall(),
        "wall_side": wall_side(),
        "reversed": flag("Extrude against the sketch plane's normal")
    })));
    object(Value::Object(properties), &["sketch"])
}

pub fn hole() -> Value {
    object(
        json!({
            "sketch": reference("The sketch holding the hole centres as free points or circles"),
            "diameter": amount("An expression such as \"6 mm\"; or give standard"),
            "depth": amount("An expression, or \"through all\""),
            "up_to": up_to(),
            "end_offset": amount("With up_to: stop this far short of it"),
            "style": choice(&["plain", "counterbore", "countersink", "stepped"], "The hole's mouth"),
            "head_diameter": amount("The counterbore's or countersink's diameter"),
            "head_depth": amount("The counterbore's depth"),
            "head_angle": amount("The countersink's angle, such as \"90 deg\""),
            "steps": {
                "type": "array",
                "description": "A stepped hole's flat-bottomed steps from the mouth inward, each narrowing into the hole (at most 8); gives style stepped",
                "items": {
                    "type": "object",
                    "properties": {
                        "diameter": amount("The step's diameter"),
                        "depth": amount("The step's depth")
                    },
                    "required": ["diameter", "depth"],
                    "additionalProperties": false
                },
                "minItems": 1
            },
            "shape": choice(&["round", "slot"], "A round hole, or a slot centred on each point"),
            "slot_length": amount("The slot's length between its ends; gives shape slot"),
            "slot_angle": amount("The slot's direction in the sketch, such as \"45 deg\"; gives shape slot"),
            "bottom": choice(&["flat", "drill_point"], "The blind hole's bottom: flat or a drill point cone"),
            "point_angle": amount("The drill point's angle, such as \"118 deg\"; gives bottom drill_point"),
            "sizing": choice(&["typed", "circles", "circles_and_heads"], "Size each hole by the diameter given, by the circle drawn at it, or by the circle with its head scaled with it"),
            "standard": { "type": "string", "description": "A metric screw size, such as \"M6\", sizing the hole and its head" },
            "fit": choice(&["close", "normal", "loose", "tapped", "tapped_fine", "heat_set_insert"], "With standard: a clearance fit, a tap drill or an insert hole"),
            "thread_form": choice(&["modelled", "cosmetic"], "A tapped hole's thread: cut into the body or drawn"),
            "thread_hand": choice(&["right", "left"], "A tapped hole's thread hand"),
            "thread_depth": amount("How deep a tapped hole's thread runs; all of it by default"),
            "body": reference("The body drilled; by default the one the sketch lies on"),
            "reversed": flag("Drill along the sketch plane's normal instead of against it"),
            "name": name()
        }),
        &["sketch"],
    )
}

pub fn sweep() -> Value {
    let properties = with_target(properties(json!({
        "sketch": reference("The sketch whose closed regions are swept"),
        "path": path(),
        "orientation": choice(&["square", "parallel"], "Keep the section square to the path (the default) or parallel to itself"),
        "twist": amount("The angle the section turns about the path by its end, counter-clockwise looking back along the path, 0 deg by default"),
        "scale": amount("The section's size at the end of the path against its start, such as 0.5; 1 by default")
    })));
    object(Value::Object(properties), &["sketch", "path"])
}

pub fn pipe() -> Value {
    let properties = with_target(properties(json!({
        "path": path(),
        "shape": choice(&["round", "square", "triangle"], "The section's shape, round by default"),
        "size": amount("The section's diameter, width or side, 10 mm by default"),
        "wall": amount("Hollow it with a wall of this thickness"),
        "hollow": flag("false makes it solid; true hollows it with a 1 mm wall"),
        "bend": amount("The radius sharp corners of the path bend by, 10 mm by default")
    })));
    object(Value::Object(properties), &["path"])
}

pub fn loft() -> Value {
    let properties = with_target(properties(json!({
        "profiles": {
            "type": "array",
            "description": "The sketches it passes through, in order, two at least (one with an end point); a sketch may instead be {sketch, inside: [x, y]} for the one region around that point",
            "items": {
                "anyOf": [
                    { "type": ["integer", "string"] },
                    {
                        "type": "object",
                        "properties": {
                            "sketch": reference("The sketch, by feature id or name"),
                            "inside": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 }
                        },
                        "required": ["sketch", "inside"],
                        "additionalProperties": false
                    }
                ]
            }
        },
        "end": {
            "description": "Ends in a point after the last profile, as a cone's apex: (\"origin\", a corner {body, corner}, a round edge's centre, a sketch or datum point); \"off\" takes it away",
            "anyOf": [{ "type": "string", "enum": ["off"] }, point()]
        },
        "skin": choice(&["smooth", "ruled"], "Smooth through every profile (the default) or straight between each pair"),
        "closed": flag("Run on from the last profile back into the first, making a ring of three profiles or more"),
        "start_weight": amount("Leave the first profile square to its plane with this weight, such as 1; 0 leaves it free. Smooth, open lofts only"),
        "end_weight": amount("Reach the last profile square to its plane with this weight; 0 leaves it free")
    })));
    object(Value::Object(properties), &["profiles"])
}

pub fn coil() -> Value {
    let properties = with_target(properties(json!({
        "axis": axis(),
        "reversed": flag("Climb the other way along the axis"),
        "diameter": amount("The coil's diameter, 20 mm by default"),
        "turns": amount("How many turns; give two of turns, pitch and height"),
        "pitch": amount("The rise per turn"),
        "height": amount("The whole height"),
        "shape": choice(&["round", "square"], "The section's shape"),
        "section": amount("The section's diameter or width, 2 mm by default"),
        "placement": choice(&["inside", "centred", "outside"], "Where the section sits against the diameter"),
        "taper": amount("A cone angle, such as \"5 deg\""),
        "clockwise": flag("Wind clockwise instead of counter-clockwise")
    })));
    object(Value::Object(properties), &[])
}

pub fn rib() -> Value {
    object(
        json!({
            "sketch": reference("The sketch holding the open line or arc chain"),
            "body": reference("The body the rib joins; by default the one the sketch lies on"),
            "thickness": amount("The rib's thickness, 2 mm by default"),
            "side": choice(&["symmetric", "one_side", "other_side"], "Which side of the curves the thickness lies"),
            "direction": choice(&["parallel", "square"], "Grow within the sketch plane or square to it"),
            "draft": amount("A rib's or web's draft, such as \"1 deg\": it widens by it toward the body; a rib parallel to the sketch takes at least 0.1 deg"),
            "ends": choice(&["straight", "round_arcs"], "A web's free arc ends run on straight (the default) or round their circles"),
            "reversed": flag("Grow towards the other side"),
            "name": name()
        }),
        &["sketch"],
    )
}

pub fn thread() -> Value {
    object(
        json!({
            "body": reference("The body, by id or name"),
            "face": { "type": "string", "description": "A cylindrical face's key from list_faces" },
            "family": choice(&["metric_coarse", "metric_fine", "iso_228", "iso_7", "trapezoidal"], "The thread standard, metric coarse by default"),
            "size": { "type": "string", "description": "A size of the family, such as \"M8\" or \"M8x1\"; the nearest to the face by default" },
            "class": { "type": "string", "description": "A tolerance class, such as \"6H\" or \"6g\"" },
            "hand": choice(&["right", "left"], "Right-handed by default"),
            "length": amount("How far the thread runs from the face's end"),
            "full": flag("Thread the whole face"),
            "reversed": flag("Start from the face's other end"),
            "form": choice(&["modelled", "cosmetic"], "Cut the thread into the body (the default) or draw it"),
            "name": name()
        }),
        &["body", "face"],
    )
}

pub fn split_face() -> Value {
    object(
        json!({
            "body": reference("The body, by id or name"),
            "faces": keys("Face keys from list_faces"),
            "along": along(),
            "carry": {
                "description": "With along {sketch}: how the curves reach the faces: \"square\" (the default, straight onto them), \"wrapped\" around a cylinder, cone or flat face, or along an axis as add_linear_pattern takes it",
                "anyOf": [{ "type": "string", "enum": ["square", "wrapped"] }, axis()]
            },
            "name": name()
        }),
        &["body", "faces", "along"],
    )
}

pub fn delete_face() -> Value {
    object(
        json!({
            "body": reference("The body, by id or name"),
            "faces": keys("Face keys from list_faces"),
            "name": name()
        }),
        &["body", "faces"],
    )
}

pub fn emboss() -> Value {
    object(
        json!({
            "body": reference("The body, by id or name"),
            "faces": keys("Face keys from list_faces: flat faces of one plane, or faces of one cylinder or cone"),
            "sketch": reference("The sketch of closed outlines or text; lying parallel to a flat face, or on a plane along the axis of a round one"),
            "depth": amount("How far the outlines stand out or sink in, such as \"0.5 mm\""),
            "form": choice(&["raise", "recess"], "Raise the outlines from the faces (the default) or recess them into the faces"),
            "name": name()
        }),
        &["body", "faces", "sketch", "depth"],
    )
}

pub fn move_face() -> Value {
    object(
        json!({
            "body": reference("The body, by id or name"),
            "faces": keys("Face keys from list_faces"),
            "distance": amount("How far the faces shift along the direction; negative shifts them back. 0 mm by default"),
            "direction": {
                "description": "What the faces shift along: \"normal\" (the default, out of the body square to the first flat face chosen) or an axis as add_linear_pattern takes it",
                "anyOf": [{ "type": "string", "enum": ["normal"] }, axis()]
            },
            "angle": amount("How far the faces tilt about the axis, tilted before they shift; 0 deg by default"),
            "axis": {
                "description": "What the faces tilt about: \"none\" (the default) or an axis or straight edge as add_linear_pattern takes it",
                "anyOf": [{ "type": "string", "enum": ["none"] }, axis()]
            },
            "name": name()
        }),
        &["body", "faces"],
    )
}

pub fn split() -> Value {
    object(
        json!({
            "body": reference("The body split"),
            "along": along(),
            "flipped": flag("Swap which piece comes first"),
            "name": name()
        }),
        &["body", "along"],
    )
}

pub fn scale() -> Value {
    object(
        json!({
            "body": reference("The body scaled"),
            "factor": amount("How many times larger, such as 2 or 0.5"),
            "center": {
                "type": "array",
                "items": { "type": ["string", "number"] },
                "minItems": 3,
                "maxItems": 3,
                "description": "The point it scales about, [x, y, z] in mm; the origin by default"
            },
            "name": name()
        }),
        &["body", "factor"],
    )
}

pub fn edit_feature() -> Value {
    json!({
        "type": "object",
        "properties": {
            "feature": reference("The feature, by id or name from list_features")
        },
        "required": ["feature"],
        "additionalProperties": true,
        "description": "Besides feature, any arguments of the tool that adds its kind; each replaces that part of the feature"
    })
}
