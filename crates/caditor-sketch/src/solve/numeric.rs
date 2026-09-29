use std::collections::{BTreeMap, BTreeSet};

use nalgebra::{DMatrix, DVector, SVD};

use crate::{
    id::{ConstraintId, EntityId},
    solve::{
        equation::{Context, Equation, Gradient, PointHandle, value},
        sparse,
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
const DENSE_LIMIT: usize = 48;
pub(crate) const FROZEN: f64 = 0.0;
pub(crate) const STIFF: f64 = 1e-2;

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
        let variables = equation.variables(values);
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

pub(crate) struct Part<'a> {
    pub component: &'a Component,
    pub context: Context,
}

pub(crate) struct Solver<'a> {
    pub system: &'a System,
    pub cancelled: &'a dyn Fn() -> bool,
    pub stiff: &'a BTreeSet<usize>,
    pub stiffness: f64,
}

impl Solver<'_> {
    pub fn solve(&self, active: &[usize], values: &mut [f64]) -> Result<Vec<Component>, Cancelled> {
        let mut failed = Vec::new();
        for component in components(self.system, active, values) {
            if !self.solve_component(&self.part(&component), values)? {
                failed.push(component);
            }
        }
        Ok(failed)
    }

    pub fn part<'b>(&self, component: &'b Component) -> Part<'b> {
        Part {
            component,
            context: self.system.context_of(component),
        }
    }

    fn solve_component(&self, part: &Part<'_>, values: &mut [f64]) -> Result<bool, Cancelled> {
        if self.converged(part, values) {
            return Ok(true);
        }
        let component = part.component;
        let start: Vec<f64> = component
            .variables
            .iter()
            .map(|variable| value(values, *variable))
            .collect();
        let attempts = std::iter::once(0.0).chain(PERTURBATIONS);
        for magnitude in attempts {
            self.restore(component, &start, values);
            self.perturb(part, magnitude, values);
            if self.gauss_newton(part, values)? {
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

    fn perturb(&self, part: &Part<'_>, magnitude: f64, values: &mut [f64]) {
        if magnitude == 0.0 {
            return;
        }
        let offset = magnitude * self.extent(part, values);
        for (order, variable) in part.component.variables.iter().enumerate() {
            if self.system.radius_variables.contains(variable) {
                continue;
            }
            let pattern = ((order + 1) as f64 * GOLDEN_RATIO_FRACTION).fract() * 2.0 - 1.0;
            if let Some(slot) = values.get_mut(*variable) {
                *slot += offset * pattern;
            }
        }
    }

    fn extent(&self, part: &Part<'_>, values: &[f64]) -> f64 {
        let (mut low, mut high, mut radius) = (f64::INFINITY, f64::NEG_INFINITY, 0.0_f64);
        for variable in &part.component.variables {
            let current = value(values, *variable);
            if !current.is_finite() {
                continue;
            }
            if self.system.radius_variables.contains(variable) {
                radius = radius.max(current.abs());
            } else {
                low = low.min(current);
                high = high.max(current);
            }
        }
        let extent = (high - low).max(radius);
        if extent.is_finite() && extent > part.context.degenerate_length {
            extent
        } else {
            part.context.scale
        }
    }

    fn equations<'b>(&'b self, component: &'b Component) -> impl Iterator<Item = &'b Equation> {
        component
            .equations
            .iter()
            .filter_map(|index| self.system.equations.get(*index))
    }

    fn residuals(&self, part: &Part<'_>, values: &[f64]) -> Vec<f64> {
        self.equations(part.component)
            .map(|equation| equation.residual(values, &part.context))
            .collect()
    }

    fn admissible(&self, part: &Part<'_>, values: &[f64]) -> bool {
        let finite = part.component.variables.iter().all(|variable| {
            let current = value(values, *variable);
            current.is_finite()
                && (!self.system.radius_variables.contains(variable) || current > 0.0)
        });
        finite && !self.collapses(part, values)
    }

    fn collapses(&self, part: &Part<'_>, values: &[f64]) -> bool {
        self.collapsed(part, values).next().is_some()
    }

    pub(crate) fn collapsed<'b>(
        &'b self,
        part: &'b Part<'b>,
        values: &'b [f64],
    ) -> impl Iterator<Item = EntityId> + 'b {
        let moves = |handle: &PointHandle| match handle {
            PointHandle::Variable(x) => part.component.variables.binary_search(x).is_ok(),
            PointHandle::Fixed(_) => false,
        };
        let collapsed_length = part.context.collapsed_length();
        self.system
            .spans
            .iter()
            .filter(move |(_, from, to)| {
                (moves(from) || moves(to))
                    && from.at(values).distance(to.at(values)) <= collapsed_length
            })
            .map(|(entity, _, _)| *entity)
    }

    fn converged(&self, part: &Part<'_>, values: &[f64]) -> bool {
        let tolerance = CONVERGENCE_TOLERANCE * part.context.scale;
        self.admissible(part, values)
            && self
                .residuals(part, values)
                .iter()
                .all(|residual| residual.abs() <= tolerance)
    }

    fn gauss_newton(&self, part: &Part<'_>, values: &mut [f64]) -> Result<bool, Cancelled> {
        let component = part.component;
        let scale = part.context.scale;
        for _ in 0..MAX_ITERATIONS {
            if (self.cancelled)() {
                return Err(Cancelled);
            }
            if self.converged(part, values) {
                return Ok(true);
            }
            let scales: Vec<f64> = component
                .variables
                .iter()
                .map(|variable| {
                    if self.stiff.contains(variable) {
                        self.stiffness
                    } else {
                        1.0
                    }
                })
                .collect();
            let step = if component.variables.len() > DENSE_LIMIT {
                let (rows, residuals) = self.sparse_linearize(part, values);
                let rows: Vec<sparse::Row> = rows
                    .into_iter()
                    .map(|row| {
                        row.into_iter()
                            .map(|(column, value)| {
                                (column, value * scales.get(column).copied().unwrap_or(1.0))
                            })
                            .collect()
                    })
                    .collect();
                sparse::minimal_norm_step(&rows, &residuals, component.variables.len())
            } else {
                let (rows, residuals) = self.linearize(part, values);
                let rows: Vec<Vec<f64>> = rows
                    .into_iter()
                    .map(|row| {
                        row.iter()
                            .zip(&scales)
                            .map(|(value, scale)| value * scale)
                            .collect()
                    })
                    .collect();
                minimal_norm_step(&rows, &residuals, component.variables.len())
            };
            let Some(mut step) = step else {
                return Ok(false);
            };
            step.iter_mut()
                .zip(&scales)
                .for_each(|(delta, scale)| *delta *= scale);
            let length = step.iter().map(|delta| delta * delta).sum::<f64>().sqrt();
            if length > MAX_STEP * scale {
                let shrink = MAX_STEP * scale / length;
                step.iter_mut().for_each(|delta| *delta *= shrink);
            }
            if !self.line_search(part, &step, values) {
                return Ok(false);
            }
        }
        Ok(self.converged(part, values))
    }

    fn line_search(&self, part: &Part<'_>, step: &[f64], values: &mut [f64]) -> bool {
        let component = part.component;
        let cost = |values: &[f64]| -> f64 {
            self.residuals(part, values)
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
            if self.admissible(part, &trial) && candidate.is_finite() && candidate < current {
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

    fn linearize(&self, part: &Part<'_>, values: &[f64]) -> (Vec<Vec<f64>>, Vec<f64>) {
        let component = part.component;
        let mut gradient = Gradient::new();
        let mut rows = Vec::with_capacity(component.equations.len());
        let mut residuals = Vec::with_capacity(component.equations.len());
        for equation in self.equations(component) {
            let residual = equation.linearize(values, &part.context, &mut gradient);
            rows.push(dense_row(&gradient, &component.variables));
            residuals.push(residual);
        }
        (rows, residuals)
    }

    fn sparse_linearize(&self, part: &Part<'_>, values: &[f64]) -> (Vec<sparse::Row>, Vec<f64>) {
        let component = part.component;
        let mut gradient = Gradient::new();
        let mut rows = Vec::with_capacity(component.equations.len());
        let mut residuals = Vec::with_capacity(component.equations.len());
        for equation in self.equations(component) {
            let residual = equation.linearize(values, &part.context, &mut gradient);
            rows.push(sparse::row(&gradient, &component.variables));
            residuals.push(residual);
        }
        (rows, residuals)
    }

    pub fn analyze_component(&self, component: &Component, values: &[f64]) -> ComponentAnalysis {
        let mut analysis = ComponentAnalysis::default();
        let mut contributions = BTreeMap::new();
        let part = self.part(component);
        if component.variables.len() > DENSE_LIMIT {
            self.analyze_sparse(&part, values, &mut analysis, &mut contributions);
        } else {
            self.analyze_dense(&part, values, &mut analysis, &mut contributions);
        }
        analysis.contributions = contributions
            .into_iter()
            .map(|(constraint, contribution)| {
                let duplicates = contribution.duplicates.into_iter().collect();
                (constraint, contribution.adds_rank, duplicates)
            })
            .collect();
        analysis
    }

    fn analyze_dense(
        &self,
        part: &Part<'_>,
        values: &[f64],
        analysis: &mut ComponentAnalysis,
        contributions: &mut BTreeMap<ConstraintId, Contribution>,
    ) {
        let component = part.component;
        let mut groups: BTreeMap<Option<ConstraintId>, Vec<Vec<f64>>> = BTreeMap::new();
        let mut gradient = Gradient::new();
        for equation in self.equations(component) {
            equation.linearize(values, &part.context, &mut gradient);
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

impl Solver<'_> {
    fn analyze_sparse(
        &self,
        part: &Part<'_>,
        values: &[f64],
        analysis: &mut ComponentAnalysis,
        contributions: &mut BTreeMap<ConstraintId, Contribution>,
    ) {
        let component = part.component;
        let mut groups: BTreeMap<Option<ConstraintId>, Vec<sparse::Row>> = BTreeMap::new();
        let mut gradient = Gradient::new();
        for equation in self.equations(component) {
            equation.linearize(values, &part.context, &mut gradient);
            groups
                .entry(equation.owner)
                .or_default()
                .push(sparse::normalized(sparse::row(
                    &gradient,
                    &component.variables,
                )));
        }
        let mut echelon = sparse::Echelon::default();
        let mut earlier: Vec<(Option<ConstraintId>, sparse::Row)> = Vec::new();
        for (owner, rows) in groups {
            let mut adds_rank = false;
            for row in &rows {
                adds_rank |= echelon.insert(row, RANK_TOLERANCE);
            }
            if let Some(constraint) = owner {
                let contribution = contributions.entry(constraint).or_default();
                contribution.adds_rank |= adds_rank;
                if !contribution.adds_rank {
                    contribution.duplicates.extend(sparse::duplicates(
                        &rows,
                        &earlier,
                        component.variables.len(),
                    ));
                }
            }
            earlier.extend(rows.into_iter().map(|row| (owner, row)));
        }
        analysis.rank += echelon.rank();
        for (column, variable) in component.variables.iter().enumerate() {
            if echelon.spans_unit(column, NULL_SPACE_TOLERANCE.sqrt()) {
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

impl Analysis {
    pub fn combine(parts: impl IntoIterator<Item = ComponentAnalysis>) -> Self {
        let mut analysis = Self::default();
        let mut contributions: BTreeMap<ConstraintId, Contribution> = BTreeMap::new();
        for part in parts {
            analysis.rank += part.rank;
            analysis.fixed.extend(part.fixed);
            for (constraint, adds_rank, duplicates) in part.contributions {
                let contribution = contributions.entry(constraint).or_default();
                contribution.adds_rank |= adds_rank;
                contribution.duplicates.extend(duplicates);
            }
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
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ComponentAnalysis {
    pub rank: usize,
    pub fixed: Vec<usize>,
    pub contributions: Vec<(ConstraintId, bool, Vec<ConstraintId>)>,
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

pub(crate) fn duplicates(
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
