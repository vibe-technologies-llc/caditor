use std::f64::consts::PI;

use caditor_geometry::{Point3, Vector3};

use crate::import::dxf::{Fields, Record, geometry::Shape};

const CLOSED: i64 = 2;
const NO_START_CAPS: i64 = 4;
const NO_END_CAPS: i64 = 8;
const SHOW_MITERS: i64 = 2;
const START_SQUARE: i64 = 16;
const START_INNER_ARCS: i64 = 32;
const START_ROUND: i64 = 64;
const END_SQUARE: i64 = 256;
const END_INNER_ARCS: i64 = 512;
const END_ROUND: i64 = 1024;
const INNER_ARCS_FROM_ELEMENTS: usize = 4;

struct Vertex {
    location: Point3,
    miter: Vector3,
    offsets: Vec<f64>,
}

struct Cap {
    square: i64,
    round: i64,
    inner_arcs: i64,
}

const START: Cap = Cap {
    square: START_SQUARE,
    round: START_ROUND,
    inner_arcs: START_INNER_ARCS,
};

const END: Cap = Cap {
    square: END_SQUARE,
    round: END_ROUND,
    inner_arcs: END_INNER_ARCS,
};

pub(super) fn mline(record: &Record, style: i64) -> Option<Vec<Shape>> {
    let vertices = vertices(record)?;
    let elements = vertices.first()?.offsets.len();
    if vertices.len() < 2
        || elements == 0
        || vertices
            .iter()
            .any(|vertex| vertex.offsets.len() != elements)
    {
        return None;
    }
    let flags = record.flags(71);
    let closed = flags & CLOSED != 0;
    let mut rows: Vec<Vec<Point3>> = vertices
        .iter()
        .map(|vertex| {
            vertex
                .offsets
                .iter()
                .map(|offset| vertex.location + vertex.miter * *offset)
                .collect()
        })
        .collect();
    if closed && let Some(first) = rows.first().cloned() {
        rows.push(first);
    }
    let mut shapes: Vec<Shape> = rows
        .windows(2)
        .flat_map(|pair| match pair {
            [from, to] => from
                .iter()
                .zip(to)
                .map(|(start, end)| Shape::Line(*start, *end))
                .collect(),
            _ => Vec::new(),
        })
        .collect();
    let order = ordered_elements(&vertices.first()?.offsets);
    let (bottom, top) = (*order.first()?, *order.last()?);
    if style & SHOW_MITERS != 0 {
        let joints = if closed {
            0..vertices.len()
        } else {
            1..vertices.len() - 1
        };
        for row in rows.get(joints)? {
            shapes.push(Shape::Line(*row.get(top)?, *row.get(bottom)?));
        }
    }
    if !closed {
        let normal = record.normal();
        let first = vertices.first()?;
        let second = vertices.get(1)?;
        let last = vertices.last()?;
        let before_last = vertices.get(vertices.len() - 2)?;
        if flags & NO_START_CAPS == 0 {
            let outward = first.location - second.location;
            shapes.extend(cap(rows.first()?, &order, style, &START, outward, normal));
        }
        if flags & NO_END_CAPS == 0 {
            let outward = last.location - before_last.location;
            shapes.extend(cap(rows.last()?, &order, style, &END, outward, normal));
        }
    }
    shapes.retain(|shape| !matches!(shape, Shape::Line(start, end) if start == end));
    Some(shapes)
}

fn vertices(record: &Record) -> Option<Vec<Vertex>> {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut awaiting_offset = false;
    for pair in &record.pairs {
        match pair.code {
            11 => {
                vertices.push(Vertex {
                    location: Point3::new(pair.real()?, 0.0, 0.0),
                    miter: Vector3::ZERO,
                    offsets: Vec::new(),
                });
                awaiting_offset = false;
            }
            21 => vertices.last_mut()?.location.y = pair.real()?,
            31 => vertices.last_mut()?.location.z = pair.real()?,
            13 => vertices.last_mut()?.miter.x = pair.real()?,
            23 => vertices.last_mut()?.miter.y = pair.real()?,
            33 => vertices.last_mut()?.miter.z = pair.real()?,
            74 => {
                vertices.last_mut()?.offsets.push(0.0);
                awaiting_offset = pair.integer()? > 0;
            }
            41 if awaiting_offset => {
                *vertices.last_mut()?.offsets.last_mut()? = pair.real()?;
                awaiting_offset = false;
            }
            _ => {}
        }
    }
    Some(vertices)
}

fn ordered_elements(offsets: &[f64]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..offsets.len()).collect();
    order.sort_by(|a, b| {
        let offset = |index: &usize| offsets.get(*index).copied().unwrap_or_default();
        offset(a).total_cmp(&offset(b))
    });
    order
}

fn cap(
    row: &[Point3],
    order: &[usize],
    style: i64,
    kind: &Cap,
    outward: Vector3,
    normal: Vector3,
) -> Vec<Shape> {
    let mut shapes = Vec::new();
    let at = |position: usize| order.get(position).and_then(|index| row.get(*index));
    let (Some(bottom), Some(top)) = (at(0), at(order.len().saturating_sub(1))) else {
        return shapes;
    };
    if style & kind.square != 0 {
        shapes.push(Shape::Line(*top, *bottom));
    }
    if style & kind.round != 0 {
        shapes.extend(half_circle(*top, *bottom, outward, normal));
    }
    if style & kind.inner_arcs != 0
        && order.len() >= INNER_ARCS_FROM_ELEMENTS
        && let (Some(inner_bottom), Some(inner_top)) = (at(1), at(order.len() - 2))
    {
        shapes.extend(half_circle(*inner_top, *inner_bottom, outward, normal));
    }
    shapes
}

fn half_circle(from: Point3, to: Point3, outward: Vector3, normal: Vector3) -> Option<Shape> {
    let center = from.midpoint(to);
    let major = from - center;
    let side = normal.cross(major);
    let minor = side.try_normalize()? * major.length();
    let minor = if minor.dot(outward) < 0.0 {
        -minor
    } else {
        minor
    };
    Some(Shape::Conic {
        center,
        major,
        minor,
        start: 0.0,
        sweep: PI,
    })
}
