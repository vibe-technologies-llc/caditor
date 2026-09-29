use caditor_geometry::Point3;

use crate::import::dxf::{
    Fields, Record,
    geometry::{Affine, Shape},
};

const INVISIBLE_EDGES: i32 = 70;

pub(super) fn filled_outline(record: &Record) -> Option<Vec<Shape>> {
    let system = Affine::object_system(record.normal())?;
    let [first, second, third, fourth] = corners(record)?;
    let flat = |point: Point3| system.point(Point3::new(point.x, point.y, first.z));
    Some(edges(
        &[first, second, fourth, third].map(flat),
        &[false; 4],
    ))
}

pub(super) fn face_outline(record: &Record) -> Option<Vec<Shape>> {
    let invisible = record.flags(INVISIBLE_EDGES);
    let hidden = [1, 2, 4, 8].map(|bit| invisible & bit != 0);
    Some(edges(&corners(record)?, &hidden))
}

fn corners(record: &Record) -> Option<[Point3; 4]> {
    let [first, second, third] = [10, 11, 12].map(|code| record.point(code));
    let third = third?;
    Some([first?, second?, third, record.point(13).unwrap_or(third)])
}

fn edges(corners: &[Point3; 4], hidden: &[bool; 4]) -> Vec<Shape> {
    let mut shapes: Vec<Shape> = Vec::new();
    for (index, (start, hidden)) in corners.iter().zip(hidden).enumerate() {
        let Some(end) = corners.get((index + 1) % corners.len()) else {
            continue;
        };
        let line = Shape::Line(*start, *end);
        let reversed = Shape::Line(*end, *start);
        if *hidden || start == end || shapes.contains(&line) || shapes.contains(&reversed) {
            continue;
        }
        shapes.push(line);
    }
    shapes
}
