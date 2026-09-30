use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
};

use crate::{
    entity::Entity,
    id::{ConstraintId, EntityId},
    sketch::{Sketch, SketchError},
    solve::{
        equation::value,
        numeric::{Cancelled, Component, Descent, FROZEN, Failure, Solver, components},
        system::System,
    },
};

pub(super) const DIAGNOSIS_WORK: usize = 500_000;
const MID_RANGE: f64 = 0.5;

pub(super) fn diagnose_failure(
    sketch: &Sketch,
    solver: &Solver<'_>,
    failed: &[Failure],
    work: usize,
) -> Result<SketchError, SketchError> {
    let collapsed = failed.iter().find_map(|failure| {
        solver
            .collapsed(&solver.part(&failure.component), &solver.system.values)
            .next()
    });
    if let Some(entity) = collapsed {
        return Ok(SketchError::NoLength {
            entity,
            label: sketch.entity_label(entity),
        });
    }
    let Some((failure, suspects)) = failed
        .iter()
        .map(|failure| {
            let suspects = owners_newest_first(solver.system, &failure.component.equations);
            (failure, suspects)
        })
        .max_by_key(|(_, suspects)| suspects.first().copied())
    else {
        return Ok(SketchError::Unsolvable {
            entities: Vec::new(),
            newest: None,
        });
    };
    let mut diagnosis = Diagnosis::new(solver, failure, work);
    let support = diagnosis.irreducible_where_it_settled(failure);
    match diagnosis.minimal_conflict(&suspects, support) {
        Ok(Some(constraints)) => Ok(SketchError::Conflict { constraints }),
        Ok(None) | Err(Stop::Exhausted) => Ok(SketchError::Unsolvable {
            entities: named_entities(sketch, solver.system, &failure.component),
            newest: suspects.first().copied(),
        }),
        Err(Stop::Cancelled) => Err(SketchError::Cancelled),
    }
}

fn owners_newest_first(system: &System, equations: &[usize]) -> Vec<ConstraintId> {
    equations
        .iter()
        .filter_map(|index| system.equations.get(*index)?.owner)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .rev()
        .collect()
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

fn gather(values: &[f64], variables: &[usize]) -> Vec<f64> {
    variables
        .iter()
        .map(|variable| value(values, *variable))
        .collect()
}

fn overlay(values: &mut [f64], variables: &[usize], taken: &[f64]) {
    for (variable, taken) in variables.iter().zip(taken) {
        if let Some(slot) = values.get_mut(*variable) {
            *slot = *taken;
        }
    }
}

enum Stop {
    Cancelled,
    Exhausted,
}

enum Verdict {
    Contradiction,
    Holds,
    Undecided,
}

enum Settling {
    Short(f64),
    Solved,
    Unsure(f64),
}

impl From<Cancelled> for Stop {
    fn from(_: Cancelled) -> Self {
        Self::Cancelled
    }
}

struct Probe {
    holds: bool,
    variables: Vec<usize>,
    end: Vec<f64>,
    cost: f64,
    conclusive: bool,
    from_drawn: bool,
}

struct Diagnosis<'a> {
    solver: &'a Solver<'a>,
    scope: Vec<usize>,
    variables: Vec<usize>,
    probes: BTreeMap<Vec<usize>, Probe>,
    solved: Vec<f64>,
    settled: Vec<f64>,
    work_left: Cell<usize>,
}

impl<'a> Diagnosis<'a> {
    fn new(solver: &'a Solver<'a>, failure: &Failure, work: usize) -> Self {
        let drawn = &solver.system.values;
        let component = &failure.component;
        let mut settled = drawn.clone();
        if !failure.settled.pressed {
            overlay(&mut settled, &component.variables, &failure.settled.end);
        }
        let probe = Probe {
            holds: false,
            variables: component.variables.clone(),
            end: failure.settled.end.clone(),
            cost: failure.settled.cost,
            conclusive: failure.settled.conclusive,
            from_drawn: false,
        };
        Self {
            solver,
            scope: component.equations.clone(),
            variables: component.variables.clone(),
            probes: BTreeMap::from([(component.equations.clone(), probe)]),
            solved: drawn.clone(),
            settled,
            work_left: Cell::new(work),
        }
    }

    fn irreducible_where_it_settled(&self, failure: &Failure) -> Vec<ConstraintId> {
        if failure.settled.pressed {
            return Vec::new();
        }
        let irreducible = self.solver.irreducible(&failure.component, &self.settled);
        owners_newest_first(self.solver.system, &irreducible)
    }

    fn minimal_conflict(
        &mut self,
        suspects: &[ConstraintId],
        support: Vec<ConstraintId>,
    ) -> Result<Option<Vec<ConstraintId>>, Stop> {
        if suspects.is_empty() || !self.solves_with(&[])? {
            return Ok(None);
        }
        let mut support = (!support.is_empty()).then_some(support);
        loop {
            let found = match support.take() {
                Some(support) if !self.solves_with(&support)? => support,
                Some(_) | None => {
                    let found = self.conflict(&[], false, suspects)?;
                    if found.is_empty() || self.solves_with(&found)? {
                        return Ok(None);
                    }
                    found
                }
            };
            let (mut conflict, witnesses) = self.without_bystanders(found)?;
            match self.verdict(&conflict, &witnesses)? {
                Verdict::Contradiction => {
                    conflict.sort_unstable();
                    return Ok(Some(conflict));
                }
                Verdict::Holds => {}
                Verdict::Undecided => return Ok(None),
            }
        }
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
    ) -> Result<(Vec<ConstraintId>, Vec<Vec<f64>>), Stop> {
        let oldest_first: Vec<ConstraintId> = conflict.iter().rev().copied().collect();
        let mut witnesses = Vec::new();
        for constraint in oldest_first {
            let rest: Vec<ConstraintId> = conflict
                .iter()
                .copied()
                .filter(|kept| *kept != constraint)
                .collect();
            if self.solves_with(&rest)? {
                witnesses.push(self.solution_of(&rest));
            } else {
                conflict = rest;
            }
        }
        Ok((conflict, witnesses))
    }

    fn verdict(
        &mut self,
        conflict: &[ConstraintId],
        witnesses: &[Vec<f64>],
    ) -> Result<Verdict, Stop> {
        let failing = self.parts_with(conflict).into_iter().find_map(|part| {
            let probe = self.probes.get(&part.equations)?;
            (!probe.holds).then_some((probe.conclusive, probe.cost, probe.from_drawn, part))
        });
        let Some((true, mut lowest, from_drawn, part)) = failing else {
            return Ok(Verdict::Undecided);
        };
        let drawn = self.solver.system.values.clone();
        let fresh_starts = [
            (!from_drawn).then(|| drawn.clone()),
            self.mid_range_start(&part, &drawn)?,
        ];
        for start in fresh_starts.into_iter().flatten() {
            match self.settle(&part, start)? {
                Settling::Short(cost) => lowest = lowest.min(cost),
                Settling::Solved => return Ok(Verdict::Holds),
                Settling::Unsure(_) => return Ok(Verdict::Undecided),
            }
        }
        let mut start = drawn;
        let mut ranked: Vec<(f64, &Vec<f64>)> = witnesses
            .iter()
            .map(|witness| {
                overlay(&mut start, &self.variables, witness);
                (self.solver.cost(&part, &start), witness)
            })
            .collect();
        ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (rank, (cost, witness)) in ranked.into_iter().enumerate() {
            if rank > 0 && cost >= lowest {
                break;
            }
            overlay(&mut start, &self.variables, witness);
            match self.settle(&part, start.clone())? {
                Settling::Short(cost) => lowest = lowest.min(cost),
                Settling::Unsure(cost) if cost >= lowest => {}
                Settling::Solved => return Ok(Verdict::Holds),
                Settling::Unsure(_) => return Ok(Verdict::Undecided),
            }
        }
        Ok(Verdict::Contradiction)
    }

    fn mid_range_start(&self, part: &Component, drawn: &[f64]) -> Result<Option<Vec<f64>>, Stop> {
        let parameters: BTreeSet<usize> = part
            .variables
            .iter()
            .copied()
            .filter(|variable| self.solver.system.parameter_variables.contains(variable))
            .collect();
        if parameters.is_empty() {
            return Ok(None);
        }
        let mut start = drawn.to_vec();
        for parameter in &parameters {
            if let Some(slot) = start.get_mut(*parameter) {
                *slot = MID_RANGE;
            }
        }
        if let Descent::Failed(settled) = self.budgeted_by(part, &mut start, &parameters, FROZEN)? {
            overlay(&mut start, &part.variables, &settled.end);
        }
        Ok(Some(start))
    }

    fn settle(&mut self, part: &Component, mut start: Vec<f64>) -> Result<Settling, Stop> {
        Ok(match self.budgeted(part, &mut start)? {
            Descent::Failed(settled) if settled.conclusive => Settling::Short(settled.cost),
            Descent::Failed(settled) => Settling::Unsure(settled.cost),
            Descent::Solved => {
                let end = gather(&start, &part.variables);
                overlay(&mut self.solved, &part.variables, &end);
                self.probes.insert(
                    part.equations.clone(),
                    Probe {
                        holds: true,
                        variables: part.variables.clone(),
                        end,
                        cost: 0.0,
                        conclusive: false,
                        from_drawn: false,
                    },
                );
                Settling::Solved
            }
        })
    }

    fn solution_of(&self, constraints: &[ConstraintId]) -> Vec<f64> {
        let mut values = self.solver.system.values.clone();
        for part in self.parts_with(constraints) {
            if let Some(probe) = self.probes.get(&part.equations) {
                overlay(&mut values, &probe.variables, &probe.end);
            }
        }
        gather(&values, &self.variables)
    }

    fn parts_with(&self, constraints: &[ConstraintId]) -> Vec<Component> {
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
        components(system, &active, &system.values)
    }

    fn solves_with(&mut self, constraints: &[ConstraintId]) -> Result<bool, Stop> {
        let parts = self.parts_with(constraints);
        let known_to_fail = parts.iter().any(|part| {
            self.probes
                .get(&part.equations)
                .is_some_and(|probe| !probe.holds)
        });
        if known_to_fail {
            return Ok(false);
        }
        for part in parts {
            if self.probes.contains_key(&part.equations) {
                continue;
            }
            let probe = self.probe(&part)?;
            let holds = probe.holds;
            self.probes.insert(part.equations, probe);
            if !holds {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn probe(&mut self, part: &Component) -> Result<Probe, Stop> {
        let (mut values, from_drawn) = self.start_for(part);
        let variables = part.variables.clone();
        Ok(match self.budgeted(part, &mut values)? {
            Descent::Solved => {
                let end = gather(&values, &variables);
                overlay(&mut self.solved, &variables, &end);
                Probe {
                    holds: true,
                    variables,
                    end,
                    cost: 0.0,
                    conclusive: false,
                    from_drawn,
                }
            }
            Descent::Failed(settled) => {
                if !settled.pressed {
                    overlay(&mut self.settled, &variables, &settled.end);
                }
                Probe {
                    holds: false,
                    variables,
                    end: settled.end,
                    cost: settled.cost,
                    conclusive: settled.conclusive,
                    from_drawn,
                }
            }
        })
    }

    fn start_for(&self, part: &Component) -> (Vec<f64>, bool) {
        let drawn = &self.solver.system.values;
        let closest = [&self.solved, &self.settled, drawn]
            .into_iter()
            .map(|candidate| (self.solver.cost(part, candidate), candidate))
            .fold(
                None,
                |best: Option<(f64, &Vec<f64>)>, (cost, candidate)| match best {
                    Some((lowest, _)) if lowest <= cost => best,
                    _ => Some((cost, candidate)),
                },
            )
            .map_or(drawn, |(_, candidate)| candidate);
        let from_drawn = part.variables.iter().all(|variable| {
            value(closest, *variable).to_bits() == value(drawn, *variable).to_bits()
        });
        (closest.clone(), from_drawn)
    }

    fn budgeted(&self, part: &Component, values: &mut [f64]) -> Result<Descent, Stop> {
        self.budgeted_by(part, values, self.solver.stiff, self.solver.stiffness)
    }

    fn budgeted_by(
        &self,
        part: &Component,
        values: &mut [f64],
        stiff: &BTreeSet<usize>,
        stiffness: f64,
    ) -> Result<Descent, Stop> {
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
            stiff,
            stiffness,
            ..*self.solver
        };
        match probe.descend(part, values) {
            Ok(descent) => Ok(descent),
            Err(Cancelled) if exhausted.get() => Err(Stop::Exhausted),
            Err(Cancelled) => Err(Stop::Cancelled),
        }
    }
}
