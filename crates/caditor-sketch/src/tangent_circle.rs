use std::f64::consts::TAU;

use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::{Constraint, MAX_LENGTH},
    curve::{ArcGeometry, Faceting},
    fillet::centers,
    id::EntityId,
    intersect::Carrier,
    pattern::Dimensioned,
    sketch::{Sketch, SketchError},
};

const TOLERANCE: f64 = 1e-9;
const RESIDUAL: f64 = 1e-6;
const SIGNS: [f64; 2] = [1.0, -1.0];

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TangentError {
    #[error("a circle touches two curves at a given radius, or three curves; {count} were chosen")]
    CurveCount { count: usize },
    #[error("two curves leave the size of the circle open; give its radius")]
    NeedsRadius,
    #[error("three curves already fix the size of the circle, so it takes no radius")]
    RadiusWithThree,
    #[error("{label} is not a line, circle or arc, so a circle cannot touch it")]
    NotCurve { entity: EntityId, label: String },
    #[error("{label} was chosen twice")]
    Repeated { entity: EntityId, label: String },
    #[error("the radius must be greater than zero")]
    RadiusNotPositive,
    #[error("no circle touches {labels}")]
    NoCircle { labels: String },
    #[error("no circle of this radius touches {labels}")]
    NoCircleOfRadius { labels: String },
    #[error("the circle reaches further than {} m from the origin", MAX_LENGTH / 1_000.0)]
    OutOfReach,
    #[error(transparent)]
    Edit(SketchError),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TangentCircle {
    pub center: Point2,
    pub radius: f64,
}

impl TangentCircle {
    pub fn faceted(&self, faceting: Faceting) -> Vec<Point2> {
        ArcGeometry {
            center: self.center,
            radius: self.radius,
            start_angle: 0.0,
            sweep: TAU,
        }
        .faceted(faceting)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Touch {
    Line {
        through: Point2,
        direction: Vector2,
        normal: Vector2,
        offset: f64,
    },
    Circle {
        center: Point2,
        radius: f64,
    },
}

impl Touch {
    fn gap(&self, circle: &TangentCircle) -> f64 {
        match *self {
            Self::Line { normal, offset, .. } => {
                ((normal.dot(circle.center) - offset).abs() - circle.radius).abs()
            }
            Self::Circle { center, radius } => {
                let apart = circle.center.distance(center);
                (apart - (radius + circle.radius))
                    .abs()
                    .min((apart - (radius - circle.radius).abs()).abs())
            }
        }
    }

    fn carrier(&self, sign: f64, radius: f64) -> Option<Carrier> {
        match *self {
            Self::Line {
                through,
                direction,
                normal,
                ..
            } => Some(Carrier::Line {
                through: through + normal * (sign * radius),
                direction,
            }),
            Self::Circle {
                center,
                radius: own,
            } => {
                let size = (own + sign * radius).abs();
                (size > TOLERANCE * own.max(radius).max(1.0)).then_some(Carrier::Circle {
                    center,
                    radius: size,
                })
            }
        }
    }
}

type Row = ([f64; 3], f64);

impl Sketch {
    pub fn tangent_circle_near(
        &self,
        curves: &[EntityId],
        radius: Option<f64>,
        near: Option<Point2>,
    ) -> Result<TangentCircle, TangentError> {
        let touches = self.touches(curves, radius)?;
        let labels = || self.labels(curves);
        let found = match (touches.as_slice(), radius) {
            ([first, second], Some(radius)) => {
                if !(radius.is_finite() && radius > 0.0) {
                    return Err(TangentError::RadiusNotPositive);
                }
                around_two(first, second, radius)
            }
            ([first, second, third], None) => around_three([*first, *second, *third]),
            _ => Vec::new(),
        };
        let reachable: Vec<TangentCircle> = found
            .iter()
            .copied()
            .filter(|circle| reach(circle) < MAX_LENGTH)
            .collect();
        let chosen = match near {
            Some(near) => reachable
                .iter()
                .copied()
                .min_by(|a, b| a.center.distance(near).total_cmp(&b.center.distance(near))),
            None => reachable
                .iter()
                .copied()
                .min_by(|a, b| a.radius.total_cmp(&b.radius)),
        };
        match (chosen, found.is_empty(), radius) {
            (Some(circle), _, _) => Ok(circle),
            (None, false, _) => Err(TangentError::OutOfReach),
            (None, true, Some(_)) => Err(TangentError::NoCircleOfRadius { labels: labels() }),
            (None, true, None) => Err(TangentError::NoCircle { labels: labels() }),
        }
    }

    pub fn tangent_circle(
        &mut self,
        curves: &[EntityId],
        radius: Option<&Dimensioned>,
        near: Option<Point2>,
    ) -> Result<EntityId, TangentError> {
        let circle = self.tangent_circle_near(curves, radius.map(|radius| radius.value), near)?;
        let mut working = self.clone();
        let made = working.add_circle(circle.center, circle.radius);
        let edit = |result: Result<_, SketchError>| result.map_err(TangentError::Edit);
        for curve in curves {
            edit(working.add_constraint(Constraint::Tangent(*curve, made)))?;
        }
        if let Some(radius) = radius {
            edit(working.add_constraint(Constraint::Radius {
                entity: made,
                value: radius.expression.clone(),
            }))?;
        }
        *self = working;
        Ok(made)
    }

    fn labels(&self, curves: &[EntityId]) -> String {
        let labels: Vec<String> = curves
            .iter()
            .map(|curve| self.entity_label(*curve))
            .collect();
        match labels.as_slice() {
            [first, second] => format!("{first} and {second}"),
            [front @ .., last] => format!("{} and {last}", front.join(", ")),
            [] => String::new(),
        }
    }

    fn touches(
        &self,
        curves: &[EntityId],
        radius: Option<f64>,
    ) -> Result<Vec<Touch>, TangentError> {
        match (curves.len(), radius) {
            (2, Some(_)) | (3, None) => {}
            (2, None) => return Err(TangentError::NeedsRadius),
            (3, Some(_)) => return Err(TangentError::RadiusWithThree),
            (count, _) => return Err(TangentError::CurveCount { count }),
        }
        for (index, curve) in curves.iter().enumerate() {
            if curves.iter().take(index).any(|earlier| earlier == curve) {
                return Err(TangentError::Repeated {
                    entity: *curve,
                    label: self.entity_label(*curve),
                });
            }
        }
        curves.iter().map(|curve| self.touch(*curve)).collect()
    }

    fn touch(&self, curve: EntityId) -> Result<Touch, TangentError> {
        let not_curve = || TangentError::NotCurve {
            entity: curve,
            label: self.entity_label(curve),
        };
        if let Some((center, radius)) = self.circle(curve) {
            return Ok(Touch::Circle { center, radius });
        }
        let direction = self
            .line_direction(curve)
            .and_then(Vector2::try_normalize)
            .ok_or_else(not_curve)?;
        let through = if curve.is_reference() {
            Point2::ZERO
        } else {
            self.line_endpoints(curve).ok_or_else(not_curve)?.0
        };
        let normal = direction.perp();
        Ok(Touch::Line {
            through,
            direction,
            normal,
            offset: normal.dot(through),
        })
    }
}

fn reach(circle: &TangentCircle) -> f64 {
    circle.center.abs().max_element() + circle.radius
}

fn around_two(first: &Touch, second: &Touch, radius: f64) -> Vec<TangentCircle> {
    let scale = radius.max(1.0);
    let mut found = Vec::new();
    for first_sign in SIGNS {
        for second_sign in SIGNS {
            let (Some(a), Some(b)) = (
                first.carrier(first_sign, radius),
                second.carrier(second_sign, radius),
            ) else {
                continue;
            };
            for center in centers(a, b, TOLERANCE * scale) {
                add(
                    &mut found,
                    TangentCircle { center, radius },
                    &[*first, *second],
                );
            }
        }
    }
    found
}

fn around_three(touches: [Touch; 3]) -> Vec<TangentCircle> {
    let mut found = Vec::new();
    for first in SIGNS {
        for second in SIGNS {
            for third in SIGNS {
                for circle in around_three_signed(&touches, [first, second, third]) {
                    add(&mut found, circle, &touches);
                }
            }
        }
    }
    found
}

fn add(found: &mut Vec<TangentCircle>, circle: TangentCircle, touches: &[Touch]) {
    let scale = reach(&circle).max(1.0);
    let touching = touches
        .iter()
        .all(|touch| touch.gap(&circle).abs() <= RESIDUAL * scale);
    let new = !found.iter().any(|other| {
        other.center.distance(circle.center) <= RESIDUAL * scale
            && (other.radius - circle.radius).abs() <= RESIDUAL * scale
    });
    if touching && new && circle.radius > TOLERANCE * scale {
        found.push(circle);
    }
}

fn around_three_signed(touches: &[Touch; 3], signs: [f64; 3]) -> Vec<TangentCircle> {
    let mut rows: Vec<Row> = Vec::new();
    let mut circles: Vec<Row> = Vec::new();
    for (touch, sign) in touches.iter().zip(signs) {
        match *touch {
            Touch::Line { normal, offset, .. } => {
                rows.push(([normal.x, normal.y, -sign], offset));
            }
            Touch::Circle { center, radius } => circles.push((
                [-2.0 * center.x, -2.0 * center.y, -2.0 * sign * radius],
                radius * radius - center.length_squared(),
            )),
        }
    }
    let Some((base, base_rhs)) = circles.first().copied() else {
        return solve_linear(&rows)
            .map(|[x, y, r]| TangentCircle {
                center: Point2::new(x, y),
                radius: r,
            })
            .into_iter()
            .filter(|circle| circle.radius > 0.0)
            .collect();
    };
    for (linear, rhs) in circles.iter().skip(1) {
        rows.push((subtract(*linear, base), rhs - base_rhs));
    }
    let [(first, first_rhs), (second, second_rhs)] = rows.as_slice() else {
        return Vec::new();
    };
    let Some(direction) = unit(cross(*first, *second), norm(*first) * norm(*second)) else {
        return Vec::new();
    };
    let gram = [
        [dot(*first, *first), dot(*first, *second)],
        [dot(*first, *second), dot(*second, *second)],
    ];
    let determinant = gram[0][0] * gram[1][1] - gram[0][1] * gram[1][0];
    if determinant.abs() <= TOLERANCE * gram[0][0] * gram[1][1] {
        return Vec::new();
    }
    let weights = [
        (gram[1][1] * first_rhs - gram[0][1] * second_rhs) / determinant,
        (gram[0][0] * second_rhs - gram[0][1] * first_rhs) / determinant,
    ];
    let start = [
        weights[0] * first[0] + weights[1] * second[0],
        weights[0] * first[1] + weights[1] * second[1],
        weights[0] * first[2] + weights[1] * second[2],
    ];
    let a = metric(direction, direction);
    let b = 2.0 * metric(start, direction) + dot(base, direction);
    let c = metric(start, start) + dot(base, start) - base_rhs;
    quadratic(a, b, c)
        .into_iter()
        .map(|t| {
            [
                start[0] + t * direction[0],
                start[1] + t * direction[1],
                start[2] + t * direction[2],
            ]
        })
        .filter(|[_, _, r]| r.is_finite() && *r > 0.0)
        .map(|[x, y, r]| TangentCircle {
            center: Point2::new(x, y),
            radius: r,
        })
        .collect()
}

fn solve_linear(rows: &[Row]) -> Option<[f64; 3]> {
    let [(a, x), (b, y), (c, z)] = rows else {
        return None;
    };
    let determinant = dot(*a, cross(*b, *c));
    if determinant.abs() <= TOLERANCE * norm(*a) * norm(*b) * norm(*c) {
        return None;
    }
    let solution = [
        (x * cross(*b, *c)[0] + y * cross(*c, *a)[0] + z * cross(*a, *b)[0]) / determinant,
        (x * cross(*b, *c)[1] + y * cross(*c, *a)[1] + z * cross(*a, *b)[1]) / determinant,
        (x * cross(*b, *c)[2] + y * cross(*c, *a)[2] + z * cross(*a, *b)[2]) / determinant,
    ];
    solution
        .iter()
        .all(|value| value.is_finite())
        .then_some(solution)
}

fn quadratic(a: f64, b: f64, c: f64) -> Vec<f64> {
    if a.abs() <= TOLERANCE {
        return if b.abs() > TOLERANCE {
            vec![-c / b]
        } else {
            Vec::new()
        };
    }
    let discriminant = b * b - 4.0 * a * c;
    let slack = TOLERANCE * (b * b + (4.0 * a * c).abs());
    if discriminant < -slack {
        return Vec::new();
    }
    let root = discriminant.max(0.0).sqrt();
    vec![(-b + root) / (2.0 * a), (-b - root) / (2.0 * a)]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn metric(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] - a[2] * b[2]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn subtract(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn unit(a: [f64; 3], scale: f64) -> Option<[f64; 3]> {
    let length = norm(a);
    (length > TOLERANCE * scale).then(|| [a[0] / length, a[1] / length, a[2] / length])
}

#[cfg(test)]
mod tests;
