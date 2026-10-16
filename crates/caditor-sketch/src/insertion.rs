use caditor_geometry::Point2;

use crate::{
    entity::{Entity, SplineKind},
    id::EntityId,
    intersect::closest_spline_parameter,
    sketch::{Sketch, SketchError},
    trim::spline::{Inserted, KnotSpan, Knotted},
};

const SPAN_MARGIN: f64 = 0.05;
const SPAN_SAMPLES: usize = 8;
const FIRST_INSERTABLE_POINTS: usize = 4;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum InsertError {
    #[error("{label} is not a spline")]
    NotASpline { entity: EntityId, label: String },
    #[error(
        "{label} passes through its points, so a point added to it would change its shape; only \
         splines drawn from control points can take one"
    )]
    FitSpline { entity: EntityId, label: String },
    #[error("{label} is closed; a point can be added to a spline with ends")]
    ClosedSpline { entity: EntityId, label: String },
    #[error("{label} is a conic, which has exactly its start, apex and end")]
    Conic { entity: EntityId, label: String },
    #[error(
        "{label} has only {points} control points; a point can be added to a spline with at \
         least four"
    )]
    TooFewPoints {
        entity: EntityId,
        label: String,
        points: usize,
    },
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

struct Source {
    points: Vec<EntityId>,
    knotted: Knotted,
    spans: Vec<KnotSpan>,
    shape: crate::curve::BSpline,
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
        let inserted = self.inserted_near(curve, &source, near)?;
        let mut working = self.clone();
        let point = working
            .place_inserted(curve, &source.points, inserted)
            .map_err(InsertError::Edit)?;
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
        match kind {
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
            SplineKind::Control { closed: true } => {
                return Err(InsertError::ClosedSpline {
                    entity: curve,
                    label: label(),
                });
            }
            SplineKind::Control { closed: false }
            | SplineKind::Piece { .. }
            | SplineKind::Knotted { .. } => {}
        }
        if self.is_projected(curve) {
            return Err(InsertError::Projected {
                entity: curve,
                label: label(),
            });
        }
        if points.len() < FIRST_INSERTABLE_POINTS {
            return Err(InsertError::TooFewPoints {
                entity: curve,
                label: label(),
                points: points.len(),
            });
        }
        let no_length = || InsertError::NoLength {
            entity: curve,
            label: label(),
        };
        let source = self.spline_source(curve).ok_or_else(no_length)?;
        let shape = self.spline(curve).ok_or_else(no_length)?;
        let spans = source.knotted.spans();
        if spans.is_empty() {
            return Err(no_length());
        }
        Ok(Source {
            points: points.clone(),
            knotted: source.knotted,
            spans,
            shape,
        })
    }

    fn inserted_near(
        &self,
        curve: EntityId,
        source: &Source,
        near: Option<Point2>,
    ) -> Result<Inserted, InsertError> {
        let no_length = || InsertError::NoLength {
            entity: curve,
            label: self.entity_label(curve),
        };
        let candidates: Vec<(KnotSpan, f64)> = match near {
            Some(at) => {
                let parameter = closest_spline_parameter(&source.shape, at);
                let span = source
                    .spans
                    .iter()
                    .copied()
                    .find(|span| parameter < span.high)
                    .or(source.spans.last().copied())
                    .ok_or_else(no_length)?;
                vec![(span, parameter)]
            }
            None => self.spans_longest_first(source),
        };
        let mut blocked = None;
        for (span, parameter) in candidates {
            let margin = SPAN_MARGIN * (span.high - span.low);
            let parameter = parameter.clamp(span.low + margin, span.high - margin);
            let inserted = source.knotted.inserted(parameter).ok_or_else(no_length)?;
            match self.held_by_insertion(curve, source, &inserted) {
                None => return Ok(inserted),
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

    fn spans_longest_first(&self, source: &Source) -> Vec<(KnotSpan, f64)> {
        let mut spans: Vec<(KnotSpan, f64, f64)> = source
            .spans
            .iter()
            .map(|span| {
                let middle = (span.low + span.high) / 2.0;
                let length: f64 = (0..SPAN_SAMPLES)
                    .map(|step| {
                        let at = |share: f64| {
                            source
                                .shape
                                .point_at(span.low + (span.high - span.low) * share)
                        };
                        let share = |index: usize| index as f64 / SPAN_SAMPLES as f64;
                        at(share(step)).distance(at(share(step + 1)))
                    })
                    .sum();
                (*span, middle, length)
            })
            .collect();
        spans.sort_by(|a, b| b.2.total_cmp(&a.2));
        spans
            .into_iter()
            .map(|(span, middle, _)| (span, middle))
            .collect()
    }

    fn held_by_insertion(
        &self,
        curve: EntityId,
        source: &Source,
        inserted: &Inserted,
    ) -> Option<EntityId> {
        let moved = source.points.get(inserted.first..inserted.span)?;
        moved.iter().copied().find(|point| {
            let occurrences = source.points.iter().filter(|other| *other == point).count();
            occurrences != 1
                || self.is_projected(*point)
                || !self.constraints_using(*point).is_empty()
                || self.entities_using(*point) != [curve]
        })
    }

    fn place_inserted(
        &mut self,
        curve: EntityId,
        points: &[EntityId],
        inserted: Inserted,
    ) -> Result<EntityId, SketchError> {
        let Inserted {
            kind,
            positions,
            first,
            span,
        } = inserted;
        let added = positions
            .get(span)
            .copied()
            .ok_or(SketchError::NoSuchEntity(curve))?;
        for (point, position) in points
            .get(first..span)
            .unwrap_or_default()
            .iter()
            .zip(positions.get(first..span).unwrap_or_default())
        {
            self.replace_entity(*point, Entity::Point(*position))?;
        }
        let new_point = self.add_point(added);
        let mut placed = points.to_vec();
        placed.insert(span, new_point);
        self.restructure(
            curve,
            Entity::Spline {
                points: placed,
                kind,
            },
            &[],
            |_| true,
        )?;
        Ok(new_point)
    }
}

#[cfg(test)]
mod tests;
