use std::collections::BTreeMap;

use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::Constraint,
    curve::{BSpline, MAX_SPLINE_DEGREE, basis_derivatives, basis_values},
    entity::{Entity, FitSpacing, SplineKind},
    fit::averaged_knots,
    id::{ConstraintId, EntityId},
    sketch::{Sketch, SketchError},
    spacing::FitLayout,
};

const ARM_SHARE: f64 = 1.0 / 3.0;
const SINGULAR_SHARE: f64 = 1e-12;
const SEAM_SNAP: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FitHandle {
    pub constraint: ConstraintId,
    pub index: usize,
    pub anchor: EntityId,
    pub end: EntityId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Row {
    Point(usize),
    Slope(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HandledLayout {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub parameters: Vec<f64>,
    pub rows: Vec<Row>,
    pub closed: bool,
}

impl HandledLayout {
    pub fn of(
        points: &[Point2],
        closed: bool,
        spacing: FitSpacing,
        handled: &[usize],
    ) -> Option<Self> {
        let base = FitLayout::of(points, closed, spacing)
            .or_else(|| FitLayout::of(points, closed, FitSpacing::Even))?;
        let mut rows = Vec::with_capacity(points.len() + handled.len());
        let mut sites = Vec::with_capacity(points.len() + handled.len());
        for (index, parameter) in base.parameters.iter().enumerate() {
            rows.push(Row::Point(index));
            sites.push(*parameter);
            if handled.contains(&index) {
                rows.push(Row::Slope(index));
                sites.push(*parameter);
            }
        }
        let (degree, knots) = if closed {
            (
                MAX_SPLINE_DEGREE,
                looped_averaged_knots(&sites, MAX_SPLINE_DEGREE)?,
            )
        } else {
            let degree = MAX_SPLINE_DEGREE.min(rows.len().checked_sub(1)?);
            (degree, averaged_knots(&sites, degree))
        };
        Some(Self {
            degree,
            knots,
            parameters: base.parameters,
            rows,
            closed,
        })
    }

    pub fn arm(&self, index: usize) -> f64 {
        let count = self.parameters.len();
        let at = |offset: usize| self.parameters.get(offset).copied();
        let gap = if self.closed {
            let before = (index + count).checked_sub(1).map(|before| before % count);
            match (
                before.and_then(at),
                at(index),
                at((index + 1) % count.max(1)),
            ) {
                (Some(before), Some(_), Some(after)) => (after - before).rem_euclid(1.0) / 2.0,
                _ => 1.0,
            }
        } else {
            match (index.checked_sub(1).and_then(at), at(index), at(index + 1)) {
                (Some(before), Some(_), Some(after)) => (after - before) / 2.0,
                (None, Some(own), Some(after)) => after - own,
                (Some(before), Some(own), None) => own - before,
                _ => 1.0,
            }
        };
        gap * ARM_SHARE
    }

    pub fn parameter(&self, index: usize) -> Option<f64> {
        self.parameters.get(index).copied()
    }

    pub fn width(&self) -> usize {
        if self.closed {
            self.knots.len().saturating_sub(self.degree + 1)
        } else {
            self.rows.len()
        }
    }

    pub fn wrapped<T: Copy>(&self, controls: &[T]) -> Option<Vec<T>> {
        let count = controls.len();
        (0..self.width())
            .map(|slot| controls.get(slot % count.max(1)).copied())
            .collect()
    }

    pub fn row_weights(&self, row: Row) -> Option<(usize, Vec<f64>)> {
        let width = self.width();
        match row {
            Row::Point(index) => Some(basis_values(
                self.degree,
                &self.knots,
                width,
                self.parameter(index)?,
            )),
            Row::Slope(index) => {
                let (first, values) =
                    basis_derivatives(self.degree, &self.knots, width, self.parameter(index)?, 1);
                let arm = self.arm(index);
                Some((first, values.iter().map(|value| value * arm).collect()))
            }
        }
    }

    pub fn controls(&self, points: &[Point2], reaches: &[(usize, Vector2)]) -> Option<Vec<Point2>> {
        let count = self.rows.len();
        let mut matrix = vec![vec![0.0; count]; count];
        let mut right = Vec::with_capacity(count);
        for (row, kind) in matrix.iter_mut().zip(&self.rows) {
            let (first, weights) = self.row_weights(*kind)?;
            for (offset, weight) in weights.iter().enumerate() {
                *row.get_mut((first + offset) % count)? += *weight;
            }
            right.push(match kind {
                Row::Point(index) => points.get(*index)?.to_array(),
                Row::Slope(index) => reaches
                    .iter()
                    .find(|(handled, _)| handled == index)?
                    .1
                    .to_array(),
            });
        }
        let solved = solve_dense(matrix, right)?;
        Some(solved.iter().map(|[x, y]| Point2::new(*x, *y)).collect())
    }

    pub fn curve(&self, points: &[Point2], reaches: &[(usize, Vector2)]) -> Option<BSpline> {
        let controls = self.controls(points, reaches)?;
        if self.closed {
            return BSpline::periodic_clamped(self.degree, &self.knots, &self.wrapped(&controls)?);
        }
        Some(BSpline::from_parts(
            controls,
            self.degree,
            self.knots.clone(),
            None,
        ))
    }
}

fn looped_averaged_knots(sites: &[f64], degree: usize) -> Option<Vec<f64>> {
    let count = isize::try_from(sites.len())
        .ok()
        .filter(|count| *count > 0)?;
    let lead = isize::try_from(degree / 2).ok()?;
    let window = isize::try_from(degree).ok()?;
    let site = |index: isize| -> Option<f64> {
        let within = usize::try_from(index.rem_euclid(count)).ok()?;
        Some(sites.get(within)? + index.div_euclid(count) as f64)
    };
    let knot = |index: isize| -> Option<f64> {
        let total =
            (index - lead..index - lead + window).try_fold(0.0, |sum, at| Some(sum + site(at)?))?;
        let average = total / degree as f64;
        let seam = average.round();
        Some(if (average - seam).abs() <= SEAM_SNAP {
            seam
        } else {
            average
        })
    };
    let mut start = 0;
    while knot(start)? > 0.0 {
        start -= 1;
    }
    while knot(start + 1)? <= 0.0 {
        start += 1;
    }
    let first = start - window;
    let length = count + 2 * window + 2;
    (first..first + length).map(knot).collect()
}

fn solve_dense(mut matrix: Vec<Vec<f64>>, mut right: Vec<[f64; 2]>) -> Option<Vec<[f64; 2]>> {
    let size = matrix.len();
    let largest = matrix
        .iter()
        .flatten()
        .fold(0.0_f64, |largest, entry| largest.max(entry.abs()));
    for column in 0..size {
        let pivot_row = (column..size).max_by(|a, b| {
            let entry = |row: usize| {
                matrix
                    .get(row)
                    .and_then(|values| values.get(column))
                    .map_or(0.0, |value| value.abs())
            };
            entry(*a).total_cmp(&entry(*b))
        })?;
        matrix.swap(column, pivot_row);
        right.swap(column, pivot_row);
        let pivot_values = matrix.get(column)?.clone();
        let pivot = *pivot_values.get(column)?;
        if !pivot.is_finite() || pivot.abs() <= SINGULAR_SHARE * largest {
            return None;
        }
        let pivot_right = *right.get(column)?;
        for row in column + 1..size {
            let values = matrix.get_mut(row)?;
            let factor = *values.get(column)? / pivot;
            if factor == 0.0 {
                continue;
            }
            for (value, above) in values.iter_mut().zip(&pivot_values).skip(column) {
                *value -= factor * above;
            }
            let target = right.get_mut(row)?;
            for (value, above) in target.iter_mut().zip(pivot_right) {
                *value -= factor * above;
            }
        }
    }
    for row in (0..size).rev() {
        let values = matrix.get(row)?;
        let mut value = *right.get(row)?;
        for column in row + 1..size {
            let known = *right.get(column)?;
            let coefficient = *values.get(column)?;
            for (entry, known) in value.iter_mut().zip(known) {
                *entry -= coefficient * known;
            }
        }
        let pivot = *values.get(row)?;
        for entry in &mut value {
            *entry /= pivot;
        }
        *right.get_mut(row)? = value;
    }
    right
        .iter()
        .all(|row| row.iter().all(|entry| entry.is_finite()))
        .then_some(right)
}

impl Sketch {
    pub fn spline_handles(&self, spline: EntityId) -> Vec<FitHandle> {
        let Some(Entity::Spline {
            points,
            kind: SplineKind::Fit { .. },
        }) = self.entity(spline)
        else {
            return Vec::new();
        };
        let mut handles: Vec<FitHandle> = Vec::new();
        for (constraint, held) in self.active_constraints() {
            let Constraint::SplineHandle {
                spline: handled,
                line,
            } = *held
            else {
                continue;
            };
            if handled != spline {
                continue;
            }
            let Some((index, anchor, end)) = self.handle_site(points, line) else {
                continue;
            };
            if handles.iter().all(|handle| handle.index != index) {
                handles.push(FitHandle {
                    constraint,
                    index,
                    anchor,
                    end,
                });
            }
        }
        handles.sort_by_key(|handle| handle.index);
        handles
    }

    pub(crate) fn copy_handles(
        &mut self,
        curve_images: &BTreeMap<EntityId, EntityId>,
    ) -> Result<(), SketchError> {
        let copied: Vec<Constraint> = self
            .active_constraints()
            .filter_map(|(_, constraint)| match *constraint {
                Constraint::SplineHandle { spline, line } => Some(Constraint::SplineHandle {
                    spline: *curve_images.get(&spline)?,
                    line: *curve_images.get(&line)?,
                }),
                _ => None,
            })
            .collect();
        for constraint in copied {
            self.add_constraint(constraint)?;
        }
        Ok(())
    }

    pub(crate) fn handle_anchor(&self, spline: EntityId, line: EntityId) -> Option<EntityId> {
        let Some(Entity::Spline { points, .. }) = self.entity(spline) else {
            return None;
        };
        self.handle_site(points, line).map(|(_, anchor, _)| anchor)
    }

    fn handle_site(
        &self,
        points: &[EntityId],
        line: EntityId,
    ) -> Option<(usize, EntityId, EntityId)> {
        let Some(&Entity::Line { start, end }) = self.entity(line) else {
            return None;
        };
        let index_of = |point: EntityId| points.iter().position(|own| *own == point);
        match (index_of(start), index_of(end)) {
            (Some(index), None) => Some((index, start, end)),
            (None, Some(index)) => Some((index, end, start)),
            _ => None,
        }
    }

    pub(crate) fn handled_curve(
        &self,
        positions: &[Point2],
        (closed, spacing): (bool, FitSpacing),
        handles: &[FitHandle],
    ) -> Option<BSpline> {
        let reaches = handles
            .iter()
            .map(|handle| {
                Some((
                    handle.index,
                    self.point(handle.end)? - self.point(handle.anchor)?,
                ))
            })
            .collect::<Option<Vec<_>>>()?;
        let handled: Vec<usize> = handles.iter().map(|handle| handle.index).collect();
        HandledLayout::of(positions, closed, spacing, &handled)?.curve(positions, &reaches)
    }

    pub fn spline_handle_reach(&self, spline: EntityId, point: EntityId) -> Option<Vector2> {
        let Some(Entity::Spline {
            points,
            kind: SplineKind::Fit { closed, spacing },
        }) = self.entity(spline)
        else {
            return None;
        };
        let index = points.iter().position(|own| *own == point)?;
        let positions = points
            .iter()
            .map(|own| self.point(*own))
            .collect::<Option<Vec<_>>>()?;
        let curve = self.spline(spline)?;
        let layout = HandledLayout::of(&positions, *closed, *spacing, &[])?;
        let parameter = layout.parameter(index)?;
        let [slope, _] = curve.derivatives(parameter);
        let reach = slope * layout.arm(index);
        (reach.is_finite() && reach.length() > 0.0).then_some(reach)
    }

    pub fn add_spline_handle(
        &mut self,
        spline: EntityId,
        point: EntityId,
    ) -> Result<(EntityId, ConstraintId), SketchError> {
        let fit_point = matches!(
            self.entity(spline),
            Some(Entity::Spline { points, kind: SplineKind::Fit { .. } })
                if points.contains(&point)
        );
        if !fit_point {
            return Err(SketchError::NotAFitPoint {
                point,
                label: self.entity_label(point),
                spline: self.entity_label(spline),
            });
        }
        let anchor = self.point(point).ok_or(SketchError::MissingEntity(point))?;
        let reach =
            self.spline_handle_reach(spline, point)
                .ok_or_else(|| SketchError::NoTangent {
                    point,
                    label: self.entity_label(point),
                })?;
        let mut working = self.clone();
        let end = working.add_point(anchor + reach);
        let line = working.insert(Entity::Line { start: point, end });
        working.set_construction(line, true)?;
        let constraint = working.add_constraint(Constraint::SplineHandle { spline, line })?;
        *self = working;
        Ok((line, constraint))
    }

    pub(crate) fn check_spline_handle(
        &self,
        constraint: &Constraint,
        spline: EntityId,
        line: EntityId,
        refuse_second: bool,
    ) -> Result<(), SketchError> {
        let needed = "a fit-point spline";
        let Some(Entity::Spline {
            points,
            kind: SplineKind::Fit { .. },
        }) = self.entity(spline)
        else {
            return Err(SketchError::WrongKind {
                entity: spline,
                found: self.entity_label(spline),
                needed,
            });
        };
        let Some(Entity::Line { .. }) = self.entity(line) else {
            return Err(SketchError::WrongKind {
                entity: line,
                found: self.entity_label(line),
                needed: "a line",
            });
        };
        let Some((index, anchor, _)) = self.handle_site(points, line) else {
            return Err(SketchError::NotJoined {
                constraint: constraint.kind_name(),
                first: self.entity_label(spline),
                second: self.entity_label(line),
            });
        };
        if refuse_second
            && self
                .spline_handles(spline)
                .iter()
                .any(|handle| handle.index == index)
        {
            return Err(SketchError::AlreadyHandled {
                point: anchor,
                label: self.entity_label(anchor),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
