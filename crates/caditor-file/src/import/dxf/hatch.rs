use caditor_geometry::{Point2, Point3, Vector3};

use crate::import::dxf::{
    Fields, Record,
    geometry::{Affine, FitPoints, Nurbs, Shape, conic_arc},
    pairs::Pair,
    polyline_segments,
};

const PATH_COUNT: i32 = 91;
const PATH_FLAGS: i32 = 92;
const EDGE_KIND: i32 = 72;
const SOURCE_COUNT: i32 = 97;
const SOURCE_HANDLE: i32 = 330;
const PATH_CODES: [i32; 22] = [
    10, 20, 11, 21, 12, 22, 13, 23, 40, 42, 50, 51, 72, 73, 74, 92, 93, 94, 95, 96, 97, 330,
];
const POLYLINE_PATH: i64 = 2;
const TEXT_BOX_PATH: i64 = 8;
const LINE_EDGE: i64 = 1;
const ARC_EDGE: i64 = 2;
const ELLIPSE_EDGE: i64 = 3;
const SPLINE_EDGE: i64 = 4;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Boundary {
    pub shapes: Vec<Shape>,
    pub sources: Vec<String>,
}

pub(super) fn boundaries(record: &Record) -> Option<Vec<Boundary>> {
    let system = Affine::object_system(record.normal())?;
    let elevation = record.point(10).map_or(0.0, |point| point.z);
    let start = record
        .pairs
        .iter()
        .position(|pair| pair.code == PATH_COUNT)?;
    let rest = record.pairs.get(start + 1..)?;
    let length = rest
        .iter()
        .position(|pair| !PATH_CODES.contains(&pair.code))
        .unwrap_or(rest.len());
    let mut boundaries = Vec::new();
    for path in groups(rest.get(..length)?, PATH_FLAGS) {
        let (flags, body) = path.split_first()?;
        let flags = flags.integer()?;
        if flags & TEXT_BOX_PATH != 0 {
            continue;
        }
        let (body, sources) = split_sources(body);
        let shapes = if flags & POLYLINE_PATH != 0 {
            polyline_path(body, elevation)?
        } else {
            edge_path(body, elevation)?
        };
        boundaries.push(Boundary {
            shapes: shapes
                .iter()
                .map(|shape| shape.transformed(&system))
                .collect(),
            sources,
        });
    }
    Some(boundaries)
}

fn groups(pairs: &[Pair], leader: i32) -> Vec<&[Pair]> {
    let starts: Vec<usize> = pairs
        .iter()
        .enumerate()
        .filter(|(_, pair)| pair.code == leader)
        .map(|(index, _)| index)
        .collect();
    starts
        .iter()
        .enumerate()
        .filter_map(|(number, from)| {
            let to = starts.get(number + 1).copied().unwrap_or(pairs.len());
            pairs.get(*from..to)
        })
        .collect()
}

fn split_sources(path: &[Pair]) -> (&[Pair], Vec<String>) {
    let handles = path
        .iter()
        .rev()
        .take_while(|pair| pair.code == SOURCE_HANDLE)
        .count();
    let (before, after) = path.split_at(path.len().saturating_sub(handles));
    match before.split_last() {
        Some((count, rest)) if count.code == SOURCE_COUNT => (
            rest,
            after
                .iter()
                .map(|pair| pair.text().trim().to_ascii_uppercase())
                .collect(),
        ),
        _ => (path, Vec::new()),
    }
}

fn polyline_path(pairs: &[Pair], elevation: f64) -> Option<Vec<Shape>> {
    let mut vertices: Vec<(Point2, f64)> = Vec::new();
    for pair in pairs {
        match pair.code {
            10 => vertices.push((Point2::new(pair.real()?, 0.0), 0.0)),
            20 => vertices.last_mut()?.0.y = pair.real()?,
            42 => vertices.last_mut()?.1 = pair.real()?,
            _ => {}
        }
    }
    Some(polyline_segments(&vertices, true, elevation))
}

fn edge_path(pairs: &[Pair], elevation: f64) -> Option<Vec<Shape>> {
    groups(pairs, EDGE_KIND)
        .into_iter()
        .map(|edge| {
            let (kind, fields) = edge.split_first()?;
            match kind.integer()? {
                LINE_EDGE => Some(Shape::Line(
                    lifted(fields, 10, elevation)?,
                    lifted(fields, 11, elevation)?,
                )),
                ARC_EDGE => arc_edge(fields, elevation),
                ELLIPSE_EDGE => ellipse_edge(fields, elevation),
                SPLINE_EDGE => spline_edge(fields, elevation),
                _ => None,
            }
        })
        .collect()
}

fn lifted(fields: &[Pair], code: i32, elevation: f64) -> Option<Point3> {
    Some(Point3::new(
        fields.real(code)?,
        fields.real(code + 10)?,
        elevation,
    ))
}

fn counter_clockwise(fields: &[Pair]) -> bool {
    fields
        .field(73)
        .and_then(Pair::integer)
        .is_none_or(|flag| flag != 0)
}

fn arc_edge(fields: &[Pair], elevation: f64) -> Option<Shape> {
    let center = lifted(fields, 10, elevation)?;
    let radius = fields.real(40).filter(|radius| *radius > 0.0)?;
    let turn = if counter_clockwise(fields) { 1.0 } else { -1.0 };
    Some(conic_arc(
        center,
        Vector3::X * radius,
        Vector3::Y * radius * turn,
        fields.real_or(50, 0.0).to_radians(),
        fields.real_or(51, 360.0).to_radians(),
    ))
}

fn ellipse_edge(fields: &[Pair], elevation: f64) -> Option<Shape> {
    let center = lifted(fields, 10, elevation)?;
    let major = Vector3::new(fields.real(11)?, fields.real(21)?, 0.0);
    let ratio = fields.real(40).filter(|ratio| *ratio > 0.0)?;
    let turn = if counter_clockwise(fields) { 1.0 } else { -1.0 };
    let parameter = |angle: f64| {
        let angle = angle.to_radians();
        (angle.sin() / ratio).atan2(angle.cos())
    };
    (major.length() > 0.0).then(|| {
        conic_arc(
            center,
            major,
            Vector3::Z.cross(major) * ratio * turn,
            parameter(fields.real_or(50, 0.0)),
            parameter(fields.real_or(51, 360.0)),
        )
    })
}

fn spline_edge(fields: &[Pair], elevation: f64) -> Option<Shape> {
    let at_elevation = |point: Point3| Point3::new(point.x, point.y, elevation);
    let control_points: Vec<Point3> = fields.points(10).into_iter().map(at_elevation).collect();
    let weights = fields.reals(42);
    let weights = (!weights.is_empty()).then_some(weights);
    let controlled = usize::try_from(fields.flags(94))
        .ok()
        .filter(|_| !control_points.is_empty())
        .and_then(|degree| Nurbs::new(degree, fields.reals(40), control_points, weights));
    if let Some(nurbs) = controlled {
        return Some(Shape::Spline(nurbs));
    }
    let fit_points: Vec<Point3> = fields.points(11).into_iter().map(at_elevation).collect();
    let tangent = |code: i32| {
        Some(Vector3::new(
            fields.real(code)?,
            fields.real(code + 10)?,
            0.0,
        ))
    };
    (fit_points.len() >= 2)
        .then(|| Shape::Interpolated(FitPoints::new(fit_points, tangent(12), tangent(13))))
}
