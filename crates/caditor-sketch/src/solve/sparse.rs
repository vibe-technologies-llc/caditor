use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, BinaryHeap},
};

use crate::{
    id::ConstraintId,
    solve::{equation::Gradient, numeric, tally},
};

const STEP_TOLERANCE: f64 = 1e-12;
const EXTRA_ITERATIONS: usize = 100;
const ITERATIONS_PER_VARIABLE: usize = 4;
const NEGLIGIBLE: f64 = 1e-15;
const NEIGHBOURHOOD_HOPS: usize = 2;

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

#[derive(Debug, Clone, Default)]
pub(crate) struct Echelon {
    rows: Vec<BTreeMap<usize, f64>>,
    pivots: Vec<usize>,
    by_column: BTreeMap<usize, usize>,
}

impl Echelon {
    fn reduce(&self, row: impl IntoIterator<Item = (usize, f64)>) -> BTreeMap<usize, f64> {
        let mut current: BTreeMap<usize, f64> = row.into_iter().collect();
        let mut queue: BinaryHeap<Reverse<usize>> = BinaryHeap::new();
        let mut queued = BTreeSet::new();
        for column in current.keys() {
            if let Some(index) = self.by_column.get(column)
                && queued.insert(*index)
            {
                queue.push(Reverse(*index));
            }
        }
        while let Some(Reverse(index)) = queue.pop() {
            let (Some(column), Some(pivot_row)) = (self.pivots.get(index), self.rows.get(index))
            else {
                continue;
            };
            let Some(value) = current.remove(column) else {
                continue;
            };
            let Some(pivot) = pivot_row.get(column) else {
                continue;
            };
            let factor = value / pivot;
            tally::add(pivot_row.len());
            for (other, entry) in pivot_row {
                if other == column {
                    continue;
                }
                let slot = current.entry(*other).or_insert(0.0);
                *slot -= factor * entry;
                if let Some(later) = self.by_column.get(other)
                    && queued.insert(*later)
                {
                    queue.push(Reverse(*later));
                }
            }
        }
        current.retain(|_, value| value.abs() > NEGLIGIBLE);
        current
    }

    pub fn insert(&mut self, row: &Row, tolerance: f64) -> bool {
        let remainder = self.reduce(row.iter().copied());
        let Some((column, value)) = remainder
            .iter()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .map(|(column, value)| (*column, *value))
        else {
            return false;
        };
        if value.abs() <= tolerance {
            return false;
        }
        self.by_column.insert(column, self.rows.len());
        self.pivots.push(column);
        self.rows.push(remainder);
        true
    }

    pub fn rank(&self) -> usize {
        self.rows.len()
    }

    pub fn spans_unit(&self, column: usize, tolerance: f64) -> bool {
        self.reduce([(column, 1.0)])
            .values()
            .all(|value| value.abs() <= tolerance)
    }
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
