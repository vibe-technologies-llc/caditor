use std::{
    cell::OnceCell,
    collections::{BTreeMap, BTreeSet},
};

use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::Constraint,
    entity::{Entity, Role},
    id::{ConstraintId, EntityId, Reference},
    sketch::{DimensionValues, Sketch, SketchError},
    solve::{
        equation::{
            CircleHandle, Contact, Context, Equation, Form, LineHandle, PointHandle, RadiusHandle,
            fallback_direction, value,
        },
        numeric::Component,
    },
};

#[derive(Debug, Clone)]
pub(crate) struct System {
    pub values: Vec<f64>,
    pub equations: Vec<Equation>,
    pub points: BTreeMap<EntityId, usize>,
    pub fixed_points: BTreeMap<EntityId, Point2>,
    pub radii: BTreeMap<EntityId, usize>,
    pub fixed_radii: BTreeMap<EntityId, f64>,
    pub radius_variables: BTreeSet<usize>,
    pub parameters: BTreeMap<ConstraintId, usize>,
    pub parameter_variables: BTreeSet<usize>,
    pub entity_variables: BTreeMap<EntityId, Vec<usize>>,
    pub spans: Vec<(EntityId, PointHandle, PointHandle)>,
    pub spans_at_variable: BTreeMap<usize, Vec<usize>>,
}

impl System {
    pub fn build(sketch: &Sketch, dimensions: &DimensionValues) -> Result<Self, SketchError> {
        let mut values = Vec::new();
        let mut points = BTreeMap::new();
        let mut fixed_points = BTreeMap::new();
        let mut radii = BTreeMap::new();
        let mut fixed_radii = BTreeMap::new();
        for (id, entity) in sketch.entities() {
            match *entity {
                Entity::Point(position) if sketch.is_projected(id) => {
                    fixed_points.insert(id, position);
                }
                Entity::Point(position) => {
                    points.insert(id, values.len());
                    values.extend([position.x, position.y]);
                }
                Entity::Circle { radius, .. } if sketch.is_projected(id) => {
                    fixed_radii.insert(id, radius);
                }
                Entity::Circle { radius, .. } => {
                    radii.insert(id, values.len());
                    values.push(radius);
                }
                Entity::Line { .. } | Entity::Arc { .. } | Entity::Spline { .. } => {}
            }
        }
        let mut system = Self {
            radius_variables: radii.values().copied().collect(),
            values,
            equations: Vec::new(),
            points,
            fixed_points,
            radii,
            fixed_radii,
            parameters: BTreeMap::new(),
            parameter_variables: BTreeSet::new(),
            entity_variables: BTreeMap::new(),
            spans: Vec::new(),
            spans_at_variable: BTreeMap::new(),
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
                let (from, to) = (system.point(from)?, system.point(to)?);
                let index = system.spans.len();
                for end in [from, to] {
                    if let PointHandle::Variable(variable) = end {
                        system
                            .spans_at_variable
                            .entry(variable)
                            .or_default()
                            .push(index);
                    }
                }
                system.spans.push((id, from, to));
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
        let joints = OnceCell::new();
        for (id, constraint) in sketch.active_constraints() {
            if let Some(start) = system.parameter_start(sketch, &joints, constraint)? {
                let index = system.values.len();
                system.values.push(start);
                system.parameters.insert(id, index);
                system.parameter_variables.insert(index);
            }
        }
        for (id, constraint) in sketch.active_constraints() {
            let forms = system.forms(sketch, &joints, id, constraint, dimensions)?;
            system
                .equations
                .extend(forms.into_iter().map(|form| Equation {
                    owner: Some(id),
                    form,
                }));
        }
        Ok(system)
    }

    pub fn anchor_bits(&self) -> Vec<u64> {
        self.fixed_points
            .values()
            .flat_map(|position| [position.x, position.y])
            .chain(self.fixed_radii.values().copied())
            .map(f64::to_bits)
            .collect()
    }

    pub fn context_of(&self, component: &Component) -> Context {
        let coordinates = component
            .variables
            .iter()
            .map(|variable| value(&self.values, *variable));
        let lengths = component
            .equations
            .iter()
            .filter_map(|index| self.equations.get(*index)?.form.length());
        Context::at_scale(scale_of(coordinates.chain(lengths)))
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

    pub(super) fn point(&self, id: EntityId) -> Result<PointHandle, SketchError> {
        if id == EntityId::ORIGIN {
            return Ok(PointHandle::Fixed(Point2::ZERO));
        }
        if let Some(position) = self.fixed_points.get(&id) {
            return Ok(PointHandle::Fixed(*position));
        }
        self.points
            .get(&id)
            .map(|x| PointHandle::Variable(*x))
            .ok_or(SketchError::NotAPoint(id))
    }

    pub(super) fn line(&self, sketch: &Sketch, id: EntityId) -> Result<LineHandle, SketchError> {
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

    pub(super) fn circle(
        &self,
        sketch: &Sketch,
        id: EntityId,
    ) -> Result<CircleHandle, SketchError> {
        match sketch.entity(id) {
            Some(&Entity::Circle { center, .. }) => Ok(CircleHandle {
                center: self.point(center)?,
                radius: match self.fixed_radii.get(&id) {
                    Some(radius) => RadiusHandle::Fixed(*radius),
                    None => RadiusHandle::Variable(
                        *self.radii.get(&id).ok_or(SketchError::MissingEntity(id))?,
                    ),
                },
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

    fn span_context(&self, from: PointHandle, to: PointHandle, value: f64) -> Context {
        let (from, to) = (from.at(&self.values), to.at(&self.values));
        Context::at_scale(scale_of([from.x, from.y, to.x, to.y, value]))
    }

    pub(super) fn initial_direction(&self, from: PointHandle, to: PointHandle) -> Vector2 {
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
        joints: &OnceCell<Joints>,
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
                (Role::Point, Role::Spline) => {
                    self.on_spline(sketch, a, b, self.parameter_of(id)?)?
                }
                (Role::Spline, Role::Point) => {
                    self.on_spline(sketch, b, a, self.parameter_of(id)?)?
                }
                _ => return Err(not_applicable(a, b)),
            },
            Constraint::Horizontal(line) => vec![Form::Horizontal(self.line(sketch, line)?)],
            Constraint::Vertical(line) => vec![Form::Vertical(self.line(sketch, line)?)],
            Constraint::HorizontalPoints(a, b) => vec![Form::SameY(self.point(a)?, self.point(b)?)],
            Constraint::VerticalPoints(a, b) => vec![Form::SameX(self.point(a)?, self.point(b)?)],
            Constraint::Midpoint { point, line } => {
                let (point, line) = (self.point(point)?, self.line(sketch, line)?);
                [Vector2::X, Vector2::Y]
                    .into_iter()
                    .map(|along| Form::Middle {
                        point,
                        ends: (line.start, line.end),
                        along,
                    })
                    .collect()
            }
            Constraint::Concentric(a, b) => {
                let centre = |entity: EntityId| match role(entity)? {
                    Role::Point => self.point(entity),
                    Role::Line | Role::Circular | Role::Spline => {
                        Ok(self.circle(sketch, entity)?.center)
                    }
                };
                let (a, b) = (centre(a)?, centre(b)?);
                vec![Form::SameX(a, b), Form::SameY(a, b)]
            }
            Constraint::Collinear(a, b) => {
                let (anchor, other) = if b.is_reference() { (b, a) } else { (a, b) };
                let (line, other) = (self.line(sketch, anchor)?, self.line(sketch, other)?);
                vec![
                    Form::OnLine {
                        point: other.start,
                        line,
                    },
                    Form::OnLine {
                        point: other.end,
                        line,
                    },
                ]
            }
            Constraint::Symmetric {
                first,
                second,
                about,
            } => {
                let refused = not_applicable(second, about);
                let (first, second) = (self.point(first)?, self.point(second)?);
                match role(about)? {
                    Role::Point => {
                        let about = self.point(about)?;
                        [Vector2::X, Vector2::Y]
                            .into_iter()
                            .map(|along| Form::Middle {
                                point: about,
                                ends: (first, second),
                                along,
                            })
                            .collect()
                    }
                    Role::Line => {
                        let line = self.line(sketch, about)?;
                        vec![
                            Form::MirrorMiddle {
                                first,
                                second,
                                line,
                            },
                            Form::MirrorAcross {
                                first,
                                second,
                                line,
                            },
                        ]
                    }
                    Role::Circular | Role::Spline => return Err(refused),
                }
            }
            Constraint::Fix { point, at } => {
                let (point, target) = (self.point(point)?, PointHandle::Fixed(at));
                vec![Form::SameX(point, target), Form::SameY(point, target)]
            }
            Constraint::Parallel(a, b) => {
                vec![Form::Parallel(self.line(sketch, a)?, self.line(sketch, b)?)]
            }
            Constraint::Perpendicular(a, b) => vec![Form::Perpendicular(
                self.line(sketch, a)?,
                self.line(sketch, b)?,
            )],
            Constraint::Tangent(a, b) => match (role(a)?, role(b)?) {
                (Role::Line, Role::Circular) => {
                    let joints = joints.get_or_init(|| Joints::of(sketch));
                    vec![self.line_tangent(sketch, joints, a, b)?]
                }
                (Role::Circular, Role::Line) => {
                    let joints = joints.get_or_init(|| Joints::of(sketch));
                    vec![self.line_tangent(sketch, joints, b, a)?]
                }
                (Role::Circular, Role::Circular) => {
                    let joints = joints.get_or_init(|| Joints::of(sketch));
                    vec![self.circle_tangent(sketch, joints, a, b)?]
                }
                (Role::Spline, Role::Line | Role::Circular) => {
                    let joints = joints.get_or_init(|| Joints::of(sketch));
                    let parameter = self.parameters.get(&id).copied();
                    self.spline_tangent(sketch, joints, (a, b), parameter)?
                }
                (Role::Line | Role::Circular, Role::Spline) => {
                    let joints = joints.get_or_init(|| Joints::of(sketch));
                    let parameter = self.parameters.get(&id).copied();
                    self.spline_tangent(sketch, joints, (b, a), parameter)?
                }
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
                    (Role::Point, Role::Point) => {
                        let (from, to) = (self.point(from)?, self.point(to)?);
                        if value.abs() <= self.span_context(from, to, value).degenerate_length {
                            return Ok(vec![Form::SameX(from, to), Form::SameY(from, to)]);
                        }
                        vec![Form::PointDistance {
                            from,
                            to,
                            fallback: self.initial_direction(from, to),
                            value,
                        }]
                    }
                    (Role::Point, Role::Line) => vec![self.line_distance(sketch, from, to, value)?],
                    (Role::Line, Role::Point) => vec![self.line_distance(sketch, to, from, value)?],
                    (Role::Point, Role::Circular) => {
                        vec![self.circle_distance(sketch, from, to, value)?]
                    }
                    (Role::Circular, Role::Point) => {
                        vec![self.circle_distance(sketch, to, from, value)?]
                    }
                    (Role::Line, Role::Line) => self.line_spacing(sketch, from, to, value)?,
                    _ => return Err(not_applicable(from, to)),
                }
            }
            Constraint::HorizontalDistance { from, to, .. } => {
                self.offset(self.point(from)?, self.point(to)?, Vector2::X, dimension()?)
            }
            Constraint::VerticalDistance { from, to, .. } => {
                self.offset(self.point(from)?, self.point(to)?, Vector2::Y, dimension()?)
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
            Constraint::Diameter { entity, .. } => vec![Form::Radius {
                circle: self.circle(sketch, entity)?,
                value: dimension()? / 2.0,
            }],
        })
    }

    fn offset(&self, from: PointHandle, to: PointHandle, along: Vector2, value: f64) -> Vec<Form> {
        if value.abs() <= self.span_context(from, to, value).degenerate_length {
            return if along == Vector2::X {
                vec![Form::SameX(from, to)]
            } else {
                vec![Form::SameY(from, to)]
            };
        }
        let drawn = along.dot(to.at(&self.values) - from.at(&self.values));
        vec![Form::Offset {
            from,
            to,
            along,
            side: if drawn < 0.0 { -1.0 } else { 1.0 },
            value,
        }]
    }

    fn circle_distance(
        &self,
        sketch: &Sketch,
        point: EntityId,
        circle: EntityId,
        value: f64,
    ) -> Result<Form, SketchError> {
        let (point, circle) = (self.point(point)?, self.circle(sketch, circle)?);
        let values = &self.values;
        let inside = point.at(values).distance(circle.center.at(values)) < circle.radius(values);
        Ok(Form::CircleDistance {
            point,
            circle,
            fallback: self.initial_direction(circle.center, point),
            side: if inside { -1.0 } else { 1.0 },
            value,
        })
    }

    fn line_spacing(
        &self,
        sketch: &Sketch,
        a: EntityId,
        b: EntityId,
        value: f64,
    ) -> Result<Vec<Form>, SketchError> {
        let (anchor, other) = if b.is_reference() { (b, a) } else { (a, b) };
        let (line, other) = (self.line(sketch, anchor)?, self.line(sketch, other)?);
        let values = &self.values;
        let middle = (other.start.at(values) + other.end.at(values)) / 2.0;
        let start = line.start.at(values);
        let side = fallback_direction(line.end.at(values) - start).perp_dot(middle - start);
        let side = if side < 0.0 { -1.0 } else { 1.0 };
        Ok([other.start, other.end]
            .into_iter()
            .map(|point| Form::LineDistance {
                point,
                line,
                side,
                value,
            })
            .collect())
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
        joints: &Joints,
        line: EntityId,
        circle: EntityId,
    ) -> Result<Form, SketchError> {
        let joint = joints.joint(sketch, line, circle);
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
        joints: &Joints,
        a: EntityId,
        b: EntityId,
    ) -> Result<Form, SketchError> {
        let joint = joints.joint(sketch, a, b);
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

    pub(super) fn radius_line(&self, point: PointHandle, center: PointHandle) -> LineHandle {
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

fn scale_of(magnitudes: impl IntoIterator<Item = f64>) -> f64 {
    let largest = magnitudes
        .into_iter()
        .map(f64::abs)
        .filter(|magnitude| magnitude.is_finite())
        .fold(1.0, f64::max);
    let power_of_two = largest.log2().ceil().exp2();
    if power_of_two.is_finite() {
        power_of_two
    } else {
        largest
    }
}

pub(super) struct Joints {
    parents: BTreeMap<EntityId, EntityId>,
    on_curve: BTreeMap<EntityId, BTreeSet<EntityId>>,
}

impl Joints {
    pub(super) fn of(sketch: &Sketch) -> Self {
        let mut joints = Self {
            parents: BTreeMap::new(),
            on_curve: BTreeMap::new(),
        };
        for (_, constraint) in sketch.active_constraints() {
            let Constraint::Coincident(a, b) = *constraint else {
                continue;
            };
            match (sketch.role(a), sketch.role(b)) {
                (Some(Role::Point), Some(Role::Point)) => {
                    let (a, b) = (joints.class(a), joints.class(b));
                    if a != b {
                        joints.parents.insert(a.max(b), a.min(b));
                    }
                }
                (Some(Role::Point), Some(_)) => {
                    joints.on_curve.entry(b).or_default().insert(a);
                }
                (Some(_), Some(Role::Point)) => {
                    joints.on_curve.entry(a).or_default().insert(b);
                }
                _ => {}
            }
        }
        joints
    }

    fn joint(&self, sketch: &Sketch, first: EntityId, second: EntityId) -> Option<EntityId> {
        let on_second: BTreeSet<EntityId> = self
            .points_on(sketch, second)
            .into_iter()
            .map(|point| self.class(point))
            .collect();
        self.points_on(sketch, first)
            .into_iter()
            .find(|point| on_second.contains(&self.class(*point)))
    }

    pub(super) fn points_on(&self, sketch: &Sketch, curve: EntityId) -> BTreeSet<EntityId> {
        let mut points: BTreeSet<EntityId> = match sketch.entity(curve) {
            Some(Entity::Line { start, end } | Entity::Arc { start, end, .. }) => {
                BTreeSet::from([*start, *end])
            }
            Some(Entity::Point(_) | Entity::Circle { .. } | Entity::Spline { .. }) | None => {
                BTreeSet::new()
            }
        };
        if let Some(on_curve) = self.on_curve.get(&curve) {
            points.extend(on_curve);
        }
        points
    }

    pub(super) fn class(&self, point: EntityId) -> EntityId {
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
