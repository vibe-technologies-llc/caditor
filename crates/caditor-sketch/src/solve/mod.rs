mod diagnosis;
#[cfg(test)]
mod diagnosis_tests;
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

use std::collections::{BTreeMap, BTreeSet};

use caditor_expression::{EvalError, ParameterId, Quantity};
use caditor_geometry::Point2;

pub use crate::solve::{memo::SolveMemo, numeric::Redundancy};
use crate::{
    id::{ConstraintId, EntityId},
    sketch::{DimensionValues, Sketch, SketchError},
    solve::{
        diagnosis::{DIAGNOSIS_WORK, diagnose_failure},
        equation::value,
        memo::Recall,
        numeric::{Analysis, Cancelled, FROZEN, STIFF, Solver, components},
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
