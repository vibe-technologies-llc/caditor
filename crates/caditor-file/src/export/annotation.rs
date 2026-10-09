use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};

use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{ArcGeometry, Constraint, EntityId, Reference, Sketch};

use super::{
    Construction,
    figure::{Anchor, Dimension, Measure, Shape, Text},
};

const DIMENSION_OFFSET: f64 = 2.5;
const EXTENSION_GAP: f64 = 0.5;
const EXTENSION_OVERSHOOT: f64 = 0.5;
const ARROW_LENGTH: f64 = 1.0;
const ARROW_HALF_ANGLE: f64 = PI / 12.0;
const TEXT_LIFT: f64 = 0.8;
const ANGLE_RADIUS: f64 = 4.0;
const LEADER_OVERSHOOT: f64 = 1.5;
const LEADER_TEXT_GAP: f64 = 0.5;
const CIRCLE_LEADER_ANGLE: f64 = FRAC_PI_4;
const DEGENERATE: f64 = 1e-9;
const LENGTH_DECIMALS: usize = 3;
const ANGLE_DECIMALS: usize = 2;
pub(super) const DIAMETER_SIGN: char = '⌀';
pub(super) const DEGREE_SIGN: char = '°';
const ARC_LENGTH_SIGN: char = '⌒';
const RADIUS_PREFIX: char = 'R';

pub(super) fn dimensions(
    sketch: &Sketch,
    construction: Construction,
    height: f64,
    centre: Point2,
) -> Vec<Dimension> {
    let style = Style { height, centre };
    sketch
        .constraints()
        .filter(|(_, constraint)| shown(sketch, constraint, construction))
        .filter_map(|(_, constraint)| style.dimension(sketch, constraint))
        .collect()
}

pub(super) fn number(value: f64, decimals: usize) -> String {
    let fixed = format!("{value:.decimals$}");
    let trimmed = if fixed.contains('.') {
        fixed.trim_end_matches('0').trim_end_matches('.')
    } else {
        fixed.as_str()
    };
    match trimmed {
        "-0" | "" => "0".to_owned(),
        trimmed => trimmed.to_owned(),
    }
}

fn length(value: f64) -> String {
    number(value, LENGTH_DECIMALS)
}

fn degrees(value: f64) -> String {
    format!("{}{DEGREE_SIGN}", number(value.abs(), ANGLE_DECIMALS))
}

fn shown(sketch: &Sketch, constraint: &Constraint, construction: Construction) -> bool {
    construction == Construction::OnLayer
        || !constraint
            .entities()
            .into_iter()
            .any(|entity| sketch.is_construction(entity))
}

struct Style {
    height: f64,
    centre: Point2,
}

impl Style {
    fn dimension(&self, sketch: &Sketch, constraint: &Constraint) -> Option<Dimension> {
        let value = sketch.measured(constraint)?;
        match *constraint {
            Constraint::Distance { from, to, .. } => {
                let (first, second) = witnesses(sketch, from, to, self.centre)?;
                let along = (second - first).try_normalize()?;
                self.linear(first, second, along, length(value))
            }
            Constraint::HorizontalDistance { from, to, .. } => self.linear(
                sketch.point(from)?,
                sketch.point(to)?,
                Vector2::X,
                length(value),
            ),
            Constraint::VerticalDistance { from, to, .. } => self.linear(
                sketch.point(from)?,
                sketch.point(to)?,
                Vector2::Y,
                length(value),
            ),
            Constraint::Angle {
                from, to, reversed, ..
            } => self.angle(sketch, (from, to), reversed, value),
            Constraint::Radius { entity, .. } => {
                let (center, radius, toward) = leader(sketch, entity)?;
                Some(self.radial(
                    center,
                    radius,
                    toward,
                    Measured::Radius,
                    format!("{RADIUS_PREFIX}{}", length(value)),
                ))
            }
            Constraint::Diameter { entity, .. } => {
                let (center, radius, toward) = leader(sketch, entity)?;
                Some(self.radial(
                    center,
                    radius,
                    toward,
                    Measured::Diameter,
                    format!("{DIAMETER_SIGN}{}", length(value)),
                ))
            }
            Constraint::ArcLength { arc, .. } => self.along_arc(
                sketch.arc(arc)?,
                format!("{ARC_LENGTH_SIGN}{}", length(value)),
            ),
            Constraint::Sweep { arc, .. } => self.along_arc(sketch.arc(arc)?, degrees(value)),
            _ => None,
        }
    }

    fn text(&self, at: Point2, angle: f64, content: String) -> Text {
        Text {
            at,
            height: self.height,
            angle,
            content,
            anchor: Anchor::Middle,
        }
    }

    fn arrow(&self, tip: Point2, pointing: Vector2) -> [Shape; 2] {
        let back = -pointing * ARROW_LENGTH * self.height;
        [-ARROW_HALF_ANGLE, ARROW_HALF_ANGLE]
            .map(|turn| Shape::Line(tip, tip + Vector2::from_angle(turn).rotate(back)))
    }

    fn linear(
        &self,
        first: Point2,
        second: Point2,
        along: Vector2,
        content: String,
    ) -> Option<Dimension> {
        let height = self.height;
        let normal = along.perp();
        let middle = (first + second) / 2.0;
        let normal = if (middle - self.centre).dot(normal) < 0.0 {
            -normal
        } else {
            normal
        };
        let level = first.dot(normal).max(second.dot(normal)) + DIMENSION_OFFSET * height;
        let foot = |point: Point2| point + normal * (level - point.dot(normal));
        let (start, end) = (foot(first), foot(second));
        let span = (end - start).try_normalize()?;
        if start.distance(end) < DEGENERATE {
            return None;
        }
        let mut marks: Vec<Shape> = [(first, start), (second, end)]
            .into_iter()
            .map(|(point, on_line)| {
                Shape::Line(
                    point + normal * EXTENSION_GAP * height,
                    on_line + normal * EXTENSION_OVERSHOOT * height,
                )
            })
            .collect();
        marks.push(Shape::Line(start, end));
        marks.extend(self.arrow(start, -span));
        marks.extend(self.arrow(end, span));
        let angle = span.y.atan2(span.x);
        Some(Dimension {
            measure: Measure::Linear {
                first,
                second,
                line: end,
                angle,
            },
            marks,
            text: self.text(
                (start + end) / 2.0 + normal * TEXT_LIFT * height,
                angle,
                content,
            ),
        })
    }

    fn radial(
        &self,
        center: Point2,
        radius: f64,
        toward: Vector2,
        measured: Measured,
        content: String,
    ) -> Dimension {
        let height = self.height;
        let on_curve = center + toward * radius;
        let reach = on_curve + toward * LEADER_OVERSHOOT * height;
        let mut text = self.text(reach, 0.0, content);
        text.at = reach + toward * (LEADER_TEXT_GAP * height + text.width() / 2.0);
        let far = center - toward * radius;
        let (measure, mut marks) = match measured {
            Measured::Radius => (
                Measure::Radius { center, on_curve },
                vec![Shape::Line(center, reach)],
            ),
            Measured::Diameter => {
                let mut marks = vec![Shape::Line(far, reach)];
                marks.extend(self.arrow(far, -toward));
                (
                    Measure::Diameter {
                        near: on_curve,
                        far,
                    },
                    marks,
                )
            }
        };
        marks.extend(self.arrow(on_curve, toward));
        Dimension {
            measure,
            marks,
            text,
        }
    }

    fn angle(
        &self,
        sketch: &Sketch,
        (from, to): (EntityId, EntityId),
        reversed: bool,
        value: f64,
    ) -> Option<Dimension> {
        let first = sketch.angle_direction(from, to)?.try_normalize()?;
        let first = if reversed { -first } else { first };
        let vertex = sketch
            .angle_vertex(from, to)
            .or_else(|| crossing(line_of(sketch, from)?, line_of(sketch, to)?))?;
        let sweep = value.to_radians();
        let start = first.y.atan2(first.x);
        let (start, sweep) = if sweep < 0.0 {
            (start + sweep, -sweep)
        } else {
            (start, sweep)
        };
        (sweep > DEGENERATE).then(|| {
            self.arc_dimension(
                vertex,
                ANGLE_RADIUS * self.height,
                (start, sweep),
                Vec::new(),
                degrees(value),
            )
        })
    }

    fn along_arc(&self, arc: ArcGeometry, content: String) -> Option<Dimension> {
        let height = self.height;
        if arc.sweep <= DEGENERATE {
            return None;
        }
        let radius = arc.radius + DIMENSION_OFFSET * height;
        let extensions = [arc.start_angle, arc.start_angle + arc.sweep]
            .into_iter()
            .map(|angle| {
                Shape::Line(
                    Shape::arc_point(arc.center, arc.radius + EXTENSION_GAP * height, angle),
                    Shape::arc_point(arc.center, radius + EXTENSION_OVERSHOOT * height, angle),
                )
            })
            .collect();
        Some(self.arc_dimension(
            arc.center,
            radius,
            (arc.start_angle, arc.sweep),
            extensions,
            content,
        ))
    }

    fn arc_dimension(
        &self,
        center: Point2,
        radius: f64,
        (start, sweep): (f64, f64),
        mut marks: Vec<Shape>,
        content: String,
    ) -> Dimension {
        let end = start + sweep;
        let middle = start + sweep / 2.0;
        marks.push(Shape::Arc {
            center,
            radius,
            start,
            end,
        });
        let first = Shape::arc_point(center, radius, start);
        let second = Shape::arc_point(center, radius, end);
        marks.extend(self.arrow(first, -Vector2::from_angle(start).perp()));
        marks.extend(self.arrow(second, Vector2::from_angle(end).perp()));
        Dimension {
            measure: Measure::Angular {
                vertex: center,
                first,
                second,
                arc: Shape::arc_point(center, radius, middle),
            },
            marks,
            text: self.text(
                Shape::arc_point(center, radius + TEXT_LIFT * self.height, middle),
                middle - FRAC_PI_2,
                content,
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Measured {
    Radius,
    Diameter,
}

fn leader(sketch: &Sketch, entity: EntityId) -> Option<(Point2, f64, Vector2)> {
    let (center, radius) = sketch.circle(entity)?;
    let angle = sketch
        .arc(entity)
        .map_or(CIRCLE_LEADER_ANGLE, |arc| arc.start_angle + arc.sweep / 2.0);
    Some((center, radius, Vector2::from_angle(angle)))
}

fn line_of(sketch: &Sketch, id: EntityId) -> Option<(Point2, Vector2)> {
    match id.reference() {
        Some(Reference::HorizontalAxis) => Some((Point2::ZERO, Vector2::X)),
        Some(Reference::VerticalAxis) => Some((Point2::ZERO, Vector2::Y)),
        Some(Reference::Origin) => None,
        None => {
            let (start, end) = sketch.line_endpoints(id)?;
            Some((start, (end - start).try_normalize()?))
        }
    }
}

fn crossing(
    (first_origin, first): (Point2, Vector2),
    (second_origin, second): (Point2, Vector2),
) -> Option<Point2> {
    let cross = first.perp_dot(second);
    if cross.abs() < DEGENERATE {
        return None;
    }
    Some(first_origin + first * (second_origin - first_origin).perp_dot(second) / cross)
}

fn foot_on_line((origin, direction): (Point2, Vector2), point: Point2) -> Point2 {
    origin + direction * (point - origin).dot(direction)
}

fn nearest_on(sketch: &Sketch, curve: EntityId, point: Point2) -> Option<Point2> {
    if let Some(line) = line_of(sketch, curve) {
        return Some(foot_on_line(line, point));
    }
    if sketch.spline(curve).is_some() {
        return sketch.closest_on_curve(curve, point);
    }
    let (center, radius) = sketch.circle(curve)?;
    let outward = (point - center).try_normalize().unwrap_or(Vector2::X);
    Some(center + outward * radius)
}

fn witnesses(
    sketch: &Sketch,
    from: EntityId,
    to: EntityId,
    centre: Point2,
) -> Option<(Point2, Point2)> {
    match (sketch.point(from), sketch.point(to)) {
        (Some(first), Some(second)) => Some((first, second)),
        (Some(point), None) => Some((point, nearest_on(sketch, to, point)?)),
        (None, Some(point)) => Some((nearest_on(sketch, from, point)?, point)),
        (None, None) => curve_witnesses(sketch, from, to, centre),
    }
}

fn curve_witnesses(
    sketch: &Sketch,
    from: EntityId,
    to: EntityId,
    centre: Point2,
) -> Option<(Point2, Point2)> {
    match (sketch.spline(from), sketch.spline(to)) {
        (Some(_), _) => return sketch.spline_gap(from, to),
        (None, Some(_)) => return sketch.spline_gap(to, from),
        (None, None) => {}
    }
    match (sketch.circle(from), sketch.circle(to)) {
        (None, None) => {
            let (anchor, other) = if to.is_reference() {
                (to, from)
            } else {
                (from, to)
            };
            let (start, end) = sketch.line_endpoints(other)?;
            let anchor = line_of(sketch, anchor)?;
            let reach = |point: Point2| (point - centre).dot(anchor.1).abs();
            let outer = if reach(end) > reach(start) {
                end
            } else {
                start
            };
            Some((outer, foot_on_line(anchor, outer)))
        }
        (Some(circle), None) => circle_to_line(circle, line_of(sketch, to)?),
        (None, Some(circle)) => circle_to_line(circle, line_of(sketch, from)?),
        (Some(first), Some(second)) => Some(between_circles(first, second)),
    }
}

fn circle_to_line(
    (center, radius): (Point2, f64),
    line: (Point2, Vector2),
) -> Option<(Point2, Point2)> {
    let foot = foot_on_line(line, center);
    let toward = (foot - center).try_normalize().unwrap_or(line.1.perp());
    Some((center + toward * radius, foot))
}

fn between_circles(first: (Point2, f64), second: (Point2, f64)) -> (Point2, Point2) {
    let ((center, radius), (inner, inner_radius)) = if first.1 >= second.1 {
        (first, second)
    } else {
        (second, first)
    };
    let outward = (inner - center).try_normalize().unwrap_or(Vector2::X);
    let near_side = if inner.distance(center) > radius + inner_radius {
        -inner_radius
    } else {
        inner_radius
    };
    (inner + outward * near_side, center + outward * radius)
}
