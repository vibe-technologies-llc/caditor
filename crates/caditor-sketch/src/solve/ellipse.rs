use std::{cell::OnceCell, f64::consts::TAU};

use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::Constraint,
    curve::EllipseGeometry,
    entity::Role,
    id::EntityId,
    sketch::{Sketch, SketchError},
    solve::{
        equation::{Form, fallback_direction},
        pair::{curve_pair_gap, is_elliptic_pair},
        system::{Joints, System},
    },
};

const STATIONARY_SAMPLES: usize = 64;
const BISECTIONS: usize = 60;

impl System {
    pub(super) fn ellipse_parameter_start(
        &self,
        sketch: &Sketch,
        joints: &OnceCell<Joints>,
        constraint: &Constraint,
    ) -> Result<Option<(EntityId, f64)>, SketchError> {
        let (a, b) = match *constraint {
            Constraint::Distance { from, to, .. } => (from, to),
            Constraint::Tangent(a, b) => {
                let joints = joints.get_or_init(|| Joints::of(sketch));
                if joints.joint(sketch, a, b).is_some() {
                    return Ok(None);
                }
                (a, b)
            }
            _ => return Ok(None),
        };
        let tangent = matches!(constraint, Constraint::Tangent(..));
        let (ellipse, other) = match (sketch.role(a), sketch.role(b)) {
            (Some(Role::Elliptic), Some(Role::Point)) if !tangent => (a, b),
            (Some(Role::Point), Some(Role::Elliptic)) if !tangent => (b, a),
            (Some(Role::Elliptic), Some(Role::Circular)) => (a, b),
            (Some(Role::Circular), Some(Role::Elliptic)) => (b, a),
            _ => return Ok(None),
        };
        let shape = self.ellipse_shape(sketch, ellipse)?;
        let angle = if sketch.role(other) == Some(Role::Point) {
            shape.closest_parameter(self.point(other)?.at(&self.values))
        } else {
            let circle = self.circle(sketch, other)?;
            let (center, radius) = (circle.center.at(&self.values), circle.radius(&self.values));
            touching_parameter(&shape, center, radius)
        };
        Ok(Some((ellipse, (angle / TAU).rem_euclid(1.0))))
    }

    pub(super) fn ellipse_shape(
        &self,
        sketch: &Sketch,
        ellipse: EntityId,
    ) -> Result<EllipseGeometry, SketchError> {
        let handle = self.ellipse(sketch, ellipse)?;
        Ok(EllipseGeometry::full(
            handle.center.at(&self.values),
            handle.major.at(&self.values),
            handle.minor_circle().radius(&self.values),
        ))
    }

    fn start_on_ellipse(
        &self,
        sketch: &Sketch,
        ellipse: EntityId,
        parameter: usize,
    ) -> Result<(EllipseGeometry, f64), SketchError> {
        let start = self.values.get(parameter).copied().unwrap_or(0.0);
        Ok((self.ellipse_shape(sketch, ellipse)?, TAU * start))
    }

    pub(super) fn ellipse_distance(
        &self,
        sketch: &Sketch,
        (point, ellipse): (EntityId, EntityId),
        parameter: usize,
        value: f64,
    ) -> Result<Vec<Form>, SketchError> {
        let handle = self.point(point)?;
        let (shape, angle) = self.start_on_ellipse(sketch, ellipse, parameter)?;
        let fallback = fallback_direction(shape.tangent_at(angle));
        let drawn = fallback
            .perp()
            .dot(handle.at(&self.values) - shape.point_at(angle));
        let side = if drawn < 0.0 { -1.0 } else { 1.0 };
        let ellipse = self.ellipse(sketch, ellipse)?;
        Ok(vec![
            Form::EllipseFoot {
                point: handle,
                ellipse,
                parameter,
                fallback,
            },
            Form::EllipseDistance {
                point: handle,
                ellipse,
                parameter,
                fallback,
                side,
                value,
            },
        ])
    }

    pub(super) fn ellipse_circle_contact(
        &self,
        sketch: &Sketch,
        (ellipse, circle): (EntityId, EntityId),
        parameter: usize,
        value: f64,
    ) -> Result<Vec<Form>, SketchError> {
        let (shape, angle) = self.start_on_ellipse(sketch, ellipse, parameter)?;
        let circle = self.circle(sketch, circle)?;
        let at = shape.point_at(angle);
        let center = circle.center.at(&self.values);
        let radial_fallback = fallback_direction(at - center);
        let tangent_fallback = fallback_direction(shape.tangent_at(angle));
        let drawn = at.distance(center) - circle.radius(&self.values);
        let side = if value != 0.0 && drawn < 0.0 {
            -1.0
        } else {
            1.0
        };
        let ellipse = self.ellipse(sketch, ellipse)?;
        Ok(vec![
            Form::EllipseOnCircle {
                ellipse,
                parameter,
                circle,
                fallback: radial_fallback,
                side,
                value,
            },
            Form::EllipseAcrossRadius {
                ellipse,
                parameter,
                circle,
                fallbacks: (radial_fallback, tangent_fallback),
            },
        ])
    }

    pub(super) fn ellipse_line_gap(
        &self,
        sketch: &Sketch,
        (ellipse, line): (EntityId, EntityId),
        value: f64,
    ) -> Result<Form, SketchError> {
        let (line, ellipse) = (self.line(sketch, line)?, self.ellipse(sketch, ellipse)?);
        Ok(Form::EllipseTangent {
            side: self.initial_side(ellipse.center, &line),
            line,
            ellipse,
            value,
        })
    }
}

fn stationary_parameters(shape: &EllipseGeometry, from: Point2) -> Vec<f64> {
    let slope = |angle: f64| (shape.point_at(angle) - from).dot(shape.tangent_at(angle));
    let step = TAU / STATIONARY_SAMPLES as f64;
    let mut found = Vec::new();
    for index in 0..STATIONARY_SAMPLES {
        let (low, high) = (index as f64 * step, (index + 1) as f64 * step);
        let (at_low, at_high) = (slope(low), slope(high));
        if at_low == 0.0 {
            found.push(low);
        } else if at_low * at_high < 0.0 {
            let (mut low, mut high, mut at_low) = (low, high, at_low);
            for _ in 0..BISECTIONS {
                let middle = 0.5 * (low + high);
                let at_middle = slope(middle);
                if at_middle * at_low <= 0.0 {
                    high = middle;
                } else {
                    (low, at_low) = (middle, at_middle);
                }
            }
            found.push(0.5 * (low + high));
        }
    }
    found
}

pub(crate) fn touching_parameter(shape: &EllipseGeometry, center: Point2, radius: f64) -> f64 {
    let gap = |angle: f64| (shape.point_at(angle).distance(center) - radius).abs();
    stationary_parameters(shape, center)
        .into_iter()
        .min_by(|a, b| gap(*a).total_cmp(&gap(*b)))
        .unwrap_or_else(|| shape.closest_parameter(center))
}

pub(crate) fn ellipse_gap(
    sketch: &Sketch,
    ellipse: EntityId,
    other: EntityId,
) -> Option<(Point2, Point2)> {
    if is_elliptic_pair(sketch, ellipse, other) {
        return curve_pair_gap(sketch, ellipse, other);
    }
    let drawn = sketch.ellipse(ellipse)?;
    let shape = EllipseGeometry::full(drawn.center, drawn.center + drawn.major, drawn.minor_radius);
    if let Some(point) = sketch.point(other) {
        return Some((sketch.closest_on_ellipse(ellipse, point)?, point));
    }
    if let Some(direction) = sketch.line_direction(other) {
        let normal = direction.try_normalize()?.perp();
        let anchor = sketch
            .line_endpoints(other)
            .map_or(Point2::ZERO, |(start, _)| start);
        let toward = if normal.dot(shape.center - anchor) < 0.0 {
            normal
        } else {
            -normal
        };
        let (unit, across) = (shape.axis(), shape.axis().perp());
        let (a, b) = (shape.major_radius(), shape.minor_radius);
        let (along, sideways) = (toward.dot(unit), toward.dot(across));
        let reach = (a * a * along * along + b * b * sideways * sideways).sqrt();
        let extreme = if reach > 0.0 {
            shape.center + (unit * (a * a * along) + across * (b * b * sideways)) / reach
        } else {
            shape.center
        };
        let foot = extreme - normal * normal.dot(extreme - anchor);
        let beyond = toward.dot(foot - extreme) < 0.0;
        return Some(if beyond {
            (foot, foot)
        } else {
            (extreme, foot)
        });
    }
    let (center, radius) = sketch.circle(other)?;
    let at = shape.point_at(touching_parameter(&shape, center, radius));
    let outward = (at - center).try_normalize().unwrap_or(Vector2::X);
    Some((at, center + outward * radius))
}
