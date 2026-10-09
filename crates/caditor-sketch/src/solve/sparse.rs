use std::collections::{BTreeMap, BTreeSet};

use crate::{
    id::ConstraintId,
    solve::{
        equation::Gradient,
        numeric::{self, Cancelled},
        tally,
    },
};

const STEP_TOLERANCE: f64 = 1e-12;
const EXTRA_ITERATIONS: usize = 100;
const ITERATIONS_PER_VARIABLE: usize = 4;
const NEGLIGIBLE: f64 = 1e-15;
const NEIGHBOURHOOD_HOPS: usize = 2;
const CLIQUE_LIMIT: usize = 32;

pub(crate) type Row = Vec<(usize, f64)>;

pub(crate) fn row(gradient: &Gradient, variables: &[usize]) -> Row {
    let mut merged: BTreeMap<usize, f64> = BTreeMap::new();
    for (variable, partial) in gradient {
        if let Ok(column) = variables.binary_search(variable) {
            *merged.entry(column).or_default() += partial;
        }
    }
    merged
        .into_iter()
        .filter(|(_, value)| *value != 0.0)
        .collect()
}

fn norm(row: &Row) -> f64 {
    row.iter()
        .map(|(_, value)| value * value)
        .sum::<f64>()
        .sqrt()
}

pub(crate) fn normalized(row: Row) -> Row {
    let length = norm(&row);
    if length > 0.0 && length.is_finite() {
        row.into_iter()
            .map(|(column, value)| (column, value / length))
            .collect()
    } else {
        Vec::new()
    }
}

fn squared(vector: &[f64]) -> f64 {
    vector.iter().map(|value| value * value).sum()
}

pub(crate) fn minimal_norm_step(rows: &[Row], residuals: &[f64], width: usize) -> Option<Vec<f64>> {
    if rows.is_empty() || width == 0 {
        return None;
    }
    let mut scaled = Vec::with_capacity(rows.len());
    let mut target = Vec::with_capacity(rows.len());
    for (row, residual) in rows.iter().zip(residuals) {
        let length = norm(row);
        let divisor = if length > 0.0 { length } else { 1.0 };
        scaled.push(
            row.iter()
                .map(|(column, value)| (*column, value / divisor))
                .collect::<Row>(),
        );
        target.push(-residual / divisor);
    }
    let finite = scaled
        .iter()
        .flatten()
        .map(|(_, value)| value)
        .chain(&target)
        .all(|value| value.is_finite());
    if !finite {
        return None;
    }
    let multiply = |vector: &[f64]| -> Vec<f64> {
        scaled
            .iter()
            .map(|row| {
                row.iter()
                    .map(|(column, value)| value * vector.get(*column).copied().unwrap_or(0.0))
                    .sum()
            })
            .collect()
    };
    let multiply_transposed = |vector: &[f64]| -> Vec<f64> {
        let mut result = vec![0.0; width];
        for (row, factor) in scaled.iter().zip(vector) {
            for (column, value) in row {
                if let Some(slot) = result.get_mut(*column) {
                    *slot += value * factor;
                }
            }
        }
        result
    };
    let mut step = vec![0.0; width];
    let mut remaining = target;
    let mut gradient = multiply_transposed(&remaining);
    let mut direction = gradient.clone();
    let mut gamma = squared(&gradient);
    let stop = STEP_TOLERANCE * gamma.sqrt();
    let limit = width
        .saturating_mul(ITERATIONS_PER_VARIABLE)
        .saturating_add(EXTRA_ITERATIONS);
    let nonzeros: usize = scaled.iter().map(Vec::len).sum();
    for _ in 0..limit {
        if gamma.sqrt() <= stop || gamma == 0.0 {
            break;
        }
        tally::add(2 * nonzeros + width);
        let image = multiply(&direction);
        let curvature = squared(&image);
        if curvature <= 0.0 || !curvature.is_finite() {
            break;
        }
        let alpha = gamma / curvature;
        step.iter_mut()
            .zip(&direction)
            .for_each(|(value, delta)| *value += alpha * delta);
        remaining
            .iter_mut()
            .zip(&image)
            .for_each(|(value, delta)| *value -= alpha * delta);
        gradient = multiply_transposed(&remaining);
        let next = squared(&gradient);
        let beta = next / gamma;
        direction
            .iter_mut()
            .zip(&gradient)
            .for_each(|(value, fresh)| *value = fresh + beta * *value);
        gamma = next;
    }
    step.iter().all(|value| value.is_finite()).then_some(step)
}

#[derive(Debug, Clone)]
pub(crate) struct Triangular {
    columns: Vec<usize>,
    positions: Vec<usize>,
    rows: Vec<Option<Row>>,
    rank: usize,
}

impl Triangular {
    pub fn ordered_for<'a>(width: usize, rows: impl IntoIterator<Item = &'a Row>) -> Self {
        let columns = fill_reducing_order(width, rows);
        let mut positions = vec![0; width];
        for (position, column) in columns.iter().enumerate() {
            if let Some(slot) = positions.get_mut(*column) {
                *slot = position;
            }
        }
        Self {
            rows: vec![None; columns.len()],
            columns,
            positions,
            rank: 0,
        }
    }

    pub fn insert(&mut self, row: &Row, tolerance: f64) -> bool {
        let mut remainder: Row = row
            .iter()
            .filter_map(|(column, value)| Some((*self.positions.get(*column)?, *value)))
            .collect();
        remainder.sort_by_key(|(position, _)| *position);
        loop {
            let Some(&(position, value)) = remainder.first() else {
                return false;
            };
            let Some(slot) = self.rows.get_mut(position) else {
                return false;
            };
            match slot {
                Some(pivot_row) => remainder = rotate(pivot_row, &remainder),
                None if value.abs() > tolerance => {
                    *slot = Some(remainder);
                    self.rank += 1;
                    return true;
                }
                None => {
                    remainder.remove(0);
                }
            }
        }
    }

    pub fn rank(&self) -> usize {
        self.rank
    }

    pub fn fixed_columns(
        &self,
        tolerance: f64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<bool>, Cancelled> {
        let width = self.rows.len();
        let mut fixed = vec![true; width];
        let mut reaches_free = vec![false; width];
        for position in (0..width).rev() {
            let reaches = match self.rows.get(position) {
                Some(Some(row)) => row.iter().skip(1).any(|(later, _)| {
                    reaches_free.get(*later).copied().unwrap_or(false)
                        || matches!(self.rows.get(*later), Some(None))
                }),
                _ => {
                    if let Some(slot) = fixed.get_mut(position) {
                        *slot = false;
                    }
                    false
                }
            };
            if let Some(slot) = reaches_free.get_mut(position) {
                *slot = reaches;
            }
        }
        let free: Vec<usize> = (0..width)
            .filter(|position| matches!(self.rows.get(*position), Some(None)))
            .collect();
        let reaching: Vec<usize> = (0..width)
            .filter(|position| reaches_free.get(*position).copied().unwrap_or(false))
            .collect();
        tally::add(width);
        if free.len() <= reaching.len() {
            let largest = self.null_space_extent(&free, &reaches_free, cancelled)?;
            for position in &reaching {
                if let (Some(slot), Some(extent)) =
                    (fixed.get_mut(*position), largest.get(*position))
                {
                    *slot = *extent <= tolerance;
                }
            }
        } else {
            for position in &reaching {
                if cancelled() {
                    return Err(Cancelled);
                }
                if let Some(slot) = fixed.get_mut(*position) {
                    *slot = self.spans_unit(*position, tolerance);
                }
            }
        }
        let mut by_column = vec![false; width];
        for (column, is_fixed) in self.columns.iter().zip(fixed) {
            if let Some(slot) = by_column.get_mut(*column) {
                *slot = is_fixed;
            }
        }
        Ok(by_column)
    }

    fn null_space_extent(
        &self,
        free: &[usize],
        reaches_free: &[bool],
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<f64>, Cancelled> {
        let width = self.rows.len();
        let mut largest = vec![0.0_f64; width];
        let mut vector = vec![0.0; width];
        for &start in free {
            if cancelled() {
                return Err(Cancelled);
            }
            vector.iter_mut().for_each(|entry| *entry = 0.0);
            if let Some(slot) = vector.get_mut(start) {
                *slot = 1.0;
            }
            for position in (0..start).rev() {
                if !reaches_free.get(position).copied().unwrap_or(false) {
                    continue;
                }
                let Some(Some(row)) = self.rows.get(position) else {
                    continue;
                };
                let Some((_, diagonal)) = row.first() else {
                    continue;
                };
                tally::add(row.len());
                let sum: f64 = row
                    .iter()
                    .skip(1)
                    .map(|(later, entry)| entry * vector.get(*later).copied().unwrap_or(0.0))
                    .sum();
                let entry = -sum / diagonal;
                if let Some(slot) = vector.get_mut(position) {
                    *slot = entry;
                }
                if let Some(slot) = largest.get_mut(position) {
                    *slot = slot.max(entry.abs());
                }
            }
        }
        Ok(largest)
    }

    fn spans_unit(&self, position: usize, tolerance: f64) -> bool {
        let mut remainder = BTreeMap::from([(position, 1.0_f64)]);
        while let Some((current, value)) = remainder.pop_first() {
            if value.abs() <= NEGLIGIBLE {
                continue;
            }
            match self.rows.get(current) {
                Some(Some(row)) => {
                    let Some((_, diagonal)) = row.first() else {
                        continue;
                    };
                    let factor = value / diagonal;
                    tally::add(row.len());
                    for (later, entry) in row.iter().skip(1) {
                        *remainder.entry(*later).or_insert(0.0) -= factor * entry;
                    }
                }
                _ if value.abs() > tolerance => return false,
                _ => {}
            }
        }
        true
    }
}

fn rotate(pivot_row: &mut Row, incoming: &[(usize, f64)]) -> Row {
    let (Some(&(_, diagonal)), Some(&(_, value))) = (pivot_row.first(), incoming.first()) else {
        return Vec::new();
    };
    let radius = diagonal.hypot(value);
    let (cosine, sine) = (diagonal / radius, value / radius);
    tally::add(pivot_row.len() + incoming.len());
    let mut rotated = Vec::with_capacity(pivot_row.len() + incoming.len());
    let mut remainder = Vec::with_capacity(pivot_row.len() + incoming.len());
    let mut pivot_entries = pivot_row.iter().skip(1).peekable();
    let mut incoming_entries = incoming.iter().skip(1).peekable();
    rotated.extend(pivot_row.first().map(|(position, _)| (*position, radius)));
    loop {
        let (position, kept, arriving) = match (pivot_entries.peek(), incoming_entries.peek()) {
            (Some(&&(a, kept)), Some(&&(b, _))) if a < b => {
                pivot_entries.next();
                (a, kept, 0.0)
            }
            (Some(&&(a, _)), Some(&&(b, arriving))) if b < a => {
                incoming_entries.next();
                (b, 0.0, arriving)
            }
            (Some(&&(a, kept)), Some(&&(_, arriving))) => {
                pivot_entries.next();
                incoming_entries.next();
                (a, kept, arriving)
            }
            (Some(&&(a, kept)), None) => {
                pivot_entries.next();
                (a, kept, 0.0)
            }
            (None, Some(&&(b, arriving))) => {
                incoming_entries.next();
                (b, 0.0, arriving)
            }
            (None, None) => break,
        };
        let stays = cosine * kept + sine * arriving;
        let leaves = cosine * arriving - sine * kept;
        if stays.abs() > NEGLIGIBLE {
            rotated.push((position, stays));
        }
        if leaves.abs() > NEGLIGIBLE {
            remainder.push((position, leaves));
        }
    }
    *pivot_row = rotated;
    remainder
}

fn fill_reducing_order<'a>(width: usize, rows: impl IntoIterator<Item = &'a Row>) -> Vec<usize> {
    let mut neighbours: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); width];
    for row in rows {
        tally::add(row.len() * row.len());
        for (column, _) in row {
            if let Some(adjacent) = neighbours.get_mut(*column) {
                adjacent.extend(
                    row.iter()
                        .map(|(other, _)| *other)
                        .filter(|other| other != column && *other < width),
                );
            }
        }
    }
    let mut queue: BTreeSet<(usize, usize)> = neighbours
        .iter()
        .enumerate()
        .map(|(column, adjacent)| (adjacent.len(), column))
        .collect();
    let mut order = Vec::with_capacity(width);
    while let Some((degree, column)) = queue.pop_first() {
        order.push(column);
        let adjacent = neighbours
            .get_mut(column)
            .map(std::mem::take)
            .unwrap_or_default();
        tally::add(degree * degree.min(CLIQUE_LIMIT) + 1);
        for other in &adjacent {
            let Some(theirs) = neighbours.get_mut(*other) else {
                continue;
            };
            let before = theirs.len();
            theirs.remove(&column);
            if degree <= CLIQUE_LIMIT {
                theirs.extend(adjacent.iter().copied().filter(|next| next != other));
            }
            if queue.remove(&(before, *other)) {
                queue.insert((theirs.len(), *other));
            }
        }
    }
    order
}

pub(crate) fn duplicates(
    rows: &[Row],
    earlier: &[(Option<ConstraintId>, Row)],
    width: usize,
) -> Vec<ConstraintId> {
    let mut columns: BTreeSet<usize> = rows.iter().flatten().map(|(column, _)| *column).collect();
    let mut chosen = BTreeSet::new();
    for _ in 0..NEIGHBOURHOOD_HOPS {
        for (index, (_, row)) in earlier.iter().enumerate() {
            if row.iter().any(|(column, _)| columns.contains(column)) {
                chosen.insert(index);
            }
        }
        for index in &chosen {
            if let Some((_, row)) = earlier.get(*index) {
                columns.extend(row.iter().map(|(column, _)| *column));
            }
        }
    }
    let local: Vec<usize> = columns
        .into_iter()
        .filter(|column| *column < width)
        .collect();
    let dense = |row: &Row| -> Vec<f64> {
        let mut values = vec![0.0; local.len()];
        for (column, value) in row {
            if let Ok(position) = local.binary_search(column)
                && let Some(slot) = values.get_mut(position)
            {
                *slot = *value;
            }
        }
        values
    };
    let rows: Vec<Vec<f64>> = rows.iter().map(dense).collect();
    let earlier: Vec<(Option<ConstraintId>, Vec<f64>)> = chosen
        .iter()
        .filter_map(|index| earlier.get(*index))
        .map(|(owner, row)| (*owner, dense(row)))
        .collect();
    numeric::duplicates(&rows, &earlier)
}
