use caditor_geometry::Point2;

use crate::{
    curve::{BSpline, MAX_SPLINE_DEGREE, looped_knots},
    entity::{Entity, SplineKind},
    id::EntityId,
    intersect::closest_spline_parameter,
    sketch::{Sketch, SketchError},
    trim::spline::{Inserted, Knotted},
};

const SPAN_MARGIN: f64 = 0.05;
const SPAN_SAMPLES: usize = 8;
const ELEVATION_SHARE: f64 = 1.0 / 3.0;
const HALFWAY: f64 = 0.5;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum InsertError {
    #[error("{label} is not a spline")]
    NotASpline { entity: EntityId, label: String },
    #[error(
        "{label} passes through its points, so a point added to it would change its shape; only \
         splines drawn from control points can take one"
    )]
    FitSpline { entity: EntityId, label: String },
    #[error(
        "{label} is a conic, which has exactly its start, apex and end; a point added to it would \
         change its shape"
    )]
    Conic { entity: EntityId, label: String },
    #[error("{label} follows the geometry it was projected from, so it cannot be changed")]
    Projected { entity: EntityId, label: String },
    #[error("{label} has no length")]
    NoLength { entity: EntityId, label: String },
    #[error(
        "adding a point here moves {label}, which has constraints or other curves on it; remove \
         them, or add the point elsewhere along the spline"
    )]
    HeldPoint { point: EntityId, label: String },
    #[error(transparent)]
    Edit(SketchError),
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct KnotSpan {
    low: f64,
    high: f64,
}

enum Form {
    Straight,
    Quadratic,
    Knotted(Knotted),
    Looped(Vec<f64>),
}

struct Source {
    points: Vec<EntityId>,
    positions: Vec<Point2>,
    form: Form,
    spans: Vec<KnotSpan>,
    shape: BSpline,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Slot {
    Kept(EntityId),
    Moved(EntityId, Point2),
    Added(Point2),
}

struct Plan {
    kind: SplineKind,
    slots: Vec<Slot>,
}

impl Plan {
    fn moved(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.slots.iter().filter_map(|slot| match slot {
            Slot::Moved(point, _) => Some(*point),
            Slot::Kept(_) | Slot::Added(_) => None,
        })
    }

    fn of_inserted(points: &[EntityId], inserted: Inserted) -> Option<Self> {
        let Inserted {
            kind,
            positions,
            first,
            span,
        } = inserted;
        let slots = positions
            .iter()
            .enumerate()
            .map(|(index, position)| {
                Some(if index < first {
                    Slot::Kept(*points.get(index)?)
                } else if index < span {
                    Slot::Moved(*points.get(index)?, *position)
                } else if index == span {
                    Slot::Added(*position)
                } else {
                    Slot::Kept(*points.get(index.checked_sub(1)?)?)
                })
            })
            .collect::<Option<_>>()?;
        Some(Self { kind, slots })
    }

    fn straight(points: &[EntityId], positions: &[Point2], parameter: f64) -> Option<Self> {
        let (&[start, end], &[from, to]) = (points, positions) else {
            return None;
        };
        Some(Self {
            kind: SplineKind::OPEN,
            slots: vec![
                Slot::Kept(start),
                Slot::Added(from.lerp(to, parameter)),
                Slot::Kept(end),
            ],
        })
    }

    fn elevated(points: &[EntityId], positions: &[Point2], parameter: f64) -> Option<Self> {
        let (&[start, middle, end], &[from, apex, to]) = (points, positions) else {
            return None;
        };
        let leading = from.lerp(apex, 1.0 - ELEVATION_SHARE);
        let trailing = apex.lerp(to, ELEVATION_SHARE);
        let (inner, outer) = if parameter < HALFWAY {
            (Slot::Added(leading), Slot::Moved(middle, trailing))
        } else {
            (Slot::Moved(middle, leading), Slot::Added(trailing))
        };
        Some(Self {
            kind: SplineKind::OPEN,
            slots: vec![Slot::Kept(start), inner, outer, Slot::Kept(end)],
        })
    }

    fn looped(
        points: &[EntityId],
        positions: &[Point2],
        spans: &[f64],
        span: usize,
        parameter: f64,
    ) -> Option<Self> {
        let count = points.len();
        let knots = looped_knots(spans)?;
        let knot = |index: usize| knots.get(index).copied();
        let degree = MAX_SPLINE_DEGREE;
        let start = span + degree;
        let blended = |index: usize| {
            let low = knot(index)?;
            let share = (parameter - low) / (knot(index + degree)? - low);
            let previous = *positions.get((index - 1) % count)?;
            let current = *positions.get(index % count)?;
            share.is_finite().then(|| previous.lerp(current, share))
        };
        let point = |index: usize| points.get(index % count).copied();
        let mut slots = vec![
            Slot::Kept(point(span)?),
            Slot::Moved(point(span + 1)?, blended(start - 2)?),
            Slot::Moved(point(span + 2)?, blended(start - 1)?),
            Slot::Added(blended(start)?),
        ];
        for index in span + degree..span + count {
            slots.push(Slot::Kept(point(index)?));
        }
        let width = |index: usize| Some(knot(index + 1)? - knot(index)?);
        let mut widths = vec![parameter - knot(start)?, knot(start + 1)? - parameter];
        for index in span + 1..span + count {
            widths.push(width(index % count + degree)?);
        }
        let turn = (count + 1 - span) % (count + 1);
        slots.rotate_left(turn);
        widths.rotate_left(turn);
        let total: f64 = widths.iter().sum();
        let spans: Vec<f64> = widths.iter().map(|width| width / total).collect();
        SplineKind::spans_are_valid(&spans).then_some(Self {
            kind: SplineKind::Knotted {
                spans,
                closed: true,
            },
            slots,
        })
    }
}

impl Sketch {
    pub fn check_point_insertion(&self, curve: EntityId) -> Result<(), InsertError> {
        self.insertion_source(curve).map(|_| ())
    }

    pub fn insert_control_point(
        &mut self,
        curve: EntityId,
        near: Option<Point2>,
    ) -> Result<EntityId, InsertError> {
        let source = self.insertion_source(curve)?;
        let plan = self.plan_near(curve, &source, near)?;
        let mut working = self.clone();
        let point = working.place_plan(curve, plan).map_err(InsertError::Edit)?;
        *self = working;
        Ok(point)
    }

    fn insertion_source(&self, curve: EntityId) -> Result<Source, InsertError> {
        let label = || self.entity_label(curve);
        let Some(Entity::Spline { points, kind }) = self.entity(curve) else {
            return Err(InsertError::NotASpline {
                entity: curve,
                label: label(),
            });
        };
        if self.is_projected(curve) {
            return Err(InsertError::Projected {
                entity: curve,
                label: label(),
            });
        }
        let no_length = || InsertError::NoLength {
            entity: curve,
            label: label(),
        };
        let count = points.len();
        let form = match kind {
            SplineKind::Fit { .. } => {
                return Err(InsertError::FitSpline {
                    entity: curve,
                    label: label(),
                });
            }
            SplineKind::Conic { .. } => {
                return Err(InsertError::Conic {
                    entity: curve,
                    label: label(),
                });
            }
            SplineKind::Control { closed: false } if count == 2 => Form::Straight,
            SplineKind::Control { closed: false } if count == 3 => Form::Quadratic,
            SplineKind::Control { closed: false }
            | SplineKind::Piece { .. }
            | SplineKind::Graded { .. }
            | SplineKind::Knotted { closed: false, .. } => {
                Form::Knotted(self.spline_source(curve).ok_or_else(no_length)?.knotted)
            }
            SplineKind::Control { closed: true } => Form::Looped(vec![1.0; count]),
            SplineKind::Knotted {
                spans,
                closed: true,
            } => Form::Looped(spans.clone()),
        };
        let positions = points
            .iter()
            .map(|point| self.point(*point))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(no_length)?;
        let shape = self.spline(curve).ok_or_else(no_length)?;
        let spans = knot_spans(&shape);
        if spans.is_empty() || shape.length() <= 0.0 {
            return Err(no_length());
        }
        Ok(Source {
            points: points.clone(),
            positions,
            form,
            spans,
            shape,
        })
    }

    fn plan_near(
        &self,
        curve: EntityId,
        source: &Source,
        near: Option<Point2>,
    ) -> Result<Plan, InsertError> {
        let no_length = || InsertError::NoLength {
            entity: curve,
            label: self.entity_label(curve),
        };
        let candidates: Vec<(usize, KnotSpan, f64)> = match near {
            Some(at) => {
                let parameter = closest_spline_parameter(&source.shape, at);
                let (index, span) = source
                    .spans
                    .iter()
                    .copied()
                    .enumerate()
                    .find(|(_, span)| parameter < span.high)
                    .or(source.spans.iter().copied().enumerate().next_back())
                    .ok_or_else(no_length)?;
                vec![(index, span, parameter)]
            }
            None => spans_longest_first(source),
        };
        let mut blocked = None;
        for (index, span, parameter) in candidates {
            let margin = SPAN_MARGIN * (span.high - span.low);
            let parameter = parameter.clamp(span.low + margin, span.high - margin);
            let plan = planned(source, index, parameter).ok_or_else(no_length)?;
            match self.held_by_insertion(curve, source, &plan) {
                None => return Ok(plan),
                Some(held) => blocked = blocked.or(Some(held)),
            }
        }
        match blocked {
            Some(point) => Err(InsertError::HeldPoint {
                point,
                label: self.entity_label(point),
            }),
            None => Err(no_length()),
        }
    }

    fn held_by_insertion(&self, curve: EntityId, source: &Source, plan: &Plan) -> Option<EntityId> {
        plan.moved().find(|point| {
            let occurrences = source.points.iter().filter(|other| *other == point).count();
            occurrences != 1
                || self.is_projected(*point)
                || !self.constraints_using(*point).is_empty()
                || self.entities_using(*point) != [curve]
        })
    }

    fn place_plan(&mut self, curve: EntityId, plan: Plan) -> Result<EntityId, SketchError> {
        let mut added = None;
        let mut placed = Vec::with_capacity(plan.slots.len());
        for slot in plan.slots {
            let point = match slot {
                Slot::Kept(point) => point,
                Slot::Moved(point, position) => {
                    self.replace_entity(point, Entity::Point(position))?;
                    point
                }
                Slot::Added(position) => {
                    let point = self.add_point(position);
                    added = Some(point);
                    point
                }
            };
            placed.push(point);
        }
        let added = added.ok_or(SketchError::NoSuchEntity(curve))?;
        self.restructure(
            curve,
            Entity::Spline {
                points: placed,
                kind: plan.kind,
            },
            &[],
            |_| true,
        )?;
        Ok(added)
    }
}

fn knot_spans(shape: &BSpline) -> Vec<KnotSpan> {
    shape
        .knots()
        .windows(2)
        .filter_map(|pair| match *pair {
            [low, high] if high > low => Some(KnotSpan { low, high }),
            _ => None,
        })
        .collect()
}

fn planned(source: &Source, span: usize, parameter: f64) -> Option<Plan> {
    match &source.form {
        Form::Straight => Plan::straight(&source.points, &source.positions, parameter),
        Form::Quadratic => Plan::elevated(&source.points, &source.positions, parameter),
        Form::Knotted(knotted) => Plan::of_inserted(&source.points, knotted.inserted(parameter)?),
        Form::Looped(spans) => {
            Plan::looped(&source.points, &source.positions, spans, span, parameter)
        }
    }
}

fn spans_longest_first(source: &Source) -> Vec<(usize, KnotSpan, f64)> {
    let mut spans: Vec<(usize, KnotSpan, f64, f64)> = source
        .spans
        .iter()
        .enumerate()
        .map(|(index, span)| {
            let middle = (span.low + span.high) / 2.0;
            let at = |share: f64| {
                source
                    .shape
                    .point_at(span.low + (span.high - span.low) * share)
            };
            let share = |step: usize| step as f64 / SPAN_SAMPLES as f64;
            let length: f64 = (0..SPAN_SAMPLES)
                .map(|step| at(share(step)).distance(at(share(step + 1))))
                .sum();
            (index, *span, middle, length)
        })
        .collect();
    spans.sort_by(|a, b| b.3.total_cmp(&a.3));
    spans
        .into_iter()
        .map(|(index, span, middle, _)| (index, span, middle))
        .collect()
}

#[cfg(test)]
mod tests;
