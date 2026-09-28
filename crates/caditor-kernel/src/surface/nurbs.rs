use std::sync::Arc;

use caditor_geometry::{Aabb, Point2, Point3, RigidTransform, Vector3};

use crate::{
    bspline::{MAX_SPLINE_DEGREE, clamped_domain},
    error::GeometryError,
    interval::Interval,
    surface::{Pole, SurfaceDerivatives, projection::periodic_near},
    tolerance::LINEAR_RESOLUTION,
};

const SAMPLES_PER_SPAN: usize = 3;
const MAX_GRID_SAMPLES: usize = 48;
const PROJECTION_SEEDS: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct BSplineSurface {
    u_degree: usize,
    v_degree: usize,
    u_knots: Arc<[f64]>,
    v_knots: Arc<[f64]>,
    columns: usize,
    rows: usize,
    control_points: Arc<[Point3]>,
    weights: Option<Arc<[f64]>>,
    u_domain: Interval,
    v_domain: Interval,
    u_closed: bool,
    v_closed: bool,
    grid: Arc<[(Point2, Point3)]>,
}

#[derive(Debug, Clone, Copy)]
struct Basis {
    first: usize,
    values: [[f64; MAX_SPLINE_DEGREE + 1]; 3],
}

impl BSplineSurface {
    pub fn new(
        u_degree: usize,
        v_degree: usize,
        u_knots: Vec<f64>,
        v_knots: Vec<f64>,
        columns: usize,
        control_points: Vec<Point3>,
        weights: Option<Vec<f64>>,
    ) -> Result<Self, GeometryError> {
        for degree in [u_degree, v_degree] {
            if degree == 0 || degree > MAX_SPLINE_DEGREE {
                return Err(GeometryError::SplineDegree(degree));
            }
        }
        let rows = control_points.len().checked_div(columns).unwrap_or(0);
        if columns * rows != control_points.len() || columns <= u_degree || rows <= v_degree {
            return Err(GeometryError::TooFewControlPoints {
                degree: u_degree.max(v_degree),
                points: control_points.len(),
            });
        }
        for (degree, count, knots) in [(u_degree, columns, &u_knots), (v_degree, rows, &v_knots)] {
            if knots.len() != count + degree + 1 {
                return Err(GeometryError::KnotCount {
                    degree,
                    points: count,
                    knots: knots.len(),
                });
            }
        }
        if !control_points.iter().all(|point| point.is_finite()) {
            return Err(GeometryError::NonFinite);
        }
        let weights = match weights {
            Some(weights) if weights.len() != control_points.len() => {
                return Err(GeometryError::WeightCount {
                    points: control_points.len(),
                    weights: weights.len(),
                });
            }
            Some(weights) => {
                if let Some(bad) = weights
                    .iter()
                    .find(|weight| !(weight.is_finite() && **weight > 0.0))
                {
                    return Err(GeometryError::Weight(*bad));
                }
                weights
                    .iter()
                    .any(|weight| *weight != 1.0)
                    .then(|| Arc::from(weights))
            }
            None => None,
        };
        let u_domain = clamped_domain(&u_knots, u_degree).ok_or(GeometryError::Knots)?;
        let v_domain = clamped_domain(&v_knots, v_degree).ok_or(GeometryError::Knots)?;
        let mut surface = Self {
            u_degree,
            v_degree,
            u_knots: u_knots.into(),
            v_knots: v_knots.into(),
            columns,
            rows,
            control_points: control_points.into(),
            weights,
            u_domain,
            v_domain,
            u_closed: false,
            v_closed: false,
            grid: Arc::from(Vec::new()),
        };
        surface.u_closed = surface.boundaries_meet(true);
        surface.v_closed = surface.boundaries_meet(false);
        surface.grid = surface.sample_grid().into();
        if surface
            .grid
            .iter()
            .all(|(uv, _)| surface.evaluate(uv.x, uv.y).normal().is_none())
        {
            return Err(GeometryError::DegenerateSurface);
        }
        Ok(surface)
    }

    pub fn u_degree(&self) -> usize {
        self.u_degree
    }

    pub fn v_degree(&self) -> usize {
        self.v_degree
    }

    pub fn u_knots(&self) -> &[f64] {
        &self.u_knots
    }

    pub fn v_knots(&self) -> &[f64] {
        &self.v_knots
    }

    pub fn columns(&self) -> usize {
        self.columns
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn control_points(&self) -> &[Point3] {
        &self.control_points
    }

    pub fn weights(&self) -> Option<&[f64]> {
        self.weights.as_deref()
    }

    pub fn control_point(&self, column: usize, row: usize) -> Option<Point3> {
        self.control_points
            .get(row * self.columns + column)
            .copied()
    }

    fn weight(&self, index: usize) -> f64 {
        self.weights
            .as_ref()
            .and_then(|weights| weights.get(index))
            .copied()
            .unwrap_or(1.0)
    }

    pub fn u_domain(&self) -> Interval {
        self.u_domain
    }

    pub fn v_domain(&self) -> Interval {
        self.v_domain
    }

    pub fn u_period(&self) -> Option<f64> {
        self.u_closed.then(|| self.u_domain.length())
    }

    pub fn v_period(&self) -> Option<f64> {
        self.v_closed.then(|| self.v_domain.length())
    }

    #[must_use]
    pub fn transposed(&self) -> Self {
        let mut points = Vec::with_capacity(self.control_points.len());
        let mut weights = Vec::with_capacity(self.control_points.len());
        for column in 0..self.columns {
            for row in 0..self.rows {
                let index = row * self.columns + column;
                points.push(
                    self.control_points
                        .get(index)
                        .copied()
                        .unwrap_or(Point3::ZERO),
                );
                weights.push(self.weight(index));
            }
        }
        let mut transposed = Self {
            u_degree: self.v_degree,
            v_degree: self.u_degree,
            u_knots: Arc::clone(&self.v_knots),
            v_knots: Arc::clone(&self.u_knots),
            columns: self.rows,
            rows: self.columns,
            control_points: points.into(),
            weights: self.weights.as_ref().map(|_| Arc::from(weights)),
            u_domain: self.v_domain,
            v_domain: self.u_domain,
            u_closed: self.v_closed,
            v_closed: self.u_closed,
            grid: Arc::from(Vec::new()),
        };
        transposed.grid = transposed.sample_grid().into();
        transposed
    }

    pub fn transformed(&self, transform: &RigidTransform) -> Result<Self, GeometryError> {
        let points: Vec<Point3> = self
            .control_points
            .iter()
            .map(|point| transform.apply_point(*point))
            .collect();
        if !points.iter().all(|point| point.is_finite()) {
            return Err(GeometryError::NonFinite);
        }
        let mut moved = Self {
            control_points: points.into(),
            ..self.clone()
        };
        moved.grid = moved
            .grid
            .iter()
            .map(|(uv, point)| (*uv, transform.apply_point(*point)))
            .collect::<Vec<_>>()
            .into();
        Ok(moved)
    }

    pub fn degenerate_column(&self, column: usize) -> bool {
        let points: Vec<Point3> = (0..self.rows)
            .filter_map(|row| self.control_point(column, row))
            .collect();
        points
            .windows(2)
            .all(|pair| matches!(pair, [a, b] if a.distance(*b) <= LINEAR_RESOLUTION))
    }

    pub fn degenerate_row(&self, row: usize) -> bool {
        let points: Vec<Point3> = (0..self.columns)
            .filter_map(|column| self.control_point(column, row))
            .collect();
        points
            .windows(2)
            .all(|pair| matches!(pair, [a, b] if a.distance(*b) <= LINEAR_RESOLUTION))
    }

    pub(crate) fn poles(&self) -> Vec<Pole> {
        [
            (0, self.v_domain.start()),
            (self.rows.saturating_sub(1), self.v_domain.end()),
        ]
        .into_iter()
        .filter(|(row, _)| self.degenerate_row(*row))
        .filter_map(|(row, v)| {
            Some(Pole {
                v,
                point: self.control_point(0, row)?,
            })
        })
        .collect()
    }

    fn boundaries_meet(&self, along_u: bool) -> bool {
        let (count, other) = if along_u {
            (self.rows, self.columns)
        } else {
            (self.columns, self.rows)
        };
        let index = |at: usize, position: usize| {
            if along_u {
                position * self.columns + at
            } else {
                at * self.columns + position
            }
        };
        (0..count).all(|position| {
            let (first, last) = (index(0, position), index(other - 1, position));
            match (
                self.control_points.get(first),
                self.control_points.get(last),
            ) {
                (Some(a), Some(b)) => {
                    a.distance(*b) <= LINEAR_RESOLUTION
                        && (self.weight(first) - self.weight(last)).abs() <= 1e-12
                }
                _ => false,
            }
        })
    }

    fn wrap(&self, u: f64, v: f64) -> (f64, f64) {
        let wrap = |value: f64, domain: Interval, closed: bool| {
            if closed && value.is_finite() && domain.length() > 0.0 {
                let wrapped = domain.start() + (value - domain.start()).rem_euclid(domain.length());
                if (value - domain.end()).abs() <= 1e-12 * (1.0 + domain.end().abs()) {
                    domain.end()
                } else {
                    wrapped
                }
            } else {
                domain.clamp(value)
            }
        };
        (
            wrap(u, self.u_domain, self.u_closed),
            wrap(v, self.v_domain, self.v_closed),
        )
    }

    pub(crate) fn evaluate(&self, u: f64, v: f64) -> SurfaceDerivatives {
        let (u, v) = self.wrap(u, v);
        let (Some(along_u), Some(along_v)) = (
            basis(&self.u_knots, self.u_degree, self.columns, u),
            basis(&self.v_knots, self.v_degree, self.rows, v),
        ) else {
            return SurfaceDerivatives {
                point: Point3::ZERO,
                du: Vector3::ZERO,
                dv: Vector3::ZERO,
                duu: Vector3::ZERO,
                duv: Vector3::ZERO,
                dvv: Vector3::ZERO,
            };
        };
        let mut sums = [[(Vector3::ZERO, 0.0); 3]; 3];
        for (j, v_values) in (0..=self.v_degree).map(|j| (j, column_of(&along_v, j))) {
            let row = along_v.first + j;
            for i in 0..=self.u_degree {
                let column = along_u.first + i;
                let index = row * self.columns + column;
                let Some(point) = self.control_points.get(index) else {
                    continue;
                };
                let weight = self.weight(index);
                let u_values = column_of(&along_u, i);
                for (k, u_value) in u_values.iter().enumerate() {
                    for (l, v_value) in v_values.iter().enumerate() {
                        if k + l > 2 {
                            continue;
                        }
                        let factor = u_value * v_value * weight;
                        if let Some(slot) = sums.get_mut(k).and_then(|row| row.get_mut(l)) {
                            slot.0 += *point * factor;
                            slot.1 += factor;
                        }
                    }
                }
            }
        }
        let get = |k: usize, l: usize| {
            sums.get(k)
                .and_then(|row| row.get(l))
                .copied()
                .unwrap_or((Vector3::ZERO, 0.0))
        };
        let (a, w) = get(0, 0);
        if w <= 0.0 {
            return SurfaceDerivatives {
                point: Point3::ZERO,
                du: Vector3::ZERO,
                dv: Vector3::ZERO,
                duu: Vector3::ZERO,
                duv: Vector3::ZERO,
                dvv: Vector3::ZERO,
            };
        }
        let (a_u, w_u) = get(1, 0);
        let (a_v, w_v) = get(0, 1);
        let (a_uu, w_uu) = get(2, 0);
        let (a_uv, w_uv) = get(1, 1);
        let (a_vv, w_vv) = get(0, 2);
        let point = a / w;
        let du = (a_u - point * w_u) / w;
        let dv = (a_v - point * w_v) / w;
        SurfaceDerivatives {
            point,
            du,
            dv,
            duu: (a_uu - du * (2.0 * w_u) - point * w_uu) / w,
            duv: (a_uv - du * w_v - dv * w_u - point * w_uv) / w,
            dvv: (a_vv - dv * (2.0 * w_v) - point * w_vv) / w,
        }
    }

    pub(crate) fn project_seed(&self, point: Point3, hint: Option<Point2>) -> Vec<Point2> {
        let mut nearest: Vec<(f64, Point2)> = Vec::with_capacity(PROJECTION_SEEDS + 1);
        for (uv, sample) in self.grid.iter() {
            let distance = sample.distance_squared(point);
            let full = nearest.len() >= PROJECTION_SEEDS;
            if full && nearest.last().is_some_and(|(worst, _)| distance >= *worst) {
                continue;
            }
            let at = nearest.partition_point(|(known, _)| *known <= distance);
            nearest.insert(at, (distance, *uv));
            nearest.truncate(PROJECTION_SEEDS);
        }
        let mut chosen: Vec<Point2> = nearest.into_iter().map(|(_, uv)| uv).collect();
        if let Some(hint) = hint.filter(|hint| hint.is_finite()) {
            let (u, v) = self.wrap(hint.x, hint.y);
            chosen.insert(0, Point2::new(u, v));
        }
        chosen
    }

    pub(crate) fn place(&self, uv: Point2, hint: Option<Point2>) -> Point2 {
        let u = match self.u_period() {
            Some(period) if period > 0.0 => {
                let offset = periodic_near(
                    uv.x - self.u_domain.start(),
                    period,
                    hint.map(|hint| hint.x - self.u_domain.start()),
                );
                self.u_domain.start() + offset
            }
            _ => uv.x,
        };
        let v = match self.v_period() {
            Some(period) if period > 0.0 => {
                let offset = periodic_near(
                    uv.y - self.v_domain.start(),
                    period,
                    hint.map(|hint| hint.y - self.v_domain.start()),
                );
                self.v_domain.start() + offset
            }
            _ => uv.y,
        };
        Point2::new(u, v)
    }

    pub(crate) fn bounds(&self, u_range: Interval, v_range: Interval) -> Aabb {
        let wraps = |range: Interval, domain: Interval, closed: bool| {
            closed && (range.start() < domain.start() || range.end() > domain.end())
        };
        let span_indices = |knots: &[f64],
                            degree: usize,
                            count: usize,
                            range: Interval,
                            domain: Interval,
                            whole: bool| {
            if whole {
                return (0, count - 1);
            }
            let low = domain.clamp(range.start());
            let high = domain.clamp(range.end());
            let first = span_of(knots, degree, count, low).saturating_sub(degree);
            let last = span_of(knots, degree, count, high).min(count - 1);
            (first, last)
        };
        let (u_first, u_last) = span_indices(
            &self.u_knots,
            self.u_degree,
            self.columns,
            u_range,
            self.u_domain,
            wraps(u_range, self.u_domain, self.u_closed),
        );
        let (v_first, v_last) = span_indices(
            &self.v_knots,
            self.v_degree,
            self.rows,
            v_range,
            self.v_domain,
            wraps(v_range, self.v_domain, self.v_closed),
        );
        let points = (v_first..=v_last).flat_map(|row| {
            (u_first..=u_last).filter_map(move |column| self.control_point(column, row))
        });
        Aabb::from_points(points).unwrap_or_else(|| Aabb::from_point(Point3::ZERO))
    }

    fn sample_grid(&self) -> Vec<(Point2, Point3)> {
        let steps = |knots: &[f64], domain: Interval| {
            let mut breaks: Vec<f64> = knots
                .iter()
                .copied()
                .filter(|knot| domain.contains(*knot))
                .collect();
            breaks.dedup();
            let spans = breaks.len().saturating_sub(1).max(1);
            (spans * SAMPLES_PER_SPAN).clamp(2, MAX_GRID_SAMPLES)
        };
        let (columns, rows) = (
            steps(&self.u_knots, self.u_domain),
            steps(&self.v_knots, self.v_domain),
        );
        let mut grid = Vec::with_capacity((columns + 1) * (rows + 1));
        for row in 0..=rows {
            let v = self.v_domain.at(row as f64 / rows as f64);
            for column in 0..=columns {
                let u = self.u_domain.at(column as f64 / columns as f64);
                grid.push((Point2::new(u, v), self.evaluate(u, v).point));
            }
        }
        grid
    }
}

fn column_of(basis: &Basis, index: usize) -> [f64; 3] {
    [0, 1, 2].map(|order| {
        basis
            .values
            .get(order)
            .and_then(|row| row.get(index))
            .copied()
            .unwrap_or(0.0)
    })
}

fn span_of(knots: &[f64], degree: usize, count: usize, parameter: f64) -> usize {
    knots
        .partition_point(|knot| *knot <= parameter)
        .saturating_sub(1)
        .clamp(degree, count - 1)
}

fn basis(knots: &[f64], degree: usize, count: usize, parameter: f64) -> Option<Basis> {
    let span = span_of(knots, degree, count, parameter);
    let knot = |index: usize| knots.get(index).copied().unwrap_or(0.0);
    let mut table = [[0.0f64; MAX_SPLINE_DEGREE + 1]; MAX_SPLINE_DEGREE + 1];
    let mut left = [0.0f64; MAX_SPLINE_DEGREE + 1];
    let mut right = [0.0f64; MAX_SPLINE_DEGREE + 1];
    *table.get_mut(0)?.get_mut(0)? = 1.0;
    for j in 1..=degree {
        *left.get_mut(j)? = parameter - knot(span + 1 - j);
        *right.get_mut(j)? = knot(span + j) - parameter;
        let mut saved = 0.0;
        for r in 0..j {
            let lower = *right.get(r + 1)? + *left.get(j - r)?;
            *table.get_mut(j)?.get_mut(r)? = lower;
            let temp = if lower != 0.0 {
                *table.get(r)?.get(j - 1)? / lower
            } else {
                0.0
            };
            *table.get_mut(r)?.get_mut(j)? = saved + *right.get(r + 1)? * temp;
            saved = *left.get(j - r)? * temp;
        }
        *table.get_mut(j)?.get_mut(j)? = saved;
    }
    let mut values = [[0.0f64; MAX_SPLINE_DEGREE + 1]; 3];
    for j in 0..=degree {
        *values.get_mut(0)?.get_mut(j)? = *table.get(j)?.get(degree)?;
    }
    let mut a = [[0.0f64; MAX_SPLINE_DEGREE + 1]; 2];
    for r in 0..=degree {
        let (mut s1, mut s2) = (0usize, 1usize);
        *a.get_mut(0)?.get_mut(0)? = 1.0;
        for k in 1..=2usize.min(degree) {
            let mut d = 0.0;
            let rk = r as isize - k as isize;
            let pk = degree - k;
            if rk >= 0 {
                let rk = rk as usize;
                let denominator = *table.get(pk + 1)?.get(rk)?;
                let value = if denominator != 0.0 {
                    *a.get(s1)?.first()? / denominator
                } else {
                    0.0
                };
                *a.get_mut(s2)?.get_mut(0)? = value;
                d = value * *table.get(rk)?.get(pk)?;
            }
            let j1 = if rk >= -1 { 1 } else { (-rk) as usize };
            let j2 = if r as isize - 1 <= pk as isize {
                k - 1
            } else {
                degree - r
            };
            for j in j1..=j2 {
                let index = (rk + j as isize) as usize;
                let denominator = *table.get(pk + 1)?.get(index)?;
                let value = if denominator != 0.0 {
                    (*a.get(s1)?.get(j)? - *a.get(s1)?.get(j - 1)?) / denominator
                } else {
                    0.0
                };
                *a.get_mut(s2)?.get_mut(j)? = value;
                d += value * *table.get(index)?.get(pk)?;
            }
            if r <= pk {
                let denominator = *table.get(pk + 1)?.get(r)?;
                let value = if denominator != 0.0 {
                    -*a.get(s1)?.get(k - 1)? / denominator
                } else {
                    0.0
                };
                *a.get_mut(s2)?.get_mut(k)? = value;
                d += value * *table.get(r)?.get(pk)?;
            }
            *values.get_mut(k)?.get_mut(r)? = d;
            std::mem::swap(&mut s1, &mut s2);
        }
    }
    let mut factor = degree as f64;
    for k in 1..=2usize.min(degree) {
        for j in 0..=degree {
            *values.get_mut(k)?.get_mut(j)? *= factor;
        }
        factor *= (degree - k) as f64;
    }
    Some(Basis {
        first: span - degree,
        values,
    })
}

#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2};

    use super::*;
    use crate::surface::Surface;

    fn wavy() -> BSplineSurface {
        let mut points = Vec::new();
        for row in 0..5 {
            for column in 0..6 {
                let height = ((row * 7 + column * 3) % 5) as f64 - 2.0;
                points.push(Point3::new(column as f64 * 2.0, row as f64 * 3.0, height));
            }
        }
        BSplineSurface::new(
            3,
            2,
            vec![0.0, 0.0, 0.0, 0.0, 0.4, 0.7, 1.0, 1.0, 1.0, 1.0],
            vec![0.0, 0.0, 0.0, 0.5, 0.8, 2.0, 2.0, 2.0],
            6,
            points,
            Some(
                (0..30)
                    .map(|index| 1.0 + (index % 4) as f64 * 0.25)
                    .collect(),
            ),
        )
        .unwrap()
    }

    fn quarter_cylinder() -> BSplineSurface {
        let arc = [
            (Point3::new(5.0, 0.0, 0.0), 1.0),
            (Point3::new(5.0, 5.0, 0.0), FRAC_1_SQRT_2),
            (Point3::new(0.0, 5.0, 0.0), 1.0),
        ];
        let mut points = Vec::new();
        let mut weights = Vec::new();
        for height in [0.0, 10.0] {
            for (point, weight) in arc {
                points.push(point + Vector3::Z * height);
                weights.push(weight);
            }
        }
        BSplineSurface::new(
            2,
            1,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            3,
            points,
            Some(weights),
        )
        .unwrap()
    }

    #[test]
    fn derivatives_match_finite_differences() {
        let surface = wavy();
        let step = 1e-5;
        for (u, v) in [(0.2, 0.3), (0.55, 1.1), (0.9, 1.9), (0.3, 0.6)] {
            let at = surface.evaluate(u, v);
            let point = |u: f64, v: f64| surface.evaluate(u, v);
            let du = (point(u + step, v).point - point(u - step, v).point) / (2.0 * step);
            let dv = (point(u, v + step).point - point(u, v - step).point) / (2.0 * step);
            let duu = (point(u + step, v).du - point(u - step, v).du) / (2.0 * step);
            let duv = (point(u, v + step).du - point(u, v - step).du) / (2.0 * step);
            let dvv = (point(u, v + step).dv - point(u, v - step).dv) / (2.0 * step);
            for (name, exact, estimate) in [
                ("du", at.du, du),
                ("dv", at.dv, dv),
                ("duu", at.duu, duu),
                ("duv", at.duv, duv),
                ("dvv", at.dvv, dvv),
            ] {
                assert!(
                    exact.distance(estimate) < 1e-4 * (1.0 + exact.length()),
                    "{name} at ({u}, {v}): {exact} vs {estimate}"
                );
            }
        }
    }

    #[test]
    fn a_rational_patch_is_an_exact_cylinder_with_outward_normals() {
        let surface = quarter_cylinder();
        for step in 0..=10 {
            let u = step as f64 / 10.0;
            let at = surface.evaluate(u, 0.5);
            let radial = Vector3::new(at.point.x, at.point.y, 0.0);
            assert!((radial.length() - 5.0).abs() < 1e-12);
            assert!((at.point.z - 5.0).abs() < 1e-12);
            let normal = at.normal().unwrap();
            assert!(normal.dot(radial.normalize()) > 1.0 - 1e-12);
        }
        assert_eq!(surface.u_period(), None);
        assert!(surface.poles().is_empty());
        let wrapped = Surface::BSpline(surface);
        let point = Point3::new(6.0 * FRAC_1_SQRT_2, 6.0 * FRAC_1_SQRT_2, 3.0);
        let uv = wrapped.project(point, None);
        let foot = wrapped.point_at(uv);
        assert!(foot.distance(Point3::new(5.0 * FRAC_1_SQRT_2, 5.0 * FRAC_1_SQRT_2, 3.0)) < 1e-9);
        assert!((wrapped.distance(point) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn projection_finds_the_foot_of_points_off_the_surface() {
        let surface = Surface::BSpline(wavy());
        for (u, v, lift) in [
            (0.1, 0.2, 0.3),
            (0.5, 1.0, -0.2),
            (0.85, 1.7, 0.1),
            (0.33, 0.66, 0.0),
        ] {
            let at = surface.evaluate(u, v);
            let point = at.point + at.normal().unwrap() * lift;
            let uv = surface.project(point, None);
            assert!(
                (uv - Point2::new(u, v)).length() < 1e-6,
                "{uv} vs ({u}, {v})"
            );
            let hinted = surface.project(point, Some(Point2::new(u + 0.01, v - 0.01)));
            assert!((hinted - Point2::new(u, v)).length() < 1e-6);
        }
    }

    #[test]
    fn closed_surfaces_are_periodic_and_collapsed_rows_are_poles() {
        let ring = [
            (1.0, 0.0, 1.0),
            (1.0, 1.0, FRAC_1_SQRT_2),
            (0.0, 1.0, 1.0),
            (-1.0, 1.0, FRAC_1_SQRT_2),
            (-1.0, 0.0, 1.0),
            (-1.0, -1.0, FRAC_1_SQRT_2),
            (0.0, -1.0, 1.0),
            (1.0, -1.0, FRAC_1_SQRT_2),
            (1.0, 0.0, 1.0),
        ];
        let mut points = Vec::new();
        let mut weights = Vec::new();
        for (radius, height) in [(0.0, 0.0), (4.0, 0.0), (4.0, 6.0)] {
            for (x, y, weight) in ring {
                points.push(Point3::new(x * radius, y * radius, height));
                weights.push(weight);
            }
        }
        let knots = vec![
            0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
        ];
        let cup = BSplineSurface::new(
            2,
            2,
            knots,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            9,
            points,
            Some(weights),
        )
        .unwrap();
        assert_eq!(cup.u_period(), Some(1.0));
        assert_eq!(cup.v_period(), None);
        let poles = cup.poles();
        assert_eq!(poles.len(), 1);
        assert_eq!(poles[0].v, 0.0);
        assert!(
            cup.evaluate(1.25, 0.5)
                .point
                .distance(cup.evaluate(0.25, 0.5).point)
                < 1e-12
        );
        let surface = Surface::BSpline(cup.clone());
        let near_seam = surface.project(
            cup.evaluate(0.999, 0.7).point,
            Some(Point2::new(-0.01, 0.7)),
        );
        assert!((near_seam.x + 0.001).abs() < 1e-6, "{near_seam}");
        let turned = cup.transposed();
        assert_eq!(turned.v_period(), Some(1.0));
        assert!(turned.degenerate_column(0));
        assert!(
            turned
                .evaluate(0.5, 0.25)
                .point
                .distance(cup.evaluate(0.25, 0.5).point)
                < 1e-12
        );
        let _ = FRAC_PI_2;
    }

    #[test]
    fn bad_input_is_refused() {
        let points = vec![Point3::ZERO; 4];
        assert!(
            BSplineSurface::new(
                1,
                1,
                vec![0.0, 0.0, 1.0, 1.0],
                vec![0.0, 0.0, 1.0, 1.0],
                3,
                points.clone(),
                None
            )
            .is_err()
        );
        assert!(
            BSplineSurface::new(
                1,
                1,
                vec![0.0, 0.0, 1.0, 1.0],
                vec![0.0, 0.0, 1.0, 1.0],
                2,
                points.clone(),
                None
            )
            .is_err()
        );
        assert!(
            BSplineSurface::new(
                1,
                1,
                vec![0.0, 1.0, 1.0, 1.0],
                vec![0.0, 0.0, 1.0, 1.0],
                2,
                vec![Point3::X, Point3::Y, Point3::Z, Point3::ONE],
                None
            )
            .is_err()
        );
    }
}
