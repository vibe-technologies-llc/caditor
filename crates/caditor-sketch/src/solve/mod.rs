mod determined;
#[cfg(test)]
mod determined_tests;
mod diagnosis;
#[cfg(test)]
mod diagnosis_tests;
mod ellipse;
#[cfg(test)]
mod ellipse_tests;
mod equation;
#[cfg(test)]
mod kind_tests;
mod memo;
mod numeric;
mod pair;
#[cfg(test)]
mod projection_tests;
#[cfg(test)]
mod reach_tests;
mod sparse;
mod spline;
#[cfg(test)]
mod spline_kind_tests;
mod system;
pub(crate) mod tally;
mod target;
#[cfg(test)]
mod target_tests;
#[cfg(test)]
mod tests;
mod witness;

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

use caditor_expression::{EvalError, ParameterId, Quantity};
use caditor_geometry::Point2;

pub(crate) use crate::solve::{
    ellipse::ellipse_gap,
    pair::curve_pair_gap,
    spline::{joined_at_end, joined_ends, not_joined, spline_gap, straight_spline},
    system::arc_joint,
};
pub use crate::solve::{memo::SolveMemo, numeric::Redundancy};
use crate::{
    constraint::Constraint,
    entity::Entity,
    id::{ConstraintId, EntityId},
    sketch::{DimensionValues, Sketch, SketchError},
    solve::{
        diagnosis::{DIAGNOSIS_STEPS, DIAGNOSIS_WORK, diagnose_failure, owners_newest_first},
        equation::value,
        memo::{Held, Recall},
        numeric::{
            Analysis, Cancelled, Component, DENSE_LIMIT, Descent, FROZEN, Failure, Parts,
            REACH_WORK, ReachWork, STIFF, Solver,
        },
        system::System,
    },
};

const SPACING_ROUNDS: usize = 8;
const KNOT_SETTLING: f64 = 1e-10;
const DISTANCE_SETTLING: f64 = 1e-6;
const PARAMETER_ROUNDS: usize = 16;
const CONVERGING_SHARE: f64 = 0.5;
const MOST_PARAMETER_ROUNDS: usize = 1024;

fn spaced_fit_variables(sketch: &Sketch, system: &System) -> Vec<usize> {
    sketch
        .entities()
        .filter_map(|(_, entity)| match entity {
            Entity::Spline { points, kind } if kind.is_centripetal() => Some(points),
            _ => None,
        })
        .flatten()
        .filter_map(|point| system.points.get(point))
        .flat_map(|x| [*x, x + 1])
        .collect()
}

fn respaced(sketch: &Sketch, system: &System, values: &[f64]) -> Option<Sketch> {
    let spaced: Vec<EntityId> = sketch
        .entities()
        .filter(|(_, entity)| {
            entity
                .spline_kind()
                .is_some_and(|kind| kind.is_centripetal())
        })
        .map(|(id, _)| id)
        .collect();
    if spaced.is_empty() {
        return None;
    }
    let moved = sketch.with_values(system, values);
    spaced
        .iter()
        .any(
            |spline| match (sketch.fit_layout(*spline), moved.fit_layout(*spline)) {
                (Some(before), Some(after)) => !before.settled_against(&after, KNOT_SETTLING),
                _ => false,
            },
        )
        .then_some(moved)
}

struct Unsettled {
    moved: Sketch,
    missing: Vec<ConstraintId>,
    worst: f64,
}

fn misses_a_measured_distance(
    sketch: &Sketch,
    dimensions: &DimensionValues,
    system: &System,
    values: &[f64],
) -> Option<Unsettled> {
    let moved = sketch.with_values(system, values);
    let misses: Vec<(ConstraintId, f64)> = system
        .parameters
        .keys()
        .rev()
        .filter_map(|id| Some((*id, miss_of_its_measure(&moved, dimensions, *id)?)))
        .collect();
    let worst = misses.iter().map(|(_, miss)| *miss).fold(0.0, f64::max);
    (!misses.is_empty()).then(|| Unsettled {
        moved,
        missing: misses.into_iter().map(|(id, _)| id).collect(),
        worst,
    })
}

fn miss_of_its_measure(
    moved: &Sketch,
    dimensions: &DimensionValues,
    id: ConstraintId,
) -> Option<f64> {
    let miss = match moved.constraint(id)? {
        constraint @ Constraint::Distance { from, to, .. }
            if moved.spline(*from).is_some() || moved.spline(*to).is_some() =>
        {
            let (wanted, measured) = (dimensions.dimension(id)?, moved.measured(constraint)?);
            let miss = (measured - wanted).abs();
            return (miss > DISTANCE_SETTLING * wanted.abs().max(1.0)).then_some(miss);
        }
        Constraint::Tangent(first, second) => {
            let (spline, other) = if moved.spline(*first).is_some() {
                (*first, *second)
            } else {
                (*second, *first)
            };
            if moved.spline(other).is_some() {
                return None;
            }
            let (on_spline, on_other) = moved.spline_gap(spline, other)?;
            on_spline.distance(on_other)
        }
        Constraint::Coincident(first, second) => {
            let (point, spline) = if moved.spline(*first).is_some() {
                (*second, *first)
            } else {
                (*first, *second)
            };
            moved.spline(spline)?;
            let at = moved.point(point)?;
            moved.closest_on_curve(spline, at)?.distance(at)
        }
        _ => return None,
    };
    (miss > DISTANCE_SETTLING).then_some(miss)
}

fn keeps_converging(worst: &[f64]) -> bool {
    let Some(start) = worst.len().checked_sub(2 * PARAMETER_ROUNDS) else {
        return true;
    };
    let (before, recent) = worst.split_at(start + PARAMETER_ROUNDS);
    let largest = |misses: &[f64]| misses.iter().copied().fold(0.0, f64::max);
    largest(recent) <= CONVERGING_SHARE * largest(before.get(start..).unwrap_or_default())
}

fn solve_from_drawn(
    solver: &Solver<'_>,
    failed: Vec<Failure>,
    stiff: &BTreeSet<usize>,
    drawn: &[f64],
    values: &mut [f64],
) -> Result<Vec<Failure>, Cancelled> {
    let mut unsolved = Vec::new();
    for failure in failed {
        let held = failure
            .component
            .variables
            .iter()
            .any(|variable| stiff.contains(variable));
        if !held || !solves_from(solver, &failure.component, drawn, values)? {
            unsolved.push(failure);
        }
    }
    Ok(unsolved)
}

fn solve_again_from(
    solver: &Solver<'_>,
    failed: Vec<Failure>,
    from: &[f64],
    values: &mut [f64],
) -> Result<Vec<Failure>, Cancelled> {
    let mut unsolved = Vec::new();
    for failure in failed {
        if !solves_from(solver, &failure.component, from, values)? {
            unsolved.push(failure);
        }
    }
    Ok(unsolved)
}

fn solves_from(
    solver: &Solver<'_>,
    component: &Component,
    from: &[f64],
    values: &mut [f64],
) -> Result<bool, Cancelled> {
    take_from(component, from, values);
    Ok(solver.descend(component, values)? == Descent::Solved)
}

fn take_from(component: &Component, from: &[f64], values: &mut [f64]) {
    for variable in &component.variables {
        if let (Some(slot), Some(before)) = (values.get_mut(*variable), from.get(*variable)) {
            *slot = *before;
        }
    }
}

fn settle_pressed<'a>(
    solver: &Solver<'_>,
    frozen_failed: &'a [Failure],
    dragged: &BTreeSet<usize>,
    start: &[f64],
    values: &mut [f64],
    work: &mut ReachWork,
) -> Result<Vec<&'a Component>, Cancelled> {
    let mut settled = Vec::new();
    for failure in frozen_failed {
        let component = &failure.component;
        if !pressed_toward_its_targets(solver, component, dragged) {
            continue;
        }
        for (variable, pressed) in component.variables.iter().zip(&failure.settled.end) {
            if let Some(slot) = values.get_mut(*variable) {
                *slot = *pressed;
            }
        }
        if solver.settle_onto_constraints(component, values, work)? {
            settled.push(component);
        } else {
            take_from(component, start, values);
        }
    }
    Ok(settled)
}

fn pressed_toward_its_targets(
    solver: &Solver<'_>,
    component: &Component,
    dragged: &BTreeSet<usize>,
) -> bool {
    let mut held = component
        .variables
        .iter()
        .filter(|variable| solver.stiff.contains(variable))
        .peekable();
    component.variables.len() > DENSE_LIMIT
        && held.peek().is_some()
        && held.all(|variable| dragged.contains(variable))
}

fn meets_its_targets(
    solver: &Solver<'_>,
    component: &Component,
    targets: &[(usize, f64)],
    values: &[f64],
) -> bool {
    let tolerance = solver.tolerance(component);
    targets
        .iter()
        .filter(|(variable, _)| component.variables.binary_search(variable).is_ok())
        .all(|(variable, to)| (value(values, *variable) - to).abs() <= tolerance)
}

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
        let mut fixed = vec![false; system.values.len()];
        for variable in analysis.fixed {
            if let Some(slot) = fixed.get_mut(variable) {
                *slot = true;
            }
        }
        let entity_states = system
            .entity_variables
            .iter()
            .map(|(entity, variables)| {
                let is_fixed = |variable: &usize| fixed.get(*variable).copied().unwrap_or(false);
                let state = if variables.iter().all(is_fixed) {
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

struct Finished<'a> {
    dimensions: DimensionValues,
    system: &'a System,
    solver: &'a Solver<'a>,
    parts: Cow<'a, [Component]>,
    values: Vec<f64>,
    recall: Recall<'a>,
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
        self.solve_with(value_of, cancelled, drags, previous, |finished| {
            let Finished {
                dimensions,
                system,
                solver,
                parts,
                values,
                mut recall,
            } = finished;
            let analyses = parts
                .iter()
                .map(|component| {
                    recall.analysis(component, &values, || {
                        solver.analyze_component(component, &values)
                    })
                })
                .collect::<Result<Vec<_>, Cancelled>>()?;
            let analysis = Analysis::combine(analyses);
            Ok(Solved {
                geometry: self.with_values(system, &values),
                solution: SketchSolution::new(dimensions, system, analysis),
                memo: recall.finish(),
            })
        })
    }

    pub fn solve_geometry_from<F>(
        &self,
        value_of: &F,
        cancelled: &dyn Fn() -> bool,
        drags: &[Drag],
        previous: Option<&SolveMemo>,
    ) -> Result<(Sketch, SolveMemo), SketchError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        self.solve_with(value_of, cancelled, drags, previous, |finished| {
            let Finished {
                system,
                parts,
                values,
                mut recall,
                ..
            } = finished;
            for component in parts.iter() {
                recall.remember_geometry(component, &values);
            }
            Ok((self.with_values(system, &values), recall.finish()))
        })
    }

    fn solve_with<F, T>(
        &self,
        value_of: &F,
        cancelled: &dyn Fn() -> bool,
        drags: &[Drag],
        previous: Option<&SolveMemo>,
        finish: impl FnOnce(Finished<'_>) -> Result<T, Cancelled>,
    ) -> Result<T, SketchError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        let dimensions = self.evaluate(value_of)?;
        let mut current: Cow<'_, Sketch> = Cow::Borrowed(self);
        let mut round = 0;
        let mut reparameterised = 0;
        let mut misses = Vec::new();
        loop {
            let mut system = System::build(&current, &dimensions)?;
            let drawn = system.values.clone();
            let mut stiff = BTreeSet::new();
            let targets: Vec<(usize, f64)> = drags
                .iter()
                .flat_map(|drag| drag.targets(&system))
                .filter(|(_, target)| target.is_finite())
                .collect();
            for &(variable, target) in &targets {
                if let Some(slot) = system.values.get_mut(variable) {
                    *slot = target;
                    stiff.insert(variable);
                }
            }
            let dragged = stiff.clone();
            if round > 0 {
                stiff.extend(spaced_fit_variables(&current, &system));
            }

            let parts = Parts::of(&system);
            let fixed = Held {
                stiff: &stiff,
                targets: &targets,
            };
            let mut recall = Recall::new(&system, &dimensions, parts.drawn(), fixed, previous);
            let mut start = system.values.clone();
            recall.start_from(&mut start);

            let frozen = Solver {
                system: &system,
                cancelled,
                stiff: &stiff,
                stiffness: FROZEN,
                retrying: targets.is_empty(),
            };
            let starting = parts.at(&system, &start);
            let beyond: Vec<&Component> = starting
                .iter()
                .filter(|component| {
                    pressed_toward_its_targets(&frozen, component, &dragged)
                        && recall.ended_beyond_reach(component, &drawn)
                })
                .collect();
            let mut values = start.clone();
            let frozen_failed = if stiff.is_empty() {
                None
            } else {
                let tried = starting
                    .iter()
                    .filter(|component| !beyond.contains(component));
                Some(frozen.solve_parts(tried, &mut values)?)
            };
            let held = beyond.is_empty() && frozen_failed.as_ref().is_some_and(Vec::is_empty);
            let solver = Solver {
                stiffness: STIFF,
                retrying: true,
                ..frozen
            };
            if !held {
                values = start.clone();
                let mut work = ReachWork::new(REACH_WORK);
                let pressed = settle_pressed(
                    &solver,
                    frozen_failed.as_deref().unwrap_or_default(),
                    &dragged,
                    &start,
                    &mut values,
                    &mut work,
                )?;
                let stiffened = starting.iter().filter(|component| {
                    !pressed.contains(component) && !beyond.contains(component)
                });
                let failed = solver.solve_parts(stiffened, &mut values)?;
                let failed = solve_from_drawn(&solver, failed, &stiff, &drawn, &mut values)?;
                let failed = if round > 0 {
                    let loose = Solver {
                        stiff: &dragged,
                        ..solver
                    };
                    solve_again_from(&loose, failed, &start, &mut values)?
                } else {
                    failed
                };
                if !failed.is_empty() {
                    for found in diagnose_failure(
                        &current,
                        &solver,
                        &failed,
                        (DIAGNOSIS_WORK, DIAGNOSIS_STEPS),
                    )? {
                        found.apply(&mut values);
                    }
                    let collapsed = parts.at(&system, &values).iter().any(|component| {
                        solver
                            .collapsed(&solver.part(component), &values)
                            .next()
                            .is_some()
                    });
                    if collapsed {
                        let equations: Vec<usize> = failed
                            .iter()
                            .flat_map(|failure| failure.component.equations.iter().copied())
                            .collect();
                        return Err(SketchError::Conflict {
                            constraints: owners_newest_first(&system, &equations),
                        });
                    }
                }
                for component in parts.at(&system, &values).iter() {
                    solver.reach(component, &targets, &drawn, &mut values, &mut work)?;
                }
            }
            if round < SPACING_ROUNDS
                && let Some(respaced) = respaced(&current, &system, &values)
            {
                current = Cow::Owned(respaced);
                round += 1;
                continue;
            }
            if let Some(Unsettled {
                moved,
                missing,
                worst,
            }) = misses_a_measured_distance(&current, &dimensions, &system, &values)
            {
                let out_of_rounds = reparameterised >= MOST_PARAMETER_ROUNDS
                    || (reparameterised >= PARAMETER_ROUNDS
                        && reparameterised % PARAMETER_ROUNDS == 0
                        && !keeps_converging(&misses));
                misses.push(worst);
                if out_of_rounds {
                    return Err(SketchError::Conflict {
                        constraints: missing,
                    });
                }
                current = Cow::Owned(moved);
                reparameterised += 1;
                continue;
            }
            for component in parts.at(&system, &values).iter() {
                if pressed_toward_its_targets(&solver, component, &dragged)
                    && !meets_its_targets(&solver, component, &targets, &values)
                {
                    recall.remember_beyond_reach(component, &values);
                }
            }
            return Ok(finish(Finished {
                dimensions: dimensions.clone(),
                system: &system,
                solver: &solver,
                parts: parts.at(&system, &values),
                values,
                recall,
            })?);
        }
    }

    pub(crate) fn with_values(&self, system: &System, values: &[f64]) -> Self {
        let mut geometry = self.clone();
        for (point, x) in &system.points {
            geometry.set_point(*point, Point2::new(value(values, *x), value(values, x + 1)));
        }
        for (circle, radius) in &system.radii {
            geometry.set_radius(*circle, value(values, *radius));
        }
        for (conic, rho) in &system.rhos {
            geometry.set_rho(*conic, *rho);
        }
        geometry.set_text_sizes(&system.text_sizes);
        geometry.set_gear_values(&system.gear_values);
        geometry
    }
}
