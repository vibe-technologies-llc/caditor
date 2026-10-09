use std::f64::consts::TAU;

use caditor_geometry::{Point2, Point3, Vector2, Vector3};
use caditor_kernel::MAX_SPLINE_DEGREE;

const ARBITRARY_AXIS_LIMIT: f64 = 1.0 / 64.0;
const FULL_TURN_TOLERANCE: f64 = 1e-9;
const CIRCULAR_TOLERANCE: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::import) struct Affine {
    x: Vector3,
    y: Vector3,
    z: Vector3,
    origin: Point3,
}

impl Affine {
    pub const IDENTITY: Self = Self {
        x: Vector3::X,
        y: Vector3::Y,
        z: Vector3::Z,
        origin: Point3::ZERO,
    };

    pub fn object_system(normal: Vector3) -> Option<Self> {
        let normal = normal.try_normalize()?;
        let helper =
            if normal.x.abs() < ARBITRARY_AXIS_LIMIT && normal.y.abs() < ARBITRARY_AXIS_LIMIT {
                Vector3::Y
            } else {
                Vector3::Z
            };
        let x = helper.cross(normal).try_normalize()?;
        let y = normal.cross(x).try_normalize()?;
        Some(Self {
            x,
            y,
            z: normal,
            origin: Point3::ZERO,
        })
    }

    pub fn translation(offset: Vector3) -> Self {
        Self {
            origin: offset,
            ..Self::IDENTITY
        }
    }

    pub fn scale(factors: Vector3) -> Self {
        Self {
            x: Vector3::X * factors.x,
            y: Vector3::Y * factors.y,
            z: Vector3::Z * factors.z,
            origin: Point3::ZERO,
        }
    }

    pub fn planar(x: Vector2, y: Vector2, origin: Point2) -> Self {
        Self {
            x: x.extend(0.0),
            y: y.extend(0.0),
            z: Vector3::Z,
            origin: origin.extend(0.0),
        }
    }

    pub fn rotation_z(angle: f64) -> Self {
        let (sin, cos) = angle.sin_cos();
        Self {
            x: Vector3::new(cos, sin, 0.0),
            y: Vector3::new(-sin, cos, 0.0),
            z: Vector3::Z,
            origin: Point3::ZERO,
        }
    }

    pub fn point(&self, point: Point3) -> Point3 {
        self.origin + self.vector(point)
    }

    pub fn vector(&self, vector: Vector3) -> Vector3 {
        self.x * vector.x + self.y * vector.y + self.z * vector.z
    }

    pub fn then(&self, outer: &Self) -> Self {
        Self {
            x: outer.vector(self.x),
            y: outer.vector(self.y),
            z: outer.vector(self.z),
            origin: outer.point(self.origin),
        }
    }

    pub fn is_finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite() && self.origin.is_finite()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::import) struct Nurbs {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub points: Vec<Point3>,
    pub weights: Option<Vec<f64>>,
}

impl Nurbs {
    pub fn new(
        degree: usize,
        knots: Vec<f64>,
        points: Vec<Point3>,
        weights: Option<Vec<f64>>,
    ) -> Option<Self> {
        let valid = (1..=MAX_SPLINE_DEGREE).contains(&degree)
            && weights
                .as_ref()
                .is_none_or(|weights| weights.len() == points.len())
            && points.len() > degree
            && knots.len() == points.len() + degree + 1
            && knots.iter().all(|knot| knot.is_finite())
            && knots
                .windows(2)
                .all(|pair| matches!(pair, [a, b] if a <= b))
            && points.iter().all(|point| point.is_finite())
            && weights
                .as_ref()
                .is_none_or(|weights| weights.iter().all(|w| w.is_finite() && *w > 0.0));
        let nurbs = Self {
            degree,
            knots,
            points,
            weights: weights.filter(|weights| weights.iter().any(|weight| *weight != 1.0)),
        };
        (valid && nurbs.domain().is_some()).then_some(nurbs)
    }

    pub fn domain(&self) -> Option<(f64, f64)> {
        let start = *self.knots.get(self.degree)?;
        let end = *self.knots.get(self.points.len())?;
        (end > start).then_some((start, end))
    }

    pub fn spans(&self) -> usize {
        let Some((start, end)) = self.domain() else {
            return 0;
        };
        let mut breaks: Vec<f64> = self
            .knots
            .iter()
            .copied()
            .filter(|knot| *knot >= start && *knot <= end)
            .collect();
        breaks.dedup();
        breaks.len().saturating_sub(1).max(1)
    }

    pub fn point(&self, parameter: f64) -> Option<Point3> {
        let (start, end) = self.domain()?;
        let parameter = parameter.clamp(start, end);
        let degree = self.degree;
        let span = self.span(parameter);
        let first = span.checked_sub(degree)?;
        let weight = |index: usize| {
            self.weights
                .as_ref()
                .and_then(|weights| weights.get(index))
                .copied()
                .unwrap_or(1.0)
        };
        let mut local: Vec<(Vector3, f64)> = (first..=span)
            .map(|index| {
                let w = weight(index);
                self.points.get(index).map(|point| (*point * w, w))
            })
            .collect::<Option<_>>()?;
        for level in 1..=degree {
            for index in (level..=degree).rev() {
                let low = *self.knots.get(first + index)?;
                let high = *self.knots.get(first + index + degree + 1 - level)?;
                let width = high - low;
                let alpha = if width > 0.0 {
                    (parameter - low) / width
                } else {
                    0.0
                };
                let (previous_point, previous_weight) = *local.get(index - 1)?;
                let current = local.get_mut(index)?;
                current.0 = previous_point.lerp(current.0, alpha);
                current.1 = previous_weight + (current.1 - previous_weight) * alpha;
            }
        }
        let (point, weight) = *local.get(degree)?;
        (weight > 0.0).then(|| point / weight)
    }

    fn span(&self, parameter: f64) -> usize {
        let degree = self.degree;
        let candidates = self
            .knots
            .get(degree..self.points.len())
            .unwrap_or_default();
        let at_or_before = degree + candidates.partition_point(|knot| *knot <= parameter);
        let span = at_or_before.saturating_sub(1).max(degree);
        match (self.knots.get(span), self.knots.get(span + 1)) {
            (Some(low), Some(high)) if low == high => {
                let before_run = self.knots.partition_point(|knot| knot < high);
                before_run.saturating_sub(1).max(degree)
            }
            _ => span,
        }
    }

    pub fn transformed(&self, transform: &Affine) -> Self {
        Self {
            degree: self.degree,
            knots: self.knots.clone(),
            points: self
                .points
                .iter()
                .map(|point| transform.point(*point))
                .collect(),
            weights: self.weights.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::import) enum Shape {
    Point(Point3),
    Line(Point3, Point3),
    Conic {
        center: Point3,
        major: Vector3,
        minor: Vector3,
        start: f64,
        sweep: f64,
    },
    Spline(Nurbs),
    Interpolated(FitPoints),
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::import) struct FitPoints {
    pub points: Vec<Point3>,
    pub start_tangent: Option<Vector3>,
    pub end_tangent: Option<Vector3>,
}

impl FitPoints {
    pub fn new(
        points: Vec<Point3>,
        start_tangent: Option<Vector3>,
        end_tangent: Option<Vector3>,
    ) -> Self {
        let direction = |tangent: Option<Vector3>| tangent.filter(|tangent| tangent.length() > 0.0);
        Self {
            points,
            start_tangent: direction(start_tangent),
            end_tangent: direction(end_tangent),
        }
    }

    pub fn has_tangents(&self) -> bool {
        self.start_tangent.is_some() || self.end_tangent.is_some()
    }

    fn transformed(&self, transform: &Affine) -> Self {
        Self {
            points: self
                .points
                .iter()
                .map(|point| transform.point(*point))
                .collect(),
            start_tangent: self.start_tangent.map(|tangent| transform.vector(tangent)),
            end_tangent: self.end_tangent.map(|tangent| transform.vector(tangent)),
        }
    }

    pub fn cubic(&self) -> Option<Nurbs> {
        let mut points = self.points.clone();
        points.dedup();

        let chords = ChordParameters::along(&points)?;
        let (first, last) = (*points.first()?, *points.last()?);
        let spans = chords.spans();
        let slope = |tangent: Option<Vector3>| {
            tangent
                .and_then(|tangent| tangent.try_normalize())
                .map(|direction| direction * chords.length)
        };

        let mut rows = Vec::with_capacity(spans + 1);
        rows.push(chords.start_row(first, slope(self.start_tangent)));
        for (index, point) in points.iter().enumerate().take(spans).skip(1) {
            rows.push(chords.interior_row(isize::try_from(index).ok()?, *point));
        }
        rows.push(chords.end_row(last, slope(self.end_tangent))?);

        let mut control_points = vec![first];
        control_points.extend(solve_tridiagonal(&rows)?);
        control_points.push(last);
        let mut knots = vec![0.0; CUBIC + 1];
        knots.extend(chords.values.iter().take(spans).skip(1));
        knots.extend([1.0; CUBIC + 1]);
        Nurbs::new(CUBIC, knots, control_points, None)
    }
}

const CUBIC: usize = 3;

struct ChordParameters {
    values: Vec<f64>,
    length: f64,
}

impl ChordParameters {
    fn along(points: &[Point3]) -> Option<Self> {
        let chords: Vec<f64> = points
            .windows(2)
            .map(|pair| match pair {
                [from, to] => from.distance(*to),
                _ => 0.0,
            })
            .collect();
        let length: f64 = chords.iter().sum();
        if chords.is_empty() || !length.is_finite() || length <= 0.0 {
            return None;
        }

        let mut values = vec![0.0];
        let mut travelled = 0.0;
        for chord in chords.iter().take(chords.len() - 1) {
            travelled += chord;
            values.push(travelled / length);
        }
        values.push(1.0);

        let increasing = values
            .windows(2)
            .all(|pair| matches!(pair, [low, high] if low < high));
        increasing.then_some(Self { values, length })
    }

    fn spans(&self) -> usize {
        self.values.len() - 1
    }

    fn at(&self, index: isize) -> f64 {
        let clamped = usize::try_from(index.max(0))
            .unwrap_or_default()
            .min(self.spans());
        self.values.get(clamped).copied().unwrap_or(1.0)
    }

    fn start_row(&self, first: Point3, slope: Option<Vector3>) -> Row {
        let (near, far) = (self.at(1), self.at(2));
        match slope {
            Some(slope) => Row::fixed(first + slope * (near / 3.0)),
            None => Row {
                below: 0.0,
                diagonal: 1.0 / near + 1.0 / far,
                above: -1.0 / far,
                value: first / near,
            },
        }
    }

    fn interior_row(&self, index: isize, point: Point3) -> Row {
        let (before, here, after) = (self.at(index - 1), self.at(index), self.at(index + 1));
        let below = (after - here).powi(2) / ((after - self.at(index - 2)) * (after - before));
        let above = (here - before).powi(2) / ((self.at(index + 2) - before) * (after - before));
        Row {
            below,
            diagonal: 1.0 - below - above,
            above,
            value: point,
        }
    }

    fn end_row(&self, last: Point3, slope: Option<Vector3>) -> Option<Row> {
        let spans = isize::try_from(self.spans()).ok()?;
        let (near, far) = (1.0 - self.at(spans - 1), 1.0 - self.at(spans - 2));
        Some(match slope {
            Some(slope) => Row::fixed(last - slope * (near / 3.0)),
            None => Row {
                below: -1.0 / far,
                diagonal: 1.0 / near + 1.0 / far,
                above: 0.0,
                value: last / near,
            },
        })
    }
}

struct Row {
    below: f64,
    diagonal: f64,
    above: f64,
    value: Point3,
}

impl Row {
    fn fixed(value: Point3) -> Self {
        Self {
            below: 0.0,
            diagonal: 1.0,
            above: 0.0,
            value,
        }
    }
}

fn solve_tridiagonal(rows: &[Row]) -> Option<Vec<Point3>> {
    let mut reduced: Vec<(f64, f64, Point3)> = Vec::with_capacity(rows.len());
    for row in rows {
        let (diagonal, value) = match reduced.last() {
            Some((previous_diagonal, previous_above, previous_value)) => {
                let factor = row.below / previous_diagonal;
                (
                    row.diagonal - factor * previous_above,
                    row.value - *previous_value * factor,
                )
            }
            None => (row.diagonal, row.value),
        };
        if diagonal == 0.0 || !diagonal.is_finite() {
            return None;
        }
        reduced.push((diagonal, row.above, value));
    }
    let mut solution: Vec<Point3> = Vec::with_capacity(reduced.len());
    for (diagonal, above, value) in reduced.iter().rev() {
        let next = solution.last().map_or(Vector3::ZERO, |next| *next * *above);
        solution.push((*value - next) / *diagonal);
    }
    solution.reverse();
    solution
        .iter()
        .all(|point| point.is_finite())
        .then_some(solution)
}

impl Shape {
    pub fn size(&self) -> usize {
        match self {
            Self::Point(_) => 1,
            Self::Line(..) => 2,
            Self::Conic { .. } => 3,
            Self::Spline(nurbs) => nurbs.points.len() + nurbs.knots.len(),
            Self::Interpolated(fit) => fit.points.len(),
        }
    }

    pub fn transformed(&self, transform: &Affine) -> Self {
        match self {
            Self::Point(point) => Self::Point(transform.point(*point)),
            Self::Line(start, end) => Self::Line(transform.point(*start), transform.point(*end)),
            Self::Conic {
                center,
                major,
                minor,
                start,
                sweep,
            } => Self::Conic {
                center: transform.point(*center),
                major: transform.vector(*major),
                minor: transform.vector(*minor),
                start: *start,
                sweep: *sweep,
            },
            Self::Spline(nurbs) => Self::Spline(nurbs.transformed(transform)),
            Self::Interpolated(fit) => Self::Interpolated(fit.transformed(transform)),
        }
    }

    pub fn defining_points(&self) -> Vec<Point3> {
        match self {
            Self::Point(point) => vec![*point],
            Self::Line(start, end) => vec![*start, *end],
            Self::Conic {
                center,
                major,
                minor,
                ..
            } => vec![
                *center + *major,
                *center - *major,
                *center + *minor,
                *center - *minor,
            ],
            Self::Spline(nurbs) => nurbs.points.clone(),
            Self::Interpolated(fit) => fit.points.clone(),
        }
    }
}

pub(super) fn conic_arc(
    center: Point3,
    major: Vector3,
    minor: Vector3,
    start: f64,
    end: f64,
) -> Shape {
    Shape::Conic {
        center,
        major,
        minor,
        start,
        sweep: sweep_between(start, end),
    }
}

pub(super) fn sweep_between(start: f64, end: f64) -> f64 {
    let sweep = (end - start).rem_euclid(TAU);
    if sweep <= FULL_TURN_TOLERANCE || !sweep.is_finite() {
        TAU
    } else {
        sweep
    }
}

pub(super) fn is_full_turn(sweep: f64) -> bool {
    sweep >= TAU - FULL_TURN_TOLERANCE
}

pub(super) struct PlanarCircle {
    pub center: Point2,
    pub radius: f64,
    pub start_angle: f64,
    pub counter_clockwise: bool,
}

pub(super) fn planar_circle(
    center: Point3,
    major: Vector3,
    minor: Vector3,
) -> Option<PlanarCircle> {
    let (a, b) = (flat(major), flat(minor));
    let radius = a.length();
    let scale = radius.max(b.length());
    let circular = radius > 0.0
        && (a.length() - b.length()).abs() <= CIRCULAR_TOLERANCE * scale
        && a.dot(b).abs() <= CIRCULAR_TOLERANCE * scale * scale;
    circular.then(|| PlanarCircle {
        center: Point2::new(center.x, center.y),
        radius,
        start_angle: a.y.atan2(a.x),
        counter_clockwise: a.perp_dot(b) > 0.0,
    })
}

pub(super) fn flat(vector: Vector3) -> Vector2 {
    Vector2::new(vector.x, vector.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arbitrary_axis_algorithm_mirrors_a_downward_normal() {
        let up = Affine::object_system(Vector3::Z).unwrap();
        assert_eq!(
            up.point(Point3::new(1.0, 2.0, 3.0)),
            Point3::new(1.0, 2.0, 3.0)
        );
        let down = Affine::object_system(-Vector3::Z).unwrap();
        let mapped = down.point(Point3::new(1.0, 2.0, 3.0));
        assert!(mapped.distance(Point3::new(-1.0, 2.0, -3.0)) < 1e-12);
        let tilted = Affine::object_system(Vector3::X).unwrap();
        assert!(tilted.vector(Vector3::X).distance(Vector3::Y) < 1e-12);
        assert!(tilted.vector(Vector3::Y).distance(Vector3::Z) < 1e-12);
    }

    #[test]
    fn transforms_compose_inner_first() {
        let inner = Affine::translation(Vector3::X);
        let outer = Affine::rotation_z(std::f64::consts::FRAC_PI_2);
        let both = inner.then(&outer);
        assert!(both.point(Point3::ZERO).distance(Point3::Y) < 1e-12);
    }

    #[test]
    fn a_nurbs_quarter_circle_stays_on_the_circle() {
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let nurbs = Nurbs::new(
            2,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![Point3::X, Point3::new(1.0, 1.0, 0.0), Point3::Y],
            Some(vec![1.0, half, 1.0]),
        )
        .unwrap();
        for step in 0..=10 {
            let point = nurbs.point(step as f64 / 10.0).unwrap();
            assert!((point.length() - 1.0).abs() < 1e-12);
        }
        assert_eq!(nurbs.point(0.0), Some(Point3::X));
        assert!(nurbs.point(1.0).unwrap().distance(Point3::Y) < 1e-12);
    }

    #[test]
    fn an_unclamped_nurbs_runs_over_its_inner_knots() {
        let corner = Point3::new(1.0, 1.0, 0.0);
        let nurbs = Nurbs::new(
            1,
            vec![0.0, 1.0, 2.0, 3.0, 4.0],
            vec![Point3::ZERO, Point3::X, corner],
            None,
        )
        .unwrap();
        assert_eq!(nurbs.domain(), Some((1.0, 3.0)));
        assert!(nurbs.point(1.0).unwrap().distance(Point3::ZERO) < 1e-12);
        assert!(nurbs.point(2.0).unwrap().distance(Point3::X) < 1e-12);
        assert!(nurbs.point(3.0).unwrap().distance(corner) < 1e-12);
        assert!(Nurbs::new(3, vec![0.0; 4], vec![Point3::ZERO; 2], None).is_none());
    }

    #[test]
    fn a_circle_seen_from_below_turns_clockwise() {
        let circle = planar_circle(Point3::ZERO, -Vector3::X, Vector3::Y).unwrap();
        assert!(!circle.counter_clockwise);
        assert!(planar_circle(Point3::ZERO, Vector3::X * 2.0, Vector3::Y).is_none());
    }

    fn scanned_span(nurbs: &Nurbs, parameter: f64) -> usize {
        (nurbs.degree..nurbs.points.len())
            .rev()
            .find(|span| {
                nurbs.knots[*span] <= parameter && nurbs.knots[*span] < nurbs.knots[span + 1]
            })
            .unwrap_or(nurbs.degree)
    }

    #[test]
    fn the_span_search_finds_the_last_nonempty_span_at_or_before_a_parameter() {
        let knot_vectors = [
            vec![0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0, 3.0, 3.0, 3.0, 3.0],
            vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0],
            vec![0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 2.5, 3.0, 3.0, 3.0, 3.0],
            vec![0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 2.0, 3.0, 3.0, 3.0, 3.0],
        ];

        for knots in knot_vectors {
            let points = (0..8).map(|index| Point3::X * f64::from(index)).collect();
            let nurbs = Nurbs::new(3, knots.clone(), points, None).unwrap();
            let (start, end) = nurbs.domain().unwrap();
            for step in 0..=300 {
                let parameter = start + (end - start) * f64::from(step) / 300.0;
                assert_eq!(
                    nurbs.span(parameter),
                    scanned_span(&nurbs, parameter),
                    "{knots:?} at {parameter}"
                );
            }
            for knot in knots.iter().filter(|knot| (start..=end).contains(*knot)) {
                assert_eq!(nurbs.span(*knot), scanned_span(&nurbs, *knot));
            }
        }
    }

    fn fit_points() -> Vec<Point3> {
        vec![
            Point3::ZERO,
            Point3::new(3.0, 4.0, 0.0),
            Point3::new(9.0, 4.0, 1.0),
            Point3::new(12.0, 0.0, 1.0),
        ]
    }

    fn chord_parameters(points: &[Point3]) -> Vec<f64> {
        let chords: Vec<f64> = points.windows(2).map(|w| w[0].distance(w[1])).collect();
        let length: f64 = chords.iter().sum();
        let mut travelled = 0.0;
        let mut parameters = vec![0.0];
        for chord in chords {
            travelled += chord;
            parameters.push(travelled / length);
        }
        parameters
    }

    #[test]
    fn a_cubic_through_fit_points_leaves_along_its_tangents() {
        let points = fit_points();
        let length: f64 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
        let step = 1e-6;
        let start = Vector3::new(0.0, 2.0, 0.0);
        let end = Vector3::new(1.0, -1.0, 0.0);

        let both = FitPoints::new(points.clone(), Some(start), Some(end))
            .cubic()
            .unwrap();
        let starting = FitPoints::new(points.clone(), Some(start), None)
            .cubic()
            .unwrap();
        let leaving = (both.point(step).unwrap() - both.point(0.0).unwrap()) / step;
        let arriving = (both.point(1.0).unwrap() - both.point(1.0 - step).unwrap()) / step;
        let curvature_step = 1e-4;
        let bend = (starting.point(1.0).unwrap()
            - starting.point(1.0 - curvature_step).unwrap() * 2.0
            + starting.point(1.0 - 2.0 * curvature_step).unwrap())
            / (curvature_step * curvature_step);

        assert_eq!(both.degree, 3);
        for (point, parameter) in points.iter().zip(chord_parameters(&points)) {
            assert!(both.point(parameter).unwrap().distance(*point) < 1e-9);
            assert!(starting.point(parameter).unwrap().distance(*point) < 1e-9);
        }
        assert!(leaving.distance(start.normalize() * length) < 1e-3 * length);
        assert!(arriving.distance(end.normalize() * length) < 1e-3 * length);
        assert!(bend.length() < 1e-2 * length, "{bend}");
    }

    #[test]
    fn fit_points_that_all_coincide_have_no_cubic() {
        let fit = FitPoints::new(vec![Point3::X; 3], Some(Vector3::Y), None);

        assert!(fit.cubic().is_none());
        assert!(!FitPoints::new(fit_points(), Some(Vector3::ZERO), None).has_tangents());
    }
}
