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

pub fn modify_sketch() -> Value {
    object(
        json!({
            "sketch": reference("The sketch, by feature id or name"),
            "operation": {
                "type": "string",
                "enum": [
                    "trim", "extend", "offset", "mirror", "rectangular_pattern", "circular_pattern",
                    "fillet", "chamfer", "move", "rotate", "scale", "project"
                ],
                "description": "trim takes pieces; extend takes curve and end or near; offset takes curves and distance; mirror takes items and about; rectangular_pattern takes items, count, spacing and optionally angle and rows; circular_pattern takes items, center, count and optionally angle; fillet takes corners and radius; chamfer takes corners, distance and optionally second_distance or angle; move takes items and by or to; rotate takes items and angle, optionally about; scale takes items and factor, optionally about; project takes body and edges or faces"
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
            "near": point("extend: a position by the end to extend, instead of end"),
            "curves": entities("offset: lines, arcs and circles forming one chain, or one ellipse"),
            "whole_chain": { "type": "boolean", "description": "offset: take every curve joined end to end with the given ones, as a click on a curve does" },
            "distance": expression("offset: how far, such as \"3 mm\", negative for the other side; chamfer: how far from the corner it cuts"),
            "toward": point("offset: a position on the side to offset toward; without it the outside of a closed chain"),
            "both_sides": { "type": "string", "enum": ["round", "flat"], "description": "offset: the distance on each side, the ends joined by round arcs or flat lines" },
            "items": entities("mirror, patterns, move, rotate, scale: the entities to act on (curves bring their points)"),
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
                "description": "circular_pattern: the point to repeat about, or origin"
            }),
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
            "by": point("move: the offset to move by"),
            "to": point("move: where the first point of what moves goes"),
            "factor": { "type": "number", "exclusiveMinimum": 0, "description": "scale: the factor, above zero" },
            "body": reference("project: the body, by id or name from list_bodies"),
            "edges": { "type": "array", "items": { "type": "string" }, "maxItems": MOST_ITEMS, "description": "project: edge keys from list_edges" },
            "faces": { "type": "array", "items": { "type": "string" }, "maxItems": MOST_ITEMS, "description": "project: face keys from list_faces, each bringing every edge around it" }
        }),
        &["sketch", "operation"],
    )
}
