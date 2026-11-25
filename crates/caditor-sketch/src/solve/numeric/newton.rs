use std::{borrow::Cow, cell::OnceCell, collections::BTreeMap};

use nalgebra::{DMatrix, SymmetricEigen};

use crate::solve::{
    equation::{Gradient, value},
    numeric::{Cancelled, Part, SVD_ITERATIONS, Solver, Stride, largest_move, reach::PROBE},
    sparse, tally,
};

const RETRIES: usize = 8;
const LEAST_DAMPING: f64 = 1e-6;
const DAMPING_GROWTH: f64 = 8.0;
const DAMPING_DECAY: f64 = 10.0;
const LEAST_PIVOT: f64 = 1e-12;
const TURNING_SHIFT: f64 = 3e2;

const SETTLED_FALL: f64 = 1e-9;

pub(super) type Entries = Vec<(usize, f64)>;

type Block = Vec<(usize, usize, f64)>;

pub(super) struct Curvature {
    pub columns: Vec<BTreeMap<usize, f64>>,
    pub work: usize,
}

struct Bends {
    weighted: Vec<BTreeMap<usize, f64>>,
    blocks: Vec<Block>,
}

pub(super) struct Fell {
    pub remaining: f64,
    pub settled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Damping(f64);

impl Damping {
    pub const NONE: Self = Self(0.0);

    fn raised(self) -> Self {
        let Self(damping) = self;
        if damping == 0.0 {
            Self(LEAST_DAMPING)
        } else {
            Self(damping * DAMPING_GROWTH)
        }
    }

    fn lowered(self, gain: f64) -> Self {
        let Self(damping) = self;
        let decay = (1.0 - (2.0 * gain - 1.0).powi(3)).max(1.0 / DAMPING_DECAY);

        let lowered = damping * decay;
        if lowered < LEAST_DAMPING {
            Self::NONE
        } else {
            Self(lowered)
        }
    }
}

struct Model {
    rows: Vec<Entries>,
    hessian: Vec<Entries>,
    blocks: Vec<Block>,
    turning: OnceCell<Vec<Entries>>,
    slope: Vec<f64>,
    scales: Vec<f64>,
    typical: f64,
}

impl Model {
    fn times(&self, direction: &[f64]) -> Vec<f64> {
        self.hessian
            .iter()
            .map(|entries| {
                entries
                    .iter()
                    .map(|(other, value)| value * direction.get(*other).copied().unwrap_or(0.0))
                    .sum()
            })
            .collect()
    }

    fn turning(&self) -> &[Entries] {
        self.turning
            .get_or_init(|| turning_metric(&self.blocks, self.hessian.len(), self.typical))
    }

    fn shifted(&self, damping: f64) -> Cow<'_, [Entries]> {
        if damping == 0.0 {
            return Cow::Borrowed(&self.hessian);
        }

        let turning = self.turning();

        if turning.iter().all(Vec::is_empty) {
            return Cow::Borrowed(&self.hessian);
        }
        Cow::Owned(
            self.hessian
                .iter()
                .zip(turning)
                .map(|(entries, turning)| {
                    let mut merged: BTreeMap<usize, f64> = entries.iter().copied().collect();

                    tally::add(turning.len());
                    for (other, entry) in turning {
                        *merged.entry(*other).or_default() += damping * entry;
                    }
                    merged.into_iter().collect()
                })
                .collect(),
        )
    }

    fn solve(&self, damping: f64) -> Option<Vec<f64>> {
        let factored = sparse::Cholesky::factor(
            &self.shifted(damping),
            damping * self.typical,
            LEAST_PIVOT * self.typical,
            &self.rows,
        )?;
        let downhill: Vec<f64> = self.slope.iter().map(|slope| -slope).collect();

        let step = factored.solve(&downhill);
        step.iter().all(|value| value.is_finite()).then_some(step)
    }

    fn predicted_fall(&self, scaled: &[f64]) -> f64 {
        let image = self.times(scaled);
        -(2.0 * dot(&self.slope, scaled) + dot(scaled, &image))
    }

    fn unscaled(&self, scaled: &[f64]) -> Vec<f64> {
        scaled
            .iter()
            .zip(&self.scales)
            .map(|(delta, scale)| delta * scale)
            .collect()
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn colours(rows: &[Entries], width: usize) -> (Vec<Vec<usize>>, usize) {
    let mut rows_of_column: Vec<Vec<usize>> = vec![Vec::new(); width];

    for (index, row) in rows.iter().enumerate() {
        for (column, _) in row {
            if let Some(slot) = rows_of_column.get_mut(*column) {
                slot.push(index);
            }
        }
    }

    let mut colour_of: Vec<Option<usize>> = vec![None; width];
    let mut colours: Vec<Vec<usize>> = Vec::new();
    let mut work = 0;

    for column in 0..width {
        let taken: Vec<usize> = rows_of_column
            .get(column)
            .into_iter()
            .flatten()
            .filter_map(|row| rows.get(*row))
            .flatten()
            .filter_map(|(other, _)| colour_of.get(*other).copied().flatten())
            .collect();

        tally::add(taken.len() + 1);
        work += taken.len() + 1;

        let colour = (0..)
            .find(|candidate| !taken.contains(candidate))
            .unwrap_or(0);

        if let Some(slot) = colour_of.get_mut(column) {
            *slot = Some(colour);
        }
        match colours.get_mut(colour) {
            Some(members) => members.push(column),
            None => colours.push(vec![column]),
        }
    }
    (colours, work)
}

fn turning_metric(blocks: &[Block], width: usize, typical: f64) -> Vec<Entries> {
    let mut turning: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); width];

    for block in blocks
        .iter()
        .filter(|block| block.iter().any(|(_, _, second)| *second != 0.0))
    {
        add_curvature_size(block, &mut turning);
    }

    let typical_turning = turning
        .iter()
        .enumerate()
        .filter_map(|(column, entries)| entries.get(&column))
        .sum::<f64>()
        / width.max(1) as f64;

    if typical_turning <= 0.0 || !typical_turning.is_finite() {
        return vec![Vec::new(); width];
    }

    let weight = TURNING_SHIFT * typical / typical_turning;

    turning
        .into_iter()
        .map(|entries| {
            entries
                .into_iter()
                .map(|(other, entry)| (other, weight * entry))
                .filter(|(_, entry)| *entry != 0.0)
                .collect()
        })
        .collect()
}

fn add_curvature_size(block: &[(usize, usize, f64)], turning: &mut [BTreeMap<usize, f64>]) {
    let mut columns: Vec<usize> = block
        .iter()
        .flat_map(|(column, other, _)| [*column, *other])
        .collect();

    columns.sort_unstable();
    columns.dedup();

    let size = columns.len();
    let at = |column: usize| columns.binary_search(&column).ok();
    let mut matrix = DMatrix::<f64>::zeros(size, size);

    for (column, other, second) in block {
        if let (Some(row), Some(across)) = (at(*column), at(*other)) {
            if let Some(slot) = matrix.get_mut((row, across)) {
                *slot += 0.5 * second;
            }
            if let Some(slot) = matrix.get_mut((across, row)) {
                *slot += 0.5 * second;
            }
        }
    }
    tally::add(size * size * size);

    let Some(eigen) = SymmetricEigen::try_new(matrix, f64::EPSILON, SVD_ITERATIONS) else {
        return;
    };

    for (value, vector) in eigen
        .eigenvalues
        .iter()
        .map(|value| value.abs())
        .zip(eigen.eigenvectors.column_iter())
        .filter(|(value, _)| *value > 0.0)
    {
        for (row, column) in columns.iter().enumerate() {
            let Some(slot) = turning.get_mut(*column) else {
                continue;
            };
            for (across, other) in columns.iter().enumerate() {
                let entry = value
                    * vector.get(row).copied().unwrap_or(0.0)
                    * vector.get(across).copied().unwrap_or(0.0);

                *slot.entry(*other).or_default() += entry;
            }
        }
    }
}

impl Solver<'_> {
    pub(super) fn newton_descent(
        &self,
        part: &Part<'_>,
        values: &mut [f64],
        stride: &mut Stride,
    ) -> Result<Option<Fell>, Cancelled> {
        let longest = stride.longest;
        let damping = &mut stride.damping;
        let mut scales = self.column_scales(part.component);

        let mut model = self.model(part, values, scales.clone());
        let mut held_at_bounds = false;

        for attempt in 0..RETRIES {
            if attempt > 0 && (self.cancelled)() {
                return Err(Cancelled);
            }

            let Damping(shift) = *damping;
            let Some(mut scaled) = model.solve(shift) else {
                *damping = damping.raised();
                continue;
            };
            let pushed = self.pushed_past_bounds(part.component, values, &model.unscaled(&scaled));

            if !pushed.is_empty() && !held_at_bounds {
                for column in pushed {
                    if let Some(scale) = scales.get_mut(column) {
                        *scale = 0.0;
                    }
                }
                model = self.model(part, values, scales.clone());
                held_at_bounds = true;
                continue;
            }

            let largest = largest_move(&model.unscaled(&scaled));

            if largest > longest {
                scaled
                    .iter_mut()
                    .for_each(|delta| *delta *= longest / largest);
            }

            let step = model.unscaled(&scaled);
            let current = self.squared_residuals(part, values);
            let fall = model.predicted_fall(&scaled);

            let settled =
                shift <= LEAST_DAMPING && largest <= longest && fall <= SETTLED_FALL * current;

            if let Some(remaining) = self.line_search(part, &step, values, 1) {
                *damping = damping.lowered(current * (1.0 - remaining) / fall);
                return Ok(Some(Fell { remaining, settled }));
            }
            if settled {
                return Ok(Some(Fell {
                    remaining: 1.0,
                    settled,
                }));
            }
            *damping = damping.raised();
        }
        Ok(None)
    }

    fn model(&self, part: &Part<'_>, values: &[f64], scales: Vec<f64>) -> Model {
        let width = part.component.variables.len();
        let (rows, residuals) = self.structural_linearize(part, values);

        let Bends {
            weighted: curvature,
            blocks,
        } = self.bends(part, values, &rows, &residuals);
        let scale_of = |column: usize| scales.get(column).copied().unwrap_or(1.0);
        let rows: Vec<Entries> = rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|(column, entry)| (column, entry * scale_of(column)))
                    .collect()
            })
            .collect();
        let mut hessian: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); width];

        for (column, entries) in curvature.iter().enumerate() {
            for (other, entry) in entries {
                let scaled = 0.5 * entry * scale_of(column) * scale_of(*other);

                if let Some(slot) = hessian.get_mut(column) {
                    *slot.entry(*other).or_default() += scaled;
                }
                if let Some(slot) = hessian.get_mut(*other) {
                    *slot.entry(column).or_default() += scaled;
                }
            }
        }

        let mut slope = vec![0.0; width];
        let mut diagonal = 0.0;

        for (row, residual) in rows.iter().zip(&residuals) {
            tally::add(row.len() * row.len());
            for (column, entry) in row {
                if let Some(slot) = slope.get_mut(*column) {
                    *slot += entry * residual;
                }
                diagonal += entry * entry;
                if let Some(slot) = hessian.get_mut(*column) {
                    for (other, other_entry) in row {
                        *slot.entry(*other).or_default() += entry * other_entry;
                    }
                }
            }
        }

        let typical = diagonal / width.max(1) as f64;
        let typical = if typical > 0.0 { typical } else { 1.0 };

        for (column, scale) in scales.iter().enumerate() {
            if *scale == 0.0
                && let Some(slot) = hessian.get_mut(column)
            {
                *slot.entry(column).or_default() += typical;
            }
        }

        let blocks: Vec<Block> = blocks
            .into_iter()
            .map(|block| {
                block
                    .into_iter()
                    .map(|(column, other, second)| {
                        (column, other, second * scale_of(column) * scale_of(other))
                    })
                    .collect()
            })
            .collect();

        Model {
            rows,
            hessian: hessian
                .into_iter()
                .map(|entries| entries.into_iter().collect())
                .collect(),
            blocks,
            turning: OnceCell::new(),
            slope,
            scales,
            typical,
        }
    }

    fn bends(&self, part: &Part<'_>, values: &[f64], rows: &[Entries], weights: &[f64]) -> Bends {
        let width = part.component.variables.len();
        let probe = PROBE * part.context.scale;
        let mut weighted: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); width];
        let mut blocks: Vec<Block> = vec![Vec::new(); rows.len()];

        self.probe_curvature(part, values, rows, |row, column, other, change| {
            if let Some(weight) = weights.get(row)
                && let Some(bent) = weighted.get_mut(column)
            {
                *bent.entry(other).or_default() += weight * change / probe;
            }
            if let Some(block) = blocks.get_mut(row) {
                block.push((column, other, change / probe));
            }
        });

        Bends { weighted, blocks }
    }

    pub(super) fn weighted_curvature(
        &self,
        part: &Part<'_>,
        values: &[f64],
        rows: &[Entries],
        weights: &[f64],
    ) -> Curvature {
        let width = part.component.variables.len();
        let probe = PROBE * part.context.scale;
        let mut curvature: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); width];

        let work = self.probe_curvature(part, values, rows, |row, column, other, change| {
            if let Some(weight) = weights.get(row)
                && let Some(bent) = curvature.get_mut(column)
            {
                *bent.entry(other).or_default() += weight * change / probe;
            }
        });
        Curvature {
            columns: curvature,
            work,
        }
    }

    fn probe_curvature(
        &self,
        part: &Part<'_>,
        values: &[f64],
        rows: &[Entries],
        mut bent: impl FnMut(usize, usize, usize, f64),
    ) -> usize {
        let component = part.component;
        let width = component.variables.len();
        let probe = PROBE * part.context.scale;
        let mut probed = values.to_vec();
        let (colours, colouring) = colours(rows, width);

        let work = colouring + colours.len() * rows.len();

        for members in colours {
            for column in &members {
                if let Some(variable) = component.variables.get(*column)
                    && let Some(slot) = probed.get_mut(*variable)
                {
                    *slot = value(values, *variable) + probe;
                }
            }

            let (moved, _) = self.structural_linearize(part, &probed);

            for (index, (row, moved_row)) in rows.iter().zip(&moved).enumerate() {
                let Some(column) = row
                    .iter()
                    .map(|(column, _)| *column)
                    .find(|column| members.binary_search(column).is_ok())
                else {
                    continue;
                };
                for (other, after) in moved_row {
                    let before = row
                        .iter()
                        .find(|(at, _)| at == other)
                        .map_or(0.0, |(_, before)| *before);

                    bent(index, column, *other, after - before);
                }
            }
            for column in &members {
                if let Some(variable) = component.variables.get(*column)
                    && let Some(slot) = probed.get_mut(*variable)
                {
                    *slot = value(values, *variable);
                }
            }
        }
        work
    }

    pub(super) fn structural_linearize(
        &self,
        part: &Part<'_>,
        values: &[f64],
    ) -> (Vec<Entries>, Vec<f64>) {
        let component = part.component;
        let mut gradient = Gradient::new();
        let mut rows = Vec::with_capacity(component.equations.len());
        let mut residuals = Vec::with_capacity(component.equations.len());

        tally::add(component.equations.len());
        for equation in self.equations(component) {
            let residual = equation.linearize(values, &part.context, &mut gradient);
            let mut merged: BTreeMap<usize, f64> = BTreeMap::new();

            for (variable, partial) in &gradient {
                if let Ok(column) = component.variables.binary_search(variable) {
                    *merged.entry(column).or_default() += partial;
                }
            }
            rows.push(merged.into_iter().collect());
            residuals.push(residual);
        }
        (rows, residuals)
    }
}
