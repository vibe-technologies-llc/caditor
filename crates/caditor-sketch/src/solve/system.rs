use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::Constraint,
    entity::{Entity, Role},
    id::{ConstraintId, EntityId, Reference},
    sketch::{DimensionValues, Sketch, SketchError},
    solve::equation::{
        CircleHandle, Contact, Context, Equation, Form, LineHandle, PointHandle, RadiusHandle,
        fallback_direction,
    },
};

const DEGENERATE_LENGTH: f64 = 1e-12;
const COLLAPSED_LENGTH: f64 = 1e-9;

#[derive(Debug, Clone)]
pub(crate) struct System {
    pub values: Vec<f64>,
    pub equations: Vec<Equation>,
    pub context: Context,
    pub points: BTreeMap<EntityId, usize>,
    pub radii: BTreeMap<EntityId, usize>,
    pub radius_variables: BTreeSet<usize>,
    pub entity_variables: BTreeMap<EntityId, Vec<usize>>,
    pub spans: Vec<(EntityId, PointHandle, PointHandle)>,
    pub collapsed_length: f64,
}

impl System {
    pub fn build(sketch: &Sketch, dimensions: &DimensionValues) -> Result<Self, SketchError> {
        let mut values = Vec::new();
        let mut points = BTreeMap::new();
        let mut radii = BTreeMap::new();
        for (id, entity) in sketch.entities() {
            match *entity {
                Entity::Point(position) => {
                    points.insert(id, values.len());
                    values.extend([position.x, position.y]);
                }
                Entity::Circle { radius, .. } => {
                    radii.insert(id, values.len());
                    values.push(radius);
                }
                Entity::Line { .. } | Entity::Arc { .. } | Entity::Spline { .. } => {}
            }
        }
        let lengths = dimensions.iter().filter_map(|(id, value)| {
            matches!(
                sketch.constraint(id),
                Some(Constraint::Distance { .. } | Constraint::Radius { .. })
            )
            .then_some(value)
        });
        let scale = values
            .iter()
            .copied()
            .chain(lengths)
            .map(f64::abs)
            .filter(|value| value.is_finite())
            .fold(1.0, f64::max);
        let mut system = Self {
            radius_variables: radii.values().copied().collect(),
            values,
            equations: Vec::new(),
            context: Context {
                scale,
                degenerate_length: DEGENERATE_LENGTH * scale,
            },
            points,
            radii,
            entity_variables: BTreeMap::new(),
            spans: Vec::new(),
            collapsed_length: COLLAPSED_LENGTH * scale,
        };
        system.entity_variables = sketch
            .entities()
            .map(|(id, entity)| (id, system.variables_of(id, entity)))
            .collect();

        for (id, entity) in sketch.entities() {
            let span = match *entity {
                Entity::Line { start, end } => Some((start, end)),
                Entity::Arc { center, start, .. } => Some((center, start)),
                Entity::Point(_) | Entity::Circle { .. } | Entity::Spline { .. } => None,
            };
            if let Some((from, to)) = span {
                system
                    .spans
                    .push((id, system.point(from)?, system.point(to)?));
            }
        }
        for (id, entity) in sketch.entities() {
            if let Entity::Arc { end, .. } = *entity {
                let circle = system.circle(sketch, id)?;
                let point = system.point(end)?;
                let form = Form::OnCircle {
                    point,
                    circle,
                    fallback: system.initial_direction(circle.center, point),
                };
                system.equations.push(Equation { owner: None, form });
            }
        }
        for (id, constraint) in sketch.constraints() {
            let forms = system.forms(sketch, id, constraint, dimensions)?;
            system
                .equations
                .extend(forms.into_iter().map(|form| Equation {
                    owner: Some(id),
                    form,
                }));
        }
        Ok(system)
    }

    fn variables_of(&self, id: EntityId, entity: &Entity) -> Vec<usize> {
        let point_variables =
            |point: &EntityId| self.points.get(point).into_iter().flat_map(|x| [*x, x + 1]);
        match entity {
            Entity::Point(_) => point_variables(&id).collect(),
            Entity::Circle { center, .. } => point_variables(center)
                .chain(self.radii.get(&id).copied())
                .collect(),
            Entity::Line { .. } | Entity::Arc { .. } | Entity::Spline { .. } => {
                entity.points().iter().flat_map(point_variables).collect()
            }
        }
    }

    fn point(&self, id: EntityId) -> Result<PointHandle, SketchError> {
        if id == EntityId::ORIGIN {
            return Ok(PointHandle::Fixed(Point2::ZERO));
        }
        self.points
            .get(&id)
            .map(|x| PointHandle::Variable(*x))
            .ok_or(SketchError::NotAPoint(id))
    }

    fn line(&self, sketch: &Sketch, id: EntityId) -> Result<LineHandle, SketchError> {
        let axis = |direction: Vector2| LineHandle {
            start: PointHandle::Fixed(Point2::ZERO),
            end: PointHandle::Fixed(direction),
            fallback: direction,
        };
        match id.reference() {
            Some(Reference::HorizontalAxis) => return Ok(axis(Vector2::X)),
            Some(Reference::VerticalAxis) => return Ok(axis(Vector2::Y)),
            Some(Reference::Origin) | None => {}
        }
        let Some(&Entity::Line { start, end }) = sketch.entity(id) else {
            return Err(SketchError::MissingEntity(id));
        };
        let (start, end) = (self.point(start)?, self.point(end)?);
        Ok(LineHandle {
            start,
            end,
            fallback: self.initial_direction(start, end),
        })
    }

    fn circle(&self, sketch: &Sketch, id: EntityId) -> Result<CircleHandle, SketchError> {
        match sketch.entity(id) {
            Some(&Entity::Circle { center, .. }) => Ok(CircleHandle {
                center: self.point(center)?,
                radius: RadiusHandle::Variable(
                    *self.radii.get(&id).ok_or(SketchError::MissingEntity(id))?,
                ),
            }),
            Some(&Entity::Arc { center, start, .. }) => {
                let (center, start) = (self.point(center)?, self.point(start)?);
                Ok(CircleHandle {
                    center,
                    radius: RadiusHandle::ToArcStart {
                        start,
                        fallback: self.initial_direction(center, start),
                    },
                })
            }
            _ => Err(SketchError::MissingEntity(id)),
        }
    }

    fn initial_direction(&self, from: PointHandle, to: PointHandle) -> Vector2 {
        fallback_direction(to.at(&self.values) - from.at(&self.values))
    }

    fn initial_side(&self, point: PointHandle, line: &LineHandle) -> f64 {
        let start = line.start.at(&self.values);
        let direction = fallback_direction(line.end.at(&self.values) - start);
        let side = direction.perp_dot(point.at(&self.values) - start);
        if side < 0.0 { -1.0 } else { 1.0 }
    }

    fn initial_contact(&self, first: &CircleHandle, second: &CircleHandle) -> Contact {
        let values = &self.values;
        let distance = first.center.at(values).distance(second.center.at(values));
        let (first_radius, second_radius) = (first.radius(values), second.radius(values));
        let external_error = (distance - (first_radius + second_radius)).abs();
        let internal_error = (distance - (first_radius - second_radius).abs()).abs();
        if external_error <= internal_error {
            Contact::External
        } else {
            Contact::Internal {
                larger_first: if first_radius < second_radius {
                    -1.0
                } else {
                    1.0
                },
            }
        }
    }

    fn forms(
        &self,
        sketch: &Sketch,
        id: ConstraintId,
        constraint: &Constraint,
        dimensions: &DimensionValues,
    ) -> Result<Vec<Form>, SketchError> {
        let role = |entity: EntityId| {
            sketch
                .role(entity)
                .ok_or(SketchError::MissingEntity(entity))
        };
        let dimension = || {
            dimensions
                .dimension(id)
                .ok_or(SketchError::MissingConstraint(id))
        };
        let not_applicable = |a: EntityId, b: EntityId| SketchError::NotApplicable {
            constraint: constraint.kind_name(),
            first: sketch.entity_label(a),
            second: sketch.entity_label(b),
        };
        Ok(match *constraint {
            Constraint::Coincident(a, b) => match (role(a)?, role(b)?) {
                (Role::Point, Role::Point) => {
                    let (a, b) = (self.point(a)?, self.point(b)?);
                    vec![Form::SameX(a, b), Form::SameY(a, b)]
                }
                (Role::Point, Role::Line) => vec![self.on_line(sketch, a, b)?],
                (Role::Line, Role::Point) => vec![self.on_line(sketch, b, a)?],
                (Role::Point, Role::Circular) => vec![self.on_circle(sketch, a, b)?],
                (Role::Circular, Role::Point) => vec![self.on_circle(sketch, b, a)?],
                _ => return Err(not_applicable(a, b)),
            },
            Constraint::Horizontal(line) => vec![Form::Horizontal(self.line(sketch, line)?)],
            Constraint::Vertical(line) => vec![Form::Vertical(self.line(sketch, line)?)],
            Constraint::Parallel(a, b) => {
                vec![Form::Parallel(self.line(sketch, a)?, self.line(sketch, b)?)]
            }
            Constraint::Perpendicular(a, b) => vec![Form::Perpendicular(
                self.line(sketch, a)?,
                self.line(sketch, b)?,
            )],
            Constraint::Tangent(a, b) => match (role(a)?, role(b)?) {
                (Role::Line, Role::Circular) => vec![self.line_tangent(sketch, a, b)?],
                (Role::Circular, Role::Line) => vec![self.line_tangent(sketch, b, a)?],
                (Role::Circular, Role::Circular) => vec![self.circle_tangent(sketch, a, b)?],
                _ => return Err(not_applicable(a, b)),
            },
            Constraint::Equal(a, b) => match (role(a)?, role(b)?) {
                (Role::Line, Role::Line) => vec![Form::EqualLength(
                    self.line(sketch, a)?,
                    self.line(sketch, b)?,
                )],
                (Role::Circular, Role::Circular) => vec![Form::EqualRadius(
                    self.circle(sketch, a)?,
                    self.circle(sketch, b)?,
                )],
                _ => return Err(not_applicable(a, b)),
            },
            Constraint::Distance { from, to, .. } => {
                let value = dimension()?;
                match (role(from)?, role(to)?) {
                    (Role::Point, Role::Point) if value.abs() <= self.context.degenerate_length => {
                        let (from, to) = (self.point(from)?, self.point(to)?);
                        vec![Form::SameX(from, to), Form::SameY(from, to)]
                    }
                    (Role::Point, Role::Point) => {
                        let (from, to) = (self.point(from)?, self.point(to)?);
                        vec![Form::PointDistance {
                            from,
                            to,
                            fallback: self.initial_direction(from, to),
                            value,
                        }]
                    }
                    (Role::Point, Role::Line) => vec![self.line_distance(sketch, from, to, value)?],
                    (Role::Line, Role::Point) => vec![self.line_distance(sketch, to, from, value)?],
                    _ => return Err(not_applicable(from, to)),
                }
            }
            Constraint::Angle {
                from, to, reversed, ..
            } => vec![Form::Angle {
                from: self.line(sketch, from)?,
                to: self.line(sketch, to)?,
                reversed,
                radians: dimension()?.to_radians(),
            }],
            Constraint::Radius { entity, .. } => vec![Form::Radius {
                circle: self.circle(sketch, entity)?,
                value: dimension()?,
            }],
        })
    }

    fn on_line(
        &self,
        sketch: &Sketch,
        point: EntityId,
        line: EntityId,
    ) -> Result<Form, SketchError> {
        Ok(Form::OnLine {
            point: self.point(point)?,
            line: self.line(sketch, line)?,
        })
    }

    fn on_circle(
        &self,
        sketch: &Sketch,
        point: EntityId,
        circle: EntityId,
    ) -> Result<Form, SketchError> {
        let (point, circle) = (self.point(point)?, self.circle(sketch, circle)?);
        Ok(Form::OnCircle {
            point,
            circle,
            fallback: self.initial_direction(circle.center, point),
        })
    }

    fn line_tangent(
        &self,
        sketch: &Sketch,
        line: EntityId,
        circle: EntityId,
    ) -> Result<Form, SketchError> {
        let joint = joint(sketch, line, circle);
        let (line, circle) = (self.line(sketch, line)?, self.circle(sketch, circle)?);
        if let Some(point) = joint {
            let point = self.point(point)?;
            return Ok(Form::Perpendicular(
                line,
                self.radius_line(point, circle.center),
            ));
        }
        Ok(Form::LineTangent {
            side: self.initial_side(circle.center, &line),
            line,
            circle,
        })
    }

    fn circle_tangent(
        &self,
        sketch: &Sketch,
        a: EntityId,
        b: EntityId,
    ) -> Result<Form, SketchError> {
        let joint = joint(sketch, a, b);
        let (first, second) = (self.circle(sketch, a)?, self.circle(sketch, b)?);
        if let Some(point) = joint {
            let point = self.point(point)?;
            return Ok(Form::Parallel(
                self.radius_line(point, first.center),
                self.radius_line(point, second.center),
            ));
        }
        Ok(Form::CircleTangent {
            first,
            second,
            fallback: self.initial_direction(second.center, first.center),
            contact: self.initial_contact(&first, &second),
        })
    }

    fn radius_line(&self, point: PointHandle, center: PointHandle) -> LineHandle {
        LineHandle {
            start: point,
            end: center,
            fallback: self.initial_direction(point, center),
        }
    }

    fn line_distance(
        &self,
        sketch: &Sketch,
        point: EntityId,
        line: EntityId,
        value: f64,
    ) -> Result<Form, SketchError> {
        let (point, line) = (self.point(point)?, self.line(sketch, line)?);
        Ok(Form::LineDistance {
            side: self.initial_side(point, &line),
            point,
            line,
            value,
        })
    }
}

fn joint(sketch: &Sketch, first: EntityId, second: EntityId) -> Option<EntityId> {
    let classes = Coincidence::of(sketch);
    let on_second: BTreeSet<EntityId> = points_on(sketch, second)
        .into_iter()
        .map(|point| classes.class(point))
        .collect();
    points_on(sketch, first)
        .into_iter()
        .find(|point| on_second.contains(&classes.class(*point)))
}

fn points_on(sketch: &Sketch, curve: EntityId) -> BTreeSet<EntityId> {
    let mut points: BTreeSet<EntityId> = match sketch.entity(curve) {
        Some(Entity::Line { start, end } | Entity::Arc { start, end, .. }) => {
            BTreeSet::from([*start, *end])
        }
        Some(Entity::Point(_) | Entity::Circle { .. } | Entity::Spline { .. }) | None => {
            BTreeSet::new()
        }
    };
    for (_, constraint) in sketch.constraints() {
        if let Constraint::Coincident(a, b) = *constraint {
            match (sketch.role(a), sketch.role(b)) {
                (Some(Role::Point), _) if b == curve => {
                    points.insert(a);
                }
                (_, Some(Role::Point)) if a == curve => {
                    points.insert(b);
                }
                _ => {}
            }
        }
    }
    points
}

struct Coincidence {
    parents: BTreeMap<EntityId, EntityId>,
}

impl Coincidence {
    fn of(sketch: &Sketch) -> Self {
        let mut classes = Self {
            parents: BTreeMap::new(),
        };
        for (_, constraint) in sketch.constraints() {
            if let Constraint::Coincident(a, b) = *constraint
                && sketch.role(a) == Some(Role::Point)
                && sketch.role(b) == Some(Role::Point)
            {
                let (a, b) = (classes.class(a), classes.class(b));
                if a != b {
                    classes.parents.insert(a.max(b), a.min(b));
                }
            }
        }
        classes
    }

    fn class(&self, point: EntityId) -> EntityId {
        let mut current = point;
        let mut steps = 0;
        while let Some(parent) = self.parents.get(&current)
            && *parent != current
            && steps <= self.parents.len()
        {
            current = *parent;
            steps += 1;
        }
        current
    }
}
