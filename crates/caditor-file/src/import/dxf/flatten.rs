use std::{
    collections::BTreeSet,
    num::NonZeroUsize,
    panic,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    thread,
};

use caditor_geometry::{Point2, Point3, Vector2};
use caditor_kernel::{check_interrupt, current_interrupt, interruptible};
use caditor_sketch::{BSpline, FittedSpline, SplineKind};
use parking_lot::Mutex;

use crate::import::{
    Drawing, DrawingCurve, ImportError, MAX_FIT_WORK,
    dxf::{
        capitalized, counted,
        fit_cache::{Canonical, FitCache},
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
const SHAPES_PER_WORKER: usize = 64;

#[derive(Default)]
struct Tally {
    splines: usize,
    rebuilt: usize,
    fit_point_splines: usize,
    collapsed: usize,
    deviation: f64,
}

impl Tally {
    fn absorb(&mut self, other: Tally) {
        self.splines += other.splines;
        self.rebuilt += other.rebuilt;
        self.fit_point_splines += other.fit_point_splines;
        self.collapsed += other.collapsed;
        self.deviation = self.deviation.max(other.deviation);
    }
}

enum Bound {
    Work,
    Interrupt,
}

struct Fitting {
    bound: Bound,
    cache: Mutex<FitCache>,
    work: AtomicUsize,
    exhausted: AtomicBool,
    cancelled: AtomicBool,
}

impl Fitting {
    fn new(bound: Bound) -> Self {
        Self {
            bound,
            cache: Mutex::new(FitCache::default()),
            work: AtomicUsize::new(0),
            exhausted: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        }
    }

    fn charge(&self, work: usize) -> bool {
        match self.bound {
            Bound::Work => {
                let total = self
                    .work
                    .fetch_add(work, Ordering::Relaxed)
                    .saturating_add(work);
                if total > MAX_FIT_WORK {
                    self.exhausted.store(true, Ordering::Relaxed);
                }
            }
            Bound::Interrupt => {
                if check_interrupt().is_err() {
                    self.cancelled.store(true, Ordering::Relaxed);
                }
            }
        }
        !self.stopped()
    }

    fn stopped(&self) -> bool {
        self.exhausted.load(Ordering::Relaxed) || self.cancelled.load(Ordering::Relaxed)
    }

    fn outcome(&self) -> Result<(), ImportError> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err(ImportError::Cancelled)
        } else if self.exhausted.load(Ordering::Relaxed) {
            Err(ImportError::TooManySplines)
        } else {
            Ok(())
        }
    }
}

struct Flattened {
    curves: Vec<(usize, DrawingCurve)>,
    tally: Tally,
}

fn flatten_from(
    shapes: &[Shape],
    next: &AtomicUsize,
    tolerance: f64,
    fitting: &Fitting,
) -> Flattened {
    let mut flattened = Flattened {
        curves: Vec::new(),
        tally: Tally::default(),
    };
    while !fitting.stopped() {
        let index = next.fetch_add(1, Ordering::Relaxed);
        let Some(shape) = shapes.get(index) else {
            break;
        };
        if let Some(curve) = flatten_shape(shape, tolerance, &mut flattened.tally, fitting) {
            flattened.curves.push((index, curve));
        }
    }
    flattened
}

fn flatten_all(shapes: &[Shape], tolerance: f64, fitting: &Fitting) -> Flattened {
    let next = AtomicUsize::new(0);
    let interrupt = match fitting.bound {
        Bound::Work => None,
        Bound::Interrupt => current_interrupt(),
    };
    let Some(interrupt) = interrupt else {
        return flatten_from(shapes, &next, tolerance, fitting);
    };
    let helpers = thread::available_parallelism()
        .map_or(1, NonZeroUsize::get)
        .min(shapes.len() / SHAPES_PER_WORKER)
        .saturating_sub(1);
    thread::scope(|scope| {
        let spawned: Vec<_> = (0..helpers)
            .filter_map(|_| {
                let interrupt = interrupt.clone();
                let next = &next;
                thread::Builder::new()
                    .name("drawing import".to_owned())
                    .spawn_scoped(scope, move || {
                        interruptible(interrupt, || flatten_from(shapes, next, tolerance, fitting))
                    })
                    .ok()
            })
            .collect();
        let mut flattened = flatten_from(shapes, &next, tolerance, fitting);
        for helper in spawned {
            match helper.join() {
                Ok(helped) => {
                    flattened.curves.extend(helped.curves);
                    flattened.tally.absorb(helped.tally);
                }
                Err(payload) => panic::resume_unwind(payload),
            }
        }
        flattened.curves.sort_unstable_by_key(|(index, _)| *index);
        flattened
    })
}

pub(in crate::import) fn flatten(
    shapes: &[Shape],
    layers: &[usize],
    dashed: &BTreeSet<usize>,
    mut notes: Vec<String>,
) -> Result<Drawing, ImportError> {
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
    let bound = if current_interrupt().is_some() {
        Bound::Interrupt
    } else {
        Bound::Work
    };
    let fitting = Fitting::new(bound);
    let flattened = flatten_all(shapes, tolerance, &fitting);
    fitting.outcome()?;
    let mut curves = Vec::new();
    let mut curve_layers = Vec::new();
    let mut construction = BTreeSet::new();
    for (index, curve) in flattened.curves {
        if dashed.contains(&index) && !matches!(curve, DrawingCurve::Point(_)) {
            construction.insert(curves.len());
        }
        curves.push(curve);
        curve_layers.push(layers.get(index).copied().unwrap_or_default());
    }
    flattened.tally.report(tolerance, &mut notes);
    if !construction.is_empty() {
        notes.push(format!(
            "{} with a dashed or centre linetype {} imported as construction geometry, which \
             forms no regions.",
            capitalized(&counted(construction.len(), "curve", "curves")),
            were(construction.len()),
        ));
    }
    Ok(Drawing {
        curves,
        construction,
        notes,
        unit_scale: 1.0,
        layers: Vec::new(),
        curve_layers,
    })
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

fn flatten_shape(
    shape: &Shape,
    tolerance: f64,
    tally: &mut Tally,
    fitting: &Fitting,
) -> Option<DrawingCurve> {
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
        Shape::Spline(nurbs) => spline(nurbs, tolerance, tally, fitting),
        Shape::Interpolated(fit) => {
            if fit.has_tangents()
                && let Some(nurbs) = fit.cubic()
                && let Some(curve) = sampled(&nurbs, tolerance, tally, fitting)
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
                    if !fitting.charge(BSpline::through_work(points.len(), MAX_FIT_CONTROL_POINTS))
                    {
                        return None;
                    }
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

fn spline(
    nurbs: &Nurbs,
    tolerance: f64,
    tally: &mut Tally,
    fitting: &Fitting,
) -> Option<DrawingCurve> {
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
    let curve = sampled(nurbs, tolerance, tally, fitting);
    if curve.is_some() {
        tally.splines += 1;
    }
    curve
}

fn sampled(
    nurbs: &Nurbs,
    tolerance: f64,
    tally: &mut Tally,
    fitting: &Fitting,
) -> Option<DrawingCurve> {
    let (start, end) = nurbs.domain()?;
    let canonical = Canonical::of(nurbs);
    let found = canonical
        .as_ref()
        .and_then(|canonical| fitting.cache.lock().find(canonical, tolerance));
    if let Some((control_points, deviation)) = found {
        tally.deviation = tally.deviation.max(deviation);
        return Some(DrawingCurve::Spline { control_points });
    }
    let count = (nurbs.spans() * SAMPLES_PER_SPAN).clamp(MIN_SAMPLES, MAX_SAMPLES);
    if !fitting.charge(BSpline::fit_work(count + 1, MAX_FIT_CONTROL_POINTS)) {
        return None;
    }
    let samples: Vec<Point2> = (0..=count)
        .map(|index| {
            let parameter = start + (end - start) * index as f64 / count as f64;
            nurbs
                .point(parameter)
                .map(|point| Point2::new(point.x, point.y))
        })
        .collect::<Option<_>>()?;
    let fit = fitted(&samples, tolerance, tally)?;
    let control_points = fit.spline.control_points().to_vec();
    if let Some(canonical) = canonical {
        fitting
            .cache
            .lock()
            .store(canonical, &control_points, fit.deviation);
    }
    Some(DrawingCurve::Spline { control_points })
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

fn fitted(samples: &[Point2], tolerance: f64, tally: &mut Tally) -> Option<FittedSpline> {
    let Some(fit) = BSpline::fit(samples, tolerance, MAX_FIT_CONTROL_POINTS) else {
        tally.collapsed += 1;
        return None;
    };
    tally.deviation = tally.deviation.max(fit.deviation);
    Some(fit)
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
