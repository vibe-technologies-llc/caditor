use serde_json::{Value, json};

use crate::mcp::{
    feature_schemas::{amount, axis, features, name, on_edge, on_face, plane, point},
    sweep_schemas::{choice, flag},
    tools::{object, reference},
};

fn pair(item: Value, description: &str) -> Value {
    json!({ "type": "array", "items": item, "minItems": 2, "maxItems": 2, "description": description })
}

fn three(item: Value, description: &str) -> Value {
    json!({ "type": "array", "items": item, "minItems": 3, "maxItems": 3, "description": description })
}

fn skipped() -> Value {
    json!({
        "type": "array",
        "items": { "type": "integer", "minimum": 1 },
        "description": "Copies left out, counted from the original as 0"
    })
}

pub fn datum_plane() -> Value {
    object(
        json!({
            "base": plane(),
            "offset": amount("With base: how far along the base plane's normal"),
            "turn_axis": axis(),
            "turn_angle": amount("With turn_axis: the plane turned about it"),
            "through_points": three(point(), "Instead of base: the three points it passes through"),
            "midway": pair(plane(), "Instead of base: the two planes it lies midway between"),
            "lines": pair(axis(), "Instead of base: two lines in one plane that it holds"),
            "through_axis": axis(),
            "square_to_axis": axis(),
            "tangent_to": on_face(),
            "square_to_edge": on_edge(),
            "point": point(),
            "distance": amount("With square_to_edge: how far along the edge from its start, 0 by default"),
            "name": name()
        }),
        &[],
    )
}

pub fn datum_axis() -> Value {
    object(
        json!({
            "along": axis(),
            "planes": pair(plane(), "Where two planes meet"),
            "through_points": pair(point(), "The two points it passes through"),
            "square_to_plane": plane(),
            "square_to_face": on_face(),
            "point": point(),
            "name": name()
        }),
        &[],
    )
}

pub fn datum_point() -> Value {
    object(
        json!({
            "at": point(),
            "offset": three(amount("A length"), "With at: how far from it along X, Y and Z"),
            "lines": pair(axis(), "Where two lines cross"),
            "axis": axis(),
            "plane": plane(),
            "planes": three(plane(), "Where three planes meet"),
            "along_edge": on_edge(),
            "distance": amount("With along_edge: how far along the edge from its start, 0 by default"),
            "middle_of": on_edge(),
            "centre_of": on_face(),
            "name": name()
        }),
        &[],
    )
}

pub fn coordinate_system() -> Value {
    object(
        json!({
            "origin": point(),
            "x_axis": axis(),
            "plane": plane(),
            "reverse_x": flag("Point its X axis the other way"),
            "reverse_z": flag("Point its Z axis the other way"),
            "name": name()
        }),
        &[],
    )
}

pub fn mate() -> Value {
    object(
        json!({
            "body": reference("The body the mate moves, by id or name"),
            "face": { "type": "string", "description": "A face key of the body from list_faces: a flat face (with to_plane or to_round) or a cylindrical or spherical one (with to_plane)" },
            "axis": axis(),
            "point": point(),
            "to_plane": plane(),
            "to_axis": axis(),
            "to_point": point(),
            "to_round": on_face(),
            "distance": amount("A flush mate's gap between the faces, 0 by default"),
            "angle": amount("Turn the face or axis to this angle to the other instead, such as \"90 deg\""),
            "flipped": flag("Flip the side the body ends up on"),
            "name": name()
        }),
        &["body"],
    )
}

pub fn helix() -> Value {
    object(
        json!({
            "axis": axis(),
            "reversed": flag("Climb the other way along the axis"),
            "diameter": amount("The helix's diameter, 20 mm by default"),
            "turns": amount("How many turns; give two of turns, pitch and height"),
            "pitch": amount("The rise per turn"),
            "height": amount("The whole height"),
            "taper": amount("A cone angle, such as \"5 deg\""),
            "start_angle": amount("Where the helix starts round its axis, an angle counter-clockwise from its frame's X direction; 0 deg by default. Giving it clears start_point"),
            "start_point": point(),
            "clockwise": flag("Wind clockwise instead of counter-clockwise"),
            "name": name()
        }),
        &[],
    )
}

pub fn curve_pattern() -> Value {
    object(
        json!({
            "body": reference("The body repeated"),
            "features": features("Instead of a body: holes, cuts, bosses or embossed outlines of one body, repeated on it"),
            "sketch": reference("The sketch whose curves, joined end to end, the copies follow"),
            "count": amount("How many, the original included; 4 by default"),
            "spacing": amount("How far apart the copies stand along the curve; gives spread false"),
            "spread": flag("Share the whole curve evenly between the copies (the default)"),
            "follow": flag("Turn each copy as the curve turns"),
            "reversed": flag("Run along the curve the other way"),
            "skipped": skipped(),
            "name": name()
        }),
        &["sketch"],
    )
}

pub fn point_pattern() -> Value {
    object(
        json!({
            "body": reference("The body repeated"),
            "features": features("Instead of a body: holes, cuts, bosses or embossed outlines of one body, repeated on it"),
            "sketch": reference("The sketch whose lone points place the copies"),
            "from": point(),
            "skipped": skipped(),
            "name": name()
        }),
        &["sketch"],
    )
}

pub fn import() -> Value {
    object(
        json!({
            "file": { "type": "string", "description": "The file to import, in the model's folder or an allowed one: .step, .stp, .stl, .obj, .3mf, .dxf or .svg" },
            "sketch": reference("A drawing only: the sketch it goes into"),
            "plane": choice(&["XY", "XZ", "YZ"], "A drawing only: the plane of the new sketch it goes into, XY by default"),
            "unit": choice(&["as_in_file", "micrometres", "millimetres", "centimetres", "metres"], "A drawing only: the unit its numbers are in, as the file says by default"),
            "scale": { "type": "number", "description": "A drawing only: a factor from 0.000001 to 1000000 to scale it by, 1 by default" },
            "recentre": { "type": "boolean", "description": "A drawing only: move it so its middle lies on the sketch's origin" },
            "left_out_layers": { "type": "array", "items": { "type": "string" }, "description": "A drawing only: layers (a DXF's layers, an SVG's layers and top groups, named as the answer's layers lists them) to leave out" }
        }),
        &["file"],
    )
}

pub fn save_as() -> Value {
    object(
        json!({
            "file": { "type": "string", "description": "The new model file, ending in .caditor, in the model's folder or an allowed one" }
        }),
        &["file"],
    )
}
