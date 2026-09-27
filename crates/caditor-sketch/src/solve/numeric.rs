use std::collections::{BTreeMap, BTreeSet};

use nalgebra::{DMatrix, DVector, SVD};

use crate::{
    id::ConstraintId,
    solve::{
        equation::{Equation, Gradient, value},
        system::System,
    },
};

const CONVERGENCE_TOLERANCE: f64 = 1e-10;
const MAX_ITERATIONS: usize = 100;
const LINE_SEARCH_STEPS: usize = 40;
const MAX_STEP: f64 = 1.0;
const STEP_RANK_TOLERANCE: f64 = 1e-10;
const SVD_ITERATIONS: usize = 100_000;
const PERTURBATIONS: [f64; 2] = [1e-3, 3e-2];
const GOLDEN_RATIO_FRACTION: f64 = 0.618_033_988_749_894_9;
const RANK_TOLERANCE: f64 = 1e-8;
const NULL_SPACE_TOLERANCE: f64 = 1e-10;
const DUPLICATE_TOLERANCE: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cancelled;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Component {
    pub variables: Vec<usize>,
    pub equations: Vec<usize>,
}

pub(crate) fn components(system: &System, active: &[usize], values: &[f64]) -> Vec<Component> {
    let mut parents: BTreeMap<usize, usize> = BTreeMap::new();
    let mut constant = Vec::new();
    let mut linked = Vec::new();
    for &index in active {
        let Some(equation) = system.equations.get(index) else {
            continue;
        };
        let variables = equation.variables(values, &system.context);
        let Some(&first) = variables.first() else {
            constant.push(index);
            continue;
        };
        for variable in &variables {
            union(&mut parents, first, *variable);
        }
        linked.push((index, first));
    }
    let mut groups: BTreeMap<usize, Component> = BTreeMap::new();
    for (index, first) in linked {
        let root = find(&mut parents, first);
        groups
            .entry(root)
            .or_insert_with(|| Component {
                variables: Vec::new(),
                equations: Vec::new(),
            })
            .equations
            .push(index);
    }
    let variables: Vec<usize> = parents.keys().copied().collect();
    for variable in variables {
        let root = find(&mut parents, variable);
        if let Some(group) = groups.get_mut(&root) {
            group.variables.push(variable);
        }
    }
    groups
        .into_values()
        .chain(constant.into_iter().map(|index| Component {
            variables: Vec::new(),
            equations: vec![index],
        }))
        .collect()
}

fn find(parents: &mut BTreeMap<usize, usize>, variable: usize) -> usize {
    let mut root = variable;
    while let Some(&parent) = parents.get(&root) {
        if parent == root {
            break;
        }
        root = parent;
    }
    let mut current = variable;
    while current != root {
        let next = parents.get(&current).copied().unwrap_or(root);
        parents.insert(current, root);
        current = next;
    }
    parents.entry(root).or_insert(root);
    root
}

fn union(parents: &mut BTreeMap<usize, usize>, a: usize, b: usize) {
    let (a, b) = (find(parents, a), find(parents, b));
    if a != b {
        parents.insert(a.max(b), a.min(b));
    }
}

pub(crate) struct Solver<'a> {
    pub system: &'a System,
    pub cancelled: &'a dyn Fn() -> bool,
}

impl Solver<'_> {
    pub fn solve(&self, active: &[usize], values: &mut [f64]) -> Result<Vec<Component>, Cancelled> {
        let mut failed = Vec::new();
        for component in components(self.system, active, values) {
            if !self.solve_component(&component, values)? {
                failed.push(component);
            }
        }
        Ok(failed)
    }

    fn solve_component(
        &self,
        component: &Component,
        values: &mut [f64],
    ) -> Result<bool, Cancelled> {
        if self.converged(component, values) {
            return Ok(true);
        }
        let start: Vec<f64> = component
            .variables
            .iter()
            .map(|variable| value(values, *variable))
            .collect();
        let attempts = std::iter::once(0.0).chain(PERTURBATIONS);
        for magnitude in attempts {
            self.restore(component, &start, values);
            self.perturb(component, magnitude, values);
            if self.gauss_newton(component, values)? {
                return Ok(true);
            }
        }
        self.restore(component, &start, values);
        Ok(false)
    }

    fn restore(&self, component: &Component, start: &[f64], values: &mut [f64]) {
        for (variable, original) in component.variables.iter().zip(start) {
            if let Some(slot) = values.get_mut(*variable) {
                *slot = *original;
            }
        }
    }

    fn perturb(&self, component: &Component, magnitude: f64, values: &mut [f64]) {
        if magnitude == 0.0 {
            return;
        }
        let offset = magnitude * self.system.context.scale;
        for (order, variable) in component.variables.iter().enumerate() {
            if self.system.radius_variables.contains(variable) {
                continue;
            }
            let pattern = ((order + 1) as f64 * GOLDEN_RATIO_FRACTION).fract() * 2.0 - 1.0;
            if let Some(slot) = values.get_mut(*variable) {
                *slot += offset * pattern;
            }
        }
    }

    fn tolerance(&self) -> f64 {
        CONVERGENCE_TOLERANCE * self.system.context.scale
    }

    fn equations<'b>(&'b self, component: &'b Component) -> impl Iterator<Item = &'b Equation> {
        component
            .equations
            .iter()
            .filter_map(|index| self.system.equations.get(*index))
    }

    fn residuals(&self, component: &Component, values: &[f64]) -> Vec<f64> {
        self.equations(component)
            .map(|equation| equation.residual(values, &self.system.context))
            .collect()
    }

    fn admissible(&self, component: &Component, values: &[f64]) -> bool {
        component.variables.iter().all(|variable| {
            let current = value(values, *variable);
            current.is_finite()
                && (!self.system.radius_variables.contains(variable) || current > 0.0)
        })
    }

    fn converged(&self, component: &Component, values: &[f64]) -> bool {
        let tolerance = self.tolerance();
        self.admissible(component, values)
            && self
                .residuals(component, values)
                .iter()
                .all(|residual| residual.abs() <= tolerance)
    }

    fn gauss_newton(&self, component: &Component, values: &mut [f64]) -> Result<bool, Cancelled> {
        let scale = self.system.context.scale;
        for _ in 0..MAX_ITERATIONS {
            if (self.cancelled)() {
                return Err(Cancelled);
            }
            if self.converged(component, values) {
                return Ok(true);
            }
            let (rows, residuals) = self.linearize(component, values);
            let Some(mut step) = minimal_norm_step(&rows, &residuals, component.variables.len())
            else {
                return Ok(false);
            };
            let length = step.iter().map(|delta| delta * delta).sum::<f64>().sqrt();
            if length > MAX_STEP * scale {
                let shrink = MAX_STEP * scale / length;
                step.iter_mut().for_each(|delta| *delta *= shrink);
            }
            if !self.line_search(component, &step, values) {
                return Ok(false);
            }
        }
        Ok(self.converged(component, values))
    }

    fn line_search(&self, component: &Component, step: &[f64], values: &mut [f64]) -> bool {
        let cost = |values: &[f64]| -> f64 {
            self.residuals(component, values)
                .iter()
                .map(|residual| residual * residual)
                .sum()
        };
        let current = cost(values);
        let mut trial = values.to_vec();
        let mut fraction = 1.0;
        for _ in 0..LINE_SEARCH_STEPS {
            for (variable, delta) in component.variables.iter().zip(step) {
                if let (Some(slot), Some(start)) = (trial.get_mut(*variable), values.get(*variable))
                {
                    *slot = start + fraction * delta;
                }
            }
            let candidate = cost(&trial);
            if self.admissible(component, &trial) && candidate.is_finite() && candidate < current {
                for variable in &component.variables {
                    if let (Some(slot), Some(accepted)) =
                        (values.get_mut(*variable), trial.get(*variable))
                    {
                        *slot = *accepted;
                    }
                }
                return true;
            }
            fraction *= 0.5;
        }
        false
    }

    fn linearize(&self, component: &Component, values: &[f64]) -> (Vec<Vec<f64>>, Vec<f64>) {
        let mut gradient = Gradient::new();
        let mut rows = Vec::with_capacity(component.equations.len());
        let mut residuals = Vec::with_capacity(component.equations.len());
        for equation in self.equations(component) {
            let residual = equation.linearize(values, &self.system.context, &mut gradient);
            rows.push(dense_row(&gradient, &component.variables));
            residuals.push(residual);
        }
        (rows, residuals)
    }

    pub fn analyze(&self, active: &[usize], values: &[f64]) -> Analysis {
        let mut analysis = Analysis::default();
        let mut contributions = BTreeMap::new();
        for component in components(self.system, active, values) {
            self.analyze_component(&component, values, &mut analysis, &mut contributions);
        }
        analysis.redundancies = contributions
            .into_iter()
            .filter(|(_, contribution)| !contribution.adds_rank)
            .map(|(constraint, contribution)| Redundancy {
                constraint,
                duplicates: contribution.duplicates.into_iter().collect(),
            })
            .collect();
        analysis
    }

    fn analyze_component(
        &self,
        component: &Component,
        values: &[f64],
        analysis: &mut Analysis,
        contributions: &mut BTreeMap<ConstraintId, Contribution>,
    ) {
        let mut groups: BTreeMap<Option<ConstraintId>, Vec<Vec<f64>>> = BTreeMap::new();
        let mut gradient = Gradient::new();
        for equation in self.equations(component) {
            equation.linearize(values, &self.system.context, &mut gradient);
            groups
                .entry(equation.owner)
                .or_default()
                .push(normalized(dense_row(&gradient, &component.variables)));
        }

        let mut basis: Vec<Vec<f64>> = Vec::new();
        let mut earlier: Vec<(Option<ConstraintId>, Vec<f64>)> = Vec::new();
        for (owner, rows) in groups {
            let adds_rank = rows
                .iter()
                .any(|row| norm(&orthogonalized(row, &basis)) > RANK_TOLERANCE);
            if let Some(constraint) = owner {
                let contribution = contributions.entry(constraint).or_default();
                contribution.adds_rank |= adds_rank;
                if !contribution.adds_rank {
                    contribution.duplicates.extend(duplicates(&rows, &earlier));
                }
            }
            for row in &rows {
                let remainder = orthogonalized(row, &basis);
                let length = norm(&remainder);
                if length > RANK_TOLERANCE {
                    basis.push(remainder.iter().map(|entry| entry / length).collect());
                }
            }
            earlier.extend(rows.into_iter().map(|row| (owner, row)));
        }

        analysis.rank += basis.len();
        for (column, variable) in component.variables.iter().enumerate() {
            let in_row_space: f64 = basis
                .iter()
                .filter_map(|vector| vector.get(column))
                .map(|entry| entry * entry)
                .sum();
            if 1.0 - in_row_space <= NULL_SPACE_TOLERANCE {
                analysis.fixed.push(*variable);
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Analysis {
    pub rank: usize,
    pub fixed: Vec<usize>,
    pub redundancies: Vec<Redundancy>,
}

#[derive(Debug, Clone, Default)]
struct Contribution {
    adds_rank: bool,
    duplicates: BTreeSet<ConstraintId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redundancy {
    pub constraint: ConstraintId,
    pub duplicates: Vec<ConstraintId>,
}

fn dense_row(gradient: &Gradient, variables: &[usize]) -> Vec<f64> {
    let mut row = vec![0.0; variables.len()];
    for (variable, partial) in gradient {
        if let Ok(column) = variables.binary_search(variable)
            && let Some(entry) = row.get_mut(column)
        {
            *entry += partial;
        }
    }
    row
}

fn norm(row: &[f64]) -> f64 {
    row.iter().map(|entry| entry * entry).sum::<f64>().sqrt()
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn normalized(row: Vec<f64>) -> Vec<f64> {
    let length = norm(&row);
    if length > 0.0 && length.is_finite() {
        row.into_iter().map(|entry| entry / length).collect()
    } else {
        row.into_iter().map(|_| 0.0).collect()
    }
}

fn orthogonalized(row: &[f64], basis: &[Vec<f64>]) -> Vec<f64> {
    let mut remainder = row.to_vec();
    for _ in 0..2 {
        for vector in basis {
            let projection = dot(&remainder, vector);
            remainder
                .iter_mut()
                .zip(vector)
                .for_each(|(entry, basis_entry)| *entry -= projection * basis_entry);
        }
    }
    remainder
}

fn duplicates(
    rows: &[Vec<f64>],
    earlier: &[(Option<ConstraintId>, Vec<f64>)],
) -> Vec<ConstraintId> {
    let Some(width) = rows.first().map(Vec::len) else {
        return Vec::new();
    };
    if earlier.is_empty() || width == 0 {
        return Vec::new();
    }
    let transposed = DMatrix::from_fn(width, earlier.len(), |row, column| {
        earlier
            .get(column)
            .and_then(|(_, values)| values.get(row))
            .copied()
            .unwrap_or(0.0)
    });
    let Some(svd) = SVD::try_new(transposed, true, true, f64::EPSILON, SVD_ITERATIONS) else {
        return Vec::new();
    };
    let cutoff = svd.singular_values.max() * STEP_RANK_TOLERANCE;
    let mut weights: BTreeMap<ConstraintId, f64> = BTreeMap::new();
    for row in rows {
        let Ok(coefficients) = svd.solve(&DVector::from_column_slice(row), cutoff) else {
            continue;
        };
        for ((owner, _), coefficient) in earlier.iter().zip(coefficients.iter()) {
            if let Some(owner) = owner {
                *weights.entry(*owner).or_default() += coefficient.abs();
            }
        }
    }
    let largest = weights.values().copied().fold(0.0, f64::max);
    weights
        .into_iter()
        .filter(|(_, weight)| *weight > DUPLICATE_TOLERANCE * largest && *weight > 0.0)
        .map(|(owner, _)| owner)
        .collect()
}

fn minimal_norm_step(rows: &[Vec<f64>], residuals: &[f64], width: usize) -> Option<Vec<f64>> {
    if rows.is_empty() || width == 0 {
        return None;
    }
    let mut scaled_residuals = Vec::with_capacity(residuals.len());
    let mut data = Vec::with_capacity(rows.len() * width);
    for (row, residual) in rows.iter().zip(residuals) {
        let length = norm(row);
        let divisor = if length > 0.0 { length } else { 1.0 };
        data.extend(row.iter().map(|entry| entry / divisor));
        scaled_residuals.push(-residual / divisor);
    }
    if data
        .iter()
        .chain(&scaled_residuals)
        .any(|entry| !entry.is_finite())
    {
        return None;
    }
    let jacobian = DMatrix::from_row_slice(rows.len(), width, &data);
    let svd = SVD::try_new(jacobian, true, true, f64::EPSILON, SVD_ITERATIONS)?;
    let cutoff = svd.singular_values.max() * STEP_RANK_TOLERANCE;
    let step = svd
        .solve(&DVector::from_vec(scaled_residuals), cutoff)
        .ok()?;
    let step: Vec<f64> = step.iter().copied().collect();
    step.iter().all(|delta| delta.is_finite()).then_some(step)
}
