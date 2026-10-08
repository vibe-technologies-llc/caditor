use std::{ops::Range, sync::Arc};

use caditor_geometry::{Aabb, Point2, Point3, RigidTransform, Vector3};

use crate::{
    box_tree::BoxTree,
    bspline::{BSpline, CUBIC_WIDTH, MAX_SPLINE_DEGREE, NARROW_WIDTH, WIDE_WIDTH, clamped_domain},
    error::GeometryError,
    interval::Interval,
    surface::{Pole, SurfaceDerivatives, SurfaceSide, projection::periodic_near},
    tolerance::LINEAR_RESOLUTION,
};

const SAMPLES_PER_SPAN: usize = 3;
const MAX_GRID_SAMPLES: usize = 48;
const PROJECTION_SEEDS: usize = 3;
const BLOCK_SIZE: usize = 8;
const MAX_SPAN_SEARCHES: usize = 32;
const SPAN_SAMPLES: usize = 4;

#[cfg(test)]
pub(crate) mod counting {
    use std::cell::Cell;

    thread_local! {
        pub(crate) static DERIVATIVES: Cell<usize> = const { Cell::new(0) };
        pub(crate) static POINTS: Cell<usize> = const { Cell::new(0) };
    }
}

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
    grid: Arc<SampleGrid>,
    spans: Arc<SpanIndex>,
    poles: [Option<Pole>; 2],
}

#[derive(Debug, Clone, Default, PartialEq)]
struct SpanIndex {
    tree: BoxTree,
    spans: Vec<Span>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Span {
    hull: Aabb,
    u: Interval,
    v: Interval,
}

#[derive(Debug, Clone, PartialEq)]
struct GridBlock {
    bounds: Aabb,
    rows: Range<usize>,
    columns: Range<usize>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct SampleGrid {
    width: usize,
    samples: Vec<(Point2, Point3)>,
    blocks: Vec<GridBlock>,
}

impl SampleGrid {
    fn new(width: usize, samples: Vec<(Point2, Point3)>) -> Self {
        let height = samples.len().checked_div(width).unwrap_or(0);
        let starts = |count: usize| (0..count).step_by(BLOCK_SIZE);
        let mut blocks = Vec::new();
        for row in starts(height) {
            for column in starts(width) {
                blocks.push(GridBlock {
                    bounds: Aabb::from_point(Point3::ZERO),
                    rows: row..(row + BLOCK_SIZE).min(height),
                    columns: column..(column + BLOCK_SIZE).min(width),
                });
            }
        }
        let mut grid = Self {
            width,
            samples,
            blocks,
        };
        grid.bound_blocks();
        grid
    }

    fn members(&self, block: &GridBlock) -> impl Iterator<Item = usize> {
        let width = self.width;
        let columns = block.columns.clone();
        block
            .rows
            .clone()
            .flat_map(move |row| columns.clone().map(move |column| row * width + column))
    }

    fn bound_blocks(&mut self) {
        let bounds: Vec<Option<Aabb>> = self
            .blocks
            .iter()
            .map(|block| {
                Aabb::from_points(
                    self.members(block)
                        .filter_map(|index| self.samples.get(index))
                        .map(|(_, point)| *point),
                )
            })
            .collect();
        for (block, found) in self.blocks.iter_mut().zip(bounds) {
            if let Some(found) = found {
                block.bounds = found;
            }
        }
    }

    fn mapped(&self, map: &impl Fn(Point3) -> Point3) -> Self {
        let mut moved = Self {
            width: self.width,
            samples: self
                .samples
                .iter()
                .map(|(uv, point)| (*uv, map(*point)))
                .collect(),
            blocks: self.blocks.clone(),
        };
        moved.bound_blocks();
        moved
    }

    fn nearest(&self, point: Point3, count: usize) -> Vec<Point2> {
        let mut blocks: Vec<(f64, &GridBlock)> = self
            .blocks
            .iter()
            .map(|block| {
                let clamped = point.clamp(block.bounds.min(), block.bounds.max());
                (clamped.distance_squared(point), block)
            })
            .collect();
        blocks.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut nearest: Vec<(f64, usize)> = Vec::with_capacity(count + 1);
        for (lower, block) in blocks {
            let full = nearest.len() >= count;
            if full && nearest.last().is_some_and(|(worst, _)| lower > *worst) {
                break;
            }
            for index in self.members(block) {
                let Some((_, sample)) = self.samples.get(index) else {
                    continue;
                };
                let candidate = (sample.distance_squared(point), index);
                let full = nearest.len() >= count;
                if full && nearest.last().is_some_and(|worst| candidate >= *worst) {
                    continue;
                }
                let at = nearest.partition_point(|known| *known <= candidate);
                nearest.insert(at, candidate);
                nearest.truncate(count);
            }
        }
        nearest
            .into_iter()
            .filter_map(|(_, index)| self.samples.get(index).map(|(uv, _)| *uv))
            .collect()
    }
}

struct BasisTable<const WIDTH: usize> {
    span: usize,
    table: [[f64; WIDTH]; WIDTH],
}

struct BasisValues<const WIDTH: usize> {
    first: usize,
    values: [f64; WIDTH],
}

#[derive(Debug, Clone, Copy)]
struct Basis<const WIDTH: usize> {
    first: usize,
    values: [[f64; WIDTH]; 3],
}

impl BSplineSurface {
    pub(crate) fn heap_size(&self) -> usize {
        size_of_val(&*self.u_knots)
            + size_of_val(&*self.v_knots)
            + size_of_val(&*self.control_points)
            + self
                .weights
                .as_ref()
                .map_or(0, |weights| size_of_val(&**weights))
            + size_of::<SampleGrid>()
            + size_of_val(self.grid.samples.as_slice())
            + size_of_val(self.grid.blocks.as_slice())
            + size_of_val(self.spans.spans.as_slice())
            + self.spans.tree.heap_size()
    }

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
            grid: Arc::default(),
            spans: Arc::default(),
            poles: [None; 2],
        };
        surface.u_closed = surface.boundaries_meet(true);
        surface.v_closed = surface.boundaries_meet(false);
        surface.grid = Arc::new(surface.sample_grid());
        surface.spans = Arc::new(surface.span_index());
        surface.poles = surface.find_poles();
        if surface
            .grid
            .samples
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
            grid: Arc::default(),
            spans: Arc::default(),
            poles: [None; 2],
        };
        transposed.grid = Arc::new(transposed.sample_grid());
        transposed.spans = Arc::new(transposed.span_index());
        transposed.poles = transposed.find_poles();
        transposed
    }

    pub fn transformed(&self, transform: &RigidTransform) -> Result<Self, GeometryError> {
        self.mapped(|point| transform.apply_point(point))
    }

    pub(crate) fn mapped(&self, map: impl Fn(Point3) -> Point3) -> Result<Self, GeometryError> {
        let points: Vec<Point3> = self
            .control_points
            .iter()
            .map(|point| map(*point))
            .collect();
        if !points.iter().all(|point| point.is_finite()) {
            return Err(GeometryError::NonFinite);
        }
        let mut moved = Self {
            control_points: points.into(),
            ..self.clone()
        };
        moved.grid = Arc::new(self.grid.mapped(&map));
        moved.spans = Arc::new(moved.span_index());
        moved.poles = moved.find_poles();
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

    pub(crate) fn pole_slots(&self) -> [Option<Pole>; 2] {
        self.poles
    }

    #[cfg(test)]
    pub(crate) fn poles(&self) -> Vec<Pole> {
        self.poles.into_iter().flatten().collect()
    }

    fn find_poles(&self) -> [Option<Pole>; 2] {
        [
            (0, self.v_domain.start()),
            (self.rows.saturating_sub(1), self.v_domain.end()),
        ]
        .map(|(row, v)| {
            self.degenerate_row(row)
                .then(|| self.control_point(0, row))
                .flatten()
                .map(|point| Pole { v, point })
        })
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

    pub(crate) fn point(&self, u: f64, v: f64) -> Point3 {
        #[cfg(test)]
        counting::POINTS.with(|count| count.set(count.get() + 1));
        let (u, v) = self.wrap(u, v);
        let degree = self.u_degree.max(self.v_degree);
        if degree < CUBIC_WIDTH {
            self.point_within::<CUBIC_WIDTH>(u, v)
        } else if degree < NARROW_WIDTH {
            self.point_within::<NARROW_WIDTH>(u, v)
        } else {
            self.point_within::<WIDE_WIDTH>(u, v)
        }
    }

    fn point_within<const WIDTH: usize>(&self, u: f64, v: f64) -> Point3 {
        let (Some(along_u), Some(along_v)) = (
            basis_values::<WIDTH>(&self.u_knots, self.u_degree, self.columns, u),
            basis_values::<WIDTH>(&self.v_knots, self.v_degree, self.rows, v),
        ) else {
            return Point3::ZERO;
        };
        let (mut sum, mut weight_sum) = (Vector3::ZERO, 0.0);
        for (j, v_value) in along_v.values.iter().take(self.v_degree + 1).enumerate() {
            let row = along_v.first + j;
            for (i, u_value) in along_u.values.iter().take(self.u_degree + 1).enumerate() {
                let index = row * self.columns + along_u.first + i;
                let Some(point) = self.control_points.get(index) else {
                    continue;
                };
                let factor = u_value * v_value * self.weight(index);
                sum += *point * factor;
                weight_sum += factor;
            }
        }
        if weight_sum <= 0.0 {
            return Point3::ZERO;
        }
        sum / weight_sum
    }

    pub(crate) fn with_knots(&self, u_parameters: &[f64], v_parameters: &[f64]) -> Option<Self> {
        let mut grid = self.homogeneous_grid();
        let u_knots = insert_into_lines(
            &mut grid,
            &self.u_knots,
            self.u_degree,
            self.u_domain,
            u_parameters,
        )?;
        let mut columns: Vec<Vec<[f64; 4]>> = transpose(&grid);
        let v_knots = insert_into_lines(
            &mut columns,
            &self.v_knots,
            self.v_degree,
            self.v_domain,
            v_parameters,
        )?;
        self.rebuilt(
            self.u_degree,
            self.v_degree,
            u_knots,
            v_knots,
            &transpose(&columns),
        )
    }

    pub fn restricted(&self, u_range: Interval, v_range: Interval) -> Option<Self> {
        let grid = self.homogeneous_grid();
        let (u_knots, grid) = if self.u_closed {
            (self.u_knots.to_vec(), grid)
        } else {
            restrict_lines(grid, &self.u_knots, self.u_degree, self.u_domain, u_range)?
        };
        let columns = transpose(&grid);
        let (v_knots, columns) = if self.v_closed {
            (self.v_knots.to_vec(), columns)
        } else {
            restrict_lines(
                columns,
                &self.v_knots,
                self.v_degree,
                self.v_domain,
                v_range,
            )?
        };
        self.rebuilt(
            self.u_degree,
            self.v_degree,
            u_knots,
            v_knots,
            &transpose(&columns),
        )
    }

    pub fn extended(&self, fraction: f64, closed_within: f64) -> Option<Self> {
        let sides = |closed: bool, poles: [bool; 2]| poles.map(|pole| !closed && !pole);
        let u_sides = sides(
            self.u_closed || self.boundaries_within(true, closed_within),
            [
                self.degenerate_column(0),
                self.degenerate_column(self.columns.checked_sub(1)?),
            ],
        );
        let v_sides = sides(
            self.v_closed || self.boundaries_within(false, closed_within),
            [
                self.degenerate_row(0),
                self.degenerate_row(self.rows.checked_sub(1)?),
            ],
        );
        let (u_knots, grid) = extend_lines(
            self.homogeneous_grid(),
            &self.u_knots,
            self.u_degree,
            self.u_domain,
            u_sides.map(|side| if side { fraction } else { 0.0 }),
        )?;
        let (v_knots, columns) = extend_lines(
            transpose(&grid),
            &self.v_knots,
            self.v_degree,
            self.v_domain,
            v_sides.map(|side| if side { fraction } else { 0.0 }),
        )?;
        self.rebuilt(
            self.u_degree,
            self.v_degree,
            u_knots,
            v_knots,
            &transpose(&columns),
        )
    }

    fn boundaries_within(&self, along_u: bool, distance: f64) -> bool {
        let (across, along) = if along_u {
            (self.u_domain, self.v_domain)
        } else {
            (self.v_domain, self.u_domain)
        };
        (0..=BOUNDARY_SAMPLES).all(|index| {
            let at = along.at(index as f64 / BOUNDARY_SAMPLES as f64);
            let [first, last] = [across.start(), across.end()].map(|side| {
                if along_u {
                    self.point(side, at)
                } else {
                    self.point(at, side)
                }
            });
            first.distance(last) <= distance
        })
    }

    pub(crate) fn cubic_along_u(&self) -> Option<Self> {
        if self.u_degree != 1 {
            return Some(self.clone());
        }
        let lines: Option<Vec<Vec<[f64; 4]>>> = self
            .homogeneous_grid()
            .iter()
            .map(|line| cubic_line(line))
            .collect();
        self.rebuilt(
            3,
            self.v_degree,
            cubic_knots(&self.u_knots),
            self.v_knots.to_vec(),
            &lines?,
        )
    }

    pub fn side(&self, side: SurfaceSide) -> Option<BSpline<Point3>> {
        let indices = self.side_indices(side);
        let points: Vec<Point3> = indices
            .iter()
            .filter_map(|index| self.control_points.get(*index).copied())
            .collect();
        let (degree, knots) = match side {
            SurfaceSide::VStart | SurfaceSide::VEnd => (self.u_degree, self.u_knots.to_vec()),
            SurfaceSide::UStart | SurfaceSide::UEnd => (self.v_degree, self.v_knots.to_vec()),
        };
        let curve = match self.weights {
            Some(_) => BSpline::rational(
                degree,
                knots,
                points,
                indices.iter().map(|index| self.weight(*index)).collect(),
            ),
            None => BSpline::new(degree, knots, points),
        };
        curve.ok()
    }

    pub(crate) fn with_side(&self, side: SurfaceSide, curve: &BSpline<Point3>) -> Option<Self> {
        let indices = self.side_indices(side);
        let knots = match side {
            SurfaceSide::VStart | SurfaceSide::VEnd => &self.u_knots,
            SurfaceSide::UStart | SurfaceSide::UEnd => &self.v_knots,
        };
        if curve.knots() != &knots[..] || curve.control_points().len() != indices.len() {
            return None;
        }
        let mut points = self.control_points.to_vec();
        let mut weights: Option<Vec<f64>> = self.weights.as_ref().map(|weights| weights.to_vec());
        for (offset, index) in indices.iter().enumerate() {
            *points.get_mut(*index)? = *curve.control_points().get(offset)?;
            if let Some(weights) = weights.as_mut() {
                *weights.get_mut(*index)? = curve
                    .weights()
                    .and_then(|curve_weights| curve_weights.get(offset))
                    .copied()
                    .unwrap_or(1.0);
            }
        }
        Self::new(
            self.u_degree,
            self.v_degree,
            self.u_knots.to_vec(),
            self.v_knots.to_vec(),
            self.columns,
            points,
            weights,
        )
        .ok()
    }

    fn side_indices(&self, side: SurfaceSide) -> Vec<usize> {
        let last_row = self.rows.saturating_sub(1);
        let last_column = self.columns.saturating_sub(1);
        match side {
            SurfaceSide::VStart => (0..self.columns).collect(),
            SurfaceSide::VEnd => (0..self.columns)
                .map(|column| last_row * self.columns + column)
                .collect(),
            SurfaceSide::UStart => (0..self.rows).map(|row| row * self.columns).collect(),
            SurfaceSide::UEnd => (0..self.rows)
                .map(|row| row * self.columns + last_column)
                .collect(),
        }
    }

    fn homogeneous_grid(&self) -> Vec<Vec<[f64; 4]>> {
        (0..self.rows)
            .map(|row| {
                (0..self.columns)
                    .map(|column| self.homogeneous(row * self.columns + column))
                    .collect()
            })
            .collect()
    }

    fn homogeneous(&self, index: usize) -> [f64; 4] {
        let weight = self.weight(index);
        let point = self
            .control_points
            .get(index)
            .copied()
            .unwrap_or(Point3::ZERO)
            * weight;
        [point.x, point.y, point.z, weight]
    }

    fn rebuilt(
        &self,
        u_degree: usize,
        v_degree: usize,
        u_knots: Vec<f64>,
        v_knots: Vec<f64>,
        grid: &[Vec<[f64; 4]>],
    ) -> Option<Self> {
        let width = grid.first().map_or(0, Vec::len);
        let homogeneous: Vec<[f64; 4]> = grid.iter().flatten().copied().collect();
        let points = homogeneous
            .iter()
            .map(|[x, y, z, weight]| Point3::new(*x, *y, *z) / *weight)
            .collect();
        let weights = self.weights.as_ref().map(|_| {
            homogeneous
                .iter()
                .map(|[_, _, _, weight]| *weight)
                .collect()
        });
        Self::new(u_degree, v_degree, u_knots, v_knots, width, points, weights).ok()
    }

    pub(crate) fn evaluate(&self, u: f64, v: f64) -> SurfaceDerivatives {
        #[cfg(test)]
        counting::DERIVATIVES.with(|count| count.set(count.get() + 1));
        let (u, v) = self.wrap(u, v);
        self.derivatives_at(u, v)
    }

    pub fn extended_evaluate(&self, u: f64, v: f64) -> SurfaceDerivatives {
        let (wrapped_u, wrapped_v) = self.wrap(u, v);
        let u = if self.u_closed { wrapped_u } else { u };
        let v = if self.v_closed { wrapped_v } else { v };
        self.derivatives_at(u, v)
    }

    fn derivatives_at(&self, u: f64, v: f64) -> SurfaceDerivatives {
        let degree = self.u_degree.max(self.v_degree);
        if degree < CUBIC_WIDTH {
            self.derivatives_within::<CUBIC_WIDTH>(u, v)
        } else if degree < NARROW_WIDTH {
            self.derivatives_within::<NARROW_WIDTH>(u, v)
        } else {
            self.derivatives_within::<WIDE_WIDTH>(u, v)
        }
    }

    fn derivatives_within<const WIDTH: usize>(&self, u: f64, v: f64) -> SurfaceDerivatives {
        let (Some(along_u), Some(along_v)) = (
            basis::<WIDTH>(&self.u_knots, self.u_degree, self.columns, u),
            basis::<WIDTH>(&self.v_knots, self.v_degree, self.rows, v),
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

    pub(crate) fn project_seed(&self, point: Point3) -> Vec<Point2> {
        self.grid.nearest(point, PROJECTION_SEEDS)
    }

    pub(crate) fn span_seeds(&self, point: Point3) -> Vec<Point2> {
        let mut holding: Vec<&Span> = self
            .spans
            .tree
            .overlapping(&Aabb::from_point(point), LINEAR_RESOLUTION)
            .into_iter()
            .filter_map(|index| self.spans.spans.get(index))
            .collect();
        holding.sort_by(|a, b| {
            a.hull
                .center()
                .distance_squared(point)
                .total_cmp(&b.hull.center().distance_squared(point))
        });
        holding
            .into_iter()
            .take(MAX_SPAN_SEARCHES)
            .filter_map(|span| self.nearest_in_span(span, point))
            .collect()
    }

    fn nearest_in_span(&self, span: &Span, point: Point3) -> Option<Point2> {
        let fraction = |index: usize| (index as f64 + 0.5) / SPAN_SAMPLES as f64;
        (0..SPAN_SAMPLES)
            .flat_map(|row| {
                (0..SPAN_SAMPLES).map(move |column| {
                    Point2::new(span.u.at(fraction(column)), span.v.at(fraction(row)))
                })
            })
            .map(|uv| (self.evaluate(uv.x, uv.y).point.distance_squared(point), uv))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, uv)| uv)
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
        let hull = || {
            let points = (v_first..=v_last).flat_map(|row| {
                (u_first..=u_last).filter_map(move |column| self.control_point(column, row))
            });
            Aabb::from_points(points).unwrap_or_else(|| Aabb::from_point(Point3::ZERO))
        };
        let whole_u = wraps(u_range, self.u_domain, self.u_closed);
        let whole_v = wraps(v_range, self.v_domain, self.v_closed);
        if whole_u || whole_v {
            return hull();
        }
        let restricted = self.restricted_net(
            (
                u_first,
                u_last,
                self.u_domain.clamp(u_range.start()),
                self.u_domain.clamp(u_range.end()),
            ),
            (
                v_first,
                v_last,
                self.v_domain.clamp(v_range.start()),
                self.v_domain.clamp(v_range.end()),
            ),
        );
        restricted
            .and_then(|net| {
                Aabb::from_points(
                    net.into_iter()
                        .filter(|[_, _, _, w]| *w > 0.0)
                        .map(|[x, y, z, w]| Point3::new(x / w, y / w, z / w)),
                )
            })
            .unwrap_or_else(hull)
    }

    fn restricted_net(
        &self,
        (u_first, u_last, u_low, u_high): (usize, usize, f64, f64),
        (v_first, v_last, v_low, v_high): (usize, usize, f64, f64),
    ) -> Option<Vec<[f64; 4]>> {
        let homogeneous = |column: usize, row: usize| {
            let index = row * self.columns + column;
            let point = self.control_points.get(index)?;
            let weight = self.weight(index);
            Some([point.x * weight, point.y * weight, point.z * weight, weight])
        };
        let u_knots = self.u_knots.get(u_first..=u_last + self.u_degree + 1)?;
        let v_knots = self.v_knots.get(v_first..=v_last + self.v_degree + 1)?;
        let mut rows = Vec::with_capacity(v_last - v_first + 1);
        for row in v_first..=v_last {
            let points = (u_first..=u_last)
                .map(|column| homogeneous(column, row))
                .collect::<Option<Vec<_>>>()?;
            rows.push(restrict(u_knots, self.u_degree, points, u_low, u_high)?);
        }
        let width = rows.first()?.len();
        let mut net = Vec::with_capacity(width * rows.len());
        for column in 0..width {
            let points = rows
                .iter()
                .map(|row| row.get(column).copied())
                .collect::<Option<Vec<_>>>()?;
            net.extend(restrict(v_knots, self.v_degree, points, v_low, v_high)?);
        }
        Some(net)
    }

    fn span_index(&self) -> SpanIndex {
        let spans_of = |knots: &[f64], degree: usize, count: usize, domain: Interval| {
            (degree..count)
                .filter_map(|index| {
                    let interval = Interval::new(*knots.get(index)?, *knots.get(index + 1)?)?;
                    let inside = interval.length() > 0.0
                        && interval.start() >= domain.start()
                        && interval.end() <= domain.end();
                    inside.then_some((index - degree..=index, interval))
                })
                .collect::<Vec<_>>()
        };
        let columns = spans_of(&self.u_knots, self.u_degree, self.columns, self.u_domain);
        let rows = spans_of(&self.v_knots, self.v_degree, self.rows, self.v_domain);
        let sampled_by_grid = |spans: usize| spans * SAMPLES_PER_SPAN <= MAX_GRID_SAMPLES;
        if sampled_by_grid(columns.len()) && sampled_by_grid(rows.len()) {
            return SpanIndex::default();
        }
        let spans: Vec<Span> = rows
            .iter()
            .flat_map(|(row_range, v)| {
                columns.iter().filter_map(move |(column_range, u)| {
                    let hull = Aabb::from_points(row_range.clone().flat_map(|row| {
                        column_range
                            .clone()
                            .filter_map(move |column| self.control_point(column, row))
                    }))?;
                    Some(Span { hull, u: *u, v: *v })
                })
            })
            .collect();
        SpanIndex {
            tree: BoxTree::new(spans.iter().map(|span| span.hull)),
            spans,
        }
    }

    fn sample_grid(&self) -> SampleGrid {
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
        SampleGrid::new(columns + 1, grid)
    }
}

fn column_of<const WIDTH: usize>(basis: &Basis<WIDTH>, index: usize) -> [f64; 3] {
    [0, 1, 2].map(|order| {
        basis
            .values
            .get(order)
            .and_then(|row| row.get(index))
            .copied()
            .unwrap_or(0.0)
    })
}

fn restrict(
    knots: &[f64],
    degree: usize,
    points: Vec<[f64; 4]>,
    low: f64,
    high: f64,
) -> Option<Vec<[f64; 4]>> {
    if low.is_nan() || high.is_nan() || low >= high || degree == 0 {
        return None;
    }
    let mut knots = knots.to_vec();
    let mut points = points;
    for parameter in [low, high] {
        let present = knots.iter().filter(|knot| **knot == parameter).count();
        for _ in present..degree {
            insert_knot(&mut knots, &mut points, degree, parameter)?;
        }
    }
    let count = points.len();
    let first = knots
        .partition_point(|knot| *knot <= low)
        .checked_sub(1)?
        .min(count.checked_sub(1)?);
    let last = knots
        .partition_point(|knot| *knot < high)
        .checked_sub(1)?
        .min(count.checked_sub(1)?);
    points
        .get(first.checked_sub(degree)?..=last)
        .map(<[_]>::to_vec)
}

fn insert_knot(
    knots: &mut Vec<f64>,
    points: &mut Vec<[f64; 4]>,
    degree: usize,
    parameter: f64,
) -> Option<()> {
    let count = points.len();
    let span = knots
        .partition_point(|knot| *knot <= parameter)
        .checked_sub(1)?
        .clamp(degree, count.checked_sub(1)?);
    let mut inserted = Vec::with_capacity(count + 1);
    for index in 0..=count {
        let point = if index + degree <= span {
            *points.get(index)?
        } else if index > span {
            *points.get(index - 1)?
        } else {
            let (start, end) = (*knots.get(index)?, *knots.get(index + degree)?);
            let along = if end > start {
                (parameter - start) / (end - start)
            } else {
                0.0
            };
            let (before, after) = (points.get(index - 1)?, points.get(index)?);
            [0, 1, 2, 3].map(|axis| {
                let (a, b) = (before.get(axis).copied(), after.get(axis).copied());
                a.zip(b).map_or(f64::NAN, |(a, b)| a + (b - a) * along)
            })
        };
        inserted.push(point);
    }
    knots.insert(span + 1, parameter);
    *points = inserted;
    Some(())
}

fn insert_into_lines(
    lines: &mut [Vec<[f64; 4]>],
    knots: &[f64],
    degree: usize,
    domain: Interval,
    parameters: &[f64],
) -> Option<Vec<f64>> {
    let mut knots = knots.to_vec();
    for parameter in parameters {
        let inside = domain.start() < *parameter && *parameter < domain.end();
        let present = knots.iter().filter(|knot| **knot == *parameter).count();
        if !inside || present >= degree {
            continue;
        }
        let mut inserted = None;
        for line in lines.iter_mut() {
            let mut line_knots = knots.clone();
            let mut points = std::mem::take(line);
            insert_knot(&mut line_knots, &mut points, degree, *parameter)?;
            *line = points;
            inserted = Some(line_knots);
        }
        knots = inserted?;
    }
    Some(knots)
}

type Lines = Vec<Vec<[f64; 4]>>;

const EXTENSION_ATTEMPTS: usize = 4;
const BOUNDARY_SAMPLES: usize = 16;
const EXTENSION_SHRINK: f64 = 0.25;

fn restrict_lines(
    mut lines: Lines,
    knots: &[f64],
    degree: usize,
    domain: Interval,
    range: Interval,
) -> Option<(Vec<f64>, Lines)> {
    let (low, high) = (domain.clamp(range.start()), domain.clamp(range.end()));
    if low >= high {
        return None;
    }
    let mut parameters = vec![low; degree];
    parameters.extend(std::iter::repeat_n(high, degree));
    let knots = insert_into_lines(&mut lines, knots, degree, domain, &parameters)?;
    let occurrences = |value: f64| knots.iter().filter(|knot| **knot == value).count();
    let first_low = knots.iter().position(|knot| *knot == low)?;
    let first_high = knots.iter().position(|knot| *knot == high)?;
    let first = (first_low + occurrences(low)).checked_sub(degree + 1)?;
    let last = first_high.checked_sub(1)?;
    let interior = knots.iter().filter(|knot| **knot > low && **knot < high);
    let kept_knots: Vec<f64> = std::iter::repeat_n(low, degree + 1)
        .chain(interior.copied())
        .chain(std::iter::repeat_n(high, degree + 1))
        .collect();
    let kept: Option<Lines> = lines
        .iter()
        .map(|line| line.get(first..=last).map(<[_]>::to_vec))
        .collect();
    Some((kept_knots, kept?))
}

fn extend_lines(
    lines: Lines,
    knots: &[f64],
    degree: usize,
    domain: Interval,
    fractions: [f64; 2],
) -> Option<(Vec<f64>, Lines)> {
    let (start, end) = (domain.start(), domain.end());
    let occurrences = |value: f64| knots.iter().filter(|knot| **knot == value).count();
    if fractions == [0.0; 2] {
        return Some((knots.to_vec(), lines));
    }
    if occurrences(start) <= degree || occurrences(end) <= degree {
        return None;
    }
    let after_start = knots.iter().copied().find(|knot| *knot > start)?;
    let before_end = knots.iter().copied().rev().find(|knot| *knot < end)?;
    let mut parameters = Vec::new();
    for (fraction, inner) in fractions.into_iter().zip([after_start, before_end]) {
        if fraction > 0.0 && inner > start && inner < end {
            parameters.extend(std::iter::repeat_n(
                inner,
                degree.saturating_sub(occurrences(inner)),
            ));
        }
    }
    let mut lines = lines;
    let mut knots = insert_into_lines(&mut lines, knots, degree, domain, &parameters)?;
    let mut reaches = [0.0; 2];
    for (side, fraction) in fractions.into_iter().enumerate() {
        let span = if side == 0 {
            after_start - start
        } else if before_end > start {
            end - before_end
        } else {
            end - start + reaches[0]
        };
        let attempts = (0..EXTENSION_ATTEMPTS).map(|attempt| {
            fraction * span * EXTENSION_SHRINK.powi(i32::try_from(attempt).unwrap_or(i32::MAX))
        });
        for reach in attempts.filter(|reach| *reach > 0.0) {
            let extended: Option<Lines> = lines
                .iter()
                .map(|line| extend_line(line, degree, side == 0, reach / span))
                .collect();
            if let (Some(extended), Some(slot)) = (extended, reaches.get_mut(side)) {
                lines = extended;
                *slot = reach;
                break;
            }
        }
    }
    let count = knots.len();
    let [start_by, end_by] = reaches;
    for (index, knot) in knots.iter_mut().enumerate() {
        if index <= degree {
            *knot -= start_by;
        } else if index + degree + 1 >= count {
            *knot += end_by;
        }
    }
    Some((knots, lines))
}

fn extend_line(
    line: &[[f64; 4]],
    degree: usize,
    at_start: bool,
    beyond: f64,
) -> Option<Vec<[f64; 4]>> {
    let mut extended = line.to_vec();
    if at_start {
        let first = line.get(..=degree)?;
        for (slot, point) in extended.iter_mut().zip(bezier_after(first, -beyond)) {
            *slot = point;
        }
    } else {
        let first = line.len().checked_sub(degree + 1)?;
        let last = line.get(first..)?;
        for (slot, point) in extended
            .iter_mut()
            .skip(first)
            .zip(bezier_before(last, 1.0 + beyond))
        {
            *slot = point;
        }
    }
    extended
        .iter()
        .all(|[x, y, z, weight]| weight.is_finite() && *weight > 0.0 && (x + y + z).is_finite())
        .then_some(extended)
}

fn casteljau_levels(points: &[[f64; 4]], along: f64) -> Vec<Vec<[f64; 4]>> {
    let mut levels = vec![points.to_vec()];
    while let Some(level) = levels.last().filter(|level| level.len() > 1) {
        let next = level
            .windows(2)
            .map(|pair| match pair {
                [a, b] => [0, 1, 2, 3].map(|axis| {
                    let (a, b) = (a.get(axis).copied(), b.get(axis).copied());
                    a.zip(b).map_or(f64::NAN, |(a, b)| a + (b - a) * along)
                }),
                _ => [f64::NAN; 4],
            })
            .collect();
        levels.push(next);
    }
    levels
}

fn bezier_before(points: &[[f64; 4]], along: f64) -> Vec<[f64; 4]> {
    casteljau_levels(points, along)
        .iter()
        .filter_map(|level| level.first().copied())
        .collect()
}

fn bezier_after(points: &[[f64; 4]], along: f64) -> Vec<[f64; 4]> {
    casteljau_levels(points, along)
        .iter()
        .rev()
        .filter_map(|level| level.last().copied())
        .collect()
}

fn cubic_line(line: &[[f64; 4]]) -> Option<Vec<[f64; 4]>> {
    let mut points = vec![*line.first()?];
    for pair in line.windows(2) {
        let [start, end] = pair else {
            return None;
        };
        let mix = |along: f64| {
            let mut mixed = *start;
            for (slot, target) in mixed.iter_mut().zip(end) {
                *slot += (target - *slot) * along;
            }
            mixed
        };
        points.extend([mix(1.0 / 3.0), mix(2.0 / 3.0), *end]);
    }
    Some(points)
}

fn cubic_knots(knots: &[f64]) -> Vec<f64> {
    let mut distinct = knots.to_vec();
    distinct.dedup();
    let last = distinct.len().saturating_sub(1);
    distinct
        .iter()
        .enumerate()
        .flat_map(|(index, knot)| {
            let repeat = if index == 0 || index == last { 4 } else { 3 };
            std::iter::repeat_n(*knot, repeat)
        })
        .collect()
}

fn transpose(lines: &[Vec<[f64; 4]>]) -> Vec<Vec<[f64; 4]>> {
    let width = lines.first().map_or(0, Vec::len);
    (0..width)
        .map(|index| {
            lines
                .iter()
                .filter_map(|line| line.get(index).copied())
                .collect()
        })
        .collect()
}

fn span_of(knots: &[f64], degree: usize, count: usize, parameter: f64) -> usize {
    knots
        .partition_point(|knot| *knot <= parameter)
        .saturating_sub(1)
        .clamp(degree, count - 1)
}

fn basis_table<const WIDTH: usize>(
    knots: &[f64],
    degree: usize,
    count: usize,
    parameter: f64,
) -> Option<BasisTable<WIDTH>> {
    let span = span_of(knots, degree, count, parameter);
    let knot = |index: usize| knots.get(index).copied().unwrap_or(0.0);
    let mut table = [[0.0f64; WIDTH]; WIDTH];
    let mut left = [0.0f64; WIDTH];
    let mut right = [0.0f64; WIDTH];
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
    Some(BasisTable { span, table })
}

fn basis_values<const WIDTH: usize>(
    knots: &[f64],
    degree: usize,
    count: usize,
    parameter: f64,
) -> Option<BasisValues<WIDTH>> {
    let BasisTable { span, table } = basis_table::<WIDTH>(knots, degree, count, parameter)?;
    let mut values = [0.0f64; WIDTH];
    for (j, value) in values.iter_mut().enumerate().take(degree + 1) {
        *value = *table.get(j)?.get(degree)?;
    }
    Some(BasisValues {
        first: span - degree,
        values,
    })
}

fn basis<const WIDTH: usize>(
    knots: &[f64],
    degree: usize,
    count: usize,
    parameter: f64,
) -> Option<Basis<WIDTH>> {
    let BasisTable { span, table } = basis_table::<WIDTH>(knots, degree, count, parameter)?;
    let mut values = [[0.0f64; WIDTH]; 3];
    for j in 0..=degree {
        *values.get_mut(0)?.get_mut(j)? = *table.get(j)?.get(degree)?;
    }
    let mut a = [[0.0f64; WIDTH]; 2];
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
    use std::{
        cell::Cell,
        f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2},
    };

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

    fn bumpy(rational: bool) -> BSplineSurface {
        let size = 14;
        let mut points = Vec::new();
        for row in 0..size {
            for column in 0..size {
                let (x, y) = (column as f64 * 4.0, row as f64 * 4.0);
                points.push(Point3::new(x, y, 3.0 * (x * 0.21).sin() * (y * 0.17).cos()));
            }
        }
        let knots: Vec<f64> = std::iter::repeat_n(0.0, 4)
            .chain((1..size - 3).map(|index| index as f64 / (size - 3) as f64))
            .chain(std::iter::repeat_n(1.0, 4))
            .collect();
        let weights = rational.then(|| {
            (0..size * size)
                .map(|index| 1.0 + (index % 5) as f64 * 0.2)
                .collect()
        });
        BSplineSurface::new(3, 3, knots.clone(), knots, size, points, weights).unwrap()
    }

    fn lifted_points(surface: &Surface, seed: u64, count: usize) -> Vec<(Point3, Point2)> {
        let mut random = crate::test_support::Random::new(seed);
        (0..count)
            .map(|_| {
                let uv = Point2::new(
                    surface
                        .u_domain()
                        .clipped(1.0)
                        .at(random.between(0.05, 0.95)),
                    surface
                        .v_domain()
                        .clipped(1.0)
                        .at(random.between(0.05, 0.95)),
                );
                let at = surface.evaluate(uv.x, uv.y);
                let lift = random.between(-0.5, 0.5);
                (at.point + at.normal().unwrap() * lift, uv)
            })
            .collect()
    }

    #[test]
    fn an_extended_surface_keeps_its_points_and_continues_its_boundary_spans() {
        for (index, surface) in [wavy(), bumpy(true), quarter_cylinder()]
            .into_iter()
            .enumerate()
        {
            let extended = surface
                .extended(0.1, 0.0)
                .unwrap_or_else(|| panic!("surface {index}"));
            let (u, v) = (surface.u_domain(), surface.v_domain());
            let (wide_u, wide_v) = (extended.u_domain(), extended.v_domain());

            assert!(wide_u.start() < u.start() && wide_u.end() > u.end());
            assert!(wide_v.start() < v.start() && wide_v.end() > v.end());
            for row in 0..=12 {
                for column in 0..=12 {
                    let (s, t) = (column as f64 / 12.0, row as f64 / 12.0);
                    let inside = surface.evaluate(u.at(s), v.at(t)).point;
                    let (outer_u, outer_v) = (wide_u.at(s), wide_v.at(t));
                    let continued = surface.extended_evaluate(outer_u, outer_v).point;
                    let gap = extended.evaluate(u.at(s), v.at(t)).point.distance(inside);
                    assert!(
                        gap < 1e-9,
                        "{index} {s} {t} {gap} {u:?} {v:?} {wide_u:?} {wide_v:?}"
                    );
                    assert!(
                        extended
                            .evaluate(outer_u, outer_v)
                            .point
                            .distance(continued)
                            < 1e-9,
                        "{s} {t}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_closed_or_pinched_side_is_not_extended() {
        let size = 6;
        let mut points = Vec::new();
        for row in 0..size {
            for column in 0..size {
                let angle = column as f64 / (size - 1) as f64 * std::f64::consts::PI;
                let radius = row as f64;
                points.push(Point3::new(
                    radius * angle.cos(),
                    radius * angle.sin(),
                    radius,
                ));
            }
        }
        let knots: Vec<f64> = std::iter::repeat_n(0.0, 4)
            .chain([0.4, 0.6])
            .chain(std::iter::repeat_n(1.0, 4))
            .collect();
        let pinched = BSplineSurface::new(3, 3, knots.clone(), knots, size, points, None).unwrap();

        let extended = pinched.extended(0.1, 0.0).unwrap();

        assert!(pinched.degenerate_row(0));
        assert_eq!(extended.v_domain().start(), 0.0);
        assert!((extended.v_domain().end() - 1.04).abs() < 1e-12);
        assert!((extended.u_domain().length() - 1.08).abs() < 1e-12);
    }

    #[test]
    fn the_point_alone_is_the_point_of_the_full_evaluation() {
        let mut random = crate::test_support::Random::new(21);
        for spline in [wavy(), quarter_cylinder(), bumpy(false), bumpy(true)] {
            let (u, v) = (spline.u_domain, spline.v_domain);
            for _ in 0..200 {
                let (s, t) = (random.between(-0.2, 1.2), random.between(-0.2, 1.2));
                let (s, t) = (u.at(s), v.at(t));
                assert_eq!(
                    spline.point(s, t),
                    spline.evaluate(s, t).point,
                    "at ({s}, {t})"
                );
            }
        }
    }

    #[test]
    fn projection_lands_on_the_foot_to_rounding() {
        for surface in [wavy(), bumpy(false), bumpy(true)].map(Surface::BSpline) {
            for (point, uv) in lifted_points(&surface, 5, 400) {
                for hint in [None, Some(uv + Point2::new(0.01, -0.01))] {
                    let found = surface.project(point, hint);
                    let at = surface.evaluate(found.x, found.y);
                    let offset = at.point - point;
                    let tangential = offset
                        .dot(at.du.normalize())
                        .abs()
                        .max(offset.dot(at.dv.normalize()).abs());
                    assert!(tangential < 1e-12, "{point} with {hint:?}: {tangential}");
                    assert!((found - uv).length() < 1e-9, "{point}: {found} vs {uv}");
                }
            }
        }
    }

    #[test]
    fn projection_evaluates_the_surface_sparingly() {
        for surface in [wavy(), quarter_cylinder(), bumpy(true)].map(Surface::BSpline) {
            let points = lifted_points(&surface, 11, 500);
            for hinted in [false, true] {
                counting::DERIVATIVES.with(|count| count.set(0));
                counting::POINTS.with(|count| count.set(0));
                for (point, uv) in &points {
                    surface.project(*point, hinted.then_some(*uv + Point2::splat(0.01)));
                }
                let per_projection = |count: usize| count as f64 / points.len() as f64;
                let derivatives = per_projection(counting::DERIVATIVES.with(Cell::get));
                let evaluated_points = per_projection(counting::POINTS.with(Cell::get));
                assert!(derivatives < 20.0, "{derivatives} derivative evaluations");
                assert!(
                    evaluated_points < 26.0,
                    "{evaluated_points} point evaluations"
                );
            }
        }
    }

    #[test]
    fn a_small_box_is_bounded_tightly_and_still_contains_its_surface() {
        for surface in [wavy(), quarter_cylinder()] {
            let (u, v) = (surface.u_domain, surface.v_domain);
            let whole = surface.bounds(u, v);
            for (low, high) in [(0.1, 0.2), (0.45, 0.55), (0.8, 0.95)] {
                let (u_box, v_box) = (
                    Interval::new(u.at(low), u.at(high)).unwrap(),
                    Interval::new(v.at(low), v.at(high)).unwrap(),
                );
                let bounds = surface.bounds(u_box, v_box);
                assert!(bounds.diagonal() < 0.5 * whole.diagonal(), "{bounds:?}");
                for row in 0..=8 {
                    for column in 0..=8 {
                        let point = surface
                            .evaluate(u_box.at(column as f64 / 8.0), v_box.at(row as f64 / 8.0))
                            .point;
                        let margin = bounds.expanded(1e-9);
                        let inside =
                            point.cmpge(margin.min()).all() && point.cmple(margin.max()).all();
                        assert!(inside, "{point} is outside {bounds:?}");
                    }
                }
            }
        }
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
    fn the_nearest_grid_samples_match_a_full_scan() {
        let surface = wavy();
        let grid = &surface.grid;
        assert!(grid.blocks.len() > 1);
        let mut random = crate::test_support::Random::new(7);
        for _ in 0..200 {
            let point = random.point(15.0) + Vector3::new(5.0, 6.0, 0.0);
            let mut scanned: Vec<(f64, usize)> = grid
                .samples
                .iter()
                .enumerate()
                .map(|(index, (_, sample))| (sample.distance_squared(point), index))
                .collect();
            scanned.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let expected: Vec<Point2> = scanned
                .iter()
                .take(PROJECTION_SEEDS)
                .map(|(_, index)| grid.samples[*index].0)
                .collect();
            assert_eq!(grid.nearest(point, PROJECTION_SEEDS), expected);
        }
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
    fn projection_finds_points_on_a_dense_rough_net() {
        let size = 60;
        let mut random = crate::test_support::Random::new(11);
        let points = (0..size * size)
            .map(|index| {
                Point3::new(
                    (index % size) as f64,
                    (index / size) as f64,
                    random.between(-20.0, 20.0),
                )
            })
            .collect();
        let knots = |count: usize| {
            let inner = count - 3;
            let mut knots = vec![0.0; 4];
            knots.extend((1..inner).map(|knot| knot as f64 / inner as f64));
            knots.extend([1.0; 4]);
            knots
        };
        let rough =
            BSplineSurface::new(3, 3, knots(size), knots(size), size, points, None).unwrap();
        let surface = Surface::BSpline(rough);

        let mut missed = 0;
        for _ in 0..2000 {
            let (u, v) = (random.unit(), random.unit());
            let point = surface.point_at(Point2::new(u, v));
            let found = surface.point_at(surface.project(point, None));
            if found.distance(point) > LINEAR_RESOLUTION {
                missed += 1;
            }
        }

        assert_eq!(missed, 0);
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
