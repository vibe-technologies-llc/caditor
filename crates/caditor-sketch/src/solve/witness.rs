use std::collections::{BTreeMap, BTreeSet};

use nalgebra::{DMatrix, DVector, SymmetricEigen};

use crate::{
    id::ConstraintId,
    solve::{
        numeric::{Component, RANK_TOLERANCE, Solver},
        sparse::{self, Row, Triangular, norm, normalized},
        system::System,
    },
};

const CHORD_STEPS: usize = 8;
const CHORD_SHRINK: f64 = 0.5;
const NO_WEIGHT: f64 = 1e-12;
const EIGEN_ITERATIONS: usize = 1_000;
const TRANSFER_STEPS: usize = 8;
const TRANSFER_REACHES: [usize; 5] = [0, 1, 2, 4, 8];
const TRANSFER_SHARE: usize = 4;

pub(super) struct Strained {
    pub values: Vec<f64>,
    constraint: ConstraintId,
}

pub(super) struct Factored {
    pub component: Component,
    equations: BTreeSet<usize>,
    owners: Vec<Option<ConstraintId>>,
    values: Vec<f64>,
    lengths: Vec<f64>,
    rows: Vec<Row>,
    residuals: Vec<f64>,
    triangular: Triangular,
    base: Vec<f64>,
    left: Vec<f64>,
    tolerance: f64,
    reach: Vec<Vec<usize>>,
    touching: BTreeMap<usize, BTreeSet<usize>>,
}

impl Factored {
    pub fn at(solver: &Solver<'_>, component: &Component, values: &[f64]) -> Self {
        let (rows, residuals) = solver.linearized(component, values);
        let mut lengths = Vec::with_capacity(rows.len());
        let mut scaled = Vec::with_capacity(rows.len());
        let mut normalised = Vec::with_capacity(rows.len());
        for (row, residual) in rows.into_iter().zip(residuals) {
            let length = norm(&row);
            let length = if length > 0.0 && length.is_finite() {
                length
            } else {
                1.0
            };
            lengths.push(length);
            scaled.push(normalized(row));
            normalised.push(residual / length);
        }
        let mut triangular = Triangular::ordered_for(component.variables.len(), &scaled);
        for row in &scaled {
            triangular.insert(row, RANK_TOLERANCE);
        }
        let reach: Vec<Vec<usize>> = component
            .equations
            .iter()
            .map(|index| moving_variables(solver.system, component, *index, values))
            .collect();
        let mut touching: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for (row, variables) in reach.iter().enumerate() {
            for variable in variables {
                touching.entry(*variable).or_default().insert(row);
            }
        }
        let mut factored = Self {
            component: component.clone(),
            equations: component.equations.iter().copied().collect(),
            owners: owners(solver.system, component),
            values: values.to_vec(),
            lengths,
            rows: scaled,
            residuals: normalised,
            triangular,
            base: Vec::new(),
            left: Vec::new(),
            tolerance: solver.tolerance(component),
            reach,
            touching,
        };
        let target: Vec<f64> = factored
            .residuals
            .iter()
            .map(|residual| -residual)
            .collect();
        factored.base = factored.solve(&target);
        factored.left = factored
            .residuals
            .iter()
            .zip(factored.image(&factored.base))
            .map(|(residual, moved)| residual + moved)
            .collect();
        factored
    }

    pub fn reaches(&self, constraint: ConstraintId) -> bool {
        self.owners.contains(&Some(constraint))
    }

    pub fn contains(&self, part: &Component) -> bool {
        part.equations
            .iter()
            .all(|equation| self.equations.contains(equation))
    }

    pub fn strained(
        &self,
        solver: &Solver<'_>,
        values: Vec<f64>,
        constraint: ConstraintId,
    ) -> Option<Strained> {
        let kept_hold = self
            .component
            .equations
            .iter()
            .zip(&self.owners)
            .filter(|(_, owner)| **owner != Some(constraint))
            .all(|(index, _)| holds_alone(solver, *index, &values));
        let admissible = solver.admissible_within(&self.component, &self.component, &values)
            && within_range(solver, &self.component.variables, &values);
        (kept_hold && admissible).then_some(Strained { values, constraint })
    }

    pub fn transferred<E>(
        &mut self,
        solver: &Solver<'_>,
        from: &Strained,
        constraint: ConstraintId,
        solve_cost: usize,
        charge: &mut impl FnMut(usize) -> Result<(), E>,
    ) -> Result<Option<Strained>, E> {
        if from.constraint == constraint || !self.reaches(constraint) {
            return Ok(None);
        }
        let removed = self.rows_of(constraint);
        let mut allowance = solve_cost * removed.len();
        let mut moving: BTreeSet<usize> = self
            .rows_of(from.constraint)
            .iter()
            .chain(&removed)
            .filter_map(|row| self.reach.get(*row))
            .flatten()
            .copied()
            .collect();
        let mut frontier = moving.clone();
        let mut hops = 0;
        for reach in TRANSFER_REACHES {
            let before = moving.len();
            while hops < reach && !frontier.is_empty() {
                frontier = self.next_hop(&frontier, &removed, &mut moving);
                hops += 1;
            }
            let widened = reach == 0 || moving.len() > before;
            if !widened || moving.len() * TRANSFER_SHARE > self.component.variables.len() {
                break;
            }
            let rows = self.rows_touching(&moving, &removed);
            if let Some(values) =
                self.settled_locally(solver, &from.values, &moving, &rows, &mut allowance, charge)?
            {
                self.widen_reach(solver, &rows, &values);
                return Ok(Some(Strained { values, constraint }));
            }
        }
        Ok(None)
    }

    fn rows_of(&self, constraint: ConstraintId) -> BTreeSet<usize> {
        self.owners
            .iter()
            .enumerate()
            .filter(|(_, owner)| **owner == Some(constraint))
            .map(|(row, _)| row)
            .collect()
    }

    fn next_hop(
        &self,
        frontier: &BTreeSet<usize>,
        removed: &BTreeSet<usize>,
        moving: &mut BTreeSet<usize>,
    ) -> BTreeSet<usize> {
        let mut reached = BTreeSet::new();
        for row in self.rows_touching(frontier, removed) {
            for variable in self.reach.get(row).into_iter().flatten() {
                if moving.insert(*variable) {
                    reached.insert(*variable);
                }
            }
        }
        reached
    }

    fn rows_touching(
        &self,
        variables: &BTreeSet<usize>,
        removed: &BTreeSet<usize>,
    ) -> BTreeSet<usize> {
        variables
            .iter()
            .filter_map(|variable| self.touching.get(variable))
            .flatten()
            .filter(|row| !removed.contains(row))
            .copied()
            .collect()
    }

    fn settled_locally<E>(
        &self,
        solver: &Solver<'_>,
        from: &[f64],
        moving: &BTreeSet<usize>,
        rows: &BTreeSet<usize>,
        allowance: &mut usize,
        charge: &mut impl FnMut(usize) -> Result<(), E>,
    ) -> Result<Option<Vec<f64>>, E> {
        let mut spans: Vec<usize> = moving
            .iter()
            .filter_map(|variable| solver.system.spans_at_variable.get(variable))
            .flatten()
            .copied()
            .collect();
        spans.sort_unstable();
        spans.dedup();
        let local = Component {
            variables: moving.iter().copied().collect(),
            equations: rows
                .iter()
                .filter_map(|row| self.component.equations.get(*row))
                .copied()
                .collect(),
            spans,
        };
        let mut values = from.to_vec();
        let mut left = norm_of(&solver.residuals_of(&local, &values));
        for _ in 0..TRANSFER_STEPS {
            if self.holds_locally(solver, &local, &values) {
                return Ok(Some(values));
            }
            let Some(left_over) = allowance.checked_sub(local.equations.len()) else {
                return Ok(None);
            };
            *allowance = left_over;
            charge(local.equations.len())?;
            let (rows, residuals) = solver.linearized(&local, &values);
            let Some(step) = sparse::minimal_norm_step(&rows, &residuals, local.variables.len())
            else {
                return Ok(None);
            };
            for (variable, delta) in local.variables.iter().zip(&step) {
                if let Some(slot) = values.get_mut(*variable) {
                    *slot += delta;
                }
            }
            let now = norm_of(&solver.residuals_of(&local, &values));
            let shrank = now < CHORD_SHRINK * left;
            if !shrank {
                return Ok(None);
            }
            left = now;
        }
        Ok(self
            .holds_locally(solver, &local, &values)
            .then_some(values))
    }

    fn holds_locally(&self, solver: &Solver<'_>, local: &Component, values: &[f64]) -> bool {
        solver.admissible_within(local, &self.component, values)
            && within_range(solver, &local.variables, values)
            && local
                .equations
                .iter()
                .all(|index| holds_alone(solver, *index, values))
    }

    fn widen_reach(&mut self, solver: &Solver<'_>, rows: &BTreeSet<usize>, values: &[f64]) {
        for row in rows {
            let Some(index) = self.component.equations.get(*row) else {
                continue;
            };
            let now = moving_variables(solver.system, &self.component, *index, values);
            let Some(reach) = self.reach.get_mut(*row) else {
                continue;
            };
            for variable in now {
                if !reach.contains(&variable) {
                    reach.push(variable);
                    self.touching.entry(variable).or_default().insert(*row);
                }
            }
        }
    }

    pub fn without<E>(
        &self,
        solver: &Solver<'_>,
        constraint: ConstraintId,
        charge: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<Option<Vec<f64>>, E> {
        let removed: Vec<usize> = self
            .owners
            .iter()
            .enumerate()
            .filter(|(_, owner)| **owner == Some(constraint))
            .map(|(row, _)| row)
            .collect();
        if removed.is_empty() {
            return Ok(None);
        }
        let mut columns = Vec::with_capacity(removed.len());
        for row in &removed {
            charge()?;
            columns.push(self.solve_unit(*row));
        }
        let images: Vec<Vec<f64>> = columns
            .iter()
            .map(|column| kept_rows(self.image(column), &removed))
            .collect();
        let mut base = self.base.clone();
        let mut misfit = kept_rows(self.left.clone(), &removed);
        for ((row, column), image) in removed.iter().zip(&columns).zip(&images) {
            let load = self.residuals.get(*row).copied().unwrap_or(0.0);
            add_scaled(&mut base, column, load);
            add_scaled(&mut misfit, image, load);
        }
        let mut values = self.values.clone();
        let mut previous = kept_norm(&self.residuals, &removed);
        for step in 0..CHORD_STEPS {
            if step > 0 {
                charge()?;
                let residuals = self.normalised(solver, &values);
                let target: Vec<f64> = kept_rows(residuals.clone(), &removed)
                    .into_iter()
                    .map(|residual| -residual)
                    .collect();
                base = self.solve(&target);
                misfit = residuals
                    .iter()
                    .zip(self.image(&base))
                    .map(|(residual, moved)| residual + moved)
                    .collect();
                misfit = kept_rows(misfit, &removed);
            }
            let Some(absorbed) = absorbed(&images, &misfit, &columns, &base) else {
                return Ok(None);
            };
            let mut moved = base.clone();
            for (weight, column) in absorbed.iter().zip(&columns) {
                add_scaled(&mut moved, column, *weight);
            }
            for (variable, delta) in self.component.variables.iter().zip(&moved) {
                if let Some(slot) = values.get_mut(*variable) {
                    *slot += delta;
                }
            }
            let raw = solver.residuals_of(&self.component, &values);
            let kept = raw
                .iter()
                .enumerate()
                .filter(|(row, _)| !removed.contains(row));
            if kept.clone().any(|(_, residual)| !residual.is_finite()) {
                return Ok(None);
            }
            if kept
                .clone()
                .all(|(_, residual)| residual.abs() <= self.tolerance)
            {
                return Ok(Some(values));
            }
            let current = kept_norm(&self.scaled(raw), &removed);
            if current >= CHORD_SHRINK * previous {
                return Ok(None);
            }
            previous = current;
        }
        Ok(None)
    }

    fn normalised(&self, solver: &Solver<'_>, values: &[f64]) -> Vec<f64> {
        self.scaled(solver.residuals_of(&self.component, values))
    }

    fn scaled(&self, residuals: Vec<f64>) -> Vec<f64> {
        residuals
            .into_iter()
            .zip(&self.lengths)
            .map(|(residual, length)| residual / length)
            .collect()
    }

    fn solve(&self, target: &[f64]) -> Vec<f64> {
        let mut gradient = vec![0.0; self.component.variables.len()];
        for (row, load) in self.rows.iter().zip(target) {
            for (column, entry) in row {
                if let Some(slot) = gradient.get_mut(*column) {
                    *slot += entry * load;
                }
            }
        }
        self.triangular.normal_solution(&gradient)
    }

    fn solve_unit(&self, row: usize) -> Vec<f64> {
        let mut gradient = vec![0.0; self.component.variables.len()];
        for (column, entry) in self.rows.get(row).into_iter().flatten() {
            if let Some(slot) = gradient.get_mut(*column) {
                *slot += entry;
            }
        }
        self.triangular.normal_solution(&gradient)
    }

    fn image(&self, step: &[f64]) -> Vec<f64> {
        self.rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|(column, entry)| entry * step.get(*column).copied().unwrap_or(0.0))
                    .sum()
            })
            .collect()
    }
}

fn owners(system: &System, component: &Component) -> Vec<Option<ConstraintId>> {
    component
        .equations
        .iter()
        .map(|index| {
            system
                .equations
                .get(*index)
                .and_then(|equation| equation.owner)
        })
        .collect()
}

fn moving_variables(
    system: &System,
    component: &Component,
    index: usize,
    values: &[f64],
) -> Vec<usize> {
    system
        .equations
        .get(index)
        .map(|equation| equation.variables(values))
        .unwrap_or_default()
        .into_iter()
        .filter(|variable| component.variables.binary_search(variable).is_ok())
        .collect()
}

fn holds_alone(solver: &Solver<'_>, index: usize, values: &[f64]) -> bool {
    let Some(equation) = solver.system.equations.get(index) else {
        return false;
    };
    let alone = Component {
        variables: equation.variables(values),
        equations: vec![index],
        spans: Vec::new(),
    };
    solver.holds(&alone, values)
}

fn within_range(solver: &Solver<'_>, variables: &[usize], values: &[f64]) -> bool {
    variables
        .iter()
        .filter(|variable| solver.system.parameter_variables.contains(variable))
        .all(|variable| {
            values
                .get(*variable)
                .is_some_and(|value| (0.0..=1.0).contains(value))
        })
}

fn norm_of(residuals: &[f64]) -> f64 {
    residuals
        .iter()
        .map(|residual| residual * residual)
        .sum::<f64>()
        .sqrt()
}

fn kept_rows(mut vector: Vec<f64>, removed: &[usize]) -> Vec<f64> {
    for row in removed {
        if let Some(slot) = vector.get_mut(*row) {
            *slot = 0.0;
        }
    }
    vector
}

fn kept_norm(residuals: &[f64], removed: &[usize]) -> f64 {
    residuals
        .iter()
        .enumerate()
        .filter(|(row, _)| !removed.contains(row))
        .map(|(_, residual)| residual * residual)
        .sum::<f64>()
        .sqrt()
}

fn add_scaled(target: &mut [f64], vector: &[f64], factor: f64) {
    target
        .iter_mut()
        .zip(vector)
        .for_each(|(entry, along)| *entry += factor * along);
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn gram(vectors: &[Vec<f64>]) -> DMatrix<f64> {
    let count = vectors.len();
    DMatrix::from_fn(count, count, |i, j| {
        match (vectors.get(i), vectors.get(j)) {
            (Some(a), Some(b)) => dot(a, b),
            _ => 0.0,
        }
    })
}

fn projections(vectors: &[Vec<f64>], onto: &[f64]) -> DVector<f64> {
    DVector::from_iterator(
        vectors.len(),
        vectors.iter().map(|vector| -dot(vector, onto)),
    )
}

fn absorbed(
    images: &[Vec<f64>],
    misfit: &[f64],
    columns: &[Vec<f64>],
    base: &[f64],
) -> Option<Vec<f64>> {
    let (mut weights, free) = least_norm(gram(images), projections(images, misfit), NO_WEIGHT)?;
    if !free.is_empty() {
        let combined = |direction: &DVector<f64>| -> Vec<f64> {
            let mut sum = vec![0.0; base.len()];
            for (weight, column) in direction.iter().zip(columns) {
                add_scaled(&mut sum, column, *weight);
            }
            sum
        };
        let mut start = base.to_vec();
        add_scaled(&mut start, &combined(&weights), 1.0);
        let along: Vec<Vec<f64>> = free.iter().map(combined).collect();
        let sizes = gram(&along);
        let largest = sizes.diagonal().max();
        let (shift, _) = least_norm(sizes, projections(&along, &start), NO_WEIGHT * largest)?;
        for (amount, direction) in shift.iter().zip(&free) {
            weights += direction * *amount;
        }
    }
    let weights: Vec<f64> = weights.iter().copied().collect();
    weights
        .iter()
        .all(|weight| weight.is_finite())
        .then_some(weights)
}

fn least_norm(
    gram: DMatrix<f64>,
    rhs: DVector<f64>,
    tolerance: f64,
) -> Option<(DVector<f64>, Vec<DVector<f64>>)> {
    let eigen = SymmetricEigen::try_new(gram, f64::EPSILON, EIGEN_ITERATIONS)?;
    let mut solution = DVector::zeros(rhs.len());
    let mut free = Vec::new();
    for (value, vector) in eigen
        .eigenvalues
        .iter()
        .zip(eigen.eigenvectors.column_iter())
    {
        if *value > tolerance {
            solution += vector * (vector.dot(&rhs) / value);
        } else {
            free.push(vector.into_owned());
        }
    }
    Some((solution, free))
}
