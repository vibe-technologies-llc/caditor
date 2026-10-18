use serde_json::{Value, json};

use crate::mcp::{
    tools::{entity, object, point, reference},
    writing::MOST_ITEMS,
};

fn entities(description: &str) -> Value {
    json!({
        "type": "array",
        "items": entity(),
        "minItems": 1,
        "maxItems": MOST_ITEMS,
        "description": description
    })
}

fn expression(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn blend_end(description: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "curve": entity(),
            "end": entity()
        },
        "required": ["curve", "end"],
        "additionalProperties": false,
        "description": format!("{description}: the line, arc or spline and the point ending it")
    })
}

fn corners_property() -> Value {
    json!({
        "type": "array",
        "items": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
        "maxItems": MOST_ITEMS,
        "description": "project: corners of body as [x, y, z] positions of its vertices"
    })
}

pub fn modify_sketch() -> Value {
    object(
        json!({
            "sketch": reference("The sketch, by feature id or name"),
            "operation": {
                "type": "string",
                "enum": [
                    "trim", "extend", "offset", "mirror", "rectangular_pattern", "circular_pattern",
                    "fillet", "chamfer", "move", "rotate", "scale", "project", "intersect",
                    "construction", "split", "break", "respace", "insert_control_point", "tangent_circle", "blend", "gear",
                    "copy"
                ],
                "description": "trim takes pieces; extend takes curve and end or near; offset takes curves and distance; mirror takes items and about; rectangular_pattern takes items, count, spacing and optionally angle and rows; circular_pattern takes items, center, count and optionally angle; fillet takes corners and radius; chamfer takes corners, distance and optionally second_distance or angle; move takes items and by or to; rotate takes items and angle, optionally about; scale takes items and factor, optionally about; project takes any of: body with edges, faces or corners, source_sketch (with source_curves, else the whole sketch), axes, datums, origin; intersect takes body (with a face, else the whole body), datum or plane; construction takes items and optionally construction (else switches them all the other way); split takes curve and point; break takes curves; respace takes splines; insert_control_point takes splines and optionally point (with a single spline); tangent_circle takes curves (two with a radius, or three) and optionally near; blend takes first and second ends and optionally continuity; gear takes optionally shape, center or at, and the gear's sizes; copy takes items, optionally from_sketch and by or to"
            },
            "pieces": {
                "type": "array",
                "minItems": 1,
                "maxItems": MOST_ITEMS,
                "items": {
                    "type": "object",
                    "properties": {
                        "curve": entity(),
                        "near": point("A position on or by the piece to trim away, between the curves crossing it")
                    },
                    "required": ["curve", "near"],
                    "additionalProperties": false
                },
                "description": "trim: the pieces to cut away, each trimmed back to the curves crossing it; a curve nothing crosses is deleted"
            },
            "curve": json!({
                "type": ["integer", "string"],
                "description": "extend: the line, arc, elliptical arc or open spline to extend to the next curve it meets"
            }),
            "end": json!({
                "type": ["integer", "string"],
                "description": "extend: the point ending the curve at the end to extend"
            }),
            "near": point("extend: a position by the end to extend, instead of end; tangent_circle: of the circles that fit, the one whose centre is nearest this position"),
            "curves": entities("offset: lines, arcs, circles, ellipses and splines, any number of chains, each side set by toward; break: the lines, arcs, elliptical arcs and control-point splines to break at every crossing with other curves and the axes; tangent_circle: the lines, circles and arcs the circle touches"),
            "whole_chain": { "type": "boolean", "description": "offset: take every curve joined end to end with the given ones, as a click on a curve does" },
            "distance": expression("offset: how far, such as \"3 mm\", negative for the other side; chamfer: how far from the corner it cuts"),
            "toward": point("offset: a position on the side to offset toward; without it the outside of a closed chain"),
            "both_sides": { "type": "string", "enum": ["round", "flat"], "description": "offset: the distance on each side, the ends joined by round arcs or flat lines" },
            "items": entities("mirror, patterns, move, rotate, scale, construction, copy: the entities to act on (curves bring their points)"),
            "about": json!({
                "type": ["integer", "string"],
                "description": "mirror: the line or axis to mirror about; rotate, scale: a point not part of what moves to turn or scale about (else the middle of what moves)"
            }),
            "count": { "type": "integer", "minimum": 2, "description": "patterns: how many, counting the original" },
            "spacing": expression("rectangular_pattern: the distance between copies, such as \"10 mm\""),
            "angle": expression("rectangular_pattern: the direction of the row from the horizontal axis (0 by default); circular_pattern: the angle the copies spread over (a full turn by default); rotate: the turn, counter-clockwise; chamfer: the cut's angle from the first curve"),
            "rows": {
                "type": "object",
                "properties": {
                    "count": { "type": "integer", "minimum": 1 },
                    "spacing": expression("The distance between rows"),
                    "angle": expression("The rows' direction, square to the first by default")
                },
                "required": ["count", "spacing"],
                "additionalProperties": false,
                "description": "rectangular_pattern: a second direction making a grid"
            },
            "center": json!({
                "type": ["integer", "string"],
                "description": "circular_pattern: the point to repeat about, or origin; gear: the point at the gear's middle (origin by default)"
            }),
            "construction": { "type": "boolean", "description": "construction: true to make the curves construction geometry, false to make them ordinary; without it every curve switches the way the first one is not" },
            "point": json!({
                "type": ["integer", "string"],
                "description": "split: the point lying on the curve between its ends where it is cut; insert_control_point: a point near where the one spline gets its new control point (else its longest stretch)"
            }),
            "splines": entities("respace: fit-point splines saved before centripetal spacing"),
            "first": blend_end("blend: the end of the first curve"),
            "second": blend_end("blend: the end of the second curve"),
            "continuity": { "type": "string", "enum": ["tangent", "curvature"], "description": "blend: tangent (the default) or curvature-continuous" },
            "shape": { "type": "string", "enum": ["gear", "pair", "sprocket"], "description": "gear: one spur gear (the default), a meshing pair (the second to the right of the first) or a roller chain sprocket" },
            "at": point("gear: a position for the gear's middle, instead of center"),
            "module": expression("gear: the pitch diameter over the number of teeth, such as \"2 mm\" (the default)"),
            "teeth": { "type": "integer", "minimum": 1, "description": "gear: the number of teeth (20 by default)" },
            "pressure_angle": expression("gear: such as \"20 deg\" (the default)"),
            "profile_shift": { "type": "number", "description": "gear: how far the teeth move out, in modules (0 by default)" },
            "root_fillet": expression("gear: the radius rounding the root of each gap, such as \"0.75 mm\" (the default); 0 for a sharp root"),
            "bore": expression("gear: the diameter of the hole in the middle (none by default)"),
            "meshing_teeth": { "type": "integer", "minimum": 1, "description": "gear with shape pair: the second gear's teeth (30 by default)" },
            "meshing_bore": expression("gear with shape pair: the second gear's hole (none by default)"),
            "chain": { "type": "string", "description": "gear with shape sprocket: the ISO 606 chain designation such as \"08B\" (the default) or \"10A\"" },
            "from_sketch": reference("copy: the sketch to copy from, by feature id or name (this sketch by default)"),
            "corners": {
                "type": "array",
                "minItems": 1,
                "maxItems": MOST_ITEMS,
                "items": {
                    "description": "A point where two lines, arcs or elliptical arcs meet, or a pair [curve, curve] meeting at their ends",
                    "oneOf": [entity(), { "type": "array", "items": entity(), "minItems": 2, "maxItems": 2 }]
                },
                "description": "fillet, chamfer: the corners to round or cut"
            },
            "radius": expression("fillet: the radius, such as \"2 mm\""),
            "second_distance": expression("chamfer: the distance along the second curve"),
            "by": point("move, copy: the offset to move or place the copy by"),
            "to": point("move: where the first point of what moves goes; copy: where the middle of the copy goes"),
            "factor": { "type": "number", "exclusiveMinimum": 0, "description": "scale: the factor, above zero" },
            "body": reference("project, intersect: the body, by id or name from list_bodies"),
            "edges": { "type": "array", "items": { "type": "string" }, "maxItems": MOST_ITEMS, "description": "project: edge keys from list_edges" },
            "faces": { "type": "array", "items": { "type": "string" }, "maxItems": MOST_ITEMS, "description": "project: face keys from list_faces, each bringing every edge around it" },
            "face": { "type": "string", "description": "intersect: one face key from list_faces, to draw only the cut through that face (the whole body without it)" },
            "corners": corners_property(),
            "source_sketch": reference("project: another sketch above this one in the tree, by feature id or name, to project curves of"),
            "source_curves": entities("project: the curves or points of source_sketch to project; without them every curve and lone point of it"),
            "axes": { "type": "array", "items": { "type": "string", "enum": ["X", "Y", "Z"] }, "maxItems": 3, "description": "project: the model's axes, each a line spanning the shown bodies or a point where the axis stands square to the sketch" },
            "datums": { "type": "array", "items": reference("A datum axis or point"), "maxItems": MOST_ITEMS, "description": "project: datum axes and points above the sketch, by feature id or name" },
            "origin": { "type": "boolean", "description": "project: the model's origin" },
            "datum": reference("intersect: a datum plane above the sketch, giving the construction line where it crosses the sketch"),
            "plane": { "type": "string", "enum": ["XY", "XZ", "YZ"], "description": "intersect: a principal plane, giving the construction line where it crosses the sketch" }
        }),
        &["sketch", "operation"],
    )
}
