use std::collections::BTreeSet;

use caditor_geometry::{Point2, Point3, Vector2};
use caditor_sketch::{BSpline, SplineKind};

use crate::import::{
    Drawing, DrawingCurve,
    dxf::{
        capitalized, counted,
        geometry::{Nurbs, Shape, flat, is_full_turn, planar_circle},
        list, were,
    },
};

const RELATIVE_FIT_TOLERANCE: f64 = 1e-6;
const MIN_FIT_TOLERANCE: f64 = 1e-6;
const MAX_FIT_CONTROL_POINTS: usize = 500;
const SAMPLES_PER_SPAN: usize = 48;
const MIN_SAMPLES: usize = 64;
const MAX_SAMPLES: usize = 16_384;
const RELATIVE_FLATNESS: f64 = 1e-9;
const KNOT_TOLERANCE: f64 = 1e-9;
const MAX_FIT_POINTS: usize = MAX_FIT_CONTROL_POINTS;
const MIN_CLOSED_FIT_POINTS: usize = 3;

#[derive(Default)]
struct Tally {
    splines: usize,
    rebuilt: usize,
    fit_point_splines: usize,
    collapsed: usize,
    deviation: f64,
}

pub(in crate::import) fn flatten(
    shapes: &[Shape],
    layers: &[usize],
    dashed: &BTreeSet<usize>,
    mut notes: Vec<String>,
) -> Drawing {
    let points: Vec<Point3> = shapes.iter().flat_map(Shape::defining_points).collect();
    let extent = extent(&points);
    if let (Some(low), Some(high)) = (
        points.iter().map(|point| point.z).reduce(f64::min),
        points.iter().map(|point| point.z).reduce(f64::max),
    ) && high - low > RELATIVE_FLATNESS * extent.max(1.0)
    {
        notes.push("The drawing is not flat, so it was projected onto its XY plane.".to_owned());
    }
    let tolerance = (extent * RELATIVE_FIT_TOLERANCE).max(MIN_FIT_TOLERANCE);
    let mut tally = Tally::default();
    let mut curves = Vec::new();
    let mut curve_layers = Vec::new();
    let mut construction = BTreeSet::new();
    for (index, shape) in shapes.iter().enumerate() {
        if let Some(curve) = flatten_shape(shape, tolerance, &mut tally) {
            if dashed.contains(&index) && !matches!(curve, DrawingCurve::Point(_)) {
                construction.insert(curves.len());
            }
            curves.push(curve);
            curve_layers.push(layers.get(index).copied().unwrap_or_default());
        }
    }
    tally.report(tolerance, &mut notes);
    if !construction.is_empty() {
        notes.push(format!(
            "{} with a dashed or centre linetype {} imported as construction geometry, which \
             forms no regions.",
            capitalized(&counted(construction.len(), "curve", "curves")),
            were(construction.len()),
        ));
    }
    Drawing {
        curves,
        construction,
        notes,
        unit_scale: 1.0,
        layers: Vec::new(),
        curve_layers,
    }
}

fn extent(points: &[Point3]) -> f64 {
    let flat_points = points.iter().map(|point| Point2::new(point.x, point.y));
    let bounds = flat_points.fold(None, |bounds: Option<(Point2, Point2)>, point| {
        Some(match bounds {
            Some((low, high)) => (low.min(point), high.max(point)),
            None => (point, point),
        })
    });
    bounds.map_or(0.0, |(low, high)| low.distance(high))
}

fn flatten_shape(shape: &Shape, tolerance: f64, tally: &mut Tally) -> Option<DrawingCurve> {
    let point2 = |point: Point3| Point2::new(point.x, point.y);
    match shape {
        Shape::Point(point) => Some(DrawingCurve::Point(point2(*point))),
        Shape::Line(start, end) => {
            let (start, end) = (point2(*start), point2(*end));
            if start == end {
                tally.collapsed += 1;
                return None;
            }
            Some(DrawingCurve::Line { start, end })
        }
        Shape::Conic {
            center,
            major,
            minor,
            start,
            sweep,
        } => {
            if let Some(circle) = planar_circle(*center, *major, *minor) {
                if is_full_turn(*sweep) {
                    return Some(DrawingCurve::Circle {
                        center: circle.center,
                        radius: circle.radius,
                    });
                }
                let at = |angle: f64| circle.center + Vector2::from_angle(angle) * circle.radius;
                let (from, to) = if circle.counter_clockwise {
                    let from = circle.start_angle + start;
                    (from, from + sweep)
                } else {
                    let to = circle.start_angle - start;
                    (to - sweep, to)
                };
                return Some(DrawingCurve::Arc {
                    center: circle.center,
                    start: at(from),
                    end: at(to),
                });
            }
            if flat(*major).perp_dot(flat(*minor)).abs()
                <= RELATIVE_FLATNESS
                    * flat(*major)
                        .length_squared()
                        .max(flat(*minor).length_squared())
            {
                tally.collapsed += 1;
                return None;
            }
            Some(planar_ellipse(
                point2(*center),
                (flat(*major), flat(*minor)),
                *start,
                *sweep,
            ))
        }
        Shape::Spline(nurbs) => spline(nurbs, tolerance, tally),
        Shape::Interpolated(fit) => {
            if fit.has_tangents()
                && let Some(nurbs) = fit.cubic()
                && let Some(curve) = sampled(&nurbs, tolerance, tally)
            {
                tally.rebuilt += 1;
                return Some(curve);
            }
            let mut points: Vec<Point2> = fit.points.iter().map(|point| point2(*point)).collect();
            points.dedup();
            if fit.closed && points.len() > MIN_CLOSED_FIT_POINTS && points.first() == points.last()
            {
                points.pop();
            }
            let closed = fit.closed && points.len() >= MIN_CLOSED_FIT_POINTS;
            match points.as_slice() {
                [] | [_] => {
                    tally.collapsed += 1;
                    None
                }
                [start, end] => Some(DrawingCurve::Line {
                    start: *start,
                    end: *end,
                }),
                _ if points.len() <= MAX_FIT_POINTS
                    && SplineKind::fit(closed).curve(&points).is_some() =>
                {
                    tally.fit_point_splines += 1;
                    Some(DrawingCurve::FitSpline {
                        fit_points: points,
                        closed,
                    })
                }
                _ => {
                    let Some(fit) = BSpline::through(&points, tolerance, MAX_FIT_CONTROL_POINTS)
                    else {
                        tally.collapsed += 1;
                        return None;
                    };
                    tally.rebuilt += 1;
                    tally.deviation = tally.deviation.max(fit.deviation);
                    Some(DrawingCurve::Spline {
                        control_points: fit.spline.control_points().to_vec(),
                    })
                }
            }
        }
    }
}

fn spline(nurbs: &Nurbs, tolerance: f64, tally: &mut Tally) -> Option<DrawingCurve> {
    let control_points: Vec<Point2> = nurbs
        .points
        .iter()
        .map(|point| Point2::new(point.x, point.y))
        .collect();
    if nurbs.weights.is_none()
        && let Some(native) = BSpline::clamped(control_points.clone())
        && native.degree() == nurbs.degree
        && same_knots(native.knots(), &nurbs.knots)
    {
        return Some(match control_points.as_slice() {
            [start, end] => DrawingCurve::Line {
                start: *start,
                end: *end,
            },
            _ => DrawingCurve::Spline { control_points },
        });
    }
    let curve = sampled(nurbs, tolerance, tally);
    if curve.is_some() {
        tally.splines += 1;
    }
    curve
}

fn sampled(nurbs: &Nurbs, tolerance: f64, tally: &mut Tally) -> Option<DrawingCurve> {
    let (start, end) = nurbs.domain()?;
    let count = (nurbs.spans() * SAMPLES_PER_SPAN).clamp(MIN_SAMPLES, MAX_SAMPLES);
    let samples: Vec<Point2> = (0..=count)
        .map(|index| {
            let parameter = start + (end - start) * index as f64 / count as f64;
            nurbs
                .point(parameter)
                .map(|point| Point2::new(point.x, point.y))
        })
        .collect::<Option<_>>()?;
    fitted(&samples, tolerance, tally)
}

fn same_knots(native: &[f64], given: &[f64]) -> bool {
    let (Some(first), Some(last)) = (given.first(), given.last()) else {
        return false;
    };
    let width = last - first;
    native.len() == given.len()
        && width > 0.0
        && native
            .iter()
            .zip(given)
            .all(|(native, given)| (native - (given - first) / width).abs() <= KNOT_TOLERANCE)
}

fn planar_ellipse(
    center: Point2,
    (first, second): (Vector2, Vector2),
    start: f64,
    sweep: f64,
) -> DrawingCurve {
    let turn =
        0.5 * (2.0 * first.dot(second)).atan2(first.length_squared() - second.length_squared());
    let (sin, cos) = turn.sin_cos();
    let along = first * cos + second * sin;
    let across = second * cos - first * sin;
    let (major, minor_radius) = if along.length() >= across.length() {
        (along, across.length())
    } else {
        (across, along.length())
    };
    let ends = (!is_full_turn(sweep)).then(|| {
        let at = |angle: f64| center + first * angle.cos() + second * angle.sin();
        let (from, to) = (at(start), at(start + sweep));
        if first.perp_dot(second) > 0.0 {
            (from, to)
        } else {
            (to, from)
        }
    });
    DrawingCurve::Ellipse {
        center,
        major,
        minor_radius,
        ends,
    }
}

fn fitted(samples: &[Point2], tolerance: f64, tally: &mut Tally) -> Option<DrawingCurve> {
    let Some(fit) = BSpline::fit(samples, tolerance, MAX_FIT_CONTROL_POINTS) else {
        tally.collapsed += 1;
        return None;
    };
    tally.deviation = tally.deviation.max(fit.deviation);
    Some(DrawingCurve::Spline {
        control_points: fit.spline.control_points().to_vec(),
    })
}

impl Tally {
    fn report(&self, tolerance: f64, notes: &mut Vec<String>) {
        let mut converted = Vec::new();
        if self.splines > 0 {
            converted.push(counted(self.splines, "spline", "splines"));
        }
        if !converted.is_empty() {
            let deviation = self.deviation.max(tolerance);
            notes.push(format!(
                "{} {} converted to sketch splines that stay within {} of the original.",
                list(&converted),
                were(self.splines),
                millimetres(deviation)
            ));
        }
        if self.rebuilt > 0 {
            notes.push(format!(
                "{} given only by points on the curve {} rebuilt through the same points.",
                counted(self.rebuilt, "spline", "splines"),
                were(self.rebuilt)
            ));
        }
        if self.fit_point_splines > 0 {
            notes.push(format!(
                "{} given only by points on the curve {} drawn through the same points as \
                 fit-point splines, which keep those points to edit.",
                capitalized(&counted(self.fit_point_splines, "spline", "splines")),
                were(self.fit_point_splines)
            ));
        }
        if self.collapsed > 0 {
            notes.push(format!(
                "{} that would shrink to a point or a line in the sketch plane {} left out.",
                counted(self.collapsed, "curve", "curves"),
                were(self.collapsed)
            ));
        }
    }
}

fn millimetres(value: f64) -> String {
    let digits = (1.0 - value.log10().floor()).clamp(0.0, 9.0) as usize;
    format!("{value:.digits$} mm")
}
