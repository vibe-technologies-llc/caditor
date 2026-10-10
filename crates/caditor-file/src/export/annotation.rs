use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};

use caditor_geometry::{Aabb2, Point2, Vector2};
use caditor_sketch::{
    ArcGeometry, Constraint, EntityId, Reference, Sketch,
    annotation::{self, Footprint, Measured, Obstacles, away_from},
};

use super::{
    Construction,
    figure::{Anchor, Dimension, Measure, Shape, Text},
};

const DIMENSION_OFFSET: f64 = 2.5;
const LANE_SPACING: f64 = 2.0;
const EXTENSION_GAP: f64 = 0.5;
const EXTENSION_OVERSHOOT: f64 = 0.5;
const ARROW_LENGTH: f64 = 1.0;
const ARROW_HALF_ANGLE: f64 = PI / 12.0;
const TEXT_LIFT: f64 = 0.8;
const ANGLE_RADIUS: f64 = 4.0;
const LEADER_OVERSHOOT: f64 = 1.5;
const LEADER_TEXT_GAP: f64 = 0.5;
const CIRCLE_LEADER_ANGLE: f64 = FRAC_PI_4;
const LABEL_CLEARANCE: f64 = 0.5;
const LABEL_CELL: f64 = 4.0;
const SLIDES: [f64; 5] = [0.0, 0.5, -0.5, 1.0, -1.0];
const LEADER_TURN: f64 = PI / 12.0;
const LEADER_TURNS: usize = 12;
const NEARBY_LANES: usize = 4;
const MAX_ESCAPE_LANES: usize = 1 << 16;
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
    bounds: Aabb2,
) -> Vec<Dimension> {
    let style = Style {
        height,
        centre: bounds.center(),
    };
    let shown: Vec<&Constraint> = sketch
        .constraints()
        .map(|(_, constraint)| constraint)
        .filter(|constraint| shown(sketch, constraint, construction))
        .collect();
    let measured: Vec<Option<Measured>> = shown
        .iter()
        .map(|constraint| lane_taking(sketch, constraint))
        .collect();
    let extent = bounds.min().abs().max(bounds.max().abs()).max_element();
    let lanes = annotation::lanes(&measured, Some(style.centre), extent);
    let mut labels = Obstacles::new(LABEL_CELL * height);
    shown
        .into_iter()
        .zip(lanes)
        .filter_map(|(constraint, lane)| {
            let dimension = style.placed(sketch, constraint, lane, &labels)?;
            labels.add(style.footprint(&dimension.text));
            Some(dimension)
        })
        .collect()
}

fn lane_taking(sketch: &Sketch, constraint: &Constraint) -> Option<Measured> {
    match constraint {
        Constraint::Distance { .. }
        | Constraint::HorizontalDistance { .. }
        | Constraint::VerticalDistance { .. } => annotation::measured(sketch, constraint),
        _ => None,
    }
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

#[derive(Debug, Clone, Copy, PartialEq)]
struct Shift {
    lane: usize,
    slide: f64,
    turn: f64,
}

impl Shift {
    fn out(lane: usize) -> Self {
        Self {
            lane,
            slide: 0.0,
            turn: 0.0,
        }
    }

    fn around(lane: usize) -> impl Iterator<Item = Self> {
        let slides = SLIDES.into_iter().map(move |slide| Self {
            lane,
            slide,
            turn: 0.0,
        });
        let turns = (1..=LEADER_TURNS).flat_map(move |step| {
            let turn = step as f64 * LEADER_TURN;
            [turn, -turn].map(|turn| Self {
                lane,
                slide: 0.0,
                turn,
            })
        });
        slides.chain(turns)
    }

    fn reach(&self) -> f64 {
        LANE_SPACING * self.lane as f64
    }
}

struct Style {
    height: f64,
    centre: Point2,
}

impl Style {
    fn placed(
        &self,
        sketch: &Sketch,
        constraint: &Constraint,
        lane: usize,
        labels: &Obstacles,
    ) -> Option<Dimension> {
        let first = self.dimension(sketch, constraint, Shift::out(lane))?;
        let free = |dimension: &Dimension| labels.overlap(&self.footprint(&dimension.text)) <= 0.0;
        if free(&first) {
            return Some(first);
        }
        let escape = self.lanes_to_clear(&first.text, labels);
        for extra in 0..escape {
            let found = if extra < NEARBY_LANES {
                Shift::around(lane + extra)
                    .filter_map(|shift| self.dimension(sketch, constraint, shift))
                    .find(free)
            } else {
                self.dimension(sketch, constraint, Shift::out(lane + extra))
                    .filter(free)
            };
            if found.is_some() {
                return found;
            }
        }
        self.dimension(sketch, constraint, Shift::out(lane + escape))
    }

    fn lanes_to_clear(&self, text: &Text, labels: &Obstacles) -> usize {
        let Some(taken) = labels.bounds() else {
            return 0;
        };
        let label = self.footprint(text);
        let apart = (label.center - taken.center).length();
        let reach = apart + taken.half.length() + label.half.length();
        let lanes = (reach / (LANE_SPACING * self.height)).ceil() + 1.0;
        if lanes.is_finite() {
            (lanes.max(0.0) as usize).min(MAX_ESCAPE_LANES)
        } else {
            0
        }
    }

    fn footprint(&self, text: &Text) -> Footprint {
        let corners = text.corners();
        let first = corners.first().copied().unwrap_or(text.at);
        let (low, high) = corners.iter().fold((first, first), |(low, high), corner| {
            (low.min(*corner), high.max(*corner))
        });
        Footprint {
            center: (low + high) / 2.0,
            half: (high - low) / 2.0 + Vector2::splat(LABEL_CLEARANCE * self.height / 2.0),
        }
    }

    fn dimension(
        &self,
        sketch: &Sketch,
        constraint: &Constraint,
        shift: Shift,
    ) -> Option<Dimension> {
        let value = sketch.measured(constraint)?;
        match *constraint {
            Constraint::Distance { from, to, .. } => {
                let (first, second) = witnesses(sketch, from, to, self.centre)?;
                let along = (second - first).try_normalize()?;
                self.linear((first, second), along, length(value), shift)
            }
            Constraint::HorizontalDistance { .. } | Constraint::VerticalDistance { .. } => {
                let (from, to, along) = sketch.axis_offset_ends(constraint)?;
                self.linear((from, to), along, length(value), shift)
            }
            Constraint::Angle {
                from, to, reversed, ..
            } => self.angle(sketch, (from, to), reversed, value, shift),
            Constraint::Radius { entity, .. } => {
                let (center, radius, toward) = leader(sketch, entity, shift)?;
                Some(self.radial(
                    (center, radius, toward),
                    Leader::Radius,
                    format!("{RADIUS_PREFIX}{}", length(value)),
                    shift,
                ))
            }
            Constraint::Diameter { entity, .. } => {
                let (center, radius, toward) = leader(sketch, entity, shift)?;
                Some(self.radial(
                    (center, radius, toward),
                    Leader::Diameter,
                    format!("{DIAMETER_SIGN}{}", length(value)),
                    shift,
                ))
            }
            Constraint::ArcLength { arc, .. } => self.along_arc(
                sketch.arc(arc)?,
                format!("{ARC_LENGTH_SIGN}{}", length(value)),
                shift,
            ),
            Constraint::Sweep { arc, .. } => {
                self.along_arc(sketch.arc(arc)?, degrees(value), shift)
            }
            Constraint::MajorRadius { ellipse, .. } | Constraint::MinorRadius { ellipse, .. } => {
                let shape = sketch.ellipse(ellipse)?;
                let toward = match constraint {
                    Constraint::MajorRadius { .. } => shape.axis(),
                    _ => shape.axis().perp(),
                };
                Some(self.radial(
                    (shape.center, value, toward),
                    Leader::Radius,
                    format!("{RADIUS_PREFIX}{}", length(value)),
                    shift,
                ))
            }
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
        (first, second): (Point2, Point2),
        along: Vector2,
        content: String,
        shift: Shift,
    ) -> Option<Dimension> {
        let height = self.height;
        let middle = (first + second) / 2.0;
        let normal = away_from(along.perp(), middle, Some(self.centre));
        let level =
            first.dot(normal).max(second.dot(normal)) + (DIMENSION_OFFSET + shift.reach()) * height;
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
        let mut text = self.text(
            (start + end) / 2.0 + normal * TEXT_LIFT * height,
            angle,
            content,
        );
        let room = ((start.distance(end) - text.width()) / 2.0 - LABEL_CLEARANCE * height).max(0.0);
        text.at += span * room * shift.slide;
        Some(Dimension {
            measure: Measure::Linear {
                first,
                second,
                line: end,
                angle,
            },
            marks,
            text,
        })
    }

    fn radial(
        &self,
        (center, radius, toward): (Point2, f64, Vector2),
        leader: Leader,
        content: String,
        shift: Shift,
    ) -> Dimension {
        let height = self.height;
        let on_curve = center + toward * radius;
        let reach = on_curve + toward * (LEADER_OVERSHOOT + shift.reach()) * height;
        let mut text = self.text(reach, 0.0, content);
        text.at = reach + toward * (LEADER_TEXT_GAP * height + text.width() / 2.0);
        let far = center - toward * radius;
        let (measure, mut marks) = match leader {
            Leader::Radius => (
                Measure::Radius { center, on_curve },
                vec![Shape::Line(center, reach)],
            ),
            Leader::Diameter => {
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
        shift: Shift,
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
                (ANGLE_RADIUS + shift.reach()) * self.height,
                (start, sweep),
                Vec::new(),
                degrees(value),
            )
        })
    }

    fn along_arc(&self, arc: ArcGeometry, content: String, shift: Shift) -> Option<Dimension> {
        let height = self.height;
        if arc.sweep <= DEGENERATE {
            return None;
        }
        let radius = arc.radius + (DIMENSION_OFFSET + shift.reach()) * height;
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
enum Leader {
    Radius,
    Diameter,
}

fn leader(sketch: &Sketch, entity: EntityId, shift: Shift) -> Option<(Point2, f64, Vector2)> {
    let (center, radius) = sketch.circle(entity)?;
    let angle = match sketch.arc(entity) {
        Some(arc) => {
            let middle = arc.start_angle + arc.sweep / 2.0;
            let half = arc.sweep.abs() / 2.0;
            middle + shift.turn.clamp(-half, half)
        }
        None => CIRCLE_LEADER_ANGLE + shift.turn,
    };
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
    if sketch.is_elliptic(curve) {
        return sketch.closest_on_ellipse(curve, point);
    }
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
    if sketch.is_elliptic(from) {
        return sketch.ellipse_gap(from, to);
    }
    if sketch.is_elliptic(to) {
        return sketch.ellipse_gap(to, from);
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
