use serde_json::{Map, Value, json};

use crate::mcp::{
    sweep_schemas::{regions, thin_wall, wall_side},
    tools::{object, reference},
};

pub(super) fn amount(description: &str) -> Value {
    json!({ "type": ["string", "number"], "description": description })
}

pub(super) fn keys(description: &str) -> Value {
    json!({ "type": "array", "items": { "type": "string" }, "description": description })
}

pub(super) fn features(description: &str) -> Value {
    json!({ "type": "array", "items": { "type": ["integer", "string"] }, "description": description })
}

pub(super) fn name() -> Value {
    json!({ "type": "string", "description": "The new feature's name; by default the tool's own" })
}

pub(super) fn on_face() -> Value {
    json!({
        "type": "object",
        "properties": {
            "body": reference("The body, by id or name"),
            "face": { "type": "string", "description": "A face key from list_faces" }
        },
        "required": ["body", "face"],
        "additionalProperties": false
    })
}

fn datum(kind: &str) -> Value {
    json!({
        "type": "object",
        "properties": { "datum": reference(&format!("A datum {kind}, by feature id or name")) },
        "required": ["datum"],
        "additionalProperties": false
    })
}

fn frame_part(part: &str, names: &[&str]) -> Value {
    let mut properties = Map::new();
    properties.insert(
        "frame".to_owned(),
        reference("A coordinate system, by feature id or name"),
    );
    properties.insert(part.to_owned(), json!({ "type": "string", "enum": names }));
    json!({
        "type": "object",
        "properties": properties,
        "required": ["frame", part],
        "additionalProperties": false
    })
}

pub(super) fn on_edge() -> Value {
    json!({
        "type": "object",
        "properties": {
            "body": reference("The body, by id or name"),
            "edge": { "type": "string", "description": "An edge key from list_edges" }
        },
        "required": ["body", "edge"],
        "additionalProperties": false
    })
}

pub fn plane() -> Value {
    json!({
        "description": "XY, XZ or YZ; a flat face as {body, face}; a datum plane as {datum}; or a coordinate system's plane as {frame, plane}",
        "anyOf": [
            { "type": "string", "enum": ["XY", "XZ", "YZ"] },
            on_face(),
            datum("plane"),
            frame_part("plane", &["XY", "XZ", "YZ"])
        ]
    })
}

pub fn point() -> Value {
    json!({
        "description": "\"origin\"; a corner as {body, corner: [x, y, z]} (list_edges gives the ends of straight edges); a round edge's centre as {body, edge}; a sphere's or torus's centre as {body, face}; a sketch point as {sketch, point}; or a datum point or a coordinate system's origin as {datum}",
        "anyOf": [
            { "type": "string", "enum": ["origin"] },
            {
                "type": "object",
                "properties": {
                    "body": reference("The body, by id or name"),
                    "corner": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["body", "corner"],
                "additionalProperties": false
            },
            on_edge(),
            on_face(),
            {
                "type": "object",
                "properties": {
                    "sketch": reference("The sketch, by feature id or name"),
                    "point": { "type": ["integer", "string"], "description": "A point's entity id, or origin" }
                },
                "required": ["sketch", "point"],
                "additionalProperties": false
            },
            datum("point or coordinate system")
        ]
    })
}

pub fn axis() -> Value {
    json!({
        "description": "X, Y or Z; a straight or round edge as {body, edge}; a round face's axis as \
                        {body, face}; a sketch line as {sketch, line}; a datum axis as {datum}; \
                        or a coordinate system's axis as {frame, axis}",
        "anyOf": [
            { "type": "string", "enum": ["X", "Y", "Z"] },
            on_edge(),
            on_face(),
            {
                "type": "object",
                "properties": {
                    "sketch": reference("The sketch, by feature id or name"),
                    "line": { "type": ["integer", "string"], "description": "A line's entity id" }
                },
                "required": ["sketch", "line"],
                "additionalProperties": false
            },
            datum("axis"),
            frame_part("axis", &["X", "Y", "Z"])
        ]
    })
}

fn spot() -> Value {
    json!({
        "description": "A point [x, y, z] in mm, a face as {body, face} or an edge as {body, edge}",
        "anyOf": [
            { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
            on_face(),
            {
                "type": "object",
                "properties": {
                    "body": reference("The body, by id or name"),
                    "edge": { "type": "string", "description": "An edge key from list_edges" }
                },
                "required": ["body", "edge"],
                "additionalProperties": false
            }
        ]
    })
}

pub(super) fn operation() -> Value {
    json!({
        "type": "string",
        "enum": ["new_body", "add", "remove", "intersect"],
        "description": "What it does to the body; decided as the app decides when left out"
    })
}

fn triple(description: &str) -> Value {
    json!({
        "type": "array",
        "items": { "type": ["string", "number"] },
        "minItems": 3,
        "maxItems": 3,
        "description": description
    })
}

pub fn list_edges() -> Value {
    object(
        json!({
            "body": reference("The body, by id or name from list_bodies"),
            "before": reference("A feature by id or name: list the body as that feature received it, which is what edit_feature resolves the faces and edges it is given on")
        }),
        &["body"],
    )
}

pub fn measure() -> Value {
    object(json!({ "from": spot(), "to": spot() }), &["from", "to"])
}

pub fn revolve() -> Value {
    object(
        json!({
            "sketch": reference("The sketch whose closed regions are revolved"),
            "axis": {
                "description": "A line of the sketch by entity id, horizontal_axis or vertical_axis (the default), or a model axis as add_linear_pattern takes it",
                "anyOf": [{ "type": ["integer", "string"] }, axis()]
            },
            "angle": amount("How far it turns, such as \"90 deg\"; a full turn when left out"),
            "second_angle": amount("How far it also turns the other way from the sketch"),
            "up_to": {
                "description": "Turn until it reaches a plane instead of by an angle: XY, XZ, YZ, a flat face {body, face} or a datum plane {datum}",
                "anyOf": [plane()]
            },
            "symmetric": { "type": "boolean", "description": "Half the angle each way from the sketch" },
            "reversed": { "type": "boolean", "description": "Turn the other way" },
            "regions": regions(),
            "thin_wall": thin_wall(),
            "wall_side": wall_side(),
            "operation": operation(),
            "body": reference("The body it changes; with no operation, it adds to it"),
            "name": name()
        }),
        &["sketch"],
    )
}

fn edge_choice(size: &str, size_description: &str) -> serde_json::Map<String, Value> {
    let mut properties = serde_json::Map::new();
    properties.insert("body".to_owned(), reference("The body, by id or name"));
    properties.insert("edges".to_owned(), keys("Edge keys from list_edges"));
    properties.insert(
        "faces".to_owned(),
        keys("Face keys from list_faces: every edge around each face"),
    );
    properties.insert(
        "all_edges".to_owned(),
        json!({ "type": "boolean", "description": "Every edge of the body" }),
    );
    properties.insert(
        "convex_edges".to_owned(),
        json!({
            "type": "boolean",
            "description": "Every convex edge of the body, kept as edits add or remove convex edges"
        }),
    );
    properties.insert(
        "concave_edges".to_owned(),
        json!({
            "type": "boolean",
            "description": "Every concave edge of the body, kept as edits add or remove concave edges"
        }),
    );
    properties.insert(
        "edges_made_by".to_owned(),
        features(
            "Features, by id or name, whose edges to take: every edge they made, kept as edits change them",
        ),
    );
    properties.insert(size.to_owned(), amount(size_description));
    properties.insert("name".to_owned(), name());
    properties
}

pub fn fillet() -> Value {
    object(
        Value::Object(edge_choice("radius", "Such as \"2 mm\"")),
        &["body", "radius"],
    )
}

pub fn chamfer() -> Value {
    let mut properties = edge_choice("distance", "Such as \"1 mm\"");
    properties.insert(
        "second_distance".to_owned(),
        amount("A second distance, along the other face"),
    );
    properties.insert(
        "angle".to_owned(),
        amount("An angle from the first face instead of a second distance"),
    );
    properties.insert(
        "flipped".to_owned(),
        json!({ "type": "boolean", "description": "Swap the faces the distances are measured on" }),
    );
    object(Value::Object(properties), &["body", "distance"])
}

pub fn shell() -> Value {
    object(
        json!({
            "body": reference("The body hollowed out"),
            "open_faces": keys("Face keys from list_faces to leave open; none makes a closed hollow"),
            "thickness": amount("The wall thickness, such as \"2 mm\""),
            "face_thicknesses": {
                "type": "array",
                "description": "Walls of their own thickness at some faces, as the faces left open do not have: replaces the earlier list, and [] gives every wall the same thickness",
                "items": {
                    "type": "object",
                    "properties": {
                        "face": { "type": "string", "description": "A face key from list_faces" },
                        "thickness": amount("That wall's thickness")
                    },
                    "required": ["face", "thickness"],
                    "additionalProperties": false
                }
            },
            "side": { "type": "string", "enum": ["inside", "outside", "both_sides"] },
            "name": name()
        }),
        &["body", "thickness"],
    )
}

pub fn offset_face() -> Value {
    object(
        json!({
            "body": reference("The body whose faces move"),
            "faces": keys("Face keys from list_faces"),
            "distance": amount("How far the faces move outward; negative moves them in"),
            "tangent": { "type": "boolean", "description": "Also move the faces running smoothly on from them" },
            "name": name()
        }),
        &["body", "faces", "distance"],
    )
}

pub fn primitive() -> Value {
    let size = |description: &str| amount(description);
    object(
        json!({
            "kind": { "type": "string", "enum": ["box", "cylinder", "sphere", "torus", "cone", "wedge", "prism"] },
            "length": size("box, wedge"),
            "width": size("box, wedge"),
            "height": size("box, cylinder, cone, wedge, prism"),
            "diameter": size("cylinder, sphere, torus (ring), prism (across corners)"),
            "tube": size("torus: the tube's diameter"),
            "bottom": size("cone: bottom diameter"),
            "top": size("cone: top diameter, 0 for a point"),
            "top_length": size("wedge: length of the top, 0 for a point"),
            "sides": size("prism: number of sides"),
            "plane": plane(),
            "at": {
                "type": "array",
                "items": { "type": ["string", "number"] },
                "minItems": 2,
                "maxItems": 2,
                "description": "Where on the plane, in its own x and y (mm); a face's middle or the origin by default"
            },
            "anchor": { "type": "string", "enum": ["corner", "base_centre", "centre"] },
            "reversed": { "type": "boolean", "description": "Grow against the plane's normal" },
            "operation": operation(),
            "body": reference("The body it changes"),
            "name": name()
        }),
        &["kind"],
    )
}

pub fn mirror() -> Value {
    object(
        json!({
            "body": reference("The body mirrored"),
            "features": features("Instead of a body: holes, cuts, bosses or embossed outlines of one body, mirrored on it"),
            "faces": keys("Instead of the body or features: face keys from list_faces of the body, mirrored on it, such as a pocket's walls and floor; [] mirrors the whole body again"),
            "plane": plane(),
            "keep_original": { "type": "boolean", "description": "Keep the body as well as its image (default true)" },
            "name": name()
        }),
        &["plane"],
    )
}

fn direction_properties() -> serde_json::Map<String, Value> {
    let mut properties = serde_json::Map::new();
    properties.insert("axis".to_owned(), axis());
    properties.insert(
        "count".to_owned(),
        amount("How many, the original included"),
    );
    properties.insert(
        "spacing".to_owned(),
        amount("The distance between copies, or across all with total"),
    );
    properties.insert(
        "total".to_owned(),
        json!({ "type": "boolean", "description": "The spacing spans from the first copy to the last" }),
    );
    properties.insert(
        "reversed".to_owned(),
        json!({ "type": "boolean", "description": "Go against the axis" }),
    );
    properties
}

pub fn linear_pattern() -> Value {
    let mut properties = direction_properties();
    properties.insert("body".to_owned(), reference("The body repeated"));
    properties.insert(
        "features".to_owned(),
        features("Instead of a body: holes, cuts, bosses or embossed outlines of one body, repeated on it"),
    );
    properties.insert(
        "second".to_owned(),
        object(
            Value::Object(direction_properties()),
            &["axis", "count", "spacing"],
        ),
    );
    properties.insert(
        "skipped".to_owned(),
        json!({
            "type": "array",
            "items": {
                "anyOf": [
                    { "type": "integer", "minimum": 1 },
                    { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 2, "maxItems": 2 }
                ]
            },
            "description": "Copies left out: n for the nth copy along the row (the original is 0), [column, row] in a grid"
        }),
    );
    properties.insert("name".to_owned(), name());
    object(Value::Object(properties), &["axis", "count", "spacing"])
}

pub fn circular_pattern() -> Value {
    object(
        json!({
            "body": reference("The body repeated"),
            "features": features("Instead of a body: holes, cuts, bosses or embossed outlines of one body, repeated on it"),
            "axis": axis(),
            "count": amount("How many, the original included"),
            "angle": amount("The angle they spread over; a full turn by default"),
            "reversed": { "type": "boolean" },
            "skipped": {
                "type": "array",
                "items": { "type": "integer", "minimum": 1 },
                "description": "Copies left out, counted from the original as 0"
            },
            "name": name()
        }),
        &["axis", "count"],
    )
}

pub fn combine() -> Value {
    object(
        json!({
            "body": reference("The body kept and changed"),
            "tools": features("The bodies joined to it, cut from it or intersected with it"),
            "operation": { "type": "string", "enum": ["join", "cut", "intersect"] },
            "keep_tools": { "type": "boolean", "description": "Keep the tool bodies as well" },
            "name": name()
        }),
        &["body", "tools", "operation"],
    )
}

pub fn move_body() -> Value {
    object(
        json!({
            "body": reference("The body moved"),
            "offset": triple("How far along X, Y and Z, such as [10, 0, \"height\"]"),
            "turn": triple("Turns about X, Y and Z through the body's centre, in degrees"),
            "turn_axis": axis(),
            "turn_angle": amount("With turn_axis: the turn about it"),
            "copy": { "type": "boolean", "description": "Move a copy and keep the body where it is" },
            "name": name()
        }),
        &["body"],
    )
}

pub fn rename_feature() -> Value {
    object(
        json!({
            "feature": reference("The feature, by id or name"),
            "name": { "type": "string" }
        }),
        &["feature", "name"],
    )
}

pub fn suppress_features() -> Value {
    object(
        json!({
            "features": features("The features, by id or name"),
            "suppressed": { "type": "boolean", "description": "false brings suppressed features back (default true)" }
        }),
        &["features"],
    )
}

pub fn move_feature() -> Value {
    object(
        json!({
            "feature": reference("The feature moved in the tree"),
            "before": reference("The feature it goes before"),
            "after": reference("The feature it goes after")
        }),
        &["feature"],
    )
}

pub fn delete_features() -> Value {
    object(
        json!({
            "features": features("The features, by id or name"),
            "dependents": {
                "type": "string",
                "enum": ["delete", "keep"],
                "description": "What happens to features using them: needed when there are any"
            }
        }),
        &["features"],
    )
}
