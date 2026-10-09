use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Drag, Entity, EntityId, Sketch, SplineKind};
use libfuzzer_sys::arbitrary::{Result, Unstructured};

use crate::{angle, length, point};

const MOST_SHAPES: usize = 8;
const MOST_CONSTRAINTS: usize = 12;
const MOST_SPLINE_POINTS: usize = 5;
const STRANGE_LENGTHS: [f64; 4] = [0.0, -1.0, 1e-9, 2e6];

pub fn no_parameters(_: ParameterId) -> std::result::Result<Quantity, EvalError> {
    Err(EvalError::ParameterMissing)
}

fn points(sketch: &Sketch) -> Vec<EntityId> {
    sketch
        .entities()
        .filter(|(_, entity)| matches!(entity, Entity::Point(_)))
        .map(|(id, _)| id)
        .collect()
}

fn point_id(input: &mut Unstructured, sketch: &mut Sketch) -> Result<EntityId> {
    let existing = points(sketch);
    match input.int_in_range(0u8..=3)? {
        0 if !existing.is_empty() => Ok(*input.choose(&existing)?),
        1 => Ok(EntityId::ORIGIN),
        _ => Ok(sketch.add_point(point(input)?)),
    }
}

fn insert(sketch: &mut Sketch, entity: Entity) -> Option<EntityId> {
    let id = EntityId::from_raw(sketch.next_id());
    sketch.insert_entity(id, entity).ok().map(|()| id)
}

fn shape(input: &mut Unstructured, sketch: &mut Sketch) -> Result<Option<EntityId>> {
    Ok(match input.int_in_range(0u8..=5)? {
        0 => Some(sketch.add_point(point(input)?)),
        1 => {
            let (start, end) = (point_id(input, sketch)?, point_id(input, sketch)?);
            insert(sketch, Entity::Line { start, end })
        }
        2 => {
            let center = point_id(input, sketch)?;
            insert(
                sketch,
                Entity::Circle {
                    center,
                    radius: length(input)?,
                },
            )
        }
        3 => {
            let center = point_id(input, sketch)?;
            let start = point_id(input, sketch)?;
            let end = point_id(input, sketch)?;
            insert(sketch, Entity::Arc { center, start, end })
        }
        4 => {
            let count = input.int_in_range(2..=MOST_SPLINE_POINTS)?;
            let points = (0..count)
                .map(|_| point_id(input, sketch))
                .collect::<Result<Vec<_>>>()?;
            let kind = match input.int_in_range(0u8..=2)? {
                0 => SplineKind::Control {
                    closed: input.arbitrary()?,
                },
                1 => SplineKind::Fit {
                    closed: input.arbitrary()?,
                },
                _ => SplineKind::Conic {
                    rho: input.arbitrary()?,
                },
            };
            insert(sketch, Entity::Spline { points, kind })
        }
        _ => {
            let corner = point(input)?;
            let far = Point2::new(corner.x + length(input)?, corner.y + length(input)?);
            let corners = [
                corner,
                Point2::new(far.x, corner.y),
                far,
                Point2::new(corner.x, far.y),
            ]
            .map(|position| sketch.add_point(position));
            let mut last = None;
            for (index, start) in corners.iter().enumerate() {
                let end = corners[(index + 1) % corners.len()];
                last = insert(sketch, Entity::Line { start: *start, end }).or(last);
            }
            last
        }
    })
}

fn any_entity(input: &mut Unstructured, sketch: &Sketch) -> Result<EntityId> {
    let mut candidates: Vec<EntityId> = sketch.entities().map(|(id, _)| id).collect();
    candidates.extend(EntityId::REFERENCES);
    if input.ratio(1u8, 32)? {
        return Ok(EntityId::from_raw(input.arbitrary()?));
    }
    Ok(*input.choose(&candidates)?)
}

pub fn length_value(input: &mut Unstructured) -> Result<Expression> {
    let value = if input.ratio(1u8, 16)? {
        *input.choose(&STRANGE_LENGTHS)?
    } else {
        length(input)?
    };
    Ok(Expression::measure(value, Unit::Millimetre))
}

pub fn angle_value(input: &mut Unstructured) -> Result<Expression> {
    Ok(Expression::measure(
        angle(input)?.to_degrees(),
        Unit::Degree,
    ))
}

pub fn constraint(input: &mut Unstructured, sketch: &Sketch) -> Result<Constraint> {
    let entity = |input: &mut Unstructured| any_entity(input, sketch);
    Ok(match input.int_in_range(0u8..=17)? {
        0 => Constraint::Coincident(entity(input)?, entity(input)?),
        1 => Constraint::Horizontal(entity(input)?),
        2 => Constraint::Vertical(entity(input)?),
        3 => Constraint::HorizontalPoints(entity(input)?, entity(input)?),
        4 => Constraint::VerticalPoints(entity(input)?, entity(input)?),
        5 => Constraint::Parallel(entity(input)?, entity(input)?),
        6 => Constraint::Perpendicular(entity(input)?, entity(input)?),
        7 => Constraint::Tangent(entity(input)?, entity(input)?),
        8 => Constraint::Equal(entity(input)?, entity(input)?),
        9 => Constraint::Midpoint {
            point: entity(input)?,
            curve: entity(input)?,
        },
        10 => Constraint::Concentric(entity(input)?, entity(input)?),
        11 => Constraint::Collinear(entity(input)?, entity(input)?),
        12 => Constraint::Symmetric {
            first: entity(input)?,
            second: entity(input)?,
            about: entity(input)?,
        },
        13 => Constraint::Fix {
            point: entity(input)?,
            at: point(input)?,
        },
        14 => Constraint::Distance {
            from: entity(input)?,
            to: entity(input)?,
            value: length_value(input)?,
        },
        15 => {
            let (from, to) = (entity(input)?, entity(input)?);
            let value = length_value(input)?;
            if input.arbitrary::<bool>()? {
                Constraint::HorizontalDistance { from, to, value }
            } else {
                Constraint::VerticalDistance { from, to, value }
            }
        }
        16 => Constraint::Angle {
            from: entity(input)?,
            to: entity(input)?,
            reversed: input.arbitrary()?,
            value: angle_value(input)?,
        },
        _ => {
            let circle = entity(input)?;
            let value = length_value(input)?;
            if input.arbitrary::<bool>()? {
                Constraint::Radius {
                    entity: circle,
                    value,
                }
            } else {
                Constraint::Diameter {
                    entity: circle,
                    value,
                }
            }
        }
    })
}

pub fn sketch(input: &mut Unstructured, plane: Plane) -> Result<Sketch> {
    let mut sketch = Sketch::new(plane);
    for _ in 0..input.int_in_range(1..=MOST_SHAPES)? {
        if let Some(curve) = shape(input, &mut sketch)?
            && input.ratio(1u8, 6)?
        {
            let _ = sketch.set_construction(curve, true);
        }
    }
    for _ in 0..input.int_in_range(0..=MOST_CONSTRAINTS)? {
        let constraint = constraint(input, &sketch)?;
        let _ = sketch.add_constraint(constraint);
    }
    Ok(sketch)
}

pub fn drag(input: &mut Unstructured, sketch: &Sketch) -> Result<Option<Drag>> {
    let existing = points(sketch);
    if existing.is_empty() {
        return Ok(None);
    }
    Ok(Some(Drag::Point {
        point: *input.choose(&existing)?,
        to: point(input)?,
    }))
}
