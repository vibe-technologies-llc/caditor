use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

use nalgebra::{DMatrix, DVector, SVD};

use crate::{
    id::{ConstraintId, EntityId},
    solve::{
        equation::{Context, Equation, Gradient, value},
        sparse,
        system::System,
        tally,
    },
};

const CONVERGENCE_TOLERANCE: f64 = 1e-10;
const CLEAR_RESIDUAL: f64 = 100.0;
const PRESSED_MARGIN: f64 = 16.0;
const SUPPORT_SHARE: f64 = 1e-3;
const MAX_ITERATIONS: usize = 100;
const LINE_SEARCH_STEPS: usize = 40;
const STALLED_COST_RATIO: f64 = 0.999;
const STALLED_ITERATIONS: usize = 3;
const MAX_STEP: f64 = 1.0;
const STEP_RANK_TOLERANCE: f64 = 1e-10;
const SVD_ITERATIONS: usize = 100_000;
const PERTURBATIONS: [f64; 2] = [1e-3, 3e-2];
const GOLDEN_RATIO_FRACTION: f64 = 0.618_033_988_749_894_9;
const RANK_TOLERANCE: f64 = 1e-8;
const NULL_SPACE_TOLERANCE: f64 = 1e-10;
const DUPLICATE_TOLERANCE: f64 = 1e-6;
pub(super) const DENSE_LIMIT: usize = 48;
pub(crate) const FROZEN: f64 = 0.0;
pub(crate) const STIFF: f64 = 1e-2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cancelled;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Component {
    pub variables: Vec<usize>,
    pub equations: Vec<usize>,
    pub spans: Vec<usize>,
}

pub(crate) fn components(system: &System, active: &[usize], values: &[f64]) -> Vec<Component> {
    let mut joints = Joints::default();
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
            joints.union(first, *variable);
        }
        linked.push((index, first));
    }
    let mut group_of_root = vec![None; joints.len()];
    let mut groups: Vec<(usize, Component)> = Vec::new();
    for (index, first) in linked {
        let root = joints.find(first);
        let group = match group_of_root.get(root).copied().flatten() {
            Some(group) => group,
            None => {
                groups.push((
                    root,
                    Component {
                        variables: Vec::new(),
                        equations: Vec::new(),
                        spans: Vec::new(),
                    },
                ));
                let group = groups.len() - 1;
                if let Some(slot) = group_of_root.get_mut(root) {
                    *slot = Some(group);
                }
                group
            }
        };
        if let Some((_, component)) = groups.get_mut(group) {
            component.equations.push(index);
        }
    }
    for variable in 0..joints.len() {
        if !joints.contains(variable) {
            continue;
        }
        let root = joints.find(variable);
        if let Some((_, component)) = group_of_root
            .get(root)
            .copied()
            .flatten()
            .and_then(|group| groups.get_mut(group))
        {
            component.variables.push(variable);
        }
    }
    for (_, group) in &mut groups {
        let mut spans: Vec<usize> = group
            .variables
            .iter()
            .filter_map(|variable| system.spans_at_variable.get(variable))
            .flatten()
            .copied()
            .collect();
        spans.sort_unstable();
        spans.dedup();
        group.spans = spans;
    }
    groups.sort_by_key(|(root, _)| *root);
    groups
        .into_iter()
        .map(|(_, component)| component)
        .chain(constant.into_iter().map(|index| Component {
            variables: Vec::new(),
            equations: vec![index],
            spans: Vec::new(),
        }))
        .collect()
}

pub(crate) struct Parts {
    drawn: Vec<Component>,
    follow_values: Option<Vec<usize>>,
}

impl Parts {
    pub fn of(system: &System) -> Self {
        let every_equation: Vec<usize> = (0..system.equations.len()).collect();
        let drawn = components(system, &every_equation, &system.values);
        let follow_values = system.supports_move_with_values().then_some(every_equation);
        Self {
            drawn,
            follow_values,
        }
    }

    pub fn drawn(&self) -> &[Component] {
        &self.drawn
    }

    pub fn at(&self, system: &System, values: &[f64]) -> Cow<'_, [Component]> {
        match &self.follow_values {
            Some(every_equation) => Cow::Owned(components(system, every_equation, values)),
            None => Cow::Borrowed(&self.drawn),
        }
    }
}

#[derive(Default)]
struct Joints {
    parents: Vec<Option<usize>>,
}

impl Joints {
    fn len(&self) -> usize {
        self.parents.len()
    }

    fn contains(&self, variable: usize) -> bool {
        self.parents.get(variable).copied().flatten().is_some()
    }

    fn parent(&mut self, variable: usize) -> usize {
        if variable >= self.parents.len() {
            self.parents.resize(variable + 1, None);
        }
        match self.parents.get_mut(variable) {
            Some(Some(parent)) => *parent,
            Some(slot) => {
                *slot = Some(variable);
                variable
            }
            None => variable,
        }
    }

    fn find(&mut self, variable: usize) -> usize {
        let mut root = variable;
        loop {
            let parent = self.parent(root);
            if parent == root {
                break;
            }
            root = parent;
        }
        let mut current = variable;
        while current != root {
            let next = self.parent(current);
            if let Some(slot) = self.parents.get_mut(current) {
                *slot = Some(root);
            }
            current = next;
        }
        root
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b
            && let Some(slot) = self.parents.get_mut(a.max(b))
        {
            *slot = Some(a.min(b));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Elimination {
    Dense,
    Sparse,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    Converged,
    Minimum,
    Collapse,
    Unfinished,
}

enum Linearized {
    Dense(Vec<Vec<f64>>, Vec<f64>),
    Sparse(Vec<sparse::Row>, Vec<f64>),
}

impl Linearized {
    fn step(&self, scales: &[f64], width: usize) -> Option<Vec<f64>> {
        let scale_of = |column: usize| scales.get(column).copied().unwrap_or(1.0);
        let mut step = match self {
            Self::Dense(rows, residuals) => {
                let rows: Vec<Vec<f64>> = rows
                    .iter()
                    .map(|row| {
                        row.iter()
                            .enumerate()
                            .map(|(column, value)| value * scale_of(column))
                            .collect()
                    })
                    .collect();
                minimal_norm_step(&rows, residuals, width)
            }
            Self::Sparse(rows, residuals) => {
                let rows: Vec<sparse::Row> = rows
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|(column, value)| (*column, value * scale_of(*column)))
                            .collect()
                    })
                    .collect();
                sparse::minimal_norm_step(&rows, residuals, width)
            }
        }?;
        step.iter_mut()
            .enumerate()
            .for_each(|(column, delta)| *delta *= scale_of(column));
        Some(step)
    }

    fn left_after(&self, step: &[f64]) -> Vec<(f64, f64)> {
        let along = |column: usize| step.get(column).copied().unwrap_or(0.0);
        let left = |residual: f64, moved: f64, length: f64| {
            let remaining = residual + moved;
            let divisor = if length > 0.0 { length } else { 1.0 };
            (remaining, remaining / divisor)
        };
        match self {
            Self::Dense(rows, residuals) => rows
                .iter()
                .zip(residuals)
                .map(|(row, residual)| {
                    let moved = row
                        .iter()
                        .enumerate()
                        .map(|(column, value)| value * along(column))
                        .sum();
                    left(*residual, moved, norm(row))
                })
                .collect(),
            Self::Sparse(rows, residuals) => rows
                .iter()
                .zip(residuals)
                .map(|(row, residual)| {
                    let moved = row
                        .iter()
                        .map(|(column, value)| value * along(*column))
                        .sum();
                    let length = row
                        .iter()
                        .map(|(_, value)| value * value)
                        .sum::<f64>()
                        .sqrt();
                    left(*residual, moved, length)
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Settled {
    pub end: Vec<f64>,
    pub cost: f64,
    pub conclusive: bool,
    pub pressed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Descent {
    Solved,
    Failed(Settled),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Failure {
    pub component: Component,
    pub settled: Settled,
}

impl Solver<'_> {
    #[cfg(test)]
    pub fn solve(&self, active: &[usize], values: &mut [f64]) -> Result<Vec<Failure>, Cancelled> {
        let parts = components(self.system, active, values);
        self.solve_parts(&parts, values)
    }

    pub fn solve_parts(
        &self,
        parts: &[Component],
        values: &mut [f64],
    ) -> Result<Vec<Failure>, Cancelled> {
        let mut failed = Vec::new();
        for component in parts {
            if let Descent::Failed(settled) = self.solve_component(&self.part(component), values)? {
                failed.push(Failure {
                    component: component.clone(),
                    settled,
                });
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

    pub fn descend(&self, component: &Component, values: &mut [f64]) -> Result<Descent, Cancelled> {
        self.solve_component(&self.part(component), values)
    }

    pub fn cost(&self, component: &Component, values: &[f64]) -> f64 {
        squared_sum(&self.residuals(&self.part(component), values))
    }

    pub fn irreducible(&self, component: &Component, values: &[f64]) -> Vec<usize> {
        let part = self.part(component);
        let clear = clear_residual(&part);
        let (linearized, step) = self.bounded_step(&part, values);
        let Some(step) = step else {
            return Vec::new();
        };
        let left = linearized.left_after(&step);
        let largest = left
            .iter()
            .map(|(_, along_gradient)| along_gradient.abs())
            .fold(0.0, f64::max);
        component
            .equations
            .iter()
            .zip(left)
            .filter(|(_, (residual, along_gradient))| {
                residual.abs() > clear && along_gradient.abs() >= SUPPORT_SHARE * largest
            })
            .map(|(index, _)| *index)
            .collect()
    }

    fn solve_component(&self, part: &Part<'_>, values: &mut [f64]) -> Result<Descent, Cancelled> {
        if self.converged(part, values) {
            return Ok(Descent::Solved);
        }
        let component = part.component;
        let start: Vec<f64> = component
            .variables
            .iter()
            .map(|variable| value(values, *variable))
            .collect();
        let mut lowest_cost = squared_sum(&self.residuals(part, values));
        let mut lowest_end = start.clone();
        let mut pressed = false;
        let mut unperturbed_contradicts = None;
        let attempts = std::iter::once(0.0).chain(PERTURBATIONS);
        for magnitude in attempts {
            self.restore(component, &start, values);
            self.perturb(part, magnitude, values);
            let ending = self.gauss_newton(part, values)?;
            if ending == Ending::Converged {
                return Ok(Descent::Solved);
            }
            unperturbed_contradicts.get_or_insert_with(|| self.contradicts(part, values, ending));
            let cost = squared_sum(&self.residuals(part, values));
            if cost < lowest_cost {
                lowest_cost = cost;
                lowest_end = component
                    .variables
                    .iter()
                    .map(|variable| value(values, *variable))
                    .collect();
                pressed = ending == Ending::Collapse;
            }
        }
        self.restore(component, &start, values);
        Ok(Descent::Failed(Settled {
            end: lowest_end,
            cost: lowest_cost,
            conclusive: unperturbed_contradicts == Some(true),
            pressed,
        }))
    }

    fn contradicts(&self, part: &Part<'_>, values: &[f64], ending: Ending) -> bool {
        match ending {
            Ending::Collapse => true,
            Ending::Minimum => {
                let clear = clear_residual(part);
                self.residuals(part, values)
                    .iter()
                    .any(|residual| residual.abs() > clear)
            }
            Ending::Converged | Ending::Unfinished => false,
        }
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
            if self.system.radius_variables.contains(variable)
                || self.system.parameter_variables.contains(variable)
            {
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
            if !current.is_finite() || self.system.parameter_variables.contains(variable) {
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
        self.spans_within(part, values, part.context.collapsed_length())
    }

    fn spans_within<'b>(
        &'b self,
        part: &'b Part<'b>,
        values: &'b [f64],
        limit: f64,
    ) -> impl Iterator<Item = EntityId> + 'b {
        part.component
            .spans
            .iter()
            .filter_map(|index| self.system.spans.get(*index))
            .filter(move |(_, from, to)| from.at(values).distance(to.at(values)) <= limit)
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

    fn gauss_newton(&self, part: &Part<'_>, values: &mut [f64]) -> Result<Ending, Cancelled> {
        let component = part.component;
        let scale = part.context.scale;
        if self.converged(part, values) {
            return Ok(Ending::Converged);
        }
        if component.variables.is_empty() {
            return Ok(Ending::Minimum);
        }
        let mut stalled = 0;
        for _ in 0..MAX_ITERATIONS {
            if (self.cancelled)() {
                return Err(Cancelled);
            }
            let Some(mut step) = self.bounded_step(part, values).1 else {
                return Ok(Ending::Unfinished);
            };
            let length = step.iter().map(|delta| delta * delta).sum::<f64>().sqrt();
            if length > MAX_STEP * scale {
                let shrink = MAX_STEP * scale / length;
                step.iter_mut().for_each(|delta| *delta *= shrink);
            }
            let Some(remaining) = self.line_search(part, &step, values) else {
                return Ok(self.minimum(part, values));
            };
            if self.converged(part, values) {
                return Ok(Ending::Converged);
            }
            stalled = if remaining > STALLED_COST_RATIO {
                stalled + 1
            } else {
                0
            };
            if stalled >= STALLED_ITERATIONS {
                return Ok(self.minimum(part, values));
            }
        }
        Ok(Ending::Unfinished)
    }

    fn bounded_step(&self, part: &Part<'_>, values: &[f64]) -> (Linearized, Option<Vec<f64>>) {
        let component = part.component;
        let mut scales: Vec<f64> = component
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
        let linearized = if component.variables.len() > DENSE_LIMIT {
            let (rows, residuals) = self.sparse_linearize(part, values);
            Linearized::Sparse(rows, residuals)
        } else {
            let (rows, residuals) = self.linearize(part, values);
            Linearized::Dense(rows, residuals)
        };
        let width = component.variables.len();
        let step = linearized.step(&scales, width);
        let pushed =
            self.pushed_past_bounds(component, values, step.as_deref().unwrap_or_default());
        if pushed.is_empty() {
            return (linearized, step);
        }
        for column in pushed {
            if let Some(scale) = scales.get_mut(column) {
                *scale = 0.0;
            }
        }
        let step = linearized.step(&scales, width);
        (linearized, step)
    }

    fn pushed_past_bounds(
        &self,
        component: &Component,
        values: &[f64],
        step: &[f64],
    ) -> Vec<usize> {
        if self.system.parameter_variables.is_empty() {
            return Vec::new();
        }
        component
            .variables
            .iter()
            .zip(step)
            .enumerate()
            .filter(|(_, (variable, delta))| {
                let current = value(values, **variable);
                self.system.parameter_variables.contains(variable)
                    && ((current <= 0.0 && **delta < 0.0) || (current >= 1.0 && **delta > 0.0))
            })
            .map(|(column, _)| column)
            .collect()
    }

    fn minimum(&self, part: &Part<'_>, values: &[f64]) -> Ending {
        let limit = PRESSED_MARGIN * part.context.collapsed_length();
        let radius_pressed = part.component.variables.iter().any(|variable| {
            self.system.radius_variables.contains(variable) && value(values, *variable) <= limit
        });
        if radius_pressed || self.spans_within(part, values, limit).next().is_some() {
            Ending::Collapse
        } else {
            Ending::Minimum
        }
    }

    fn line_search(&self, part: &Part<'_>, step: &[f64], values: &mut [f64]) -> Option<f64> {
        let component = part.component;
        let cost = |values: &[f64]| squared_sum(&self.residuals(part, values));
        let current = cost(values);
        let mut trial = values.to_vec();
        let mut fraction = 1.0;
        for _ in 0..LINE_SEARCH_STEPS {
            for (variable, delta) in component.variables.iter().zip(step) {
                if let (Some(slot), Some(start)) = (trial.get_mut(*variable), values.get(*variable))
                {
                    let moved = start + fraction * delta;
                    *slot = if self.system.parameter_variables.contains(variable) {
                        moved.clamp(0.0, 1.0)
                    } else {
                        moved
                    };
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
                return Some(candidate / current);
            }
            fraction *= 0.5;
        }
        None
    }

    fn linearize(&self, part: &Part<'_>, values: &[f64]) -> (Vec<Vec<f64>>, Vec<f64>) {
        let component = part.component;
        let mut gradient = Gradient::new();
        let mut rows = Vec::with_capacity(component.equations.len());
        let mut residuals = Vec::with_capacity(component.equations.len());
        tally::add(component.equations.len());
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
        tally::add(component.equations.len());
        for equation in self.equations(component) {
            let residual = equation.linearize(values, &part.context, &mut gradient);
            rows.push(sparse::row(&gradient, &component.variables));
            residuals.push(residual);
        }
        (rows, residuals)
    }

    pub fn analyze_component(&self, component: &Component, values: &[f64]) -> ComponentAnalysis {
        let elimination = if component.variables.len() > DENSE_LIMIT {
            Elimination::Sparse
        } else {
            Elimination::Dense
        };
        self.analyze_by(elimination, component, values)
    }

    pub(super) fn analyze_by(
        &self,
        elimination: Elimination,
        component: &Component,
        values: &[f64],
    ) -> ComponentAnalysis {
        let mut analysis = ComponentAnalysis::default();
        let mut contributions = BTreeMap::new();
        let part = self.part(component);
        match elimination {
            Elimination::Sparse => {
                self.analyze_sparse(&part, values, &mut analysis, &mut contributions);
            }
            Elimination::Dense => {
                self.analyze_dense(&part, values, &mut analysis, &mut contributions);
            }
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
        let mut contributions = Vec::new();
        for part in parts {
            analysis.rank += part.rank;
            analysis.fixed.extend(part.fixed);
            contributions.extend(part.contributions);
        }
        contributions.sort_by_key(|(constraint, _, _)| *constraint);
        analysis.redundancies = contributions
            .chunk_by(|a, b| a.0 == b.0)
            .filter(|group| !group.iter().any(|(_, adds_rank, _)| *adds_rank))
            .filter_map(|group| {
                let (constraint, _, _) = group.first()?;
                let duplicates: BTreeSet<ConstraintId> = group
                    .iter()
                    .flat_map(|(_, _, duplicates)| duplicates)
                    .copied()
                    .collect();
                Some(Redundancy {
                    constraint: *constraint,
                    duplicates: duplicates.into_iter().collect(),
                })
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

fn clear_residual(part: &Part<'_>) -> f64 {
    CLEAR_RESIDUAL * CONVERGENCE_TOLERANCE * part.context.scale
}

fn squared_sum(residuals: &[f64]) -> f64 {
    residuals.iter().map(|residual| residual * residual).sum()
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
    tally::add(2 * basis.len() * row.len());
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
    tally::add(width * earlier.len() * width.min(earlier.len()));
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
    tally::add(rows.len() * width * rows.len().min(width));
    let jacobian = DMatrix::from_row_slice(rows.len(), width, &data);
    let svd = SVD::try_new(jacobian, true, true, f64::EPSILON, SVD_ITERATIONS)?;
    let cutoff = svd.singular_values.max() * STEP_RANK_TOLERANCE;
    let step = svd
        .solve(&DVector::from_vec(scaled_residuals), cutoff)
        .ok()?;
    let step: Vec<f64> = step.iter().copied().collect();
    step.iter().all(|delta| delta.is_finite()).then_some(step)
}
