use std::collections::BTreeMap;

use crate::solve::{
    equation::value,
    numeric::{
        CONVERGENCE_TOLERANCE, Cancelled, LINE_SEARCH_STEPS, MAX_ITERATIONS, MAX_STEP, Part,
        Solver,
        newton::Entries,
        reach::{Aim, REACH_TOLERANCE, ReachWork, miss},
    },
    sparse::{Cholesky, NotDefinite, Row},
    tally,
};

const AUGMENTATION: f64 = 1e6;
const LEAST_SHIFT: f64 = 1e-10;
const RESTORING_SHIFT: f64 = 1e-3;
const SHIFT_GROWTH: f64 = 8.0;
const SHIFTS: usize = 24;
const GRAM_SHIFT: f64 = 1e-12;
const GRAM_SHIFT_GROWTH: f64 = 100.0;
const GRAM_SHIFTS: usize = 4;
const LEAST_PIVOT: f64 = 1e-14;
const RESTORING_STEPS: usize = 8;
const RESTORING_FALL: f64 = 0.5;
const CLOSE_SHARE: f64 = 1e-3;

struct Gram {
    factored: Cholesky,
}

impl Gram {
    fn factor(rows: &[Entries], width: usize, work: &mut ReachWork) -> Option<Self> {
        let mut by_column: Vec<Row> = vec![Vec::new(); width];
        for (index, row) in rows.iter().enumerate() {
            for (column, entry) in row {
                if let Some(slot) = by_column.get_mut(*column) {
                    slot.push((index, *entry));
                }
            }
        }
        let mut gram: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); rows.len()];
        let mut trace = 0.0;
        let mut built = 0;
        for column in &by_column {
            built += column.len() * column.len();
            for (index, entry) in column {
                trace += entry * entry;
                let Some(slot) = gram.get_mut(*index) else {
                    continue;
                };
                for (other, other_entry) in column {
                    *slot.entry(*other).or_default() += entry * other_entry;
                }
            }
        }
        tally::add(built);
        work.spend(built);
        let gram: Vec<Row> = gram
            .into_iter()
            .map(|entries| entries.into_iter().collect())
            .collect();
        let typical = trace / rows.len().max(1) as f64;
        let typical = if typical > 0.0 { typical } else { 1.0 };
        let mut shift = GRAM_SHIFT * typical;
        for _ in 0..GRAM_SHIFTS {
            match Cholesky::factor_counting(&gram, shift, LEAST_PIVOT * typical, &by_column) {
                Ok(factored) => {
                    work.spend(factored.work());
                    return Some(Self { factored });
                }
                Err(NotDefinite { work: refused }) => work.spend(refused),
            }
            shift *= GRAM_SHIFT_GROWTH;
        }
        None
    }

    fn solve(&self, load: &[f64], work: &mut ReachWork) -> Vec<f64> {
        solved(&self.factored, load, work)
    }
}

fn solved(factored: &Cholesky, load: &[f64], work: &mut ReachWork) -> Vec<f64> {
    work.spend(2 * factored.entries());
    factored.solve(load)
}

fn times(rows: &[Entries], vector: &[f64]) -> Vec<f64> {
    rows.iter()
        .map(|row| {
            row.iter()
                .map(|(column, entry)| entry * vector.get(*column).copied().unwrap_or(0.0))
                .sum()
        })
        .collect()
}

fn transposed_times(rows: &[Entries], vector: &[f64], width: usize) -> Vec<f64> {
    let mut result = vec![0.0; width];
    for (row, factor) in rows.iter().zip(vector) {
        for (column, entry) in row {
            if let Some(slot) = result.get_mut(*column) {
                *slot += entry * factor;
            }
        }
    }
    result
}

struct Augmented {
    matrix: Vec<Row>,
    steepest: f64,
    augmentation: f64,
}

impl Augmented {
    fn factor(&self, rows: &[Entries], least: f64, work: &mut ReachWork) -> Option<Cholesky> {
        let mut shift = least * self.steepest;
        let least_pivot = LEAST_PIVOT * self.steepest;
        for _ in 0..SHIFTS {
            match Cholesky::factor_counting(&self.matrix, shift, least_pivot, rows) {
                Ok(factored) => {
                    work.spend(factored.work());
                    return Some(factored);
                }
                Err(NotDefinite { work: refused }) => work.spend(refused),
            }
            shift *= SHIFT_GROWTH;
        }
        None
    }
}

fn largest_move(step: &[f64]) -> f64 {
    step.iter()
        .fold(0.0_f64, |largest, delta| largest.max(delta.abs()))
}

impl Solver<'_> {
    pub(super) fn reach_far(
        &self,
        part: &Part<'_>,
        aims: &[Aim],
        values: &mut [f64],
        work: &mut ReachWork,
    ) -> Result<(), Cancelled> {
        let tolerance = REACH_TOLERANCE * part.context.scale;
        let longest = MAX_STEP * part.context.scale;
        let mut gap = miss(aims, values);
        let mut fraction = 1.0_f64;
        for _ in 0..MAX_ITERATIONS {
            if (self.cancelled)() {
                return Err(Cancelled);
            }
            if work.spent() {
                break;
            }
            let Some(mut step) = self.far_step(part, aims, values, work) else {
                break;
            };
            let largest = largest_move(&step);
            if largest > longest {
                step.iter_mut()
                    .for_each(|delta| *delta *= longest / largest);
            }
            fraction = (fraction * 2.0).min(1.0);
            let start = self.start(part, values);
            let mut closer = None;
            for _ in 0..LINE_SEARCH_STEPS {
                if work.spent() {
                    break;
                }
                self.place(part.component, &start, &step, fraction, values);
                if self.restored(part, values, work)? {
                    let reached = miss(aims, values);
                    if reached < gap {
                        closer = Some(reached);
                        break;
                    }
                }
                self.restore(part.component, &start, values);
                fraction *= 0.5;
            }
            let Some(closer) = closer else {
                break;
            };
            let gained = gap.sqrt() - closer.sqrt();
            gap = closer;
            if gained <= tolerance {
                break;
            }
        }
        Ok(())
    }

    fn far_step(
        &self,
        part: &Part<'_>,
        aims: &[Aim],
        values: &[f64],
        work: &mut ReachWork,
    ) -> Option<Vec<f64>> {
        let width = part.component.variables.len();
        let (rows, _) = self.structural_linearize(part, values);
        work.spend(rows.len());
        let mut pull = vec![0.0; width];
        for aim in aims {
            if let Some(slot) = pull.get_mut(aim.column) {
                *slot = value(values, aim.variable) - aim.to;
            }
        }

        let gram = Gram::factor(&rows, width, work)?;
        let load: Vec<f64> = times(&rows, &pull).iter().map(|entry| -entry).collect();
        let multipliers = gram.solve(&load, work);
        let downhill: Vec<f64> = transposed_times(&rows, &multipliers, width)
            .iter()
            .zip(&pull)
            .map(|(held, pulled)| -(held + pulled))
            .collect();

        let bent = self.augmented(part, values, &rows, &multipliers, aims, work);
        let factored = bent.factor(&rows, LEAST_SHIFT, work)?;
        let step = solved(&factored, &downhill, work);
        let across = gram.solve(&times(&rows, &step), work);
        let step: Vec<f64> = step
            .iter()
            .zip(transposed_times(&rows, &across, width))
            .map(|(delta, across)| delta - across)
            .collect();
        step.iter().all(|delta| delta.is_finite()).then_some(step)
    }

    fn augmented(
        &self,
        part: &Part<'_>,
        values: &[f64],
        rows: &[Entries],
        weights: &[f64],
        aims: &[Aim],
        work: &mut ReachWork,
    ) -> Augmented {
        let curvature = self.weighted_curvature(part, values, rows, weights);
        work.spend(curvature.work);
        let mut matrix: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); curvature.columns.len()];
        for (column, entries) in curvature.columns.iter().enumerate() {
            for (other, entry) in entries {
                if let Some(slot) = matrix.get_mut(column) {
                    *slot.entry(*other).or_default() += 0.5 * entry;
                }
                if let Some(slot) = matrix.get_mut(*other) {
                    *slot.entry(column).or_default() += 0.5 * entry;
                }
            }
        }
        for aim in aims {
            if let Some(slot) = matrix.get_mut(aim.column) {
                *slot.entry(aim.column).or_default() += 1.0;
            }
        }
        let steepest = matrix
            .iter()
            .enumerate()
            .filter_map(|(column, entries)| entries.get(&column))
            .fold(1.0_f64, |steepest, entry| steepest.max(entry.abs()));
        let augmentation = AUGMENTATION * steepest;
        let mut product = 0;
        for row in rows {
            product += row.len() * row.len();
            for (column, entry) in row {
                if let Some(slot) = matrix.get_mut(*column) {
                    for (other, other_entry) in row {
                        *slot.entry(*other).or_default() += augmentation * entry * other_entry;
                    }
                }
            }
        }
        tally::add(product);
        work.spend(product);
        Augmented {
            matrix: matrix
                .into_iter()
                .map(|entries| entries.into_iter().collect())
                .collect(),
            steepest,
            augmentation,
        }
    }

    fn restored(
        &self,
        part: &Part<'_>,
        values: &mut [f64],
        work: &mut ReachWork,
    ) -> Result<bool, Cancelled> {
        let width = part.component.variables.len();
        let mut cost = self.squared_residuals(part, values);
        for _ in 0..RESTORING_STEPS {
            if self.closely_held(part, values) {
                return Ok(true);
            }
            if (self.cancelled)() {
                return Err(Cancelled);
            }
            if work.spent() {
                return Ok(false);
            }
            let (rows, residuals) = self.structural_linearize(part, values);
            work.spend(rows.len());
            let unit = vec![1.0; rows.len()];
            let metric = self.augmented(part, values, &rows, &unit, &[], work);
            let Some(factored) = metric.factor(&rows, RESTORING_SHIFT, work) else {
                return Ok(false);
            };
            let load: Vec<f64> = transposed_times(&rows, &residuals, width)
                .iter()
                .map(|entry| -metric.augmentation * entry)
                .collect();
            let correction = solved(&factored, &load, work);
            let start = self.start(part, values);
            self.place(part.component, &start, &correction, 1.0, values);
            let after = self.squared_residuals(part, values);
            let halved = after < RESTORING_FALL * cost;
            if !halved {
                self.restore(part.component, &start, values);
                return Ok(self.converged(part, values));
            }
            cost = after;
        }
        Ok(self.converged(part, values))
    }

    fn closely_held(&self, part: &Part<'_>, values: &[f64]) -> bool {
        let tolerance = CLOSE_SHARE * CONVERGENCE_TOLERANCE * part.context.scale;
        self.admissible(part, values)
            && self
                .residuals(part, values)
                .all(|residual| residual.abs() <= tolerance)
    }
}
