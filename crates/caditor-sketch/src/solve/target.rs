use std::sync::Arc;

use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::Constraint,
    entity::{Entity, Role},
    id::{ConstraintId, EntityId},
    sketch::{Sketch, SketchError},
    solve::{
        equation::{Form, LineHandle, PointHandle, TargetHandle},
        system::System,
    },
    target::PointTarget,
};

fn tracked(constraint: &Constraint) -> Option<(EntityId, &PointTarget)> {
    match constraint {
        Constraint::HorizontalPoints(point, target)
        | Constraint::VerticalPoints(point, target)
        | Constraint::OnDirection {
            point,
            through: target,
            ..
        } => Some((*point, target)),
        _ => None,
    }
}

fn across(sketch: &Sketch, constraint: &Constraint) -> Vector2 {
    match *constraint {
        Constraint::VerticalPoints(..) => Vector2::X,
        Constraint::OnDirection { line, square, .. } => {
            let along = sketch
                .line_direction(line)
                .and_then(Vector2::try_normalize)
                .unwrap_or(Vector2::X);
            if square { along } else { along.perp() }
        }
        _ => Vector2::Y,
    }
}

fn is_hidden(sketch: &Sketch, target: &PointTarget) -> bool {
    match target {
        PointTarget::Crossing(..) => true,
        PointTarget::Middle(curve) => matches!(
            sketch.entity(*curve),
            Some(Entity::Arc { .. } | Entity::EllipticalArc { .. })
        ),
        PointTarget::Point(_)
        | PointTarget::Centroid(_)
        | PointTarget::Quadrant { .. }
        | PointTarget::EllipseEnd { .. } => false,
    }
}

fn weighed(points: Vec<PointHandle>) -> TargetHandle {
    let weight = 1.0 / points.len().max(1) as f64;
    TargetHandle::Sum(points.into_iter().map(|point| (point, weight)).collect())
}

impl System {
    pub(super) fn hidden_target_start(
        &self,
        sketch: &Sketch,
        constraint: &Constraint,
    ) -> Option<Point2> {
        let (point, target) = tracked(constraint)?;
        if !is_hidden(sketch, target) {
            return None;
        }
        let held = self.point(point).ok()?.at(&self.values);
        Some(
            sketch
                .target_position(target, held, across(sketch, constraint))
                .unwrap_or(held),
        )
    }

    pub(super) fn target_parameter_start(
        &self,
        sketch: &Sketch,
        constraint: &Constraint,
    ) -> Result<Vec<(EntityId, f64)>, SketchError> {
        let Some((_, PointTarget::Crossing(first, second))) = tracked(constraint) else {
            return Ok(Vec::new());
        };
        let Some(start) = self.hidden_target_start(sketch, constraint) else {
            return Ok(Vec::new());
        };
        [*first, *second]
            .into_iter()
            .filter(|curve| sketch.role(*curve) == Some(Role::Spline))
            .map(|spline| Ok((spline, self.closest_parameter(sketch, spline, start)?)))
            .collect()
    }

    pub(super) fn tracking_forms(
        &self,
        sketch: &Sketch,
        id: ConstraintId,
        constraint: &Constraint,
    ) -> Result<Vec<Form>, SketchError> {
        match constraint {
            Constraint::HorizontalPoints(point, PointTarget::Point(other)) => {
                Ok(vec![Form::SameY(self.point(*point)?, self.point(*other)?)])
            }
            Constraint::VerticalPoints(point, PointTarget::Point(other)) => {
                Ok(vec![Form::SameX(self.point(*point)?, self.point(*other)?)])
            }
            Constraint::HorizontalPoints(point, target)
            | Constraint::VerticalPoints(point, target) => {
                let (target, mut forms) = self.target_handle(sketch, id, target)?;
                forms.push(Form::Level {
                    point: self.point(*point)?,
                    target,
                    along: across(sketch, constraint),
                });
                Ok(forms)
            }
            Constraint::OnDirection {
                point,
                through,
                line,
                square,
            } => {
                let (target, mut forms) = self.target_handle(sketch, id, through)?;
                forms.push(Form::OnDirection {
                    point: self.point(*point)?,
                    target,
                    line: self.line(sketch, *line)?,
                    square: *square,
                });
                Ok(forms)
            }
            Constraint::Centroid { point, corners } => {
                let point = self.point(*point)?;
                let weight = 1.0 / corners.len().max(1) as f64;
                let terms: Arc<[(PointHandle, f64)]> = corners
                    .iter()
                    .map(|corner| Ok((self.point(*corner)?, weight)))
                    .collect::<Result<_, SketchError>>()?;
                Ok([Vector2::X, Vector2::Y]
                    .into_iter()
                    .map(|along| Form::Through {
                        point,
                        terms: Arc::clone(&terms),
                        along,
                    })
                    .collect())
            }
            _ => Ok(Vec::new()),
        }
    }

    fn target_handle(
        &self,
        sketch: &Sketch,
        id: ConstraintId,
        target: &PointTarget,
    ) -> Result<(TargetHandle, Vec<Form>), SketchError> {
        let hidden = || {
            self.targets
                .get(&id)
                .map(|x| PointHandle::Variable(*x))
                .ok_or(SketchError::MissingConstraint(id))
        };
        match target {
            PointTarget::Point(point) => Ok((weighed(vec![self.point(*point)?]), Vec::new())),
            PointTarget::Middle(curve) => match sketch.entity(*curve) {
                Some(Entity::Line { .. }) => {
                    let line = self.line(sketch, *curve)?;
                    Ok((weighed(vec![line.start, line.end]), Vec::new()))
                }
                _ => {
                    let point = hidden()?;
                    let forms = self.middle_forms(sketch, point, *curve)?;
                    Ok((weighed(vec![point]), forms))
                }
            },
            PointTarget::Crossing(first, second) => {
                let point = hidden()?;
                let mut parameters = self.parameters.get(&id).into_iter().flatten().copied();
                let mut forms = Vec::new();
                for curve in [*first, *second] {
                    let parameter = if sketch.role(curve) == Some(Role::Spline) {
                        parameters.next()
                    } else {
                        None
                    };
                    forms.extend(self.through_curve(sketch, point, curve, parameter)?);
                }
                Ok((weighed(vec![point]), forms))
            }
            PointTarget::Centroid(corners) => Ok((
                weighed(
                    corners
                        .iter()
                        .map(|corner| self.point(*corner))
                        .collect::<Result<_, _>>()?,
                ),
                Vec::new(),
            )),
            PointTarget::Quadrant { curve, side } => Ok((
                TargetHandle::Rim {
                    circle: self.circle(sketch, *curve)?,
                    toward: side.outward(),
                },
                Vec::new(),
            )),
            PointTarget::EllipseEnd { ellipse, end } => {
                let (cos, sin) = end.turn();
                Ok((
                    TargetHandle::Turned {
                        ellipse: self.ellipse(sketch, *ellipse)?,
                        cos,
                        sin,
                    },
                    Vec::new(),
                ))
            }
        }
    }

    fn through_curve(
        &self,
        sketch: &Sketch,
        point: PointHandle,
        curve: EntityId,
        parameter: Option<usize>,
    ) -> Result<Vec<Form>, SketchError> {
        match sketch.role(curve) {
            Some(Role::Line) => Ok(vec![Form::OnLine {
                point,
                line: self.line(sketch, curve)?,
            }]),
            Some(Role::Circular) => {
                let circle = self.circle(sketch, curve)?;
                Ok(vec![Form::OnCircle {
                    point,
                    circle,
                    fallback: self.initial_direction(circle.center, point),
                }])
            }
            Some(Role::Elliptic) => Ok(vec![Form::OnEllipse {
                point,
                ellipse: self.ellipse(sketch, curve)?,
            }]),
            Some(Role::Spline) => {
                let parameter = parameter.ok_or(SketchError::MissingEntity(curve))?;
                let spline = self.spline_handle(sketch, curve)?;
                Ok([Vector2::X, Vector2::Y]
                    .into_iter()
                    .map(|along| Form::OnSpline {
                        point,
                        spline: Arc::clone(&spline),
                        parameter,
                        along,
                    })
                    .collect())
            }
            Some(Role::Point) | None => Err(SketchError::MissingEntity(curve)),
        }
    }

    pub(super) fn middle_forms(
        &self,
        sketch: &Sketch,
        point: PointHandle,
        curve: EntityId,
    ) -> Result<Vec<Form>, SketchError> {
        Ok(match sketch.entity(curve) {
            Some(&Entity::EllipticalArc { start, end, .. }) => {
                let ellipse = self.ellipse(sketch, curve)?;
                vec![
                    Form::OnEllipse { point, ellipse },
                    Form::EllipseMiddle {
                        point,
                        ends: (self.point(start)?, self.point(end)?),
                        ellipse,
                    },
                ]
            }
            Some(&Entity::Arc { start, end, .. }) => {
                let arc = self.circle(sketch, curve)?;
                let (start, end) = (self.point(start)?, self.point(end)?);
                let chord = LineHandle {
                    start,
                    end,
                    fallback: self.initial_direction(start, end),
                };
                vec![
                    Form::OnBisector { point, chord },
                    Form::ArcBulge { point, chord, arc },
                ]
            }
            _ => {
                let line = self.line(sketch, curve)?;
                [Vector2::X, Vector2::Y]
                    .into_iter()
                    .map(|along| Form::Middle {
                        point,
                        ends: (line.start, line.end),
                        along,
                    })
                    .collect()
            }
        })
    }
}
