use nalgebra::{DMatrix, DVector, SVD, SymmetricEigen};

use crate::solve::{
    equation::value,
    numeric::{
        Cancelled, Component, DENSE_LIMIT, Ending, LINE_SEARCH_STEPS, MAX_ITERATIONS, MAX_STEP,
        Part, RANK_TOLERANCE, STEP_RANK_TOLERANCE, SVD_ITERATIONS, Solver, normalized, tally,
    },
};

const LOOSE: f64 = 1.0;
const REACH_TOLERANCE: f64 = 1e-10;
const PROBE: f64 = 1e-7;
const FLATTEST: f64 = 1e-3;

struct Aim {
    column: usize,
    variable: usize,
    to: f64,
}

fn miss(aims: &[Aim], values: &[f64]) -> f64 {
    aims.iter()
        .map(|aim| (value(values, aim.variable) - aim.to).powi(2))
        .sum()
}

fn dense(rows: &[Vec<f64>], height: usize, width: usize) -> DMatrix<f64> {
    DMatrix::from_fn(height, width, |row, column| {
        rows.get(row)
            .and_then(|entries| entries.get(column))
            .copied()
            .unwrap_or(0.0)
    })
}

fn multipliers(jacobian: &DMatrix<f64>, pull: &DVector<f64>) -> Option<DVector<f64>> {
    let (height, width) = jacobian.shape();
    if height == 0 {
        return Some(DVector::zeros(0));
    }
    tally::add(height * width * height.min(width));
    let transposed = SVD::try_new(
        jacobian.transpose(),
        true,
        true,
        f64::EPSILON,
        SVD_ITERATIONS,
    )?;
    let cutoff = transposed.singular_values.max() * STEP_RANK_TOLERANCE;
    transposed.solve(&(-pull), cutoff).ok()
}

fn free_directions(rows: &[Vec<f64>], width: usize) -> Option<DMatrix<f64>> {
    let rows: Vec<Vec<f64>> = rows.iter().cloned().map(normalized).collect();
    if rows.is_empty() {
        return Some(DMatrix::identity(width, width));
    }
    let height = rows.len().max(width);
    tally::add(height * width * width);
    let svd = SVD::try_new(
        dense(&rows, height, width),
        false,
        true,
        f64::EPSILON,
        SVD_ITERATIONS,
    )?;
    let basis = svd.v_t?;
    let cutoff = svd.singular_values.max() * RANK_TOLERANCE;
    let free: Vec<DVector<f64>> = svd
        .singular_values
        .iter()
        .zip(basis.row_iter())
        .filter(|(singular, _)| **singular <= cutoff)
        .map(|(_, row)| row.transpose())
        .collect();
    if free.is_empty() {
        return Some(DMatrix::zeros(width, 0));
    }
    Some(DMatrix::from_columns(&free))
}

impl Solver<'_> {
    pub fn reach(
        &self,
        component: &Component,
        targets: &[(usize, f64)],
        values: &mut [f64],
    ) -> Result<(), Cancelled> {
        let part = self.part(component);
        let aims: Vec<Aim> = component
            .variables
            .iter()
            .enumerate()
            .filter_map(|(column, variable)| {
                targets
                    .iter()
                    .find(|(aimed, _)| aimed == variable)
                    .map(|(_, to)| Aim {
                        column,
                        variable: *variable,
                        to: *to,
                    })
            })
            .collect();
        if aims.is_empty()
            || component.variables.len() > DENSE_LIMIT
            || !self.converged(&part, values)
        {
            return Ok(());
        }
        let loose = Solver {
            stiffness: LOOSE,
            ..*self
        };
        let tolerance = REACH_TOLERANCE * part.context.scale;
        let longest = MAX_STEP * part.context.scale;
        let mut gap = miss(&aims, values);
        for _ in 0..MAX_ITERATIONS {
            if (self.cancelled)() {
                return Err(Cancelled);
            }
            let Some(mut step) = self.reaching_step(&part, &aims, values) else {
                break;
            };
            let length = step.iter().map(|delta| delta * delta).sum::<f64>().sqrt();
            if length > longest {
                step.iter_mut().for_each(|delta| *delta *= longest / length);
            }
            let Some(closer) = loose.reach_along(&part, &aims, &step, gap, values)? else {
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

    fn reaching_step(&self, part: &Part<'_>, aims: &[Aim], values: &[f64]) -> Option<Vec<f64>> {
        let width = part.component.variables.len();
        let (rows, _) = self.linearize(part, values);
        let height = rows.len();
        let jacobian = dense(&rows, height, width);
        let mut pull = DVector::zeros(width);
        for aim in aims {
            if let Some(slot) = pull.get_mut(aim.column) {
                *slot = value(values, aim.variable) - aim.to;
            }
        }

        let multipliers = multipliers(&jacobian, &pull)?;
        let free = free_directions(&rows, width)?;
        if free.ncols() == 0 {
            return None;
        }

        let probe = PROBE * part.context.scale;
        let mut bent = DMatrix::zeros(width, free.ncols());
        let start = self.start(part, values);
        let mut probed = values.to_vec();
        for (index, direction) in free.column_iter().enumerate() {
            let moving: Vec<f64> = direction.iter().copied().collect();
            self.place(part.component, &start, &moving, probe, &mut probed);
            let (moved, _) = self.linearize(part, &probed);
            let mut curvature =
                (dense(&moved, height, width) - &jacobian).transpose() * &multipliers / probe;
            for aim in aims {
                if let (Some(slot), Some(along)) =
                    (curvature.get_mut(aim.column), direction.get(aim.column))
                {
                    *slot += along;
                }
            }
            bent.set_column(index, &curvature);
        }
        let reduced = free.transpose() * &bent;
        let reduced = (&reduced + reduced.transpose()) * 0.5;
        let slope = free.transpose() * &pull;
        let eigen = SymmetricEigen::try_new(reduced, f64::EPSILON, SVD_ITERATIONS)?;
        let mut along = DVector::zeros(free.ncols());
        for (curvature, axis) in eigen
            .eigenvalues
            .iter()
            .zip(eigen.eigenvectors.column_iter())
        {
            along -= axis * (axis.dot(&slope) / curvature.abs().max(FLATTEST));
        }
        let step: Vec<f64> = (free * along).iter().copied().collect();
        step.iter().all(|delta| delta.is_finite()).then_some(step)
    }

    fn start(&self, part: &Part<'_>, values: &[f64]) -> Vec<f64> {
        part.component
            .variables
            .iter()
            .map(|variable| value(values, *variable))
            .collect()
    }

    fn reach_along(
        &self,
        part: &Part<'_>,
        aims: &[Aim],
        step: &[f64],
        gap: f64,
        values: &mut [f64],
    ) -> Result<Option<f64>, Cancelled> {
        let start = self.start(part, values);
        let mut fraction = 1.0;
        for _ in 0..LINE_SEARCH_STEPS {
            self.place(part.component, &start, step, fraction, values);
            if self.gauss_newton(part, values)? == Ending::Converged {
                let closer = miss(aims, values);
                if closer < gap {
                    return Ok(Some(closer));
                }
            }
            fraction *= 0.5;
        }
        self.restore(part.component, &start, values);
        Ok(None)
    }
}
