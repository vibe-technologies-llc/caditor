use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::TAU,
};

use caditor_geometry::{Aabb2, Point2, Point3, Vector3};

use super::{
    HiddenLineError,
    contour::{Contour, chord_distance, facing},
    flatten::polyline_curve,
};
use crate::{
    curve::Curve, interrupt, interval::Domain, surface::Surface, tally,
    tolerance::SamplingTolerance,
};

const MIN_CELLS: usize = 24;
const MIN_CELLS_AROUND: usize = 8;
const MAX_CELLS: usize = 128;
const CELLS_PER_SPAN: usize = 4;
const CELLS_PER_TURN: f64 = 64.0;
const CROSSING_BISECTIONS: usize = 40;
const FLAT_FACING: f64 = 1e-10;
const NEWTON_STEPS: usize = 8;
const ON_CONTOUR: f64 = 1e-11;
const NEARLY_ON_CONTOUR: f64 = 1e-8;
const REFINE_DEPTH: usize = 12;
const MAX_CHAIN_POINTS: usize = 1 << 14;
const BOX_MARGIN_SHARE: f64 = 1e-2;
const CLOSING_SLACK: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Side {
    AlongU,
    AlongV,
}

type Key = (Side, usize, usize);

struct Grid<'a> {
    surface: &'a Surface,
    toward: Vector3,
    us: Vec<f64>,
    vs: Vec<f64>,
    wraps_u: bool,
    wraps_v: bool,
    positive: Vec<bool>,
}

fn axis_values(low: f64, high: f64, cells: usize) -> Vec<f64> {
    (0..=cells)
        .map(|index| low + (high - low) * index as f64 / cells as f64)
        .collect()
}

fn axis_range(low: f64, high: f64, period: Option<f64>, domain: Domain) -> (f64, f64, bool) {
    match period {
        Some(period) if high - low + CLOSING_SLACK >= period => (low, low + period, true),
        _ => {
            let margin = (high - low) * BOX_MARGIN_SHARE;
            (
                domain.clamp(low - margin),
                domain.clamp(high + margin),
                false,
            )
        }
    }
}

fn spans_within(knots: &[f64], low: f64, high: f64) -> usize {
    let mut distinct: Vec<f64> = knots
        .iter()
        .copied()
        .filter(|knot| *knot > low && *knot < high)
        .collect();
    distinct.dedup();
    distinct.len() + 1
}

fn cells_for(spans: usize) -> usize {
    (spans * CELLS_PER_SPAN).clamp(MIN_CELLS, MAX_CELLS)
}

fn cell_counts(surface: &Surface, low: Point2, high: Point2) -> (usize, usize) {
    match surface {
        Surface::BSpline(spline) => (
            cells_for(spans_within(spline.u_knots(), low.x, high.x)),
            cells_for(spans_within(spline.v_knots(), low.y, high.y)),
        ),
        Surface::Revolution(revolution) => {
            let around = ((high.x - low.x) / TAU * CELLS_PER_TURN).ceil() as usize;
            let along = match revolution.profile() {
                Curve::BSpline(spline) => cells_for(spans_within(spline.knots(), low.y, high.y)),
                _ => MIN_CELLS,
            };
            (around.clamp(MIN_CELLS_AROUND, MAX_CELLS), along)
        }
        _ => (MIN_CELLS, MIN_CELLS),
    }
}

impl<'a> Grid<'a> {
    fn new(surface: &'a Surface, uv_box: Aabb2, toward: Vector3) -> Result<Self, HiddenLineError> {
        let (u_low, u_high, wraps_u) = axis_range(
            uv_box.min().x,
            uv_box.max().x,
            surface.u_period(),
            surface.u_domain(),
        );
        let (v_low, v_high, wraps_v) = axis_range(
            uv_box.min().y,
            uv_box.max().y,
            surface.v_period(),
            surface.v_domain(),
        );
        let (across, along) = cell_counts(
            surface,
            Point2::new(u_low, v_low),
            Point2::new(u_high, v_high),
        );
        let us = axis_values(u_low, u_high, across);
        let vs = axis_values(v_low, v_high, along);
        let mut positive = Vec::with_capacity(us.len() * vs.len());
        for v in &vs {
            interrupt::check()?;
            for u in &us {
                tally::add(1);
                positive.push(facing(surface, Point2::new(*u, *v), toward) >= -FLAT_FACING);
            }
        }
        Ok(Self {
            surface,
            toward,
            us,
            vs,
            wraps_u,
            wraps_v,
            positive,
        })
    }

    fn sign(&self, column: usize, row: usize) -> bool {
        self.positive
            .get(row * self.us.len() + column)
            .copied()
            .unwrap_or(true)
    }

    fn node(&self, column: usize, row: usize) -> Option<Point2> {
        Some(Point2::new(*self.us.get(column)?, *self.vs.get(row)?))
    }

    fn step(&self) -> f64 {
        let step = |values: &[f64]| match values {
            [first, second, ..] => (second - first).abs(),
            _ => 0.0,
        };
        step(&self.us).max(step(&self.vs))
    }

    fn key(&self, side: Side, column: usize, row: usize) -> Key {
        let columns = self.us.len().saturating_sub(1);
        let rows = self.vs.len().saturating_sub(1);
        let column = if self.wraps_u && column == columns {
            0
        } else {
            column
        };
        let row = if self.wraps_v && row == rows { 0 } else { row };
        (side, column, row)
    }

    fn side_ends(&self, key: Key) -> Option<(Point2, Point2)> {
        let (side, column, row) = key;
        let start = self.node(column, row)?;
        let end = match side {
            Side::AlongU => self.node(column + 1, row)?,
            Side::AlongV => self.node(column, row + 1)?,
        };
        Some((start, end))
    }

    fn positive_at(&self, uv: Point2) -> bool {
        facing(self.surface, uv, self.toward) >= -FLAT_FACING
    }

    fn crossing(&self, key: Key) -> Option<Point2> {
        let (mut low, mut high) = self.side_ends(key)?;
        let low_sign = self.positive_at(low);
        for _ in 0..CROSSING_BISECTIONS {
            let middle = low.lerp(high, 0.5);
            if self.positive_at(middle) == low_sign {
                low = middle;
            } else {
                high = middle;
            }
        }
        Some(low.lerp(high, 0.5))
    }

    fn links(&self) -> Result<BTreeMap<Key, Vec<Key>>, HiddenLineError> {
        let mut links: BTreeMap<Key, Vec<Key>> = BTreeMap::new();
        let columns = self.us.len().saturating_sub(1);
        let rows = self.vs.len().saturating_sub(1);
        for row in 0..rows {
            interrupt::check()?;
            for column in 0..columns {
                let corners = [
                    self.sign(column, row),
                    self.sign(column + 1, row),
                    self.sign(column + 1, row + 1),
                    self.sign(column, row + 1),
                ];
                for (from, to) in self.cell_pairs(column, row, corners) {
                    links.entry(from).or_default().push(to);
                    links.entry(to).or_default().push(from);
                }
            }
        }
        Ok(links)
    }

    fn cell_pairs(&self, column: usize, row: usize, corners: [bool; 4]) -> Vec<(Key, Key)> {
        let [low_left, low_right, high_right, high_left] = corners;
        let bottom = self.key(Side::AlongU, column, row);
        let right = self.key(Side::AlongV, column + 1, row);
        let top = self.key(Side::AlongU, column, row + 1);
        let left = self.key(Side::AlongV, column, row);
        let crossed = [
            (low_left != low_right, bottom),
            (low_right != high_right, right),
            (high_right != high_left, top),
            (high_left != low_left, left),
        ];
        let sides: Vec<Key> = crossed
            .iter()
            .filter(|(crosses, _)| *crosses)
            .map(|(_, key)| *key)
            .collect();
        match sides.as_slice() {
            [first, second] => vec![(*first, *second)],
            [_, _, _, _] => {
                let centre = self
                    .node(column, row)
                    .zip(self.node(column + 1, row + 1))
                    .is_some_and(|(low, high)| self.positive_at(low.lerp(high, 0.5)));
                if centre == low_left {
                    vec![(bottom, right), (top, left)]
                } else {
                    vec![(bottom, left), (right, top)]
                }
            }
            _ => Vec::new(),
        }
    }
}

fn chains(links: &BTreeMap<Key, Vec<Key>>) -> Vec<Vec<Key>> {
    let mut seen: BTreeSet<Key> = BTreeSet::new();
    let mut found = Vec::new();
    let open_ends: Vec<Key> = links
        .iter()
        .filter(|(_, neighbours)| neighbours.len() == 1)
        .map(|(key, _)| *key)
        .collect();
    let others: Vec<Key> = links.keys().copied().collect();
    for start in open_ends.into_iter().chain(others) {
        if !seen.insert(start) {
            continue;
        }
        let mut chain = vec![start];
        let mut current = start;
        while let Some(next) = links
            .get(&current)
            .into_iter()
            .flatten()
            .find(|neighbour| !seen.contains(*neighbour))
        {
            seen.insert(*next);
            chain.push(*next);
            current = *next;
        }
        let closes = chain.len() > 2
            && links
                .get(&current)
                .is_some_and(|neighbours| neighbours.contains(&start));
        if closes {
            chain.push(start);
        }
        found.push(chain);
    }
    found
}

fn nearest_turn(value: f64, near: f64, period: Option<f64>) -> f64 {
    match period {
        Some(period) => value + ((near - value) / period).round() * period,
        None => value,
    }
}

pub(super) fn on_contour(surface: &Surface, start: Point2, toward: Vector3) -> Option<Point2> {
    let mut uv = start;
    for _ in 0..NEWTON_STEPS {
        tally::add(1);
        let at = surface.evaluate(uv.x, uv.y);
        let normal = at.du.cross(at.dv);
        let size = normal.length();
        if size <= 0.0 {
            return None;
        }
        let value = normal.dot(toward);
        if value.abs() <= ON_CONTOUR * size {
            return Some(uv);
        }
        let along_u = (at.duu.cross(at.dv) + at.du.cross(at.duv)).dot(toward);
        let along_v = (at.duv.cross(at.dv) + at.du.cross(at.dvv)).dot(toward);
        let slope = along_u * along_u + along_v * along_v;
        if slope <= 0.0 {
            return None;
        }
        uv = Point2::new(
            uv.x - value * along_u / slope,
            uv.y - value * along_v / slope,
        );
        if !uv.is_finite() {
            return None;
        }
    }
    let at = surface.evaluate(uv.x, uv.y);
    let normal = at.du.cross(at.dv);
    (normal.dot(toward).abs() <= NEARLY_ON_CONTOUR * normal.length()).then_some(uv)
}

fn refined(
    surface: &Surface,
    seeds: &[Point2],
    toward: Vector3,
    tolerance: &SamplingTolerance,
) -> Result<Vec<Point2>, HiddenLineError> {
    let mut points: Vec<Point2> = Vec::with_capacity(seeds.len());
    points.extend(seeds.first());
    for pair in seeds.windows(2) {
        let [from, to] = pair else {
            continue;
        };
        interrupt::check()?;
        let mut pending = vec![(*from, *to, 0_usize)];
        while let Some((start, end, depth)) = pending.pop() {
            let halfway = start.lerp(end, 0.5);
            let bowed = (depth < REFINE_DEPTH && points.len() + pending.len() < MAX_CHAIN_POINTS)
                .then(|| on_contour(surface, halfway, toward))
                .flatten()
                .filter(|middle| middle.distance(halfway) <= start.distance(end))
                .filter(|middle| {
                    chord_distance(
                        surface.point_at(*middle),
                        surface.point_at(start),
                        surface.point_at(end),
                    ) > tolerance.chord()
                });
            match bowed {
                Some(middle) => {
                    pending.push((middle, end, depth + 1));
                    pending.push((start, middle, depth + 1));
                }
                None => points.push(end),
            }
        }
    }
    Ok(points)
}

pub(super) fn traced_contours(
    surface: &Surface,
    uv_box: Aabb2,
    toward: Vector3,
    tolerance: &SamplingTolerance,
) -> Result<Vec<Contour>, HiddenLineError> {
    let grid = Grid::new(surface, uv_box, toward)?;
    let links = grid.links()?;
    let mut contours = Vec::new();
    for chain in chains(&links) {
        interrupt::check()?;
        let closed = chain.len() > 2 && chain.first() == chain.last();
        let mut seeds: Vec<Point2> = Vec::with_capacity(chain.len());
        for key in &chain {
            let Some(crossing) = grid.crossing(*key) else {
                continue;
            };
            let unwrapped = seeds.last().map_or(crossing, |last| {
                Point2::new(
                    nearest_turn(crossing.x, last.x, surface.u_period()),
                    nearest_turn(crossing.y, last.y, surface.v_period()),
                )
            });
            seeds.push(
                on_contour(surface, unwrapped, toward)
                    .filter(|corrected| corrected.distance(unwrapped) <= grid.step())
                    .unwrap_or(unwrapped),
            );
        }
        let path = refined(surface, &seeds, toward, tolerance)?;
        let points: Vec<Point3> = path.iter().map(|uv| surface.point_at(*uv)).collect();
        if let Some((curve, range)) = polyline_curve(&points)? {
            contours.push(Contour {
                curve,
                range,
                closed,
                sampled: true,
            });
        }
    }
    Ok(contours)
}
