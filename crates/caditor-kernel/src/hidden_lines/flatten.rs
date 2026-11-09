use std::f64::consts::PI;

use caditor_geometry::{Maths, Plane, Point2, Point3, Vector2, Vector3};

use crate::{
    bspline::BSpline,
    curve::Curve,
    curve2::{Circle2, Curve2, Ellipse2, Line2},
    error::GeometryError,
    interval::Interval,
    tolerance::SamplingTolerance,
};

const VANISHED: f64 = 1e-9;
const EDGE_ON: f64 = 1e-9;
const ROUND: f64 = 1e-12;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Mapping {
    Same,
    Offset(f64),
    AlongLine,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Flat {
    pub curve: Curve2,
    pub mapping: Mapping,
    pub range: Interval,
    pub sampled: bool,
}

pub(super) fn flat_vector(view: &Plane, vector: Vector3) -> Vector2 {
    Vector2::new(vector.dot(view.x_axis()), vector.dot(view.y_axis()))
}

struct Conic {
    center: Point2,
    along: Vector2,
    across: Vector2,
}

pub(super) fn flatten(
    curve: &Curve,
    range: Interval,
    view: &Plane,
    tolerance: &SamplingTolerance,
) -> Result<Vec<Flat>, GeometryError> {
    match curve {
        Curve::Line(_) => Ok(straight(curve, range, view)?.into_iter().collect()),
        Curve::Circle(circle) => {
            let frame = circle.frame();
            conic(
                curve,
                range,
                view,
                &Conic {
                    center: view.to_local(circle.center()),
                    along: flat_vector(view, frame.x_axis()) * circle.radius(),
                    across: flat_vector(view, frame.y_axis()) * circle.radius(),
                },
            )
        }
        Curve::Ellipse(ellipse) => {
            let frame = ellipse.frame();
            conic(
                curve,
                range,
                view,
                &Conic {
                    center: view.to_local(ellipse.center()),
                    along: flat_vector(view, frame.x_axis()) * ellipse.major_radius(),
                    across: flat_vector(view, frame.y_axis()) * ellipse.minor_radius(),
                },
            )
        }
        Curve::BSpline(spline) => {
            if !spread(curve, range, view) {
                return Ok(Vec::new());
            }
            Ok(vec![Flat {
                curve: Curve2::BSpline(spline.map_points(|point| view.to_local(point))?),
                mapping: Mapping::Same,
                range,
                sampled: false,
            }])
        }
        Curve::Intersection(_) => {
            let points: Vec<(f64, Point3)> = curve
                .sample(range, tolerance)
                .iter()
                .map(|sample| (sample.parameter, sample.point))
                .collect();
            Ok(polyline(&points, view)?.into_iter().collect())
        }
    }
}

fn spread(curve: &Curve, range: Interval, view: &Plane) -> bool {
    let corners = curve
        .bounding_box(range)
        .corners()
        .map(|corner| view.to_local(corner));
    let first = corners.first().copied().unwrap_or(Point2::ZERO);
    corners
        .iter()
        .any(|corner| corner.distance(first) > VANISHED)
}

fn polyline(points: &[(f64, Point3)], view: &Plane) -> Result<Option<Flat>, GeometryError> {
    let mut parameters: Vec<f64> = Vec::with_capacity(points.len());
    let mut flat: Vec<Point2> = Vec::with_capacity(points.len());
    for (parameter, point) in points {
        if parameters.last().is_some_and(|last| parameter <= last) {
            continue;
        }
        parameters.push(*parameter);
        flat.push(view.to_local(*point));
    }
    let (Some(start), Some(end), Some(first)) =
        (parameters.first(), parameters.last(), flat.first())
    else {
        return Ok(None);
    };
    let Some(range) = Interval::new(*start, *end) else {
        return Ok(None);
    };
    if !flat.iter().any(|point| point.distance(*first) > VANISHED) {
        return Ok(None);
    }
    let knots: Vec<f64> = std::iter::once(*start)
        .chain(parameters.iter().copied())
        .chain(std::iter::once(*end))
        .collect();
    Ok(Some(Flat {
        curve: Curve2::BSpline(BSpline::new(1, knots, flat)?),
        mapping: Mapping::Same,
        range,
        sampled: true,
    }))
}

fn straight(curve: &Curve, range: Interval, view: &Plane) -> Result<Option<Flat>, GeometryError> {
    let start = view.to_local(curve.point(range.start()));
    let end = view.to_local(curve.point(range.end()));
    if start.distance(end) <= VANISHED {
        return Ok(None);
    }
    Ok(Some(Flat {
        curve: Curve2::Line(Line2::through(start, end)?),
        mapping: Mapping::AlongLine,
        range,
        sampled: false,
    }))
}

fn conic(
    curve: &Curve,
    range: Interval,
    view: &Plane,
    conic: &Conic,
) -> Result<Vec<Flat>, GeometryError> {
    let (along, across) = (conic.along, conic.across);
    let difference = along.length_squared() - across.length_squared();
    let twice_shared = 2.0 * along.dot(across);
    let scale = along.length_squared().max(across.length_squared());
    if scale <= VANISHED * VANISHED {
        return Ok(Vec::new());
    }
    if difference.abs() <= ROUND * scale && twice_shared.abs() <= ROUND * scale {
        let counter_clockwise = along.perp_dot(across) > 0.0;
        let circle = Circle2::with_axes(conic.center, along.length(), along, counter_clockwise)?;
        return Ok(vec![Flat {
            curve: Curve2::Circle(circle),
            mapping: Mapping::Same,
            range,
            sampled: false,
        }]);
    }
    let turn = 0.5 * twice_shared.atan2_libm(difference);
    let (sin, cos) = turn.sin_cos_libm();
    let major = along * cos + across * sin;
    let minor = across * cos - along * sin;
    if minor.length() <= EDGE_ON {
        return folded(curve, range, view, turn);
    }
    let counter_clockwise = major.perp_dot(minor) > 0.0;
    let ellipse = Ellipse2::with_axes(
        conic.center,
        major.length(),
        minor.length(),
        major,
        counter_clockwise,
    )?;
    Ok(vec![Flat {
        curve: Curve2::Ellipse(ellipse),
        mapping: Mapping::Offset(turn),
        range,
        sampled: false,
    }])
}

fn folded(
    curve: &Curve,
    range: Interval,
    view: &Plane,
    turn: f64,
) -> Result<Vec<Flat>, GeometryError> {
    let mut ends = vec![range.start()];
    let mut fold = ((range.start() - turn) / PI).floor() + 1.0;
    while turn + fold * PI < range.end() {
        ends.push(turn + fold * PI);
        fold += 1.0;
    }
    ends.push(range.end());
    let mut flats = Vec::with_capacity(ends.len());
    for pair in ends.windows(2) {
        let [start, end] = pair else {
            continue;
        };
        let Some(piece) = Interval::new(*start, *end) else {
            continue;
        };
        flats.extend(straight(curve, piece, view)?);
    }
    Ok(flats)
}

pub(super) fn flat_parameter(flat: &Flat, curve: &Curve, parameter: f64, view: &Plane) -> f64 {
    match (&flat.mapping, &flat.curve) {
        (Mapping::Offset(offset), _) => parameter - offset,
        (Mapping::AlongLine, Curve2::Line(line)) => {
            line.parameter_of(view.to_local(curve.point(parameter)))
        }
        (Mapping::Same | Mapping::AlongLine, _) => parameter,
    }
}

pub(super) fn polyline_curve(
    points: &[Point3],
) -> Result<Option<(Curve, Interval)>, GeometryError> {
    let mut kept: Vec<Point3> = Vec::with_capacity(points.len());
    let mut lengths: Vec<f64> = Vec::with_capacity(points.len());
    let mut travelled = 0.0;
    for point in points {
        match kept.last() {
            Some(last) if last.distance(*point) <= VANISHED => {}
            Some(last) => {
                travelled += last.distance(*point);
                kept.push(*point);
                lengths.push(travelled);
            }
            None => {
                kept.push(*point);
                lengths.push(0.0);
            }
        }
    }
    let Some(range) = Interval::new(0.0, travelled).filter(|_| kept.len() >= 2) else {
        return Ok(None);
    };
    let knots: Vec<f64> = std::iter::once(0.0)
        .chain(lengths.iter().copied())
        .chain(std::iter::once(travelled))
        .collect();
    let spline = BSpline::new(1, knots, kept)?;
    Ok(Some((Curve::BSpline(spline), range)))
}
