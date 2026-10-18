use crate::{
    constraint::Constraint,
    entity::Entity,
    id::{ConstraintId, EntityId},
    sketch::{DetachedConstraint, Sketch, SketchError},
};

const SLACK: f64 = 1e-6;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MidpointOn {
    id: ConstraintId,
    point: EntityId,
    detached: DetachedConstraint,
}

impl MidpointOn {
    pub(crate) fn of(
        id: ConstraintId,
        detached: &DetachedConstraint,
        curve: EntityId,
    ) -> Option<Self> {
        match detached.constraint {
            Constraint::Midpoint { point, curve: on } if on == curve => Some(Self {
                id,
                point,
                detached: detached.clone(),
            }),
            _ => None,
        }
    }
}

pub(crate) struct EqualLines {
    ties: Vec<(ConstraintId, DetachedConstraint)>,
}

impl Sketch {
    pub(crate) fn settle_midpoints(
        &mut self,
        curves: &[EntityId],
        midpoints: Vec<MidpointOn>,
    ) -> Result<Vec<MidpointOn>, SketchError> {
        let mut unsettled = Vec::new();
        for midpoint in midpoints {
            let Some(curve) = curves
                .iter()
                .copied()
                .find(|curve| self.point_lies_on(midpoint.point, *curve))
            else {
                unsettled.push(midpoint);
                continue;
            };
            let on_curve = Constraint::Coincident(midpoint.point, curve);
            if self.holds_already(&on_curve) || self.check_constraint(&on_curve).is_err() {
                continue;
            }
            self.attach_constraint(midpoint.id, midpoint.detached.with_constraint(on_curve))?;
        }
        Ok(unsettled)
    }

    pub(crate) fn detach_equal_lines(
        &mut self,
        first: EntityId,
        second: EntityId,
    ) -> Result<EqualLines, SketchError> {
        let both_lines = [first, second]
            .iter()
            .all(|curve| matches!(self.entity(*curve), Some(Entity::Line { .. })));
        let tied: Vec<ConstraintId> = if both_lines {
            self.constraints_using(first)
                .into_iter()
                .filter(|id| match self.constraint(*id) {
                    Some(Constraint::Equal(a, b)) => {
                        (*a, *b) == (first, second) || (*a, *b) == (second, first)
                    }
                    _ => false,
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut ties = Vec::new();
        for id in tied {
            ties.push((id, self.detach_constraint(id)?));
        }
        Ok(EqualLines { ties })
    }

    pub(crate) fn restore_equal_lines(
        &mut self,
        first: EntityId,
        second: EntityId,
        equal: EqualLines,
    ) -> Result<(), SketchError> {
        let lengths = (self.line_length(first), self.line_length(second));
        let (Some(first_length), Some(second_length)) = lengths else {
            return Ok(());
        };
        if (first_length - second_length).abs() > SLACK * first_length.max(second_length).max(1.0) {
            return Ok(());
        }
        for (id, detached) in equal.ties {
            if self.check_constraint(&detached.constraint).is_ok() {
                self.attach_constraint(id, detached)?;
            }
        }
        Ok(())
    }

    fn line_length(&self, line: EntityId) -> Option<f64> {
        let (start, end) = self.line_endpoints(line)?;
        Some(start.distance(end))
    }

    fn point_lies_on(&self, point: EntityId, curve: EntityId) -> bool {
        let (Some(at), Some(shape)) = (self.point(point), self.shape_of(curve)) else {
            return false;
        };
        at.distance(shape.closest(at)) <= SLACK * shape.extent().max(1.0)
    }

    fn holds_already(&self, constraint: &Constraint) -> bool {
        let Constraint::Coincident(point, curve) = *constraint else {
            return false;
        };
        self.active_constraints().any(|(_, existing)| {
            matches!(*existing, Constraint::Coincident(a, b)
                if (a, b) == (point, curve) || (a, b) == (curve, point))
        })
    }
}
