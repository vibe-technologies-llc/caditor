mod equation;
mod numeric;
mod sparse;
mod system;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use caditor_expression::{EvalError, ParameterId, Quantity};
use caditor_geometry::Point2;

pub use crate::solve::numeric::Redundancy;
use crate::{
    id::{ConstraintId, EntityId},
    sketch::{DimensionValues, Sketch, SketchError},
    solve::{
        equation::value,
        numeric::{Analysis, Cancelled, Component, Solver},
        system::System,
    },
};

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

#[derive(Debug, Clone, PartialEq)]
pub struct Solved {
    pub geometry: Sketch,
    pub solution: SketchSolution,
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
        let dimensions = self.evaluate(value_of)?;
        let system = System::build(self, &dimensions)?;
        let solver = Solver {
            system: &system,
            cancelled,
        };
        let every_equation: Vec<usize> = (0..system.equations.len()).collect();
        let mut values = system.values.clone();
        let failed = solver.solve(&every_equation, &mut values)?;
        if !failed.is_empty() {
            return Err(diagnose_failure(&solver, &failed)?);
        }
        let analysis = solver.analyze(&every_equation, &values);
        Ok(Solved {
            geometry: self.with_values(&system, &values),
            solution: SketchSolution::new(dimensions, &system, analysis),
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

fn diagnose_failure(solver: &Solver<'_>, failed: &[Component]) -> Result<SketchError, SketchError> {
    let scope: Vec<usize> = failed
        .iter()
        .flat_map(|component| component.equations.iter().copied())
        .collect();
    let suspects: Vec<ConstraintId> = scope
        .iter()
        .filter_map(|index| solver.system.equations.get(*index)?.owner)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .rev()
        .collect();
    let diagnosis = Diagnosis { solver, scope };
    if suspects.is_empty() || !diagnosis.solves_with(&[])? {
        return Ok(SketchError::Unsolvable);
    }
    let mut conflicting = diagnosis.conflict(&[], false, &suspects)?;
    conflicting.sort_unstable();
    Ok(if conflicting.is_empty() {
        SketchError::Unsolvable
    } else {
        SketchError::Conflict {
            constraints: conflicting,
        }
    })
}

struct Diagnosis<'a> {
    solver: &'a Solver<'a>,
    scope: Vec<usize>,
}

impl Diagnosis<'_> {
    fn conflict(
        &self,
        kept: &[ConstraintId],
        just_added: bool,
        candidates: &[ConstraintId],
    ) -> Result<Vec<ConstraintId>, SketchError> {
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

    fn solves_with(&self, constraints: &[ConstraintId]) -> Result<bool, SketchError> {
        let kept: BTreeSet<ConstraintId> = constraints.iter().copied().collect();
        let active: Vec<usize> = self
            .scope
            .iter()
            .copied()
            .filter(|index| {
                self.solver
                    .system
                    .equations
                    .get(*index)
                    .is_some_and(|equation| {
                        equation.owner.is_none_or(|owner| kept.contains(&owner))
                    })
            })
            .collect();
        let mut values = self.solver.system.values.clone();
        Ok(self.solver.solve(&active, &mut values)?.is_empty())
    }
}
