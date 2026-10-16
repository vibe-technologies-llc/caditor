use std::cmp::Ordering;

use caditor_geometry::{Point2, Vector3};

use crate::{
    constraint::Constraint,
    curve::{BSpline, Faceting, MAX_SPLINE_DEGREE, MIN_CLOSED_POINTS, conic_weight},
    entity::{Entity, MIN_PIECE_POINTS, Role, SplineKind},
    id::{ConstraintId, EntityId},
    intersect::closest_spline_parameter,
    sketch::{DetachedConstraint, Sketch, SketchError},
    trim::{Cut, Piece, Reach, TOLERANCE, TrimError, Trimmed},
};

const KNOT_SNAP: f64 = 1e-9;
const SAME_POINT: f64 = 1e-9;
const UNIFORM_SLACK: f64 = 1e-9;
const CLOSED_TURNS: usize = 2;
const END_SAMPLES: usize = 256;
const MAX_DOUBLINGS: i32 = 40;
const WINDOW_SAMPLES: usize = 64;
const REACH_MARGIN: f64 = 2.0;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SplineCourse {
    pub(super) spline: BSpline,
    pub(super) closed: bool,
    pub(super) length: f64,
}

impl SplineCourse {
    pub(super) fn new(spline: BSpline, closed: bool) -> Self {
        let length = spline.length();
        Self {
            spline,
            closed,
            length,
        }
    }

    pub(super) fn reach_of(spline: &BSpline) -> f64 {
        spline
            .control_points()
            .iter()
            .map(|point| point.x.abs().max(point.y.abs()))
            .fold(0.0, f64::max)
    }

    pub(super) fn extent(&self) -> f64 {
        Self::reach_of(&self.spline)
    }

    pub(super) fn parameter(&self, point: Point2) -> f64 {
        let parameter = closest_spline_parameter(&self.spline, point);
        if self.closed && parameter >= 1.0 {
            0.0
        } else {
            parameter
        }
    }

    pub(super) fn at(&self, parameter: f64) -> Point2 {
        let parameter = if self.closed {
            parameter.rem_euclid(1.0)
        } else {
            parameter
        };
        self.spline.point_at(parameter)
    }

    pub(super) fn faceted(&self, from: f64, to: f64, faceting: Faceting) -> Vec<Point2> {
        let share = (to - from).abs();
        let segments = (faceting.spline_segments(&self.spline) as f64 * share)
            .ceil()
            .max(1.0) as usize;
        (0..=segments)
            .map(|index| self.at(from + (to - from) * index as f64 / segments as f64))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Knotted {
    degree: usize,
    knots: Vec<f64>,
    points: Vec<Vector3>,
}

impl Knotted {
    pub(crate) fn of(spline: &BSpline) -> Self {
        let weights = spline.weights();
        let points = spline
            .control_points()
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let weight = weights
                    .and_then(|weights| weights.get(index))
                    .copied()
                    .unwrap_or(1.0);
                (*point * weight).extend(weight)
            })
            .collect();
        Self {
            degree: spline.degree(),
            knots: spline.knots().to_vec(),
            points,
        }
    }

    fn periodic(points: &[Point2], turns: usize) -> Option<Self> {
        let count = points.len();
        if count < MIN_CLOSED_POINTS {
            return None;
        }
        let degree = MAX_SPLINE_DEGREE;
        let total = count * turns + degree;
        let wrapped = (0..total)
            .map(|index| points.get(index % count).map(|point| point.extend(1.0)))
            .collect::<Option<Vec<_>>>()?;
        let knots = (0..=total + degree)
            .map(|index| (index as f64 - degree as f64) / count as f64)
            .collect();
        Some(Self {
            degree,
            knots,
            points: wrapped,
        })
    }

    pub(crate) fn domain(&self) -> Option<(f64, f64)> {
        Some((
            *self.knots.get(self.degree)?,
            *self.knots.get(self.points.len())?,
        ))
    }

    pub(crate) fn end_span(&self, at_end: bool) -> Option<f64> {
        let (low, high) = self.domain()?;
        let inner = self
            .knots
            .iter()
            .copied()
            .filter(|knot| *knot > low && *knot < high);
        let width = if at_end {
            high - inner.fold(low, f64::max)
        } else {
            inner.fold(high, f64::min) - low
        };
        (width > 0.0).then_some(width)
    }

    fn span_at(&self, parameter: f64) -> Option<usize> {
        let last = self.points.len().checked_sub(1)?;
        let above = self.knots.partition_point(|knot| *knot <= parameter);
        Some(above.saturating_sub(1).clamp(self.degree.min(last), last))
    }

    pub(crate) fn homogeneous_at(&self, parameter: f64) -> Option<Vector3> {
        self.blossom(self.span_at(parameter)?, &vec![parameter; self.degree])
    }

    fn blossom(&self, span: usize, arguments: &[f64]) -> Option<Vector3> {
        let degree = self.degree;
        let first = span.checked_sub(degree)?;
        let mut points = self.points.get(first..=span)?.to_vec();
        for level in 1..=degree {
            let argument = *arguments.get(level - 1)?;
            for local in (level..=degree).rev() {
                let index = first + local;
                let low = *self.knots.get(index)?;
                let high = *self.knots.get(index + degree + 1 - level)?;
                let share = if high > low {
                    (argument - low) / (high - low)
                } else {
                    0.0
                };
                let previous = *points.get(local - 1)?;
                let current = points.get_mut(local)?;
                *current = previous.lerp(*current, share);
            }
        }
        points.get(degree).copied()
    }

    fn snapped(&self, parameter: f64) -> f64 {
        let Some((low, high)) = self.domain() else {
            return parameter;
        };
        let reach = KNOT_SNAP * (high - low);
        self.knots
            .iter()
            .copied()
            .filter(|knot| (*knot - parameter).abs() <= reach)
            .min_by(|a, b| (a - parameter).abs().total_cmp(&(b - parameter).abs()))
            .unwrap_or(parameter)
    }

    pub(crate) fn between(&self, from: f64, to: f64) -> Option<Part> {
        let (from, to) = (self.snapped(from), self.snapped(to));
        let (low, high) = self.domain()?;
        if from.partial_cmp(&to) != Some(Ordering::Less) {
            return None;
        }
        let degree = self.degree;
        let inner = |knot: &f64| *knot > from.max(low) && *knot < to.min(high);
        let knots: Vec<f64> = std::iter::repeat_n(from, degree + 1)
            .chain(self.knots.iter().copied().filter(inner))
            .chain(std::iter::repeat_n(to, degree + 1))
            .collect();
        let count = knots.len().checked_sub(degree + 1)?;
        let mut points = Vec::with_capacity(count);
        let mut sources = Vec::with_capacity(count);
        for index in 0..count {
            let window = knots.get(index + 1..=index + degree)?;
            let span = (index..=index + degree).find(
                |span| matches!(knots.get(*span..=*span + 1), Some([low, high]) if low < high),
            )?;
            let middle = (knots.get(span)? + knots.get(span + 1)?) / 2.0;
            let point = self.blossom(self.span_at(middle)?, window)?;
            let source = (0..self.points.len()).find(|old| {
                self.knots.get(old + 1..=old + degree) == Some(window)
                    && self.points.get(*old).is_some_and(|known| {
                        known.distance(point) <= SAME_POINT * known.length().max(1.0)
                    })
            });
            points.push(
                source
                    .and_then(|old| self.points.get(old))
                    .copied()
                    .unwrap_or(point),
            );
            sources.push(source);
        }
        Some(Part {
            degree,
            knots,
            points,
            sources,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Part {
    degree: usize,
    knots: Vec<f64>,
    points: Vec<Vector3>,
    sources: Vec<Option<usize>>,
}

impl Part {
    fn position(point: Vector3) -> Option<Point2> {
        let position = point.truncate() / point.z;
        (point.z > 0.0 && position.is_finite()).then_some(position)
    }

    pub(crate) fn spline(&self, rational: bool) -> Option<BSpline> {
        let (low, high) = (*self.knots.first()?, *self.knots.last()?);
        let width = high - low;
        if width.partial_cmp(&0.0) != Some(Ordering::Greater) {
            return None;
        }
        let positions = self
            .points
            .iter()
            .map(|point| Self::position(*point))
            .collect::<Option<Vec<_>>>()?;
        let weights = rational.then(|| self.points.iter().map(|point| point.z).collect());
        Some(BSpline::from_parts(
            positions,
            self.degree,
            self.knots.iter().map(|knot| (knot - low) / width).collect(),
            weights,
        ))
    }

    fn kind(&self, width: Option<f64>, rational: bool) -> Option<SplineKind> {
        let count = self.points.len();
        if rational {
            let [first, apex, last] = self.points.as_slice() else {
                return None;
            };
            let weight = apex.z / (first.z * last.z).sqrt();
            let rho = weight / (1.0 + weight);
            return (SplineKind::rho_is_valid(rho) && conic_weight(rho).is_some())
                .then_some(SplineKind::Conic { rho });
        }
        if count == self.degree + 1 && self.degree == MAX_SPLINE_DEGREE.min(count - 1) {
            return Some(SplineKind::OPEN);
        }
        if self.degree != MAX_SPLINE_DEGREE || count < MIN_PIECE_POINTS {
            return None;
        }
        let width = width.filter(|width| *width > 0.0)?;
        let knot = |index: usize| self.knots.get(index).copied();
        let lead = (knot(self.degree + 1)? - knot(self.degree)?) / width;
        let trail = (knot(count)? - knot(count - 1)?) / width;
        if (lead - 1.0).abs() <= UNIFORM_SLACK && (trail - 1.0).abs() <= UNIFORM_SLACK {
            return Some(SplineKind::OPEN);
        }
        (SplineKind::end_span_is_valid(lead) && SplineKind::end_span_is_valid(trail))
            .then_some(SplineKind::Piece { lead, trail })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Slot {
    Kept(EntityId),
    At(Point2),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Reshape {
    pub(crate) kind: SplineKind,
    pub(crate) slots: Vec<Slot>,
}

impl Reshape {
    pub(crate) fn set_end(&mut self, at_end: bool, slot: Slot) {
        let end = if at_end {
            self.slots.last_mut()
        } else {
            self.slots.first_mut()
        };
        if let Some(end) = end {
            *end = slot;
        }
    }
}

pub(crate) struct SplineSource {
    pub(crate) knotted: Knotted,
    pub(crate) rational: bool,
    width: Option<f64>,
    points: Vec<EntityId>,
    kind: SplineKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Stretch {
    from: f64,
    to: f64,
}

struct Halves {
    near: Stretch,
    far: Stretch,
    near_shape: Reshape,
    far_shape: Reshape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fate {
    Kept,
    Moved,
    Dropped,
}

#[derive(Default)]
struct Fates {
    dropped: Vec<ConstraintId>,
    moved: Vec<(ConstraintId, DetachedConstraint)>,
}

impl Fates {
    fn leaving(&self) -> Vec<ConstraintId> {
        self.dropped
            .iter()
            .copied()
            .chain(self.moved.iter().map(|(id, _)| *id))
            .collect()
    }
}

impl Sketch {
    pub(crate) fn spline_source(&self, curve: EntityId) -> Option<SplineSource> {
        let Entity::Spline { points, kind } = self.entity(curve)?.clone() else {
            return None;
        };
        let positions = points
            .iter()
            .map(|point| self.point(*point))
            .collect::<Option<Vec<_>>>()?;
        let count = positions.len();
        let (knotted, width) = match kind {
            SplineKind::Control { closed: true } => (
                Knotted::periodic(&positions, CLOSED_TURNS)?,
                Some(1.0 / count as f64),
            ),
            SplineKind::Control { closed: false } => {
                let spline = BSpline::clamped(positions)?;
                let spans = count.saturating_sub(spline.degree()).max(1);
                (Knotted::of(&spline), Some(1.0 / spans as f64))
            }
            SplineKind::Piece { lead, trail } => {
                let spans = lead + count.checked_sub(MIN_PIECE_POINTS)? as f64 + trail;
                (
                    Knotted::of(&BSpline::piece(positions, lead, trail)?),
                    Some(1.0 / spans),
                )
            }
            SplineKind::Conic { .. } | SplineKind::Fit { .. } => {
                (Knotted::of(&self.spline(curve)?), None)
            }
        };
        Some(SplineSource {
            knotted,
            rational: matches!(kind, SplineKind::Conic { .. }),
            width,
            points,
            kind,
        })
    }

    pub(crate) fn spline_reshape(&self, curve: EntityId, from: f64, to: f64) -> Option<Reshape> {
        let source = self.spline_source(curve)?;
        if let SplineKind::Fit { closed, spacing } = source.kind {
            let kind = SplineKind::Fit {
                closed: false,
                spacing,
            };
            return Some(Reshape {
                kind,
                slots: self.fit_slots(curve, &source.points, closed, Stretch { from, to })?,
            });
        }
        let part = source.knotted.between(from, to)?;
        let kind = part.kind(source.width, source.rational)?;
        Some(Reshape {
            kind,
            slots: source.slots(&part)?,
        })
    }

    fn fit_slots(
        &self,
        curve: EntityId,
        points: &[EntityId],
        closed: bool,
        kept: Stretch,
    ) -> Option<Vec<Slot>> {
        let layout = self.fit_layout(curve)?;
        let spline = self.spline(curve)?;
        let at = |parameter: f64| {
            let parameter = if closed {
                parameter.rem_euclid(1.0)
            } else {
                parameter
            };
            spline.point_at(parameter)
        };
        let (start, end) = (at(kept.from), at(kept.to));
        let tolerance = TOLERANCE * SplineCourse::reach_of(&spline).max(1.0);
        let start_slot = match points.first() {
            Some(first) if !closed && kept.from <= 0.0 => Slot::Kept(*first),
            _ => Slot::At(start),
        };
        let end_slot = match points.last() {
            Some(last) if !closed && kept.to >= 1.0 => Slot::Kept(*last),
            _ => Slot::At(end),
        };
        let mut inside: Vec<(f64, EntityId)> = points
            .iter()
            .zip(&layout.parameters)
            .filter_map(|(point, parameter)| {
                let along = if closed && *parameter <= kept.from {
                    parameter + 1.0
                } else {
                    *parameter
                };
                let position = self.point(*point)?;
                let clear =
                    position.distance(start) > tolerance && position.distance(end) > tolerance;
                (along > kept.from && along < kept.to && clear).then_some((along, *point))
            })
            .collect();
        inside.sort_by(|a, b| a.0.total_cmp(&b.0));
        Some(
            std::iter::once(start_slot)
                .chain(inside.into_iter().map(|(_, point)| Slot::Kept(point)))
                .chain(std::iter::once(end_slot))
                .collect(),
        )
    }

    pub(crate) fn trim_spline(&mut self, piece: &Piece) -> Result<Trimmed, TrimError> {
        let curve = piece.curve;
        let Some(Entity::Spline { points, kind }) = self.entity(curve).cloned() else {
            return Err(TrimError::NoSuchCurve(curve));
        };
        let label = self.entity_label(curve);
        let no_piece = || TrimError::NoPiece {
            entity: curve,
            label: label.clone(),
        };
        let reshape = |stretch: Stretch| {
            self.spline_reshape(curve, stretch.from, stretch.to)
                .ok_or_else(no_piece)
        };
        match (piece.start, piece.end) {
            (Some(first), Some(second)) if kind.is_closed() => {
                let to = if first.parameter > second.parameter {
                    first.parameter
                } else {
                    first.parameter + 1.0
                };
                let kept = Stretch {
                    from: second.parameter,
                    to,
                };
                let shape = reshape(kept)?;
                let fates = self.spline_fates(curve, kept, None);
                let (start, end) = self.place_spline(curve, shape, &fates.leaving())?;
                self.join(start, &second, curve)?;
                self.join(end, &first, curve)?;
                self.drop_unused(&points)?;
                Ok(Trimmed::Opened)
            }
            (None, Some(cut)) => self.shorten_spline(curve, &points, cut, false),
            (Some(cut), None) => self.shorten_spline(curve, &points, cut, true),
            (Some(first), Some(second)) => {
                let near = Stretch {
                    from: 0.0,
                    to: first.parameter,
                };
                let far = Stretch {
                    from: second.parameter,
                    to: 1.0,
                };
                let (near_shape, far_shape) = (reshape(near)?, reshape(far)?);
                let fates = self.spline_fates(curve, near, Some(far));
                let (_, near_end) = self.place_spline(curve, near_shape, &fates.leaving())?;
                let far_points = self.slot_points(&far_shape.slots);
                let far_start = *far_points.first().ok_or_else(no_piece)?;
                let split = self.add_piece(
                    curve,
                    Entity::Spline {
                        points: far_points,
                        kind: far_shape.kind,
                    },
                )?;
                self.move_constraints(fates.moved, curve, split)?;
                self.join(near_end, &first, curve)?;
                self.join(far_start, &second, curve)?;
                self.drop_unused(&points)?;
                Ok(Trimmed::Split { piece: split })
            }
            (None, None) => {
                self.delete_curve(curve)?;
                Ok(Trimmed::Deleted)
            }
        }
    }

    pub(crate) fn spline_split_is_inside(
        &self,
        curve: EntityId,
        at: Point2,
        tolerance: f64,
    ) -> Result<bool, SketchError> {
        let spline = self.spline(curve).ok_or(SketchError::NoSuchEntity(curve))?;
        let length = spline.length();
        if length.partial_cmp(&0.0) != Some(Ordering::Greater) {
            return Ok(false);
        }
        let parameter = closest_spline_parameter(&spline, at);
        let slack = tolerance / length;
        let inside = spline.point_at(parameter).distance(at) <= tolerance
            && parameter > slack
            && parameter < 1.0 - slack;
        if inside {
            self.spline_halves(curve, parameter)?;
        }
        Ok(inside)
    }

    fn spline_halves(&self, curve: EntityId, parameter: f64) -> Result<Halves, SketchError> {
        let no_piece = || SketchError::NoExactPiece {
            entity: curve,
            label: self.entity_label(curve),
        };
        let near = Stretch {
            from: 0.0,
            to: parameter,
        };
        let far = Stretch {
            from: parameter,
            to: 1.0,
        };
        Ok(Halves {
            near,
            far,
            near_shape: self
                .spline_reshape(curve, near.from, near.to)
                .ok_or_else(no_piece)?,
            far_shape: self
                .spline_reshape(curve, far.from, far.to)
                .ok_or_else(no_piece)?,
        })
    }

    pub(crate) fn split_spline(
        &mut self,
        curve: EntityId,
        point: EntityId,
    ) -> Result<EntityId, SketchError> {
        let Some(Entity::Spline { points, .. }) = self.entity(curve).cloned() else {
            return Err(SketchError::NoSuchEntity(curve));
        };
        let at = self.point(point).ok_or(SketchError::NoSuchEntity(point))?;
        let spline = self.spline(curve).ok_or(SketchError::NoSuchEntity(curve))?;
        let mut halves = self.spline_halves(curve, closest_spline_parameter(&spline, at))?;
        let fates = self.spline_fates(curve, halves.near, Some(halves.far));
        halves.near_shape.set_end(true, Slot::Kept(point));
        halves.far_shape.set_end(false, Slot::Kept(point));
        self.place_spline(curve, halves.near_shape, &fates.leaving())?;
        let far_points = self.slot_points(&halves.far_shape.slots);
        let piece = self.add_piece(
            curve,
            Entity::Spline {
                points: far_points,
                kind: halves.far_shape.kind,
            },
        )?;
        self.move_constraints(fates.moved, curve, piece)?;
        self.drop_unused(&points)?;
        Ok(piece)
    }

    fn shorten_spline(
        &mut self,
        curve: EntityId,
        points: &[EntityId],
        cut: Cut,
        keeps_start: bool,
    ) -> Result<Trimmed, TrimError> {
        let kept = if keeps_start {
            Stretch {
                from: 0.0,
                to: cut.parameter,
            }
        } else {
            Stretch {
                from: cut.parameter,
                to: 1.0,
            }
        };
        let shape = self
            .spline_reshape(curve, kept.from, kept.to)
            .ok_or_else(|| TrimError::NoPiece {
                entity: curve,
                label: self.entity_label(curve),
            })?;
        let fates = self.spline_fates(curve, kept, None);
        let (start, end) = self.place_spline(curve, shape, &fates.leaving())?;
        let cut_end = if keeps_start { end } else { start };
        self.join(cut_end, &cut, curve)?;
        self.drop_unused(points)?;
        Ok(Trimmed::Shortened)
    }

    pub(crate) fn slot_points(&mut self, slots: &[Slot]) -> Vec<EntityId> {
        let mut placed: Vec<EntityId> = Vec::with_capacity(slots.len());
        for slot in slots {
            let point = match *slot {
                Slot::Kept(point) if !placed.contains(&point) => point,
                Slot::Kept(point) => {
                    let position = self.point(point).unwrap_or(Point2::ZERO);
                    self.add_point(position)
                }
                Slot::At(position) => self.add_point(position),
            };
            placed.push(point);
        }
        placed
    }

    pub(crate) fn place_spline(
        &mut self,
        curve: EntityId,
        shape: Reshape,
        leaving: &[ConstraintId],
    ) -> Result<(EntityId, EntityId), SketchError> {
        let points = self.slot_points(&shape.slots);
        let ends = (points.first().copied(), points.last().copied());
        self.restructure(
            curve,
            Entity::Spline {
                points,
                kind: shape.kind,
            },
            leaving,
            |_| true,
        )?;
        match ends {
            (Some(start), Some(end)) => Ok((start, end)),
            _ => Err(SketchError::TooFewControlPoints),
        }
    }

    pub(crate) fn drop_unused(&mut self, points: &[EntityId]) -> Result<(), SketchError> {
        let mut seen = Vec::with_capacity(points.len());
        for point in points {
            if !seen.contains(point) {
                seen.push(*point);
                self.drop_if_unused(*point)?;
            }
        }
        Ok(())
    }

    fn spline_fates(&self, curve: EntityId, near: Stretch, far: Option<Stretch>) -> Fates {
        let mut fates = Fates::default();
        let Some(spline) = self.spline(curve) else {
            return fates;
        };
        let closed = self
            .entity(curve)
            .and_then(Entity::spline_kind)
            .is_some_and(SplineKind::is_closed);
        let length = spline.length();
        let slack = if length > 0.0 {
            TOLERANCE * SplineCourse::reach_of(&spline).max(1.0) / length
        } else {
            0.0
        };
        let on = |stretch: Stretch, parameter: f64| {
            let within = |value: f64| value >= stretch.from - slack && value <= stretch.to + slack;
            within(parameter) || (closed && within(parameter + 1.0))
        };
        let fate_at = |position: Point2| {
            let parameter = closest_spline_parameter(&spline, position);
            if on(near, parameter) {
                Fate::Kept
            } else if far.is_some_and(|far| on(far, parameter)) {
                Fate::Moved
            } else {
                Fate::Dropped
            }
        };
        let ends = self.entity(curve).and_then(Entity::spline_ends);
        let keeps_start = near.from <= 0.0;
        let keeps_end = near.to >= 1.0;
        let far_keeps_end = far.is_some_and(|far| far.to >= 1.0);
        for id in self.constraints_using(curve) {
            let Some(constraint) = self.constraint(id) else {
                continue;
            };
            let other = |a: EntityId, b: EntityId| if a == curve { b } else { a };
            let fate = match *constraint {
                Constraint::Equal(..) | Constraint::Rho { .. } => Fate::Dropped,
                Constraint::Coincident(a, b) => self.point(other(a, b)).map_or(Fate::Kept, fate_at),
                Constraint::Distance { from, to, .. } => self
                    .measured_on_spline(curve, &spline, other(from, to))
                    .map_or(Fate::Kept, fate_at),
                Constraint::Tangent(a, b) | Constraint::Curvature(a, b) => {
                    let other = other(a, b);
                    match ends {
                        Some((start, end)) => {
                            let at_start = self.joined(start, other);
                            let at_end = self.joined(end, other);
                            let held = (at_start && keeps_start) || (at_end && keeps_end);
                            if held || !(at_start || at_end) {
                                Fate::Kept
                            } else if at_end && far_keeps_end {
                                Fate::Moved
                            } else {
                                Fate::Dropped
                            }
                        }
                        None => Fate::Kept,
                    }
                }
                _ => Fate::Kept,
            };
            match fate {
                Fate::Kept => {}
                Fate::Dropped => fates.dropped.push(id),
                Fate::Moved => match self.copy_constraint(id) {
                    Some(detached) => fates.moved.push((id, detached)),
                    None => fates.dropped.push(id),
                },
            }
        }
        fates
    }

    fn measured_on_spline(
        &self,
        curve: EntityId,
        spline: &BSpline,
        other: EntityId,
    ) -> Option<Point2> {
        if self.role(other) == Some(Role::Point) {
            return self.point(other);
        }
        let (one, two) = self
            .spline_gap(curve, other)
            .or_else(|| self.ellipse_gap(other, curve))?;
        let distance = |point: Point2| {
            spline
                .point_at(closest_spline_parameter(spline, point))
                .distance(point)
        };
        Some(if distance(one) <= distance(two) {
            one
        } else {
            two
        })
    }

    pub(crate) fn spline_end_is_last(&self, curve: EntityId, near: Point2) -> Option<bool> {
        let spline = self.spline(curve)?;
        let parameter = closest_spline_parameter(&spline, near);
        let samples: Vec<Point2> = (0..=END_SAMPLES)
            .map(|index| spline.point_at(index as f64 / END_SAMPLES as f64))
            .collect();
        let (mut before, mut after) = (0.0, 0.0);
        for (index, pair) in samples.windows(2).enumerate() {
            let [low, high] = pair else {
                continue;
            };
            let middle = (index as f64 + 0.5) / END_SAMPLES as f64;
            if middle < parameter {
                before += low.distance(*high);
            } else {
                after += low.distance(*high);
            }
        }
        Some(before >= after)
    }
}

impl SplineSource {
    fn slots(&self, part: &Part) -> Option<Vec<Slot>> {
        let count = self.points.len().max(1);
        part.points
            .iter()
            .zip(&part.sources)
            .map(|(point, source)| match source {
                Some(old) => self.points.get(old % count).copied().map(Slot::Kept),
                None => Part::position(*point).map(Slot::At),
            })
            .collect()
    }

    pub(crate) fn extended(
        &self,
        at_end: bool,
        reached: f64,
        end: EntityId,
        old_end: Point2,
    ) -> Option<Reshape> {
        let (low, high) = self.knotted.domain()?;
        if let SplineKind::Fit { .. } = self.kind {
            let mut slots: Vec<Slot> = self.points.iter().copied().map(Slot::Kept).collect();
            let inner = Slot::At(old_end);
            if at_end {
                slots.insert(slots.len().saturating_sub(1), inner);
            } else {
                slots.insert(1.min(slots.len()), inner);
            }
            return Some(Reshape {
                kind: self.kind,
                slots,
            });
        }
        let part = if at_end {
            self.knotted.between(low, reached)?
        } else {
            self.knotted.between(reached, high)?
        };
        let mut shape = Reshape {
            kind: part.kind(self.width, self.rational)?,
            slots: self.slots(&part)?,
        };
        shape.set_end(at_end, Slot::Kept(end));
        Some(shape)
    }
}

impl Sketch {
    pub(super) fn reach_along_spline(
        &self,
        curve: EntityId,
        end: EntityId,
        at_end: bool,
    ) -> Option<(Point2, EntityId, Reach)> {
        let source = self.spline_source(curve)?;
        let own = self.spline(curve)?;
        let (low, high) = source.knotted.domain()?;
        let span = source.knotted.end_span(at_end)?;
        let end_parameter = if at_end { 1.0 } else { 0.0 };
        let from = own.point_at(end_parameter);
        let [tangent, _] = own.derivatives(end_parameter);
        let outward = (if at_end { tangent } else { -tangent }).try_normalize()?;
        let bound = REACH_MARGIN
            * self
                .cutters(curve)
                .map(|(_, cutter)| cutter.extent())
                .chain([SplineCourse::reach_of(&own)])
                .fold(1.0, f64::max);
        let tolerance = TOLERANCE * bound;
        let beyond = |along: f64| if at_end { high + along } else { low - along };
        let window_of = |near: f64, far: f64| {
            let part = if at_end {
                source.knotted.between(beyond(near), beyond(far))?
            } else {
                source.knotted.between(beyond(far), beyond(near))?
            };
            part.spline(source.rational)
        };
        let mut reached = 0.0;
        let mut previous = from;
        for doubling in 0..MAX_DOUBLINGS {
            let length = span * 2f64.powi(doubling);
            let step = (length - reached) / WINDOW_SAMPLES as f64;
            let mut limit = length;
            let mut last = false;
            for index in 1..=WINDOW_SAMPLES {
                let along = reached + step * index as f64;
                let onward = source
                    .knotted
                    .homogeneous_at(beyond(along))
                    .and_then(Part::position)
                    .filter(|point| {
                        point.x.abs().max(point.y.abs()) <= bound
                            && (*point - previous).dot(outward) > 0.0
                    });
                match onward {
                    Some(point) => previous = point,
                    None => {
                        limit = reached + step * (index - 1) as f64;
                        last = true;
                        break;
                    }
                }
            }
            if limit > reached {
                let window = window_of(reached, limit)?;
                let nearest = self
                    .cutters(curve)
                    .flat_map(|(target, cutter)| {
                        cutter
                            .spline_cut_positions(&window, tolerance)
                            .into_iter()
                            .map(move |point| (point, target))
                    })
                    .filter(|(point, _)| point.distance(from) > tolerance)
                    .map(|(point, target)| {
                        let parameter = closest_spline_parameter(&window, point);
                        let share = if at_end { parameter } else { 1.0 - parameter };
                        (reached + (limit - reached) * share, point, target)
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                if let Some((along, to, target)) = nearest {
                    let shape = source.extended(at_end, beyond(along), end, from)?;
                    let continuation = window_of(0.0, along)?;
                    return Some((
                        to,
                        target,
                        Reach::Spline {
                            continuation,
                            shape,
                        },
                    ));
                }
            }
            if last {
                return None;
            }
            reached = limit;
        }
        None
    }

    pub(super) fn extend_spline(
        &mut self,
        curve: EntityId,
        shape: &Reshape,
    ) -> Result<(), SketchError> {
        let old = self
            .entity(curve)
            .map(Entity::points)
            .ok_or(SketchError::NoSuchEntity(curve))?;
        self.place_spline(curve, shape.clone(), &[])?;
        self.drop_unused(&old)
    }
}

#[cfg(test)]
mod tests;
