use std::{
    collections::BTreeSet,
    f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2, TAU},
};

use caditor_document::face_plane;
use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};
use caditor_kernel::{Curve, Edge, FaceId, Interval, LINEAR_RESOLUTION, SamplingTolerance, Solid};

use super::{
    ExportError, FaceExported,
    figure::{Ellipse, Figure, Layer, Shape, Spline},
};

const POLYLINE_CHORD: f64 = 1e-3;
const POLYLINE_CHORD_FRACTION: f64 = 1e-6;
const POLYLINE_ANGLE: f64 = 2.0 * std::f64::consts::PI / 180.0;
const CONIC_ALIGNMENT: f64 = 1e-6;

pub(super) fn face_figure(
    solid: &Solid,
    face: FaceId,
) -> Result<(Figure, FaceExported), ExportError> {
    let definition = solid.face(face).ok_or(ExportError::FaceMissing)?;
    let plane = face_plane(solid, face).ok_or(ExportError::FaceNotFlat)?;
    let frame = drawing_frame(&plane).ok_or(ExportError::FaceNotFlat)?;
    let tolerance = polyline_tolerance(solid);
    let mut figure = Figure::default();
    let mut exported = FaceExported {
        loops: 0,
        curves: 0,
        approximated: 0,
    };
    let mut written = BTreeSet::new();
    for (index, loop_id) in definition.loops().iter().enumerate() {
        let layer = if index == 0 {
            Layer::Outline
        } else {
            Layer::Holes
        };
        let face_loop = solid.face_loop(*loop_id).ok_or(ExportError::Encoding)?;
        exported.loops += 1;
        for coedge in face_loop.coedges() {
            let coedge = solid.coedge(*coedge).ok_or(ExportError::Encoding)?;
            if !written.insert(coedge.edge()) {
                continue;
            }
            let edge = solid.edge(coedge.edge()).ok_or(ExportError::Encoding)?;
            let (shape, exact) =
                edge_shape(solid, edge, &frame, &tolerance).ok_or(ExportError::Encoding)?;
            exported.curves += 1;
            if !exact {
                exported.approximated += 1;
            }
            figure.push(layer, shape);
        }
    }
    if exported.curves == 0 {
        return Err(ExportError::NoCurves);
    }
    Ok((figure, exported))
}

fn drawing_frame(face: &Plane) -> Option<Plane> {
    let normal = face.normal();
    let up = if normal.dot(Vector3::Z).abs() < FRAC_1_SQRT_2 {
        Vector3::Z
    } else {
        Vector3::Y
    };
    let right = up.reject_from_normalized(normal).cross(normal);
    let origin = Point3::ZERO - normal * face.signed_distance(Point3::ZERO);
    Plane::with_x_axis(origin, normal, right)
}

fn polyline_tolerance(solid: &Solid) -> SamplingTolerance {
    let extent = solid
        .bounding_box()
        .map(|bounds| bounds.diagonal())
        .filter(|diagonal| diagonal.is_finite())
        .unwrap_or(0.0);
    let chord = POLYLINE_CHORD.max(extent * POLYLINE_CHORD_FRACTION);
    SamplingTolerance::new(chord, POLYLINE_ANGLE)
        .unwrap_or_else(|| SamplingTolerance::for_extent(extent))
}

fn edge_shape(
    solid: &Solid,
    edge: &Edge,
    frame: &Plane,
    tolerance: &SamplingTolerance,
) -> Option<(Shape, bool)> {
    let start = frame.to_local(solid.vertex(edge.start())?.point());
    let end = frame.to_local(solid.vertex(edge.end())?.point());
    let interval = edge.interval();
    let full = edge.is_closed() && interval.length() >= TAU - LINEAR_RESOLUTION;
    let exact = match edge.curve() {
        Curve::Line(_) => Some(Shape::Line(start, end)),
        Curve::Circle(circle) => {
            let radius = circle.radius();
            conic(frame, circle.frame(), radius, radius, interval).map(|conic| {
                let turn = conic.major.y.atan2(conic.major.x);
                if full {
                    Shape::Circle {
                        center: conic.center,
                        radius,
                    }
                } else {
                    Shape::Arc {
                        center: conic.center,
                        radius,
                        start: turn + conic.start,
                        end: turn + conic.end,
                    }
                }
            })
        }
        Curve::Ellipse(ellipse) => ellipse_arc(
            frame,
            ellipse.frame(),
            ellipse.major_radius(),
            ellipse.minor_radius(),
            interval,
            full,
        )
        .map(Shape::Ellipse),
        Curve::BSpline(spline) => spline
            .restricted(interval)
            .and_then(|restricted| restricted.map_points(|point| frame.to_local(point)).ok())
            .map(|planar| {
                Shape::Spline(Spline {
                    degree: planar.degree(),
                    knots: planar.knots().to_vec(),
                    control_points: planar.control_points().to_vec(),
                    weights: planar.weights().map(<[f64]>::to_vec),
                    polyline: sampled(edge, frame, tolerance, start, end),
                })
            }),
        _ => None,
    };
    match exact {
        Some(shape) => Some((shape, true)),
        None => Some((
            Shape::Polyline(sampled(edge, frame, tolerance, start, end)),
            false,
        )),
    }
}

fn sampled(
    edge: &Edge,
    frame: &Plane,
    tolerance: &SamplingTolerance,
    start: Point2,
    end: Point2,
) -> Vec<Point2> {
    let mut points: Vec<Point2> = edge
        .curve()
        .sample(edge.interval(), tolerance)
        .into_iter()
        .map(|sample| frame.to_local(sample.point))
        .collect();
    if let Some(first) = points.first_mut() {
        *first = start;
    }
    if let Some(last) = points.last_mut() {
        *last = end;
    }
    points
}

pub(super) fn ellipse_arc(
    frame: &Plane,
    ellipse: &Plane,
    along_x: f64,
    along_y: f64,
    interval: Interval,
    full: bool,
) -> Option<Ellipse> {
    let conic = conic(frame, ellipse, along_x, along_y, interval)?;
    let (start, end) = if full {
        (0.0, TAU)
    } else {
        (conic.start, conic.end)
    };
    Some(Ellipse {
        center: conic.center,
        major: conic.major,
        ratio: conic.ratio,
        start,
        end,
    })
}

struct Conic2 {
    center: Point2,
    major: Vector2,
    ratio: f64,
    start: f64,
    end: f64,
}

fn conic(
    frame: &Plane,
    conic: &Plane,
    along_x: f64,
    along_y: f64,
    interval: Interval,
) -> Option<Conic2> {
    let alignment = conic.normal().dot(frame.normal());
    if (alignment.abs() - 1.0).abs() > CONIC_ALIGNMENT || along_x <= 0.0 || along_y <= 0.0 {
        return None;
    }
    let x_axis = Vector2::new(
        conic.x_axis().dot(frame.x_axis()),
        conic.x_axis().dot(frame.y_axis()),
    )
    .try_normalize()?;
    let (start, end) = if alignment > 0.0 {
        (interval.start(), interval.end())
    } else {
        (-interval.end(), -interval.start())
    };
    let center = frame.to_local(conic.origin());
    Some(if along_x >= along_y {
        Conic2 {
            center,
            major: x_axis * along_x,
            ratio: along_y / along_x,
            start,
            end,
        }
    } else {
        Conic2 {
            center,
            major: x_axis.perp() * along_y,
            ratio: along_x / along_y,
            start: start - FRAC_PI_2,
            end: end - FRAC_PI_2,
        }
    })
}
