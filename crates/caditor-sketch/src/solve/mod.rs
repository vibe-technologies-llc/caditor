mod equation;
#[cfg(test)]
mod kind_tests;
mod memo;
mod numeric;
mod sparse;
mod spline;
mod system;
#[cfg(test)]
mod tests;

use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
};

use caditor_expression::{EvalError, ParameterId, Quantity};
use caditor_geometry::Point2;

pub use crate::solve::{memo::SolveMemo, numeric::Redundancy};
use crate::{
    entity::Entity,
    id::{ConstraintId, EntityId},
    sketch::{DimensionValues, Sketch, SketchError},
    solve::{
        equation::value,
        memo::Recall,
        numeric::{Analysis, Cancelled, Component, FROZEN, STIFF, Solver, components},
        system::System,
    },
};

const DIAGNOSIS_WORK: usize = 500_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntityState {
    FullyConstrained,
    UnderConstrained,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SketchSolution {
    dimensions: DimensionValues,
    degrees_of_freedom: usize,
    entity_states: BTreeMap<EntityId, EntityState>,
    redundancies: Vec<Redundancy>,
}

impl SketchSolution {
    pub fn dimension(&self, constraint: ConstraintId) -> Option<f64> {
        self.dimensions.dimension(constraint)
    }

    pub fn dimensions(&self) -> &DimensionValues {
        &self.dimensions
    }

    pub fn degrees_of_freedom(&self) -> usize {
        self.degrees_of_freedom
    }

    pub fn is_fully_constrained(&self) -> bool {
        self.degrees_of_freedom == 0
    }

    pub fn entity_state(&self, entity: EntityId) -> Option<EntityState> {
        if entity.is_reference() {
            return Some(EntityState::FullyConstrained);
        }
        self.entity_states.get(&entity).copied()
    }

    pub fn redundancies(&self) -> &[Redundancy] {
        &self.redundancies
    }

    pub fn redundancy(&self, constraint: ConstraintId) -> Option<&Redundancy> {
        self.redundancies
            .iter()
            .find(|redundancy| redundancy.constraint == constraint)
    }

    fn new(dimensions: DimensionValues, system: &System, analysis: Analysis) -> Self {
        let fixed: BTreeSet<usize> = analysis.fixed.into_iter().collect();
        let entity_states = system
            .entity_variables
            .iter()
            .map(|(entity, variables)| {
                let state = if variables.iter().all(|variable| fixed.contains(variable)) {
                    EntityState::FullyConstrained
                } else {
                    EntityState::UnderConstrained
                };
                (*entity, state)
            })
            .collect();
        let mut redundancies = analysis.redundancies;
        redundancies.sort_by_key(|redundancy| redundancy.constraint);
        Self {
            dimensions,
            degrees_of_freedom: system.values.len().saturating_sub(analysis.rank),
            entity_states,
            redundancies,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Drag {
    Point { point: EntityId, to: Point2 },
    Radius { circle: EntityId, to: f64 },
}

impl Drag {
    fn targets(self, system: &System) -> Vec<(usize, f64)> {
        match self {
            Self::Point { point, to } => system
                .points
                .get(&point)
                .map(|&x| vec![(x, to.x), (x + 1, to.y)])
                .unwrap_or_default(),
            Self::Radius { circle, to } => system
                .radii
                .get(&circle)
                .map(|&radius| vec![(radius, to)])
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Solved {
    pub geometry: Sketch,
    pub solution: SketchSolution,
    pub memo: SolveMemo,
}

impl From<Cancelled> for SketchError {
    fn from(_: Cancelled) -> Self {
        Self::Cancelled
    }
}

impl Sketch {
    pub fn solve<F>(
        &self,
        value_of: &F,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Solved, SketchError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        self.solve_dragging(value_of, cancelled, &[])
    }

    pub fn solve_dragging<F>(
        &self,
        value_of: &F,
        cancelled: &dyn Fn() -> bool,
        drags: &[Drag],
    ) -> Result<Solved, SketchError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        self.solve_from(value_of, cancelled, drags, None)
    }

    pub fn solve_from<F>(
        &self,
        value_of: &F,
        cancelled: &dyn Fn() -> bool,
        drags: &[Drag],
        previous: Option<&SolveMemo>,
    ) -> Result<Solved, SketchError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        let dimensions = self.evaluate(value_of)?;
        let mut system = System::build(self, &dimensions)?;
        let mut stiff = BTreeSet::new();
        let targets: Vec<(usize, f64)> = drags
            .iter()
            .flat_map(|drag| drag.targets(&system))
            .collect();
        for (variable, target) in targets {
            if let Some(slot) = system.values.get_mut(variable)
                && target.is_finite()
            {
                *slot = target;
                stiff.insert(variable);
            }
        }
        let every_equation: Vec<usize> = (0..system.equations.len()).collect();
        let mut recall = Recall::new(&system, &dimensions, &every_equation, &stiff, previous);
        let mut start = system.values.clone();
        recall.start_from(&mut start);
        let frozen = Solver {
            system: &system,
            cancelled,
            stiff: &stiff,
            stiffness: FROZEN,
        };
        let mut values = start.clone();
        let held = !stiff.is_empty() && frozen.solve(&every_equation, &mut values)?.is_empty();
        let solver = Solver {
            stiffness: STIFF,
            ..frozen
        };
        if !held {
            values = start;
            let failed = solver.solve(&every_equation, &mut values)?;
            if !failed.is_empty() {
                return Err(diagnose_failure(self, &solver, &failed, DIAGNOSIS_WORK)?);
            }
        }
        let parts: Vec<_> = components(&system, &every_equation, &values)
            .iter()
            .map(|component| {
                recall.analysis(component, &values, || {
                    solver.analyze_component(component, &values)
                })
            })
            .collect();
        let analysis = Analysis::combine(parts);
        Ok(Solved {
            geometry: self.with_values(&system, &values),
            solution: SketchSolution::new(dimensions, &system, analysis),
            memo: recall.finish(),
        })
    }

    fn with_values(&self, system: &System, values: &[f64]) -> Self {
        let mut geometry = self.clone();
        for (point, x) in &system.points {
            geometry.set_point(*point, Point2::new(value(values, *x), value(values, x + 1)));
        }
        for (circle, radius) in &system.radii {
            geometry.set_radius(*circle, value(values, *radius));
        }
        geometry
    }
}

fn diagnose_failure(
    sketch: &Sketch,
    solver: &Solver<'_>,
    failed: &[Component],
    work: usize,
) -> Result<SketchError, SketchError> {
    let collapsed = failed.iter().find_map(|component| {
        solver
            .collapsed(&solver.part(component), &solver.system.values)
            .next()
    });
    if let Some(entity) = collapsed {
        return Ok(SketchError::NoLength {
            entity,
            label: sketch.entity_label(entity),
        });
    }
    let suspects_of = |component: &Component| -> Vec<ConstraintId> {
        component
            .equations
            .iter()
            .filter_map(|index| solver.system.equations.get(*index)?.owner)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .rev()
            .collect()
    };
    let Some((component, suspects)) = failed
        .iter()
        .map(|component| (component, suspects_of(component)))
        .max_by_key(|(_, suspects)| suspects.first().copied())
    else {
        return Ok(SketchError::Unsolvable {
            entities: Vec::new(),
            newest: None,
        });
    };
    let mut diagnosis = Diagnosis {
        solver,
        scope: component.equations.clone(),
        outcomes: BTreeMap::new(),
        work_left: Cell::new(work),
    };
    match diagnosis.minimal_conflict(&suspects) {
        Ok(Some(constraints)) => Ok(SketchError::Conflict { constraints }),
        Ok(None) | Err(Stop::Exhausted) => Ok(SketchError::Unsolvable {
            entities: named_entities(sketch, solver.system, component),
            newest: suspects.first().copied(),
        }),
        Err(Stop::Cancelled) => Err(SketchError::Cancelled),
    }
}

fn named_entities(sketch: &Sketch, system: &System, component: &Component) -> Vec<EntityId> {
    let moving: BTreeSet<usize> = component.variables.iter().copied().collect();
    let involved: Vec<(EntityId, &Entity)> = system
        .entity_variables
        .iter()
        .filter(|(_, variables)| variables.iter().any(|variable| moving.contains(variable)))
        .filter_map(|(entity, _)| Some((*entity, sketch.entity(*entity)?)))
        .collect();
    let points_of_curves: BTreeSet<EntityId> = involved
        .iter()
        .flat_map(|(_, entity)| entity.points())
        .collect();
    involved
        .into_iter()
        .filter(|(id, _)| !points_of_curves.contains(id))
        .map(|(id, _)| id)
        .collect()
}

enum Stop {
    Cancelled,
    Exhausted,
}

impl From<Cancelled> for Stop {
    fn from(_: Cancelled) -> Self {
        Self::Cancelled
    }
}

struct Diagnosis<'a> {
    solver: &'a Solver<'a>,
    scope: Vec<usize>,
    outcomes: BTreeMap<Vec<usize>, bool>,
    work_left: Cell<usize>,
}

impl Diagnosis<'_> {
    fn minimal_conflict(
        &mut self,
        suspects: &[ConstraintId],
    ) -> Result<Option<Vec<ConstraintId>>, Stop> {
        if suspects.is_empty() || !self.solves_with(&[])? {
            return Ok(None);
        }
        let found = self.conflict(&[], false, suspects)?;
        if found.is_empty() || self.solves_with(&found)? {
            return Ok(None);
        }
        let mut conflict = self.without_bystanders(found)?;
        conflict.sort_unstable();
        Ok(Some(conflict))
    }

    fn conflict(
        &mut self,
        kept: &[ConstraintId],
        just_added: bool,
        candidates: &[ConstraintId],
    ) -> Result<Vec<ConstraintId>, Stop> {
        if just_added && !self.solves_with(kept)? {
            return Ok(Vec::new());
        }
        if candidates.len() <= 1 {
            return Ok(candidates.to_vec());
        }
        let (first, second) = candidates.split_at(candidates.len() / 2);
        let with_first: Vec<ConstraintId> = kept.iter().chain(first).copied().collect();
        let from_second = self.conflict(&with_first, !first.is_empty(), second)?;
        let with_found: Vec<ConstraintId> = kept.iter().chain(&from_second).copied().collect();
        let from_first = self.conflict(&with_found, !from_second.is_empty(), first)?;
        Ok(from_first.into_iter().chain(from_second).collect())
    }

    fn without_bystanders(
        &mut self,
        mut conflict: Vec<ConstraintId>,
    ) -> Result<Vec<ConstraintId>, Stop> {
        let oldest_first: Vec<ConstraintId> = conflict.iter().rev().copied().collect();
        for constraint in oldest_first {
            let rest: Vec<ConstraintId> = conflict
                .iter()
                .copied()
                .filter(|kept| *kept != constraint)
                .collect();
            match self.solves_with(&rest) {
                Ok(false) => conflict = rest,
                Ok(true) | Err(Stop::Exhausted) => {}
                Err(Stop::Cancelled) => return Err(Stop::Cancelled),
            }
        }
        Ok(conflict)
    }

    fn solves_with(&mut self, constraints: &[ConstraintId]) -> Result<bool, Stop> {
        let kept: BTreeSet<ConstraintId> = constraints.iter().copied().collect();
        let system = self.solver.system;
        let active: Vec<usize> = self
            .scope
            .iter()
            .copied()
            .filter(|index| {
                system.equations.get(*index).is_some_and(|equation| {
                    equation.owner.is_none_or(|owner| kept.contains(&owner))
                })
            })
            .collect();
        let parts = components(system, &active, &system.values);
        if parts
            .iter()
            .any(|part| self.outcomes.get(&part.equations) == Some(&false))
        {
            return Ok(false);
        }
        let unknown: Vec<Component> = parts
            .into_iter()
            .filter(|part| !self.outcomes.contains_key(&part.equations))
            .collect();
        if unknown.is_empty() {
            return Ok(true);
        }
        let mut values = system.values.clone();
        for part in unknown {
            let solved = self.budgeted(&part, &mut values)?;
            self.outcomes.insert(part.equations, solved);
            if !solved {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn budgeted(&self, part: &Component, values: &mut [f64]) -> Result<bool, Stop> {
        let step_cost = part.equations.len();
        let exhausted = Cell::new(false);
        let stop = || {
            if (self.solver.cancelled)() {
                return true;
            }
            match self.work_left.get().checked_sub(step_cost) {
                Some(left) => {
                    self.work_left.set(left);
                    false
                }
                None => {
                    exhausted.set(true);
                    true
                }
            }
        };
        let probe = Solver {
            cancelled: &stop,
            ..*self.solver
        };
        match probe.solves(part, values) {
            Ok(solved) => Ok(solved),
            Err(Cancelled) if exhausted.get() => Err(Stop::Exhausted),
            Err(Cancelled) => Err(Stop::Cancelled),
        }
    }
}
